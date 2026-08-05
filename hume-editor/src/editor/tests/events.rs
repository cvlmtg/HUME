use super::*;
use crate::editor::commands::open_pane;

// ── OnModeChange: Insert → Normal ─────────────────────────────────────────────

/// `cmd_exit_insert` (Esc) must fire `OnModeChange` for the Insert→Normal
/// transition.  Before the fix, `end_insert_session` wrote `state.mode`
/// directly, bypassing the funnel, so the hook never reached script handlers.
///
/// Verification: install an `on-mode-change` handler that calls `move-right`;
/// the cursor advances only if the hook fired. Since C4, `handle_input` no
/// longer drains itself (that moved to `Editor::run`'s loop, see
/// `Editor::settle`'s doc) — an explicit `settle()` after dispatch is what
/// now fires the queued hook.
#[test]
fn exit_insert_via_esc_fires_on_mode_change() {
    use crate::testing::MockHost;
    use hume_scripting::ScriptingHost;
    use termina::event::Event as TerminalEvent;

    // Two-char buffer; cursor starts at col 0.
    let mut ed = editor_from("-[a]>b\n");

    let mut host = ScriptingHost::new();
    let mut mock = MockHost::new();
    host.eval_source(
        r#"(register-hook! 'on-mode-change (lambda (old new) (call! "move-right")))"#,
        &mut mock,
    )
    .unwrap();
    ed.scripting = Some(host);

    // Enter Insert via `i` via handle_input + settle(), draining the
    // Normal→Insert hook before we capture the before state.
    ed.handle_input(TerminalEvent::Key(key('i')));
    ed.settle();
    assert_eq!(ed.state.mode, Mode::Insert, "must be in Insert after `i`");

    let before = state(&ed);

    // Exit via Esc, then settle() to drain the queued on-mode-change hook.
    ed.handle_input(TerminalEvent::Key(key_esc()));
    ed.settle();

    assert_eq!(ed.state.mode, Mode::Normal, "must be Normal after Esc");
    assert_ne!(
        state(&ed),
        before,
        "on-mode-change handler (move-right) must have fired on Insert→Normal via Esc"
    );
}

/// A left mouse click while in Insert mode must fire `OnModeChange` exactly
/// once for the Insert→Normal transition. The click path calls
/// `end_insert_session`, which goes through the funnel on its own — a
/// separate `set_mode(Normal)` after it would double-fire the hook. Since
/// C4, `handle_input` no longer drains itself — the `settle()` below is what
/// fires the queued hook. The click itself also repositions the cursor, so
/// the `state()` diff alone doesn't distinguish "hook fired" from "click
/// moved the cursor" — the mode assertion just above it is the load-bearing
/// check for the hook actually having run at all.
#[test]
fn mouse_click_in_insert_fires_on_mode_change() {
    use crate::testing::MockHost;
    use hume_scripting::ScriptingHost;
    use termina::event::Event as TerminalEvent;

    let mut ed = editor_from("-[a]>b\n");
    ed.view.panes[ed.state.focused_pane_id].viewport =
        hume_engine::pane::ViewportState::new(80, 24);
    // The click below is hit-tested against pane rects, which only
    // `prepare_frame` normally populates — set it directly, matching the
    // viewport size above, since this test exercises hook dispatch, not a
    // full frame.
    ed.view.last_pane_area = ratatui::layout::Rect::new(0, 0, 80, 24);

    let mut host = ScriptingHost::new();
    let mut mock = MockHost::new();
    host.eval_source(
        r#"(register-hook! 'on-mode-change (lambda (old new) (call! "move-right")))"#,
        &mut mock,
    )
    .unwrap();
    ed.scripting = Some(host);

    // Enter Insert mode.
    ed.handle_key(key('i'));
    assert_eq!(ed.state.mode, Mode::Insert, "must be in Insert after `i`");

    let before = state(&ed);

    // Left-click at (col=1, row=0) — lands in content, triggers exit-insert.
    let click = termina::event::MouseEvent {
        kind: termina::event::MouseEventKind::Down(termina::event::MouseButton::Left),
        column: 1,
        row: 0,
        modifiers: termina::event::Modifiers::NONE,
    };
    ed.handle_input(TerminalEvent::Mouse(click));
    ed.settle();

    assert_eq!(ed.state.mode, Mode::Normal, "must be Normal after click");
    assert_ne!(
        state(&ed),
        before,
        "on-mode-change handler (move-right) must have fired on Insert→Normal via click"
    );
}

