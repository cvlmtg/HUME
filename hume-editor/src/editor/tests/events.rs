use super::*;
use crate::editor::commands::open_pane;
use hume_grid::Rect;

// ── OnModeChange: Insert → Normal ─────────────────────────────────────────────

/// `cmd_exit_insert` (Esc) must fire `OnModeChange` for the Insert→Normal
/// transition.  Before the fix, `end_insert_session` wrote `state.mode`
/// directly, bypassing the funnel, so the hook never reached script handlers.
///
/// Verification: install an `on-mode-change` handler that calls `move-right`;
/// the cursor advances only if the hook fired. `handle_input` does not
/// drain itself (that lives in `Editor::run`'s loop, see
/// `Editor::settle`'s doc) — an explicit `settle()` after dispatch is what
/// fires the queued hook.
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
/// separate `set_mode(Normal)` after it would double-fire the hook.
/// `handle_input` does not drain itself — the `settle()` below is what
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
    ed.view.last_pane_area = Rect::new(0, 0, 80, 24);

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
            .any(|e| e.severity == Severity::Error
                && e.text.contains("event/callback cascade exceeded")),
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
            .any(|e| e.severity == Severity::Error
                && e.text.contains("event/callback cascade exceeded")),
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
/// `lib.rs::run()` has no separate startup drain call of its own:
/// `init_scripting` + `open_extra_files`
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
            crate::editor::event::PendingWork::Event(e) => Some(e.name()),
            crate::editor::event::PendingWork::Call(..) => None,
        })
        .collect();
    assert_eq!(
        hook_order,
        vec!["on-language-set", "on-buffer-open"],
        "on-language-set must be queued before on-buffer-open; got {hook_order:?}"
    );
}

// ── Startup buffer: OnBufferOpen ──────────────────────────────────────────────

/// The startup buffer (`Editor::open`'s `file_path` argument) predates the
/// scripting host, so it can't route through `open_buffer_and_notify` like
/// every other buffer — but it must still announce `on-buffer-open`, after
/// `on-language-set`, once `detect_pending_languages` runs.
///
/// Asserting the full two-element order (not just "did it fire") also
/// covers a double-fire: the post-init sweep (`scripting_setup.rs`) that
/// re-detects every open buffer's language calls `detect_and_set_language`
/// a second time for this buffer, which would append a spurious third entry
/// if `set_buffer_language_impl`'s unchanged-value early return ever broke.
///
/// Fail oracle: drop the `open_hook_pending`/`pending_language_detection`
/// bookkeeping `Editor::open` now does → `hook_order` is `["on-language-set"]`
/// (or empty, if language detection also finds nothing to change).
#[test]
fn startup_buffer_announces_on_buffer_open_after_on_language_set() {
    use crate::testing::MockHost;
    use hume_scripting::ScriptingHost;

    let dir = safe_tempdir();
    let file = dir.path().join("main.rs");
    std::fs::write(&file, "fn main() {}\n").unwrap();

    let mut ed = Editor::open(Some(file), std::sync::Arc::new(|| {})).unwrap();
    let bid = ed.focused_buffer_id();

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

    // Stands in for what `apply_script_effects`'s tail does during
    // `init_scripting` (`scripting_setup.rs:105`) — inspecting the queue
    // before `settle()` drains it, same as the sibling test above.
    ed.detect_pending_languages();

    let hook_order: Vec<&str> = ed
        .state
        .config
        .pending_work
        .iter()
        .filter_map(|w| match w {
            crate::editor::event::PendingWork::Event(e) => Some(e.name()),
            crate::editor::event::PendingWork::Call(..) => None,
        })
        .collect();
    assert_eq!(
        hook_order,
        vec!["on-language-set", "on-buffer-open"],
        "startup buffer {bid:?} must announce on-language-set then on-buffer-open \
         exactly once each; got {hook_order:?}"
    );
}

