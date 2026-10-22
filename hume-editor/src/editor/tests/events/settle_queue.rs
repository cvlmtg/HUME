//! `call!` from a hook, `settle()` and the pending-work queue.

use super::*;

// ── Hook (call! …) dispatch ───────────────────────────────────────────────────

/// `queue_event` must dispatch commands called by `(call! …)` inside hook bodies.
#[test]
fn hook_call_is_dispatched() {
    use crate::editor::event::EditorEvent;
    use crate::testing::MockHost;
    use hume_scripting::ScriptingHost;

    // Build a two-character buffer so move-right has room; cursor at col 0.
    let mut ed = editor_from("-[a]>b\n");
    // Wire up a scripting host with an on-buffer-open handler that calls move-right.
    let mut host = ScriptingHost::new(&Dirs::none());
    let mut mock = MockHost::new();
    host.eval_source(
        r#"(register-hook! 'on-buffer-open (lambda (bid) (call! "move-right" (focused-pane))))"#,
        &mut mock,
    )
    .unwrap();
    ed.scripting = Some(host);

    let before = state(&ed);
    let bid = ed.focused_buffer_id();
    ed.state
        .queue_event(EditorEvent::OnBufferOpen { buffer: bid });
    ed.settle();

    assert_ne!(
        state(&ed),
        before,
        "hook-queued move-right must move the cursor"
    );
}

/// Propagate an edit through two panes that view the same buffer and verify
/// the non-focused pane's engine mirror shows its translated selection once
/// the frame syncs the mirrors.
#[test]
fn propagate_cs_syncs_engine_pane_for_non_focused_pane() {
    let mut ed = editor_from("-[a]>b\n");
    let buf_id = ed.focused_buffer_id();

    // Create a second pane (not the focused one) viewing the same buffer.
    let second_pane = open_pane_in_layout(
        &mut ed.state,
        &mut ed.view,
        buf_id,
        hume_engine::pipeline::Direction::Horizontal,
    );
    assert!(ed.view.panes.contains_key(second_pane));

    // Edit in the focused pane (insert 'x' → "xab\n").
    ed.handle_key(key('i'));
    ed.handle_key(key('x'));
    ed.handle_key(key_esc());

    // The non-focused pane's cursor sat on 'a'; the insert before it moves
    // it one char right, and the frame sync writes that into its mirror.
    ed.sync_all_pane_mirrors(&ed.view.active_pane_ids());
    let engine_pane = &ed.view.panes[second_pane];
    assert_eq!(
        engine_pane
            .selections
            .as_ref()
            .expect("a written mirror")
            .iter()
            .map(|s| s.cursor.offset())
            .collect::<Vec<_>>(),
        vec![co(1)],
        "non-focused pane mirror must show the translated cursor"
    );
}

// ── settle() and the pending-work queue ───────────────────────────────────────

/// **Async events fire with no input.** An event raised from async
/// work (here, an `OnDiagnosticsChanged` event, as `drain_lsp` raises
/// when a `publishDiagnostics` batch lands) must fire once `settle()` runs,
/// even with **no input dispatched at all**. `Editor::run`'s poll loops back
/// on `Ok(false)` without dispatching anything, so this is the path a
/// diagnostics batch takes while the user sits idle (or an `(after! 0 …)`
/// timer firing between keystrokes).
///
/// A drain that lived in `handle_input`, or that only ran after a keystroke,
/// would never fire the handler here, since this test dispatches nothing.
#[test]
fn event_raised_from_async_work_fires_on_settle_with_no_input() {
    use crate::testing::MockHost;
    use hume_scripting::ScriptingHost;

    let mut ed = editor_from("-[a]>b\n");
    let mut host = ScriptingHost::new(&Dirs::none());
    let mut mock = MockHost::new();
    host.eval_source(
        r#"(register-hook! 'on-diagnostics-changed (lambda (bid) (call! "move-right" (focused-pane))))"#,
        &mut mock,
    )
    .unwrap();
    ed.scripting = Some(host);

    let before = state(&ed);
    let bid = ed.focused_buffer_id();

    // The async producer's raise call, mirroring what `drain_lsp` does when a
    // `publishDiagnostics` batch lands. No key, mouse, or paste event
    // anywhere in this test.
    ed.state
        .queue_event(crate::editor::event::EditorEvent::OnDiagnosticsChanged { buffer: bid });
    ed.settle();

    assert_ne!(
        state(&ed),
        before,
        "on-diagnostics-changed handler must have run from settle() alone, with no input"
    );
}