// ── Hook cascade cap ──────────────────────────────────────────────────────────

/// A handler feedback loop (an `on-language-set` handler that always flips the
/// language between two values) must be cut off by the drain cap instead of
/// livelocking the editor.  The watchdog only bounds each individual eval,
/// not the re-drain loop.
///
/// Fail oracle: remove the `MAX_EVENT_DRAIN` cap from `settle` →
/// this test never returns.
#[test]
fn hook_feedback_loop_is_cut_off_by_drain_cap() {
    use crate::testing::MockHost;
    use hume_scripting::ScriptingHost;

    let mut ed = editor_from("-[a]>b\n");

    let mut host = ScriptingHost::new();
    let mut mock = MockHost::new();
    host.eval_source(
        r#"(register-hook! 'on-language-set
             (lambda (bid lang)
               (set-buffer-language! bid (if (equal? lang "aaa") "bbb" "aaa"))))"#,
        &mut mock,
    )
    .unwrap();
    ed.scripting = Some(host);

    // Kick off the ping-pong: aaa → handler sets bbb → handler sets aaa → …
    let bid = ed.focused_buffer_id();
    let lang = ed.state.config.languages.intern("aaa");
    ed.set_buffer_language(bid, Some(lang));
    ed.settle(); // must return, not hang

    assert!(
        ed.state
            .message_log
            .entries()
            .any(|e| e.severity == Severity::Error && e.text.contains("hook cascade exceeded")),
        "drain cap must log an Error naming the hook cascade"
    );
    assert!(
        ed.state.config.pending_work.is_empty(),
        "pending hooks must be dropped when the cap fires"
    );
}

/// An *amplifying* handler feedback loop — one that enqueues more hooks than
/// it received — doubles the pending batch every pass (1, 2, 4, 8, …). A cap
/// on pass *count* lets total work explode geometrically (2^100 evals at the
/// old 100-pass limit, never finishing); the cap must instead bound total
/// hooks processed so this terminates quickly regardless of growth shape.
///
/// Each handler invocation sets the buffer's language to `"a"` then `"b"`;
/// since the two calls always differ, both are genuine changes and both
/// re-enqueue `OnLanguageSet` — independent of what the previous invocation
/// left behind.
///
/// Fail oracle: cap `settle` on pass count instead of total hooks
/// processed → this test times out instead of returning.
#[test]
fn amplifying_hook_cascade_is_cut_off_by_drain_cap() {
    use crate::testing::MockHost;
    use hume_scripting::ScriptingHost;

    let mut ed = editor_from("-[a]>b\n");

    let mut host = ScriptingHost::new();
    let mut mock = MockHost::new();
    host.eval_source(
        r#"(register-hook! 'on-language-set
             (lambda (bid lang)
               (set-buffer-language! bid "a")
               (set-buffer-language! bid "b")))"#,
        &mut mock,
    )
    .unwrap();
    ed.scripting = Some(host);

    let bid = ed.focused_buffer_id();
    let lang = ed.state.config.languages.intern("start");
    ed.set_buffer_language(bid, Some(lang));
    ed.settle(); // must return promptly, not after 2^100 evals

    assert!(
        ed.state
            .message_log
            .entries()
            .any(|e| e.severity == Severity::Error && e.text.contains("hook cascade exceeded")),
        "drain cap must log an Error naming the hook cascade"
    );
    assert!(
        ed.state.config.pending_work.is_empty(),
        "pending hooks must be dropped when the cap fires"
    );
}

// ── Startup hook drain ────────────────────────────────────────────────────────