/// `OnBufferClose` must never fire for the startup buffer unless its
/// `OnBufferOpen` was already announced — the pairing invariant
/// `close_buffer_and_notify` documents and
/// `unix::scripting_effects::buffer_opened_and_closed_in_one_eval_fires_neither_hook`
/// already asserts for a buffer opened mid-eval. Before the fix, the startup
/// buffer's `open_hook_pending` defaulted to `false` (it never went through
/// `open_buffer_and_notify`), so closing it fired an unpaired
/// `on-buffer-close` despite `on-buffer-open` never having fired.
#[test]
fn startup_buffer_close_before_any_drain_fires_no_on_buffer_close() {
    let dir = safe_tempdir();
    let file = dir.path().join("main.rs");
    std::fs::write(&file, "fn main() {}\n").unwrap();

    let mut ed = Editor::open(Some(file), std::sync::Arc::new(|| {})).unwrap();
    let bid = ed.focused_buffer_id();
    assert!(
        ed.state.buffers.get(bid).open_hook_pending,
        "sanity: the startup buffer's open hasn't been announced yet"
    );

    // Closed before any `detect_pending_languages` drain — no scripting host
    // is even attached yet, matching a real early-exit (e.g. `:q` before the
    // first frame).
    ed.close_buffer(bid);

    let queued_close = ed.state.config.pending_work.iter().any(|w| {
        matches!(w, crate::editor::event::PendingWork::Event(e) if e.name() == "on-buffer-close")
    });
    assert!(
        !queued_close,
        "on-buffer-close must not fire for a buffer whose on-buffer-open never did"
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

// ── settle(), merged queue, loop restructure ──────────────────────────────────

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

/// **FIFO order preserved across item kinds.** A prompt confirm queues a
/// *call* synchronously, mid-dispatch (`finish_steel_prompt`'s
/// `queue_steel_call`); `queue_buffer_save` then queues an *event*
/// synchronously right after it, still before `settle()` starts; two
/// zero-delay timers each queue a further *call*, but only once `settle()`'s
/// own `drain_async_sources` runs. They must fire in exactly the order they
/// entered the merged queue — `Call, Event, Call, Call` — not "every call
/// before every event" or vice versa: either inversion would still put the
/// two timer calls after the leading call/event pair, so a naive by-kind
/// grouping (`["call-0","call-a","call-b","event"]` or
/// `["event","call-0","call-a","call-b"]`) reads differently from the
/// correct FIFO trace and is caught either way. Pins the merge's core
/// guarantee: one FIFO queue, drained front-to-back, not the
/// old two-queue, two-drain-site split.
///
/// Fail oracle: drain every queued `Call` before any `Event` (or vice
/// versa) instead of popping the merged queue in insertion order → the
/// trace log below no longer matches
/// `["call-0", "event", "call-a", "call-b"]`.
#[test]
fn fifo_order_preserved_across_call_and_event_items() {
    let tmp = safe_tempdir();
    let mut ed = editor_from("-[a]>b\n");
    let mut host = hume_scripting::ScriptingHost::new();
    eval_with_real_host(
        &mut ed,
        &mut host,
        r#"(register-hook! 'on-buffer-save (lambda (bid) (log! 'trace "event")))
           (define-command! "arm" "" (lambda ()
             (prompt! "x" (lambda (s) (log! 'trace "call-0")))))
           (define-command! "start" "" (lambda ()
             (after 0 (lambda () (log! 'trace "call-a")))
             (after 0 (lambda () (log! 'trace "call-b")))))"#,
        tmp.path(),
    );
    ed.scripting = Some(host);

    type_cmd(&mut ed, ":start");
    type_cmd(&mut ed, ":arm");
    assert_eq!(ed.state.mode(), hume_engine::types::EditorMode::Command);

    // Confirming the prompt queues its callback as a `Call` synchronously,
    // inside this very `feed_key` — before settle() runs at all.
    ed.feed_key(key_enter());

    // Queued synchronously too, right after the call above — both are at
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
/// queued; a following `settle()` call is what fires it. Pins the
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
/// drain (`break` on `should_quit` instead of `continue`) → a `:wq` that
/// also sets `should_quit` in the same dispatch would never fire its
/// `OnBufferSave` handler.
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
/// separately (see `Editor::step`'s doc for why dispatch and settle stay two
/// calls). Pins that split: a `step()`-queued event must not
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
    // never drains.
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

// ── OnBufferEnter / OnFocusGained ──────────────────────────────────────────────

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

/// The mixed case: a pass that changes **both**
/// `focused_pane_id` (a bare field write, like pane-focus cycling) *and*
/// `pane.buffer_id` (like a buffer switch) before any `settle()` runs must
/// still coalesce into a single `OnBufferEnter` for wherever focus ends up
/// — `focused_buffer_id()` is one join evaluated once per pass, not two
/// independent things to diff separately.
///
/// Fail oracle: a raise site tied to either write individually (instead of
/// the derived-join diff at `settle()`'s single observation point) would
/// fire twice here — once for the pane move, once for the buffer switch.
#[test]
fn pane_focus_write_and_buffer_write_in_one_pass_coalesce_into_one_event() {
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

    let pid_a = ed.state.focused_pane_id;
    let bid = ed.focused_buffer_id();
    let pid_b = open_pane(&mut ed.state, &mut ed.view, bid);
    let buf2 = ed.open_buffer(Buffer::scratch());

    // Bare `focused_pane_id` write (no settle() in between)...
    ed.state.focused_pane_id = pid_b;
    // ...then a `pane.buffer_id` write on the pane that write just focused.
    ed.switch_to_buffer_with_jump(buf2);
    assert_ne!(pid_a, pid_b, "sanity: a genuinely different pane");

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
        "a pane-focus move and a buffer switch in the same pass must coalesce \
         into a single OnBufferEnter"
    );
    assert_eq!(ed.state.focused_pane_id, pid_b);
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
/// pass inside it → only one "entered" fires,
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

/// `on-option-change` fires `(key value)` after `apply_global` — the single
/// write path `:set global`, `set-option!`, and `:theme` all funnel through
/// (`settings_ops.rs`) — succeeds. Exercised via `:set global` here;
/// `tests/unix/lsp_inlay_feature.rs`'s
/// `setting_off_via_set_command_clears_hints_through_the_plugin_hook` covers
/// the same raise reached through the real shipped plugin's own handler.
///
/// Fail oracle: raise site missing or misplaced (before the write, or on the
/// buffer-scoped `apply_buffer` path too), wired to the wrong Steel name, or
/// `steel_args` passing the wrong pair.
#[test]
fn on_option_change_fires_key_and_value_after_a_set_global() {
    use crate::testing::MockHost;
    use hume_scripting::ScriptingHost;

    let mut ed = editor_from("-[a]>b\n");
    let mut host = ScriptingHost::new();
    let mut mock = MockHost::new();
    host.eval_source(
        r#"(register-hook! 'on-option-change (lambda (key value)
             (log! 'trace (string-append key "=" value))))"#,
        &mut mock,
    )
    .unwrap();
    ed.scripting = Some(host);
    ed.settle();

    type_cmd(&mut ed, ":set global lsp.inlay-hints=true");
    ed.settle();

    assert!(
        ed.state
            .message_log
            .entries()
            .any(|e| e.severity == Severity::Trace && e.text == "lsp.inlay-hints=true"),
        "on-option-change must fire with the changed key and its new value \
         after a successful :set global"
    );
}

// ── OnTextChanged ────────────────────────────────────────────────────────────

/// Typing one character fires exactly one `on-text-changed`, naming the
/// edited buffer.
///
/// Fail oracle: swap `bid` for a stale/wrong id in `steel_args`, or drop the
/// raise entirely → either the count or the buffer-id check below fails.
#[test]
fn typing_one_character_fires_on_text_changed_once() {
    use crate::testing::MockHost;
    use hume_scripting::ScriptingHost;

    let mut ed = editor_from("-[a]>b\n");
    let mut host = ScriptingHost::new();
    let mut mock = MockHost::new();
    host.eval_source(
        r#"(register-hook! 'on-text-changed
             (lambda (bid)
               (log! 'trace (if (equal? bid (current-buffer)) "correct-bid" "wrong-bid"))))"#,
        &mut mock,
    )
    .unwrap();
    ed.scripting = Some(host);
    ed.settle(); // drain the startup on-buffer-open/on-buffer-enter

    ed.feed_key(key('i'));
    ed.feed_key(key('x'));
    ed.feed_key(key_esc());
    ed.settle();

    let traces: Vec<&str> = ed
        .state
        .message_log
        .entries()
        .filter(|e| e.severity == Severity::Trace)
        .map(|e| e.text.as_str())
        .collect();
    assert_eq!(
        traces,
        vec!["correct-bid"],
        "exactly one on-text-changed must fire, naming the edited buffer"
    );
}

/// Several mutations to the same buffer before a single `settle()` coalesce
/// into one `on-text-changed` — the contract `on-text-changed`'s doc states
/// and `BufferStore::take_text_changed` implements.
///
/// Fail oracle: raise from a per-mutation write site instead of the
/// `text_gen` diff → three fires instead of one.
#[test]
fn several_edits_before_one_settle_coalesce_into_one_event() {
    use crate::testing::MockHost;
    use hume_scripting::ScriptingHost;

    let mut ed = editor_from("-[a]>b\n");
    let mut host = ScriptingHost::new();
    let mut mock = MockHost::new();
    host.eval_source(
        r#"(register-hook! 'on-text-changed (lambda (bid) (log! 'trace "changed")))"#,
        &mut mock,
    )
    .unwrap();
    ed.scripting = Some(host);
    ed.settle();

    // Three keystrokes, no settle() between them — `feed_key` only steps the
    // keymap, it never drains `pending_work` on its own.
    ed.feed_key(key('i'));
    ed.feed_key(key('a'));
    ed.feed_key(key('b'));
    ed.feed_key(key('c'));
    ed.feed_key(key_esc());
    ed.settle();

    let fires = ed
        .state
        .message_log
        .entries()
        .filter(|e| e.severity == Severity::Trace && e.text == "changed")
        .count();
    assert_eq!(
        fires, 1,
        "three mutations before one settle() must coalesce into one event"
    );
}

/// Undo fires `on-text-changed` (it bumps `text_gen` via `Buffer::undo`); a
/// second undo once history is back at its root does not, since nothing
/// mutated (`buffer/tests.rs`'s `text_gen_not_bumped_when_undo_at_root` pins
/// the same non-bump at the `Buffer` layer).
#[test]
fn undo_fires_but_a_no_op_undo_at_root_does_not() {
    use crate::testing::MockHost;
    use hume_scripting::ScriptingHost;

    let mut ed = editor_from("-[a]>b\n");
    let mut host = ScriptingHost::new();
    let mut mock = MockHost::new();
    host.eval_source(
        r#"(register-hook! 'on-text-changed (lambda (bid) (log! 'trace "changed")))"#,
        &mut mock,
    )
    .unwrap();
    ed.scripting = Some(host);
    ed.settle();

    let fire_count = |ed: &Editor| {
        ed.state
            .message_log
            .entries()
            .filter(|e| e.severity == Severity::Trace && e.text == "changed")
            .count()
    };

    // One real edit, drained, to give undo something to undo.
    ed.feed_key(key('i'));
    ed.feed_key(key('x'));
    ed.feed_key(key_esc());
    ed.settle();
    assert_eq!(fire_count(&ed), 1, "the edit itself must fire once");

    // `u` (undo) restores the pre-edit text — a real mutation, must fire.
    ed.feed_key(key('u'));
    ed.settle();
    assert_eq!(fire_count(&ed), 2, "undoing the edit must fire again");

    // History is now at its root — a second `u` is a no-op, must not fire.
    ed.feed_key(key('u'));
    ed.settle();
    assert_eq!(
        fire_count(&ed),
        2,
        "a no-op undo at the history root must not fire"
    );
}

/// `:e!` reload (`Editor::reload_buffer_in_place`) fires `on-text-changed` —
/// the case a raise site at `doc_ops::finish_edit` would have missed, since
/// reload never goes through `doc_ops` (see `BufferStore::edit_seq`'s doc).
#[test]
fn e_bang_reload_fires_on_text_changed() {
    use crate::testing::MockHost;
    use hume_scripting::ScriptingHost;

    let mut ed = editor_from("-[a]>b\n");
    let bid = ed.focused_buffer_id();
    let mut host = ScriptingHost::new();
    let mut mock = MockHost::new();
    host.eval_source(
        r#"(register-hook! 'on-text-changed (lambda (bid) (log! 'trace "changed")))"#,
        &mut mock,
    )
    .unwrap();
    ed.scripting = Some(host);
    ed.settle();

    let replacement = Buffer::new(BufferText::from("reloaded\n"), SelectionSet::default());
    ed.reload_buffer_in_place(bid, replacement);
    ed.settle();

    let fires = ed
        .state
        .message_log
        .entries()
        .filter(|e| e.severity == Severity::Trace && e.text == "changed")
        .count();
    assert_eq!(fires, 1, ":e! reload must fire on-text-changed");
}

/// An edit refused by the read-only guard (`doc_ops::apply_doc_edit`'s early
/// `return` before `cmd` ever runs) never bumps `text_gen`, so it must not
/// fire `on-text-changed`.
///
/// Fail oracle: remove the read-only guard, or move this raise upstream of
/// it → `insert_char` runs, `text_gen` bumps, and this fires.
#[test]
fn read_only_refused_edit_fires_no_on_text_changed() {
    use crate::editor::doc_ops;
    use crate::testing::MockHost;
    use hume_scripting::ScriptingHost;

    let mut ed = editor_from("-[a]>b\n");
    let bid = ed.focused_buffer_id();
    ed.state.buffers.get_mut(bid).read_only = true;

    let mut host = ScriptingHost::new();
    let mut mock = MockHost::new();
    host.eval_source(
        r#"(register-hook! 'on-text-changed (lambda (bid) (log! 'trace "changed")))"#,
        &mut mock,
    )
    .unwrap();
    ed.scripting = Some(host);
    ed.settle();

    let focused = ed.state.focused_pane_id;
    let before_gen = ed.state.buffers.get(bid).text_gen;
    doc_ops::apply_doc_edit(
        &mut ed.state.buffers,
        &ed.state.config.decorations,
        &mut ed.state.panes.state,
        focused,
        bid,
        |text, sels| hume_ops::edit::insert_char(text, sels, 'z'),
    );
    ed.settle();

    assert_eq!(
        ed.state.buffers.get(bid).text_gen,
        before_gen,
        "read-only guard must block the edit before it reaches set_text"
    );
    assert_eq!(
        ed.state
            .message_log
            .entries()
            .filter(|e| e.severity == Severity::Trace)
            .count(),
        0,
        "a read-only-refused edit must not fire on-text-changed"
    );
}

/// Opening a buffer fires `on-buffer-open`, not `on-text-changed` — a fresh
/// buffer's `text_gen` starts at 0 and `announced_text_gen` is seeded to
/// match, so there is no diff to observe.
#[test]
fn opening_a_buffer_fires_on_buffer_open_not_on_text_changed() {
    use crate::testing::MockHost;
    use hume_scripting::ScriptingHost;

    let tmp = safe_tempdir();
    let mut ed = editor_from("-[a]>b\n");
    let mut host = ScriptingHost::new();
    let mut mock = MockHost::new();
    host.eval_source(
        r#"(register-hook! 'on-buffer-open (lambda (bid) (log! 'trace "opened")))
           (register-hook! 'on-text-changed (lambda (bid) (log! 'trace "changed")))"#,
        &mut mock,
    )
    .unwrap();
    ed.scripting = Some(host);
    ed.settle();

    let path = tmp.path().join("fresh.txt");
    std::fs::write(&path, "hello\n").unwrap();
    type_cmd(&mut ed, &format!(":e {}", path.display()));
    ed.settle();

    let traces: Vec<&str> = ed
        .state
        .message_log
        .entries()
        .filter(|e| e.severity == Severity::Trace)
        .map(|e| e.text.as_str())
        .collect();
    assert_eq!(
        traces,
        vec!["opened"],
        "opening a buffer must fire on-buffer-open only, not on-text-changed"
    );
}

/// A handler that itself edits on `on-text-changed` is a feedback loop —
/// must be cut off by the same drain cap the other cascade tests exercise,
/// not livelock the editor. Detection runs *inside* `drain_pending_work`'s
/// fixpoint specifically so this cap can catch it (see the call site's doc).
///
/// The handler alternates `make-text-uppercase`/`make-text-lowercase` on the
/// selected letter — whichever direction the selection is in, at least one
/// of the two always changes the character (a lowercase letter capitalizes;
/// an uppercase one lowercases), so every invocation bumps `text_gen` and
/// re-triggers `on-text-changed`, guaranteeing the loop never runs dry on
/// its own.
///
/// Fail oracle: move `detect_text_changed` outside the fixpoint (mirroring
/// `drain_async_sources`) → this test never returns.
#[test]
fn text_changed_feedback_loop_is_cut_off_by_drain_cap() {
    // Selection starts on the letter `a` and never moves — `make-text-*`
    // transforms case in place — so the alternation below never runs dry.
    // `define-command!`'s effect (registering "kick") only takes hold once
    // applied, so this uses `eval_with_real_host` rather than
    // `eval_source`+`MockHost` — the latter never calls
    // `apply_script_effects`, so a defined command would never actually
    // reach `ed.state.config.commands`.
    let tmp = safe_tempdir();
    let mut ed = editor_from("-[a]>b\n");
    let mut host = hume_scripting::ScriptingHost::new();
    eval_with_real_host(
        &mut ed,
        &mut host,
        r#"(register-hook! 'on-text-changed
             (lambda (bid)
               (call! "make-text-uppercase")
               (call! "make-text-lowercase")))
           (define-command! "kick" "" (lambda () (call! "make-text-uppercase")))"#,
        tmp.path(),
    );
    ed.scripting = Some(host);
    ed.settle();

    type_cmd(&mut ed, ":kick");
    ed.settle(); // must return, not hang

    assert!(
        ed.state
            .message_log
            .entries()
            .any(|e| e.severity == Severity::Error
                && e.text.contains("event/callback cascade exceeded")),
        "drain cap must log an Error naming the hook cascade"
    );
    assert!(
        ed.state.config.pending_work.is_empty(),
        "pending hooks must be dropped when the cap fires"
    );
}

/// A buffer replaced in place via the last-buffer scratch swap
/// (`close_buffer`'s Case C, see `p6_close_last_buffer_becomes_scratch`) is a
/// content change under a surviving `BufferId` — `on-text-changed` must fire
/// once for it, not be silently swallowed by the swap resetting the
/// observation baseline back to a matching 0/0.
///
/// Fail oracle: drop the `text_gen`/`announced_text_gen` carry-forward in
/// `replace_buffer_in_place` → zero fires.
#[test]
fn last_buffer_close_fires_on_text_changed() {
    use crate::testing::MockHost;
    use hume_scripting::ScriptingHost;

    let mut ed = editor_from("-[a]>b\n");
    let bid = ed.focused_buffer_id();
    let mut host = ScriptingHost::new();
    let mut mock = MockHost::new();
    host.eval_source(
        r#"(register-hook! 'on-text-changed (lambda (bid) (log! 'trace "changed")))"#,
        &mut mock,
    )
    .unwrap();
    ed.scripting = Some(host);
    ed.settle();

    ed.close_buffer(bid);
    ed.settle();

    assert_eq!(
        ed.focused_buffer_id(),
        bid,
        "the scratch swap reuses the same buffer id"
    );
    let fires = ed
        .state
        .message_log
        .entries()
        .filter(|e| e.severity == Severity::Trace && e.text == "changed")
        .count();
    assert_eq!(
        fires, 1,
        "the last-buffer scratch swap must announce as one on-text-changed"
    );
}

/// Opening a read-only view (`:messages`/`:ls`) for the first time is silent
/// (fresh `Buffer`, baseline matches); refreshing an existing one via the
/// same label reuses the buffer and calls `set_view_content`, which must
/// fire `on-text-changed` — the documented trigger `store/tests.rs`'s
/// coverage never exercised end-to-end (it calls `set_view_content`
/// directly, bypassing `open_read_only_view`'s reuse path).
///
/// Fail oracle: change `open_read_only_view`'s reuse branch to close and
/// reopen the view buffer instead of calling `set_view_content` → the second
/// open also starts from a fresh baseline and this fires zero times.
#[test]
fn read_only_view_refresh_fires_on_text_changed() {
    use crate::testing::MockHost;
    use hume_scripting::ScriptingHost;

    let mut ed = editor_from("-[a]>b\n");
    let mut host = ScriptingHost::new();
    let mut mock = MockHost::new();
    host.eval_source(
        r#"(register-hook! 'on-text-changed (lambda (bid) (log! 'trace "changed")))"#,
        &mut mock,
    )
    .unwrap();
    ed.scripting = Some(host);
    ed.settle();

    let fire_count = |ed: &Editor| {
        ed.state
            .message_log
            .entries()
            .filter(|e| e.severity == Severity::Trace && e.text == "changed")
            .count()
    };

    ed.open_read_only_view("[test-view]", "one\n", 0);
    ed.settle();
    assert_eq!(
        fire_count(&ed),
        0,
        "the first open of a read-only view must not fire on-text-changed"
    );

    ed.open_read_only_view("[test-view]", "two\n", 0);
    ed.settle();
    assert_eq!(
        fire_count(&ed),
        1,
        "refreshing an existing read-only view must fire on-text-changed once"
    );
}

/// An identity edit (a command whose `ChangeSet` is the identity transform —
/// every op a `Retain`) must not bump `text_gen`: `Buffer::apply_edit` skips
/// `set_text` entirely for one, so it must not fire `on-text-changed` either.
/// Also asserts `doc_ops::finish_edit`'s matching guard: `edit_seq` (the
/// global paste-staleness counter, see `BufferStore::edit_seq`'s doc) must
/// not move either, or a no-op edit command would wrongly stale a pending
/// paste stamp.
///
/// Fail oracle: remove the `cs.is_identity()` guard in `Buffer::apply_edit`
/// → `text_gen` bumps and this fires once. Remove the matching guard in
/// `doc_ops::finish_edit` → `edit_seq` bumps even though `text_gen` didn't.
#[test]
fn identity_edit_fires_no_on_text_changed() {
    use crate::editor::doc_ops;
    use crate::testing::MockHost;
    use hume_editing::changeset::ChangeSet;
    use hume_scripting::ScriptingHost;

    let mut ed = editor_from("-[a]>b\n");
    let bid = ed.focused_buffer_id();
    let mut host = ScriptingHost::new();
    let mut mock = MockHost::new();
    host.eval_source(
        r#"(register-hook! 'on-text-changed (lambda (bid) (log! 'trace "changed")))"#,
        &mut mock,
    )
    .unwrap();
    ed.scripting = Some(host);
    ed.settle();

    let focused = ed.state.focused_pane_id;
    let before_gen = ed.state.buffers.get(bid).text_gen;
    let before_edit_seq = ed.state.buffers.edit_seq();
    doc_ops::apply_doc_edit(
        &mut ed.state.buffers,
        &ed.state.config.decorations,
        &mut ed.state.panes.state,
        focused,
        bid,
        |text, sels| {
            let len = text.len_chars();
            (text, sels, ChangeSet::identity(len))
        },
    );
    ed.settle();

    assert_eq!(
        ed.state.buffers.get(bid).text_gen,
        before_gen,
        "an identity edit must not bump text_gen"
    );
    assert_eq!(
        ed.state.buffers.edit_seq(),
        before_edit_seq,
        "an identity edit must not bump edit_seq"
    );
    assert_eq!(
        ed.state
            .message_log
            .entries()
            .filter(|e| e.severity == Severity::Trace)
            .count(),
        0,
        "an identity edit must not fire on-text-changed"
    );
}

/// An identity edit records no undo revision (`Buffer::apply_edit`'s guard
/// returns before `record_revision`): `u` right after one must undo the
/// *previous* real edit directly, not silently do nothing as if the
/// identity edit itself were on the undo stack.
///
/// Fail oracle: remove the guard (or move it after `record_revision`) → the
/// identity edit becomes an undo step of its own, and `u` reverts *it*
/// (a no-op, since it changed nothing) rather than the real edit beneath it.
#[test]
fn identity_edit_records_no_undo_revision() {
    use crate::editor::doc_ops;
    use hume_editing::changeset::ChangeSet;

    let mut ed = editor_from("-[a]>b\n");
    let bid = ed.focused_buffer_id();
    let original = ed.doc().text().to_string();

    // One real edit.
    ed.feed_key(key('i'));
    ed.feed_key(key('x'));
    ed.feed_key(key_esc());
    ed.settle();
    let after_real_edit = ed.doc().text().to_string();
    assert_ne!(
        after_real_edit, original,
        "the real edit must have changed the text"
    );

    // An identity edit: no-op, must not land on the undo stack.
    let focused = ed.state.focused_pane_id;
    doc_ops::apply_doc_edit(
        &mut ed.state.buffers,
        &ed.state.config.decorations,
        &mut ed.state.panes.state,
        focused,
        bid,
        |text, sels| {
            let len = text.len_chars();
            (text, sels, ChangeSet::identity(len))
        },
    );
    ed.settle();
    assert_eq!(
        ed.doc().text().to_string(),
        after_real_edit,
        "an identity edit must not itself change the text"
    );

    // `u` must undo the real edit directly, not a phantom identity revision.
    ed.feed_key(key('u'));
    ed.settle();
    assert_eq!(
        ed.doc().text().to_string(),
        original,
        "undo must skip the identity edit and revert straight to the pre-edit text"
    );
}

/// An insert session whose composed edits cancel out to the identity
/// transform (type a character, then backspace it, all inside one insert
/// session) must record no undo revision at all — `commit_edit_group`'s
/// identity guard. `text_gen` still moves during the session itself (each
/// keystroke is individually a real, non-identity `set_text`, so
/// `on-text-changed` correctly fires once for it, coalesced) — the guard's
/// job is narrower: making sure nothing lands on the undo stack for `u` to
/// later replay as a *second*, phantom mutation.
///
/// Fail oracle: remove `commit_edit_group`'s `cs.is_identity()` guard → the
/// no-op revision is recorded, `is_dirty()`/`can_undo()` both read `true`
/// right after `<Esc>`, and `u` fires a second, spurious `on-text-changed`
/// for a byte-identical buffer instead of being a silent no-op.
#[test]
fn insert_then_backspace_records_no_revision() {
    use crate::testing::MockHost;
    use hume_scripting::ScriptingHost;

    let mut ed = editor_from("-[a]>b\n");
    let bid = ed.focused_buffer_id();
    let original = ed.doc().text().to_string();

    let mut host = ScriptingHost::new();
    let mut mock = MockHost::new();
    host.eval_source(
        r#"(register-hook! 'on-text-changed (lambda (bid) (log! 'trace "changed")))"#,
        &mut mock,
    )
    .unwrap();
    ed.scripting = Some(host);
    ed.settle();

    let fire_count = |ed: &Editor| {
        ed.state
            .message_log
            .entries()
            .filter(|e| e.severity == Severity::Trace && e.text == "changed")
            .count()
    };

    // Type 'z', then backspace it, all within one insert session — the
    // composed ChangeSet cancels to identity (the same cancellation
    // `changeset/tests.rs`'s `compose_insert_then_delete` pins).
    ed.feed_key(key('i'));
    ed.feed_key(key('z'));
    ed.feed_key(key_backspace());
    ed.feed_key(key_esc());
    ed.settle();

    assert_eq!(
        ed.doc().text().to_string(),
        original,
        "the session must have made no net change"
    );
    assert!(
        !ed.state.buffers.get(bid).is_dirty(),
        "a session that cancelled out to identity must not read dirty"
    );
    assert!(
        !ed.state.buffers.get(bid).can_undo(),
        "a session that cancelled out to identity must record no undo revision"
    );
    assert_eq!(
        fire_count(&ed),
        1,
        "the session's own keystrokes are real intermediate mutations — must fire once, coalesced"
    );

    // `u` must be a silent no-op (nothing was ever recorded for it to undo),
    // not replay a phantom identity revision — and must therefore not fire a
    // second on-text-changed.
    ed.feed_key(key('u'));
    ed.settle();
    assert_eq!(
        ed.doc().text().to_string(),
        original,
        "undo must leave the text untouched — there is nothing to undo"
    );
    assert_eq!(
        fire_count(&ed),
        1,
        "a no-op undo (nothing was ever recorded) must not fire a second on-text-changed"
    );
}

/// A byte-identical `:e!` reload (`reload_from_text`'s `forward.is_identity()`
/// case) must not bump `text_gen`, so it must not fire `on-text-changed`.
///
/// Fail oracle: move the identity guard back below `set_text` (its original
/// position) → `text_gen` bumps before the guard returns, and this fires.
#[test]
fn identity_reload_fires_no_on_text_changed() {
    use crate::testing::MockHost;
    use hume_scripting::ScriptingHost;

    let mut ed = editor_from("-[a]>b\n");
    let bid = ed.focused_buffer_id();
    let text_before = ed.state.buffers.get(bid).text().clone();
    let mut host = ScriptingHost::new();
    let mut mock = MockHost::new();
    host.eval_source(
        r#"(register-hook! 'on-text-changed (lambda (bid) (log! 'trace "changed")))"#,
        &mut mock,
    )
    .unwrap();
    ed.scripting = Some(host);
    ed.settle();

    let before_gen = ed.state.buffers.get(bid).text_gen;
    let replacement = Buffer::new(text_before, SelectionSet::default());
    ed.reload_buffer_in_place(bid, replacement);
    ed.settle();

    assert_eq!(
        ed.state.buffers.get(bid).text_gen,
        before_gen,
        "a byte-identical reload must not bump text_gen"
    );
    assert_eq!(
        ed.state
            .message_log
            .entries()
            .filter(|e| e.severity == Severity::Trace)
            .count(),
        0,
        "a byte-identical reload must not fire on-text-changed"
    );
}

/// `OnTextChanged` is queued from the live-buffer sweep at the top of a
/// drain pass, but fires behind whatever `Call` items (timer thunks, async
/// callbacks) were already queued ahead of it in the same batch. A due timer
/// that closes the edited buffer before its own `on-text-changed` fires must
/// not hand a dead `BufferId` to any handler.
///
/// Fail oracle: remove `fire_one_event`'s liveness check → the handler
/// receives the closed buffer's dead id and its first buffer builtin on it
/// errors, which would surface as a `Severity::Error` the last assertion
/// below catches.
#[test]
fn on_text_changed_skips_a_buffer_closed_earlier_in_the_batch() {
    let tmp = safe_tempdir();
    let mut ed = editor_from("-[a]>b\n");
    let bid_b = ed.open_buffer(Buffer::new(
        BufferText::from("hello\n"),
        SelectionSet::default(),
    ));
    ed.switch_to_buffer_with_jump(bid_b);

    let mut host = hume_scripting::ScriptingHost::new();
    eval_with_real_host(
        &mut ed,
        &mut host,
        r#"(register-hook! 'on-text-changed (lambda (bid) (log! 'trace "changed")))
           (define-command! "start" ""
             (lambda () (after 0 (lambda () (close-buffer! (current-buffer))))))"#,
        tmp.path(),
    );
    ed.scripting = Some(host);
    ed.settle(); // drain startup on-buffer-open/on-buffer-enter

    // Real edit on B — bumps text_gen, not yet observed by a drain pass.
    ed.feed_key(key('i'));
    ed.feed_key(key('z'));
    ed.feed_key(key_esc());

    // Schedule the close, due immediately, and convert it to a queued Call
    // — but don't drain it yet.
    type_cmd(&mut ed, ":start");
    ed.drain_async_sources();
    assert!(
        ed.state.buffers.try_get(bid_b).is_some(),
        "scheduling the close must not run it yet"
    );

    // One settle: `detect_text_changed` queues B's event behind the
    // already-queued close Call, in the same `pending_work` batch.
    ed.settle();

    assert!(
        ed.state.buffers.try_get(bid_b).is_none(),
        "the scheduled close must have run"
    );
    assert_eq!(
        ed.state
            .message_log
            .entries()
            .filter(|e| e.severity == Severity::Trace)
            .count(),
        0,
        "on-text-changed must not fire for a buffer closed earlier in the batch"
    );
    assert!(
        ed.state
            .message_log
            .entries()
            .all(|e| e.severity != Severity::Error),
        "a dead buffer id must not surface as a hook error"
    );
}

/// **Exactly one `OnBufferEnter` per focus-changing action.** Pane-focus
/// cycling and a mouse click into another pane both move focus with no
/// write to `pane.buffer_id` at all — `focused_pane_id` is the only field
/// that changes. Counting fires (not just checking a confirm opened, which
/// a duplicate fire would still satisfy) pins that `settle()`'s diff raises
/// exactly one event per action, not once per write site it happens to
/// coalesce.
///
/// Fail oracle: a second raise site parallel to `detect_buffer_enter` (or
/// `detect_buffer_enter` re-firing on a pass where focus didn't actually
/// change again) would bump either count above 1.
#[test]
fn pane_focus_cycling_and_mouse_click_each_raise_exactly_one_on_buffer_enter() {
    use hume_scripting::ScriptingHost;

    let tmp = safe_tempdir();
    let mut ed = editor_from("-[h]>ello\n");
    type_cmd(&mut ed, ":vsplit");
    let path_b = tmp.path().join("b.txt");
    std::fs::write(&path_b, "world\n").unwrap();
    type_cmd(&mut ed, &format!(":e {}", path_b.display()));

    let mut host = ScriptingHost::new();
    eval_with_real_host(
        &mut ed,
        &mut host,
        r#"(register-hook! 'on-buffer-enter (lambda (bid) (log! 'trace "entered")))"#,
        tmp.path(),
    );
    ed.scripting = Some(host);

    // Settle the setup switches above, and establish pane geometry once —
    // `mouse_left_down` needs real pane rects, and rects don't depend on
    // which pane is focused, so one `prepare_frame` call covers both
    // actions below.
    let mut ctx = hume_engine::pipeline::RenderContext::new();
    ed.sync_viewport_dims(100, 25);
    ed.settle();
    ed.prepare_frame(&mut ctx);

    let count = |ed: &Editor| {
        ed.state
            .message_log
            .entries()
            .filter(|e| e.severity == Severity::Trace)
            .count()
    };

    // `:e` left the right pane (B) focused. Ctrl+p p, with only two panes,
    // cycles focus onto the left pane (A) — a bare `focused_pane_id` write,
    // never touching `buffer_id`.
    let before = count(&ed);
    ed.feed_event(key_ctrl('p'));
    ed.feed_event(key('p'));
    assert_eq!(
        count(&ed) - before,
        1,
        "pane-focus cycling must raise exactly one OnBufferEnter"
    );

    // Now A (left) is focused. Click into the right pane (B) — the same
    // bare `focused_pane_id` write, via `handle_input`'s mouse arm instead
    // of the keymap.
    let before = count(&ed);
    ed.handle_input(mouse_left_down(60, 0));
    ed.settle();
    assert_eq!(
        count(&ed) - before,
        1,
        "a mouse click into another pane must raise exactly one OnBufferEnter"
    );
}
