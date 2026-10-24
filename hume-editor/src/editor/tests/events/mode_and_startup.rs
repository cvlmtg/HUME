//! `on-mode-change`, the hook cascade cap, and the startup drain and buffer events.

use super::*;

// ── OnModeChange: Insert → Normal ─────────────────────────────────────────────

/// `cmd_exit_insert` (Esc) must fire `OnModeChange` for the Insert→Normal
/// transition: `end_insert_session` truncates the `Insert` layer, and
/// `detect_mode_change`'s observation-point diff at the next `settle()` is
/// what turns that into the fired hook, not any write at the truncate site
/// itself.
///
/// The test installs an `on-mode-change` handler that logs a message, so a
/// new log entry shows the hook fired. Not a cursor motion
/// (as the sibling mouse-click test below uses): with `insert-select-on-exit`
/// on, the entry hook's own `move-right` would land inside the pinned typed
/// run and get folded into the Esc-time auto-select, then cancelled out by
/// the exit hook's own `move-right`, so `state()` would spuriously come back
/// unchanged. `handle_input` does not drain itself (that lives in
/// `Editor::run`'s loop, see `Editor::settle`'s doc). An explicit `settle()`
/// after dispatch is what fires the queued hook.
#[test]
fn exit_insert_via_esc_fires_on_mode_change() {
    use crate::testing::MockHost;
    use hume_scripting::ScriptingHost;
    use termina::event::Event as TerminalEvent;

    // Two-char buffer; cursor starts at col 0.
    let mut ed = editor_from("-[a]>b\n");

    let mut host = ScriptingHost::new(&Dirs::none());
    let mut mock = MockHost::new();
    host.eval_source(
        r#"(register-hook! 'on-mode-change (lambda (old new) (log! 'trace "mode-changed")))"#,
        &mut mock,
    )
    .unwrap();
    ed.scripting = Some(host);

    let mode_changed_count = |ed: &Editor| {
        ed.state
            .message_log
            .entries()
            .filter(|e| e.severity == Severity::Trace && e.text == "mode-changed")
            .count()
    };

    // Enter Insert via `i` via handle_input + settle(), draining the
    // Normal→Insert hook before we capture the before count.
    ed.handle_input(TerminalEvent::Key(key('i')));
    ed.settle();
    assert_eq!(ed.state.mode(), Mode::Insert, "must be in Insert after `i`");

    let before = mode_changed_count(&ed);

    // Exit via Esc, then settle() to drain the queued on-mode-change hook.
    ed.handle_input(TerminalEvent::Key(key_esc()));
    ed.settle();

    assert_eq!(ed.state.mode(), Mode::Normal, "must be Normal after Esc");
    assert_eq!(
        mode_changed_count(&ed),
        before + 1,
        "on-mode-change handler must have fired exactly once on Insert→Normal via Esc"
    );
}