/// `queue_event` only enqueues; hooks must be drained explicitly via
/// `settle()` or they silently defer.
///
/// `lib.rs::run()` no longer has its own separate startup drain call
/// (SPEC.md §3 removed it as redundant): `init_scripting` + `open_extra_files`
/// enqueue `OnBufferOpen`/`OnLanguageSet` hooks before the terminal is even
/// initialized, and the *first* iteration of `Editor::run`'s loop is what
/// fires them, via its own `settle()` call — the same one that fires
/// everything else. This test pins the underlying property `settle()` relies
/// on: `queue_event` alone never fires a handler.
///
/// Fail oracle: skip calling `settle()` after `queue_event` — the handler
/// never runs, and `pending_work` never empties.
#[test]
fn queued_hooks_require_explicit_settle() {
    use crate::editor::event::EditorEvent;
    use crate::testing::MockHost;
    use hume_scripting::ScriptingHost;

    let mut ed = editor_from("-[a]>b\n");

    // Install a handler: OnBufferOpen → move-right (observable side effect).
    let mut host = ScriptingHost::new();
    let mut mock = MockHost::new();
    host.eval_source(
        r#"(register-hook! 'on-buffer-open (lambda (bid) (call! "move-right")))"#,
        &mut mock,
    )
    .unwrap();
    ed.scripting = Some(host);

    // Simulate what open_extra_files / init_scripting do: enqueue the hook.
    let bid = ed.focused_buffer_id();
    ed.state
        .queue_event(EditorEvent::OnBufferOpen { buffer: bid });

    // Hook is enqueued but has not fired yet.
    assert!(
        !ed.state.config.pending_work.is_empty(),
        "pending_work must be queued after queue_event — settle() not called yet"
    );

    let before = state(&ed);

    // settle() fires the enqueued hooks.
    ed.settle();
    assert!(
        ed.state.config.pending_work.is_empty(),
        "pending_work must be empty after settle()"
    );
    assert_ne!(
        state(&ed),
        before,
        "OnBufferOpen handler must have run: move-right should have moved the cursor"
    );
}

// ── OnBufferOpen / OnLanguageSet ordering ─────────────────────────────────────