/// **FIFO order preserved across item kinds.** A prompt confirm queues a
/// *call* synchronously, mid-dispatch (`finish_steel_prompt`'s
/// `queue_steel_call`); `queue_buffer_save` then queues an *event*
/// synchronously right after it, still before `settle()` starts; two
/// zero-delay timers each queue a further *call*, but only once `settle()`'s
/// own `drain_async_sources` runs. They must fire in exactly the order they
/// entered the merged queue (`Call, Event, Call, Call`), not "every call
/// before every event" or vice versa: either inversion would still put the
/// two timer calls after the leading call/event pair, so a naive by-kind
/// grouping (`["call-0","call-a","call-b","event"]` or
/// `["event","call-0","call-a","call-b"]`) reads differently from the
/// correct FIFO trace and is caught either way. Pins the queue's core
/// guarantee: one FIFO queue, drained front-to-back.
#[test]
fn fifo_order_preserved_across_call_and_event_items() {
    let tmp = tempfile::tempdir().unwrap();
    let mut ed = editor_from("-[a]>b\n");
    let mut host = hume_scripting::ScriptingHost::new(&Dirs::none());
    eval_with_real_host(
        &mut ed,
        &mut host,
        r#"(register-hook! 'on-buffer-save (lambda (bid) (log! 'trace "event")))
           (define-typed-command! "arm" "" (lambda (pane)
             (prompt! pane "x" (lambda (s) (log! 'trace "call-0")))))
           (define-typed-command! "start" "" (lambda ()
             (after! 0 (lambda () (log! 'trace "call-a")))
             (after! 0 (lambda () (log! 'trace "call-b")))))"#,
        tmp.path(),
    );
    ed.scripting = Some(host);

    type_cmd(&mut ed, ":start");
    type_cmd(&mut ed, ":arm");
    assert_eq!(ed.state.mode(), hume_engine::types::EditorMode::Command);

    // Confirming the prompt queues its callback as a `Call` synchronously,
    // inside this very `feed_key`, before settle() runs at all.
    ed.feed_key(key_enter());

    // Queued synchronously too, right after the call above. Both are at
    // the front of pending_work by the time the two timers convert to
    // queued calls inside settle()'s own drain.
    let bid = ed.focused_buffer_id();
    ed.queue_buffer_save(bid);

    ed.settle();

    let order: Vec<&str> = ed
        .state
        .message_log
        .entries()
        .filter(|e| e.severity == Severity::Trace)
        .map(|e| e.text.as_str())
        .collect();
    assert_eq!(
        order,
        vec!["call-0", "event", "call-a", "call-b"],
        "merged queue must drain in exact insertion order"
    );
}

/// **Fixpoint within one `settle()` call.** A handler that itself queues
/// another event (`on-language-set`'s handler calling `set-buffer-option!`'s `"language"`
/// exactly once, not repeatedly) must see that second event drained in the
/// *same* `settle()` call, not deferred to the next frame or keystroke.
/// Bounded counterpart to the cascade-cap tests below: exactly two fires,
/// not a runaway loop.
///
/// If `settle`'s inner loop made a single pass over one snapshot, the
/// second, handler-queued fire would still be in `pending_work` when
/// `settle()` returns, and the buffer's language would stay at the
/// first-fire value.
#[test]
fn handler_queued_event_drains_within_the_same_settle_call() {
    use crate::testing::MockHost;
    use hume_scripting::ScriptingHost;

    let mut ed = editor_from("-[a]>b\n");
    let mut host = ScriptingHost::new(&Dirs::none());
    let mut mock = MockHost::new();
    host.eval_source(
        r#"(register-hook! 'on-language-set
             (lambda (bid lang)
               (log! 'trace (to-string "fired:" lang))
               (if (equal? lang "first") (set-buffer-option! bid "language" "second") (begin))))"#,
        &mut mock,
    )
    .unwrap();
    ed.scripting = Some(host);

    let bid = ed.focused_buffer_id();
    let lang = ed.state.config.languages.intern("first");
    ed.set_buffer_language(bid, Some(lang));
    ed.settle();

    let fires: Vec<&str> = ed
        .state
        .message_log
        .entries()
        .filter(|e| e.severity == Severity::Trace)
        .map(|e| e.text.as_str())
        .collect();
    assert_eq!(
        fires,
        vec!["fired: first", "fired: second"],
        "the handler-queued second OnLanguageSet must drain within the same settle() call"
    );
    assert_eq!(
        ed.state.buffers.get(bid).language,
        ed.state.config.languages.id_of("second"),
        "final language must reflect the handler-queued transition"
    );
}

/// **`prepare_frame` does not drain.** Queuing an event and calling only
/// `resize_terminal` + `prepare_frame` (skipping `settle()`) must leave it
/// queued; a following `settle()` call is what fires it. Pins the
/// separation of concerns: draining stays out of the per-frame render-prep
/// path.
#[test]
fn prepare_frame_alone_does_not_drain_pending_work() {
    use crate::testing::MockHost;
    use hume_engine::pipeline::RenderContext;
    use hume_scripting::ScriptingHost;

    let mut ed = editor_from("-[a]>b\n");
    let mut host = ScriptingHost::new(&Dirs::none());
    let mut mock = MockHost::new();
    host.eval_source(
        r#"(register-hook! 'on-buffer-save (lambda (bid) (call! "move-right" (focused-pane))))"#,
        &mut mock,
    )
    .unwrap();
    ed.scripting = Some(host);

    let before = state(&ed);
    let bid = ed.focused_buffer_id();
    ed.queue_buffer_save(bid);

    let mut ctx = RenderContext::new();
    ed.resize_terminal(80, 25);
    ed.prepare_frame(&mut ctx);

    assert_eq!(
        state(&ed),
        before,
        "prepare_frame alone must not drain pending_work"
    );
    assert!(
        !ed.state.config.pending_work.is_empty(),
        "the queued hook must still be pending after prepare_frame alone"
    );

    ed.settle();
    assert_ne!(
        state(&ed),
        before,
        "settle() must fire the hook prepare_frame left untouched"
    );
}

/// **Paneless case**: `on-buffer-save`'s own
/// payload is a *buffer*-level handle (see `EditorEvent::into_steel_args`'s
/// doc): it never carries a pane, even for a
/// buffer that happens to be shown somewhere, so a hook that wants to reach
/// a `Pane`-category native command must resolve one explicitly via
/// `(buffer-panes bid)` rather than handing the hook's own `bid` straight to
/// `call!`. B here is open but shown in no pane at all, so `(buffer-panes
/// bid)` is empty and the hook's own `error` fires. `delete` never runs on
/// the wrong buffer.
///
/// Two buffers, one `on-buffer-save` hook queued per buffer (mirroring what
/// two `:wa` writes would queue), both drained in the same `settle()` batch.
///
/// Falling back to the focused pane on an empty `(buffer-panes bid)` would
/// make both hook runs delete from A (whichever buffer is focused), leaving
/// B untouched and no error in the message log.
#[test]
fn on_buffer_save_native_call_on_a_paneless_bid_errors() {
    use hume_editing::text::BufferText;

    let mut ed = Editor::for_testing(Buffer::at_start(BufferText::from("aaa\n")));
    let bid_a = ed.focused_buffer_id();
    let bid_b = ed.open_buffer(Buffer::at_start(BufferText::from("bbb\n")));
    // `open_buffer` does not move focus, nor show the buffer in any pane:
    // A stays focused, B is paneless.
    assert_eq!(ed.focused_buffer_id(), bid_a, "setup: A stays focused");

    let tmp = tempfile::tempdir().unwrap();
    run(
        &mut ed,
        tmp.path(),
        r#"(register-hook! 'on-buffer-save (lambda (bid)
             (let ((panes (buffer-panes bid)))
               (if (null? panes)
                   (error "not shown in any pane")
                   (call! "delete" (car panes))))))"#,
    );

    // Queued in buffer order, same as two sequential :wa writes would.
    ed.queue_buffer_save(bid_a);
    ed.queue_buffer_save(bid_b);
    ed.settle();

    assert_eq!(
        ed.state.buffers.get(bid_a).text().to_string(),
        "aa\n",
        "A (focused, and its own bid) must have its own hook's delete applied"
    );
    assert_eq!(
        ed.state.buffers.get(bid_b).text().to_string(),
        "bbb\n",
        "B (paneless) must be untouched: its hook run errored instead \
         of silently deleting from A"
    );
    let log = ed.state.message_log.format_for_display();
    assert!(
        log.contains("not shown in any pane"),
        "B's hook run must report it has no pane to act through: {log:?}"
    );
}

/// **Remote-pane case**: the mirror of the
/// paneless test above: B is shown in a non-focused split, so
/// `(buffer-panes bid)` names its own pane and the resulting `call!`
/// succeeds, editing B while leaving A (and focus) untouched. A native
/// `call!` requires only that some pane shows its target buffer, and the
/// hook names that pane itself.
#[test]
fn on_buffer_save_native_call_on_a_split_bid_edits_that_pane() {
    use crate::editor::commands::open_pane_in_layout;
    use hume_editing::text::BufferText;
    use hume_engine::pipeline::Direction;

    let mut ed = Editor::for_testing(Buffer::at_start(BufferText::from("aaa\n")));
    let bid_a = ed.focused_buffer_id();
    let pid_a = ed.view.focused_pane();
    let bid_b = ed.open_buffer(Buffer::at_start(BufferText::from("bbb\n")));
    open_pane_in_layout(&mut ed.state, &mut ed.view, bid_b, Direction::Horizontal);
    // The split moves focus onto the new pane in production
    // (`split_pane_onto`/`:vsplit`), but `open_pane_in_layout` itself does
    // not. Restore focus to A explicitly so this test's premise (A stays
    // focused, B lives in a background split) holds regardless.
    ed.state.focus.set_for_test(&mut ed.view, pid_a);
    assert_eq!(ed.focused_buffer_id(), bid_a, "setup: A stays focused");

    let tmp = tempfile::tempdir().unwrap();
    run(
        &mut ed,
        tmp.path(),
        r#"(register-hook! 'on-buffer-save (lambda (bid)
             (let ((panes (buffer-panes bid)))
               (if (null? panes)
                   (error "not shown in any pane")
                   (call! "delete" (car panes))))))"#,
    );

    ed.queue_buffer_save(bid_a);
    ed.queue_buffer_save(bid_b);
    ed.settle();

    assert_eq!(
        ed.state.buffers.get(bid_a).text().to_string(),
        "aa\n",
        "A (focused) must have its own hook's delete applied"
    );
    assert_eq!(
        ed.state.buffers.get(bid_b).text().to_string(),
        "bb\n",
        "B (shown in a non-focused split) must also have its own hook's \
         delete applied: a native call! does not require focus"
    );
    assert_eq!(
        ed.view.focused_pane(),
        pid_a,
        "a remote dispatch on B must not move focus off A"
    );
    let log = ed.state.message_log.format_for_display();
    assert!(
        !log.contains("not shown in any pane") && !log.contains("focused"),
        "neither hook run should have errored: {log:?}"
    );
}

/// **`:wq` fires `OnBufferSave`.** `Editor::run`'s loop observes
/// `should_quit` right after `settle()`, and the post-dispatch check
/// `continue`s so a hook queued by the same
/// dispatch that set `should_quit` (`:wq`'s `OnBufferSave`) survives to be
/// drained by the loop's *next* iteration before it actually exits. `run`
/// itself needs a live terminal to drive (see its own doc), so this pins the
/// drain half of that guarantee directly: dispatch `:wq`, then `settle()`,
/// and confirm both the hook ran and `should_quit` is set.
#[test]
fn wq_fires_on_buffer_save_before_quitting() {
    use crate::testing::MockHost;
    use hume_scripting::ScriptingHost;

    let (mut ed, _tmp) = editor_with_file("-[a]>b\n", "a\n");
    let mut host = ScriptingHost::new(&Dirs::none());
    let mut mock = MockHost::new();
    host.eval_source(
        r#"(register-hook! 'on-buffer-save (lambda (bid) (log! 'trace "saved")))"#,
        &mut mock,
    )
    .unwrap();
    ed.scripting = Some(host);

    type_cmd(&mut ed, ":wq");
    assert!(ed.state.should_quit, "sanity: :wq must set should_quit");

    ed.settle();

    assert!(
        ed.state
            .message_log
            .entries()
            .any(|e| e.severity == Severity::Trace && e.text == "saved"),
        "on-buffer-save must fire for :wq, even though should_quit is already set"
    );
}

/// **Headless path.** `Editor::step` dispatches a key but does not itself
/// settle: `hume_editor::run_keys`' loop calls `settle()` once per key,
/// separately (see `Editor::step`'s doc for why dispatch and settle stay two
/// calls). Pins that split: `on-mode-change` must not have fired yet right
/// after `step()`, and only fires once `settle()` runs, mirroring exactly
/// what `run_keys` does after every `step()`. `OnModeChange` itself isn't
/// even queued until then: `detect_mode_change`'s diff runs inside
/// `drain_pending_work`'s loop, which only `settle()` calls, so there
/// is no "queued but undrained" state to observe in between the way there
/// is for `OnBufferEnter`, so the test checks the hook's own side effect.
#[test]
fn headless_step_then_settle_fires_a_queued_hook() {
    use crate::testing::MockHost;
    use hume_scripting::ScriptingHost;

    let mut ed = editor_from("-[a]>b\n");
    let mut host = ScriptingHost::new(&Dirs::none());
    let mut mock = MockHost::new();
    host.eval_source(
        r#"(register-hook! 'on-mode-change (lambda (old new) (call! "move-right" (focused-pane))))"#,
        &mut mock,
    )
    .unwrap();
    ed.scripting = Some(host);

    let before = state(&ed);

    // Entering Insert changes `mode()`, but `step` dispatches the key
    // without settling. Nothing observes the change until `settle()`'s
    // own `detect_mode_change` diff runs.
    ed.step(key('i'));
    assert_eq!(
        ed.state.mode(),
        Mode::Insert,
        "sanity: `i` must enter Insert"
    );
    assert_eq!(
        state(&ed),
        before,
        "step() must not settle: the on-mode-change hook must not have fired yet"
    );

    // Mirrors what run_keys' loop does after every step().
    ed.settle();
    assert_ne!(
        state(&ed),
        before,
        "settle() must fire the OnModeChange handler step() left queued"
    );
}