/// A left mouse click while in Insert mode must fire `OnModeChange` exactly
/// once for the Insert→Normal transition. The click path calls
/// `end_insert_session`, which truncates the `Insert` layer on its own.
/// `detect_mode_change`'s diff only ever compares the final mode against
/// the last-observed baseline once per drain pass, so nothing here can
/// double-fire the hook regardless of how many mode-layer operations ran
/// synchronously before the first `settle()`. `handle_input` does not
/// drain itself: the `settle()` below is what
/// fires the queued hook. The click itself also repositions the cursor, so
/// the `state()` diff alone doesn't distinguish "hook fired" from "click
/// moved the cursor". The mode assertion just above it is what
/// proves the hook actually ran.
#[test]
fn mouse_click_in_insert_fires_on_mode_change() {
    use crate::testing::MockHost;
    use hume_scripting::ScriptingHost;
    use termina::event::Event as TerminalEvent;

    let mut ed = editor_from("-[a]>b\n");
    // The click below is hit-tested against pane rects: an 80x25 terminal
    // leaves the pane 80x24.
    ed.resize_terminal(80, 25);

    let mut host = ScriptingHost::new(&Dirs::none());
    let mut mock = MockHost::new();
    host.eval_source(
        r#"(register-hook! 'on-mode-change (lambda (old new) (call! "move-right" (focused-pane))))"#,
        &mut mock,
    )
    .unwrap();
    ed.scripting = Some(host);

    // Enter Insert mode.
    ed.handle_key(key('i'));
    assert_eq!(ed.state.mode(), Mode::Insert, "must be in Insert after `i`");

    let before = state(&ed);

    // Left-click at (col=1, row=0): lands in content, triggers exit-insert.
    let click = termina::event::MouseEvent {
        kind: termina::event::MouseEventKind::Down(termina::event::MouseButton::Left),
        column: 1,
        row: 0,
        modifiers: termina::event::Modifiers::NONE,
    };
    draw(&mut ed);
    ed.handle_input(TerminalEvent::Mouse(click));
    ed.settle();

    assert_eq!(ed.state.mode(), Mode::Normal, "must be Normal after click");
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
/// Without the `MAX_EVENT_DRAIN` cap in `settle`, this test would never
/// return.
#[test]
fn hook_feedback_loop_is_cut_off_by_drain_cap() {
    use crate::testing::MockHost;
    use hume_scripting::ScriptingHost;

    let mut ed = editor_from("-[a]>b\n");

    let mut host = ScriptingHost::new(&Dirs::none());
    let mut mock = MockHost::new();
    host.eval_source(
        r#"(register-hook! 'on-language-set
             (lambda (bid lang)
               (set-buffer-option! bid "language" (if (equal? lang "aaa") "bbb" "aaa"))))"#,
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

/// An *amplifying* handler feedback loop (one that enqueues more hooks than
/// it received) doubles the pending batch every pass (1, 2, 4, 8, …). A cap
/// on pass *count* lets total work explode geometrically (2^100 evals at the
/// old 100-pass limit, never finishing); the cap must instead bound total
/// hooks processed so this terminates quickly regardless of growth shape.
///
/// Each handler invocation sets the buffer's language to `"a"` then `"b"`;
/// since the two calls always differ, both are genuine changes and both
/// re-enqueue `OnLanguageSet`, independent of what the previous invocation
/// left behind.
#[test]
fn amplifying_hook_cascade_is_cut_off_by_drain_cap() {
    use crate::testing::MockHost;
    use hume_scripting::ScriptingHost;

    let mut ed = editor_from("-[a]>b\n");

    let mut host = ScriptingHost::new(&Dirs::none());
    let mut mock = MockHost::new();
    host.eval_source(
        r#"(register-hook! 'on-language-set
             (lambda (bid lang)
               (set-buffer-option! bid "language" "a")
               (set-buffer-option! bid "language" "b")))"#,
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
/// `init_scripting` + `open_extra_file`
/// enqueue `OnBufferOpen`/`OnLanguageSet` hooks before the terminal is even
/// initialized, and the *first* iteration of `Editor::run`'s loop is what
/// fires them, via its own `settle()` call: the same one that fires
/// everything else. This test pins the underlying property `settle()` relies
/// on: `queue_event` alone never fires a handler.
#[test]
fn queued_hooks_require_explicit_settle() {
    use crate::editor::event::EditorEvent;
    use crate::testing::MockHost;
    use hume_scripting::ScriptingHost;

    let mut ed = editor_from("-[a]>b\n");

    // Install a handler: OnBufferOpen → move-right (observable side effect).
    let mut host = ScriptingHost::new(&Dirs::none());
    let mut mock = MockHost::new();
    host.eval_source(
        r#"(register-hook! 'on-buffer-open (lambda (bid) (call! "move-right" (focused-pane))))"#,
        &mut mock,
    )
    .unwrap();
    ed.scripting = Some(host);

    // Simulate what open_extra_file / init_scripting do: enqueue the hook.
    let bid = ed.focused_buffer_id();
    ed.state
        .queue_event(EditorEvent::OnBufferOpen { buffer: bid });

    // Hook is enqueued but has not fired yet.
    assert!(
        !ed.state.config.pending_work.is_empty(),
        "pending_work must be queued after queue_event: settle() not called yet"
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
/// `detect_pending_languages` runs `detect_and_set_language` before it fires
/// `OnBufferOpen`, and `open_buffer_and_notify` does not push `OnBufferOpen`
/// at open time. Breaking either one flips `hook_order` to
/// `[OnBufferOpen, OnLanguageSet]`.
#[test]
fn on_buffer_open_queued_after_on_language_set() {
    use crate::editor::buffer::Buffer;
    use crate::testing::MockHost;
    use hume_scripting::ScriptingHost;

    let mut ed = editor_from("-[a]>b\n");
    let mut host = ScriptingHost::new(&Dirs::none());
    let mut mock = MockHost::new();
    host.eval_source(
        r#"(register-hook! 'on-language-set (lambda (bid lang) (call! "move-right" (focused-pane))))
           (register-hook! 'on-buffer-open (lambda (bid) (call! "move-right" (focused-pane))))"#,
        &mut mock,
    )
    .unwrap();
    ed.scripting = Some(host);
    ed.state
        .config
        .languages
        .register_identity("rust", &["rs"], &[], &[])
        .expect("a test identity registers");

    let mut doc = Buffer::scratch();
    doc.set_path(Some(std::path::PathBuf::from("/tmp/foo.rs")));
    ed.open_buffer(doc);
    ed.detect_pending_languages();

    // Inspect the queue before draining: settle() would empty it.
    let hook_order: Vec<&str> = ed
        .state
        .config
        .pending_work
        .iter()
        .filter_map(|w| match w {
            crate::editor::event::PendingWork::Event(e) => Some(e.name()),
            crate::editor::event::PendingWork::Call { .. } => None,
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
/// every other buffer, but it must still announce `on-buffer-open`, after
/// `on-language-set`, once `detect_pending_languages` runs.
///
/// Asserting the full two-element order (not just "did it fire") also
/// covers a double-fire: the post-init sweep (`scripting/init.rs`) that
/// re-detects every open buffer's language calls `detect_and_set_language`
/// a second time for this buffer, which would append a spurious third entry
/// if `set_buffer_language_impl`'s unchanged-value early return ever broke.
///
/// Without the `open_hook_pending`/`pending_language_detection` bookkeeping
/// in `Editor::open`, `hook_order` would be `["on-language-set"]` (or empty,
/// if language detection also finds nothing to change).
#[test]
fn startup_buffer_announces_on_buffer_open_after_on_language_set() {
    use crate::testing::MockHost;
    use hume_scripting::ScriptingHost;

    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("main.rs");
    std::fs::write(&file, "fn main() {}\n").unwrap();

    let mut ed = open_headless(Some(file));
    let bid = ed.focused_buffer_id();

    let mut host = ScriptingHost::new(&Dirs::none());
    let mut mock = MockHost::new();
    host.eval_source(
        r#"(register-hook! 'on-language-set (lambda (bid lang) (call! "move-right" (focused-pane))))
           (register-hook! 'on-buffer-open (lambda (bid) (call! "move-right" (focused-pane))))"#,
        &mut mock,
    )
    .unwrap();
    ed.scripting = Some(host);
    ed.state
        .config
        .languages
        .register_identity("rust", &["rs"], &[], &[])
        .expect("a test identity registers");

    // Stands in for the drain `init_scripting` runs once it attaches its
    // host, inspecting the queue before `settle()` drains it, same as the
    // sibling test above.
    ed.detect_pending_languages();

    let hook_order: Vec<&str> = ed
        .state
        .config
        .pending_work
        .iter()
        .filter_map(|w| match w {
            crate::editor::event::PendingWork::Event(e) => Some(e.name()),
            crate::editor::event::PendingWork::Call { .. } => None,
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
/// `OnBufferOpen` was already announced: the pairing invariant
/// `close_buffer_and_notify` documents and
/// `unix::scripting_effects::buffer_opened_and_closed_in_one_eval_fires_neither_hook`
/// already asserts for a buffer opened mid-eval. The startup buffer never
/// goes through `open_buffer_and_notify`, so `Editor::open` sets its
/// `open_hook_pending` itself. Left at `false`, closing the buffer would
/// fire an unpaired `on-buffer-close` though `on-buffer-open` never fired.
#[test]
fn startup_buffer_close_before_any_drain_fires_no_on_buffer_close() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("main.rs");
    std::fs::write(&file, "fn main() {}\n").unwrap();

    let mut ed = open_headless(Some(file));
    let bid = ed.focused_buffer_id();
    assert!(
        ed.state.buffers.get(bid).open_hook_pending,
        "sanity: the startup buffer's open hasn't been announced yet"
    );

    // Closed before any `detect_pending_languages` drain: no scripting host
    // is even attached yet, matching a real early-exit (e.g. `:q` before the
    // first frame).
    ed.close_buffer(bid)
        .expect("the buffer has no unsaved changes and runs no pipe");

    let queued_close = ed.state.config.pending_work.iter().any(|w| {
        matches!(w, crate::editor::event::PendingWork::Event(e) if e.name() == "on-buffer-close")
    });
    assert!(
        !queued_close,
        "on-buffer-close must not fire for a buffer whose on-buffer-open never did"
    );
}