/// `Editor::open_buffer` must queue `OnLanguageSet` before `OnBufferOpen` for a
/// buffer whose language is detected at open time, so plugins that register
/// both handlers see language detection complete before `on-buffer-open` runs
/// (e.g. an `on-language-set` handler that installs per-language state an
/// `on-buffer-open` handler then reads).
///
/// Fail oracle: reorder `detect_pending_languages` back to firing
/// `OnBufferOpen` before `detect_and_set_language`, or revert
/// `open_buffer_and_notify` to push `OnBufferOpen` at open time — either way
/// `hook_order` flips to `[OnBufferOpen, OnLanguageSet]`.
#[test]
fn on_buffer_open_queued_after_on_language_set() {
    use crate::editor::buffer::Buffer;
    use crate::testing::MockHost;
    use hume_scripting::ScriptingHost;

    let mut ed = editor_from("-[a]>b\n");
    let mut host = ScriptingHost::new();
    let mut mock = MockHost::new();
    host.eval_source(
        r#"(register-hook! 'on-language-set (lambda (bid lang) (call! "move-right")))
           (register-hook! 'on-buffer-open (lambda (bid) (call! "move-right")))"#,
        &mut mock,
    )
    .unwrap();
    ed.scripting = Some(host);
    ed.state
        .config
        .languages
        .register_identity_no_rebuild("rust", &["rs"], &[], &[], None);
    ed.state
        .config
        .languages
        .rebuild_glob_set()
        .expect("rebuild ok");

    let mut doc = Buffer::scratch();
    doc.set_path(Some(std::path::PathBuf::from("/tmp/foo.rs")));
    ed.open_buffer(doc);

    // Inspect the queue before draining — settle() would empty it.
    let hook_order: Vec<&str> = ed
        .state
        .config
        .pending_work
        .iter()
        .filter_map(|w| match w {
            crate::editor::event::PendingWork::Event(e) => e.name(),
            crate::editor::event::PendingWork::Call(..) => None,
        })
        .collect();
    assert_eq!(
        hook_order,
        vec!["on-language-set", "on-buffer-open"],
        "on-language-set must be queued before on-buffer-open; got {hook_order:?}"
    );
}

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
    let mut host = ScriptingHost::new();
    let mut mock = MockHost::new();
    host.eval_source(
        r#"(register-hook! 'on-buffer-open (lambda (bid) (call! "move-right")))"#,
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
/// the non-focused pane's engine selections are updated immediately.
#[test]
fn propagate_cs_syncs_engine_pane_for_non_focused_pane() {
    let mut ed = editor_from("-[a]>b\n");
    let buf_id = ed.focused_buffer_id();

    // Create a second pane (not the focused one) viewing the same buffer.
    let second_pane = open_pane(&mut ed.state, &mut ed.view, buf_id);
    assert!(ed.view.panes.contains_key(second_pane));

    // Edit in the focused pane (insert 'x' → "xab\n").
    ed.handle_key(key('i'));
    ed.handle_key(key('x'));
    ed.handle_key(key_esc());

    // The non-focused pane's engine selections must have been synced by
    // propagate_cs_to_panes — not left empty or stale.
    let engine_pane = &ed.view.panes[second_pane];
    assert!(
        !engine_pane.selections.is_empty(),
        "non-focused pane engine selections must be synced after edit"
    );
}

// ── C4: settle(), merged queue, loop restructure (SPEC.md §3) ────────────────

/// **The stranded-events bug, executable.** An event raised from async
/// work — here, `queue_diagnostics_changed`, the same call `drain_lsp` makes
/// when a `publishDiagnostics` batch lands — must fire once `settle()` runs,
/// even with **no input dispatched at all**. Before the merge, the drain
/// only ran inside `handle_input`; `Ok(false) => continue` in `Editor::run`'s
/// poll skipped it entirely, so a diagnostics batch landing while the user
/// sat idle (or an `(after 0 …)` timer firing between keystrokes) never
/// reached its handler — not late, never.
///
/// Fail oracle: move the drain back into `handle_input` (equivalently: make
/// `settle()`'s merged fixpoint a no-op unless a keystroke just ran) → the
/// handler never fires, since this test dispatches nothing at all.
#[test]
fn event_raised_from_async_work_fires_on_settle_with_no_input() {
    use crate::testing::MockHost;
    use hume_scripting::ScriptingHost;

    let mut ed = editor_from("-[a]>b\n");
    let mut host = ScriptingHost::new();
    let mut mock = MockHost::new();
    host.eval_source(
        r#"(register-hook! 'on-diagnostics-changed (lambda (bid) (call! "move-right")))"#,
        &mut mock,
    )
    .unwrap();
    ed.scripting = Some(host);

    let before = state(&ed);
    let bid = ed.focused_buffer_id();

    // The async producer's raise call — mirrors what `drain_lsp` does when a
    // `publishDiagnostics` batch lands. No key, mouse, or paste event
    // anywhere in this test.
    ed.queue_diagnostics_changed(bid);
    ed.settle();

    assert_ne!(
        state(&ed),
        before,
        "on-diagnostics-changed handler must have run from settle() alone, with no input"
    );
}

/// **FIFO order preserved across item kinds.** `queue_buffer_save` queues an
/// *event* synchronously, mid-dispatch — before `settle()` even starts —
/// while two zero-delay timers each queue a *call*, but only once
/// `settle()`'s own `drain_async_sources` runs. They must fire in exactly
/// the order they entered the merged queue: the event first (already
/// queued before `settle()` began), then the two calls in arm order — not
/// "every call before every event" or vice versa. Pins the merge's core
/// guarantee (SPEC.md §3): one FIFO queue, drained front-to-back, not the
/// old two-queue, two-drain-site split.
///
/// Fail oracle: drain every queued `Event` before any `Call` (or vice
/// versa) instead of popping the merged queue in insertion order → the
/// trace log below no longer matches `["event", "call-a", "call-b"]`.
#[test]
fn fifo_order_preserved_across_call_and_event_items() {
    let tmp = safe_tempdir();
    let mut ed = editor_from("-[a]>b\n");
    let mut host = hume_scripting::ScriptingHost::new();
    eval_with_real_host(
        &mut ed,
        &mut host,
        r#"(register-hook! 'on-buffer-save (lambda (bid) (log! 'trace "event")))
           (define-command! "start" "" (lambda ()
             (after 0 (lambda () (log! 'trace "call-a")))
             (after 0 (lambda () (log! 'trace "call-b")))))"#,
        tmp.path(),
    );
    ed.scripting = Some(host);

    type_cmd(&mut ed, ":start");

    // Queued synchronously, right here — before settle() runs at all, so
    // this event is already at the front of pending_work by the time the
    // two timers convert to queued calls inside settle()'s own drain.
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
        vec!["event", "call-a", "call-b"],
        "merged queue must drain in exact insertion order"
    );
}

/// **Fixpoint within one `settle()` call.** A handler that itself queues
/// another event (`on-language-set`'s handler calling `set-buffer-language!`
/// exactly once, not repeatedly) must see that second event drained in the
/// *same* `settle()` call — not deferred to the next frame or keystroke.
/// Bounded counterpart to the cascade-cap tests below: exactly two fires,
/// not a runaway loop.
///
/// Fail oracle: revert `settle`'s inner loop to a single pass over one
/// snapshot (the pre-merge `drain_pending_steel_calls` shape) → the second,
/// handler-queued fire is left in `pending_work` after this `settle()` call
/// returns, and the buffer's language stays at the first-fire value.
#[test]
fn handler_queued_event_drains_within_the_same_settle_call() {
    use crate::testing::MockHost;
    use hume_scripting::ScriptingHost;

    let mut ed = editor_from("-[a]>b\n");
    let mut host = ScriptingHost::new();
    let mut mock = MockHost::new();
    host.eval_source(
        r#"(register-hook! 'on-language-set
             (lambda (bid lang)
               (log! 'trace (to-string "fired:" lang))
               (if (equal? lang "first") (set-buffer-language! bid "second") (begin))))"#,
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

/// **`prepare_frame` no longer drains.** Queuing an event and calling only
/// `sync_viewport_dims` + `prepare_frame` (skipping `settle()`) must leave it
/// queued; a following `settle()` call is what fires it. Pins §3's
/// separation of concerns: draining moved entirely out of the per-frame
/// render-prep path.
///
/// Fail oracle: reintroduce a drain call inside `prepare_frame` → the first
/// assertion below fails (the handler already ran before `settle()` was
/// ever called).
#[test]
fn prepare_frame_alone_does_not_drain_pending_work() {
    use crate::testing::MockHost;
    use hume_engine::pipeline::RenderContext;
    use hume_scripting::ScriptingHost;

    let mut ed = editor_from("-[a]>b\n");
    let mut host = ScriptingHost::new();
    let mut mock = MockHost::new();
    host.eval_source(
        r#"(register-hook! 'on-buffer-save (lambda (bid) (call! "move-right")))"#,
        &mut mock,
    )
    .unwrap();
    ed.scripting = Some(host);

    let before = state(&ed);
    let bid = ed.focused_buffer_id();
    ed.queue_buffer_save(bid);

    let mut ctx = RenderContext::new();
    ed.sync_viewport_dims(80, 25);
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

/// **`:wq` fires `OnBufferSave`.** Regression guard for the quit-path
/// restructure: `Editor::run`'s loop observes `should_quit` right after
/// `settle()`, and the post-dispatch check that used to `break` immediately
/// now `continue`s instead — specifically so a hook queued by the same
/// dispatch that set `should_quit` (`:wq`'s `OnBufferSave`) survives to be
/// drained by the loop's *next* iteration before it actually exits. `run`
/// itself needs a live terminal to drive (see its own doc), so this pins the
/// drain half of that guarantee directly: dispatch `:wq`, then `settle()`,
/// and confirm both the hook ran and `should_quit` is set.
///
/// Fail oracle: observe `should_quit` before the hook gets a chance to
/// drain (the pre-C4 shape) → a `:wq` that also sets `should_quit` in the
/// same dispatch would never fire its `OnBufferSave` handler.
#[test]
fn wq_fires_on_buffer_save_before_quitting() {
    use crate::testing::MockHost;
    use hume_scripting::ScriptingHost;

    let (mut ed, _tmp) = editor_with_file("-[a]>b\n", "a\n");
    let mut host = ScriptingHost::new();
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
/// settle — `hume_editor::run_keys`' loop calls `settle()` once per key,
/// separately (this branch's chosen split from SPEC.md §3's sketch; see
/// `Editor::step`'s doc). Pins that split: a `step()`-queued event must not
/// have fired yet, and only fires once `settle()` runs, mirroring exactly
/// what `run_keys` does after every `step()`.
///
/// Fail oracle: fold `settle()` into `step()` itself → the first assertion
/// (still pending right after `step`) fails.
#[test]
fn headless_step_then_settle_fires_a_queued_hook() {
    use crate::testing::MockHost;
    use hume_scripting::ScriptingHost;

    let mut ed = editor_from("-[a]>b\n");
    let mut host = ScriptingHost::new();
    let mut mock = MockHost::new();
    host.eval_source(
        r#"(register-hook! 'on-mode-change (lambda (old new) (call! "move-right")))"#,
        &mut mock,
    )
    .unwrap();
    ed.scripting = Some(host);

    let before = state(&ed);

    // Entering Insert queues OnModeChange; `step` dispatches the key but
    // never drains, before or after C4.
    ed.step(key('i'));
    assert_eq!(ed.state.mode, Mode::Insert, "sanity: `i` must enter Insert");
    assert!(
        !ed.state.config.pending_work.is_empty(),
        "step() must not drain — the OnModeChange hook must still be queued"
    );

    // Mirrors what run_keys' loop does after every step().
    ed.settle();
    assert_ne!(
        state(&ed),
        before,
        "settle() must fire the OnModeChange handler step() left queued"
    );
}

// ── OnBufferEnter / OnFocusGained (SPEC.md §4, C5) ────────────────────────────

/// `last_entered_buffer` starts `None`, so the very first `settle()` a fresh
/// `Editor` ever runs must observe a diff against it and fire
/// `on-buffer-enter` for the startup buffer — matching Vim's `BufEnter`
/// firing once on open.
///
/// Fail oracle: seed `last_entered_buffer` with the startup buffer instead
/// of `None` → the diff finds nothing new and the hook never fires.
#[test]
fn startup_buffer_fires_on_buffer_enter_on_the_first_settle() {
    use crate::testing::MockHost;
    use hume_scripting::ScriptingHost;

    let mut ed = editor_from("-[a]>b\n");
    let mut host = ScriptingHost::new();
    let mut mock = MockHost::new();
    host.eval_source(
        r#"(register-hook! 'on-buffer-enter (lambda (bid) (log! 'trace "entered")))"#,
        &mut mock,
    )
    .unwrap();
    ed.scripting = Some(host);

    assert!(
        ed.state.last_entered_buffer.is_none(),
        "sanity: no settle() has run yet"
    );

    ed.settle();

    assert!(
        ed.state
            .message_log
            .entries()
            .any(|e| e.severity == Severity::Trace && e.text == "entered"),
        "the startup buffer must fire on-buffer-enter on the very first settle()"
    );
    assert_eq!(ed.state.last_entered_buffer, Some(ed.focused_buffer_id()));
}

/// A `settle()` with no focus change since the last one must not raise a
/// fresh `OnBufferEnter` — an unconditional fire would mean a `stat` (via
/// its Rust reaction) and a Steel call on every idle frame.
///
/// Fail oracle: drop the `last_entered_buffer` comparison in
/// `Editor::detect_buffer_enter` → every `settle()` fires again.
#[test]
fn settle_with_no_focus_change_raises_no_further_on_buffer_enter() {
    use crate::testing::MockHost;
    use hume_scripting::ScriptingHost;

    let mut ed = editor_from("-[a]>b\n");
    let mut host = ScriptingHost::new();
    let mut mock = MockHost::new();
    host.eval_source(
        r#"(register-hook! 'on-buffer-enter (lambda (bid) (log! 'trace "entered")))"#,
        &mut mock,
    )
    .unwrap();
    ed.scripting = Some(host);

    ed.settle();
    let trace_count = |ed: &Editor| {
        ed.state
            .message_log
            .entries()
            .filter(|e| e.severity == Severity::Trace)
            .count()
    };
    assert_eq!(trace_count(&ed), 1, "sanity: the startup buffer fires once");

    ed.settle();
    ed.settle();
    assert_eq!(
        trace_count(&ed),
        1,
        "settle() calls with no focus change must not raise a fresh OnBufferEnter"
    );
}

/// Two switches queued back to back with no `settle()` between them (the
/// shape of a hook or async callback chaining a further switch mid-drain, or
/// several Steel effects landing in one batch) must coalesce into a single
/// `OnBufferEnter`, for the *final* buffer — not one per intermediate write.
/// The diff is taken against `last_entered_buffer`, not against every raw
/// write to `focused_pane_id`/`pane.buffer_id`.
///
/// Fail oracle: a raise site on the write itself (instead of a diff at
/// `settle()`'s observation point) would fire twice here.
#[test]
fn consecutive_switches_before_settle_coalesce_into_one_event_for_the_final_buffer() {
    use crate::editor::buffer::Buffer;
    use crate::testing::MockHost;
    use hume_scripting::ScriptingHost;

    let mut ed = editor_from("-[a]>b\n");
    let mut host = ScriptingHost::new();
    let mut mock = MockHost::new();
    host.eval_source(
        r#"(register-hook! 'on-buffer-enter (lambda (bid) (log! 'trace "entered")))"#,
        &mut mock,
    )
    .unwrap();
    ed.scripting = Some(host);

    ed.settle();
    let baseline = ed
        .state
        .message_log
        .entries()
        .filter(|e| e.severity == Severity::Trace)
        .count();
    assert_eq!(baseline, 1, "sanity: the startup buffer fires once");

    let buf1 = ed.open_buffer(Buffer::scratch());
    let buf2 = ed.open_buffer(Buffer::scratch());
    ed.switch_to_buffer_with_jump(buf1);
    ed.switch_to_buffer_with_jump(buf2);

    ed.settle();

    let total = ed
        .state
        .message_log
        .entries()
        .filter(|e| e.severity == Severity::Trace)
        .count();
    assert_eq!(
        total,
        baseline + 1,
        "two switches with no settle() between them must coalesce into a single \
         OnBufferEnter for the final buffer, not one per switch"
    );
    assert_eq!(ed.focused_buffer_id(), buf2);
}

/// A handler that itself calls `switch-to-buffer!` re-triggers the diff on
/// the *next pass of the same `settle()` call* — not a frame later. Needs
/// the real host (`switch-to-buffer!`/`open-buffer!` are gated to
/// `Command`/`PluginActivation` mode, unavailable to `MockHost`); both are
/// legal from inside a fired hook, which runs under `Command` mode
/// (`ScriptingHost::run_steel_calls`).
///
/// Fail oracle: take the diff once before the drain loop instead of once per
/// pass inside it (SPEC.md §4's C5 test matrix) → only one "entered" fires,
/// and the handler's own switch is picked up a `settle()` later.
#[test]
fn handler_driven_switch_produces_a_second_on_buffer_enter_in_the_same_settle_call() {
    let tmp = safe_tempdir();
    let other = tmp.path().join("other.txt");
    std::fs::write(&other, "b\n").unwrap();
    let other_path = other.to_string_lossy().replace('\\', "/");

    let mut ed = editor_from("-[a]>b\n");
    let bid_before = ed.focused_buffer_id();
    let mut host = hume_scripting::ScriptingHost::new();
    eval_with_real_host(
        &mut ed,
        &mut host,
        &format!(
            r#"(define switched #f)
               (register-hook! 'on-buffer-enter
                 (lambda (bid)
                   (log! 'trace "entered")
                   (when (not switched)
                     (set! switched #t)
                     (switch-to-buffer! (open-buffer! "{other_path}")))))"#
        ),
        tmp.path(),
    );
    ed.scripting = Some(host);

    ed.settle();

    let entered: Vec<&str> = ed
        .state
        .message_log
        .entries()
        .filter(|e| e.severity == Severity::Trace)
        .map(|e| e.text.as_str())
        .collect();
    assert_eq!(
        entered,
        vec!["entered", "entered"],
        "the handler's own switch-to-buffer! must produce a second OnBufferEnter \
         within the same settle() call, not one settle() later"
    );
    assert_ne!(
        ed.focused_buffer_id(),
        bid_before,
        "the handler-driven switch must have actually landed on the other buffer"
    );
}

/// `on-focus-gained` fires with no args, from `handle_input(FocusIn)` +
/// `settle()` — nothing routes through `OnBufferEnter`'s per-buffer
/// mechanism, since regaining terminal focus may be relevant to every open
/// buffer, not just the focused one.
///
/// Fail oracle: raise site missing, or wired to the wrong Steel name, or
/// `steel_args` returning a non-empty payload.
#[test]
fn on_focus_gained_fires_from_handle_input_and_settle_with_no_args() {
    use crate::testing::MockHost;
    use hume_scripting::ScriptingHost;
    use termina::event::Event as TerminalEvent;

    let mut ed = editor_from("-[a]>b\n");
    let mut host = ScriptingHost::new();
    let mut mock = MockHost::new();
    host.eval_source(
        r#"(register-hook! 'on-focus-gained (lambda () (log! 'trace "focus-gained")))"#,
        &mut mock,
    )
    .unwrap();
    ed.scripting = Some(host);
    // Drain the startup OnBufferEnter first so it can't be mistaken for the
    // event under test.
    ed.settle();

    ed.handle_input(TerminalEvent::FocusIn);
    ed.settle();

    assert!(
        ed.state
            .message_log
            .entries()
            .any(|e| e.severity == Severity::Trace && e.text == "focus-gained"),
        "on-focus-gained must fire after handle_input(FocusIn) + settle()"
    );
}
