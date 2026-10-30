//! Events for closed buffers and panes, raising handlers, and one buffer-enter per focus change.

use super::*;

/// **One `OnBufferEnter` per focus-changing action.** Pane-focus cycling
/// and a mouse click into another pane both move focus with no write to
/// `pane.buffer_id` at all: the focused pane is the only thing that changes.
/// The test counts fires rather than checking that a confirm opened, which a
/// duplicate fire also does: `settle()`'s diff raises one event per action,
/// not once per write site it happens to coalesce.
#[test]
fn pane_focus_cycling_and_mouse_click_each_raise_exactly_one_on_buffer_enter() {
    use hume_scripting::ScriptingHost;

    let tmp = tempfile::tempdir().unwrap();
    let mut ed = editor_from("-[h]>ello\n");
    type_cmd(&mut ed, ":vsplit");
    let path_b = tmp.path().join("b.txt");
    std::fs::write(&path_b, "world\n").unwrap();
    type_cmd(&mut ed, &format!(":e {}", path_b.display()));

    let mut host = ScriptingHost::for_testing(&Dirs::none());
    eval_with_real_host(
        &mut ed,
        &mut host,
        r#"(register-hook! 'on-buffer-enter (lambda (bid) (log! 'trace "entered")))"#,
        tmp.path(),
    );
    ed.scripting = Some(host);

    // Settle the setup switches above, and establish pane geometry once:
    // `mouse_left_down` needs real pane rects, and rects don't depend on
    // which pane is focused, so one `prepare_frame` call covers both
    // actions below.
    let mut ctx = hume_engine::pipeline::RenderContext::new();
    ed.resize_terminal(100, 25);
    ed.settle();
    ed.prepare_frame(&mut ctx);

    let count = |ed: &Editor| {
        ed.state
            .message_log
            .entries()
            .filter(|e| e.severity == Severity::Trace)
            .count()
    };

    // `:e` left the right pane (B) focused. Ctrl-p p, with only two panes,
    // cycles focus onto the left pane (A) through `focus_pane`, never
    // touching `buffer_id`.
    let before = count(&ed);
    ed.feed_event(key_ctrl('p'));
    ed.feed_event(key('p'));
    assert_eq!(
        count(&ed) - before,
        1,
        "pane-focus cycling must raise exactly one OnBufferEnter"
    );

    // Now A (left) is focused. Click into the right pane (B), the same
    // `focus_pane` chokepoint, via `handle_input`'s mouse arm instead of the
    // keymap.
    let before = count(&ed);
    draw(&mut ed);
    ed.handle_input(mouse_left_down(60, 0));
    ed.settle();
    assert_eq!(
        count(&ed) - before,
        1,
        "a mouse click into another pane must raise exactly one OnBufferEnter"
    );
}

/// `run_pending_batch` must skip *any* buffer-scoped
/// event whose buffer has died since it was queued, not just
/// `OnTextChanged`. Queues `OnDiagnosticsChanged` for `bid`, then closes
/// `bid` before the batch drains, reproducing "something else that ran
/// first closed the buffer this event was about", the same race an
/// intervening timer thunk or async callback in the same batch would cause.
///
/// `run_pending_batch` checks every event's `EditorEvent::handle()` before
/// `fire_one_event` runs, so no handler runs against a dead bid and logs a
/// spurious `LiveBid` hook error.
#[test]
fn buffer_scoped_event_is_skipped_once_its_buffer_has_closed() {
    use crate::testing::MockHost;
    use hume_scripting::ScriptingHost;

    let mut ed = editor_from("-[a]>b\n");
    ed.open_buffer(Buffer::at_start(hume_editing::text::BufferText::from(
        "x\n",
    )));
    let bid = ed.focused_buffer_id();

    let mut host = ScriptingHost::for_testing(&Dirs::none());
    let mut mock = MockHost::new();
    host.eval_source(
        r#"(register-hook! 'on-diagnostics-changed (lambda (bid) (log! 'trace "diagnostics-fired")))"#,
        &mut mock,
    )
    .unwrap();
    ed.scripting = Some(host);

    ed.state
        .queue_event(crate::editor::event::EditorEvent::OnDiagnosticsChanged { buffer: bid });
    // Reproduces "something else in the same batch closed bid first":
    // here, a direct close between queueing and the drain, rather than a
    // timer thunk queued ahead of the event; the observable failure mode
    // (the batch reaches a dead bid) is identical either way.
    ed.close_buffer(bid)
        .expect("the buffer has no unsaved changes and runs no pipe");
    ed.settle();

    let log = ed.state.message_log.format_for_display();
    assert!(
        !log.contains("diagnostics-fired"),
        "the handler must not fire for a buffer that closed before the batch drained: {log:?}"
    );
    assert!(
        !log.contains("hook error"),
        "the event must be silently skipped, not surface as a hook error: {log:?}"
    );
}

/// The pane half of the same check: `OnViewportChange`/
/// `OnTriggerChar`/`OnCompletionAccept`/`OnBufferEnter` all carry a pane too,
/// and a handler earlier in the same batch (a timer's own `pane-close`) can
/// close it before this event's own turn, even though the buffer itself
/// stays open elsewhere.
///
/// The pane half of `run_pending_batch`'s filter keeps the handler off the
/// closed pane, where `viewport-range` (which needs a live pane) raises and
/// logs a hook error.
#[test]
fn pane_scoped_event_is_skipped_once_its_pane_has_closed() {
    use crate::testing::MockHost;
    use hume_core::PaneHandle;
    use hume_engine::pipeline::Direction;
    use hume_scripting::ScriptingHost;
    use hume_scripting::host::CommandHost;

    let mut ed = editor_from("-[a]>bbbbbbbbbb\n");
    let bid = ed.focused_buffer_id();
    let pid_b = open_pane_in_layout(&mut ed.state, &mut ed.view, bid, Direction::Horizontal);

    let mut host = ScriptingHost::for_testing(&Dirs::none());
    let mut mock = MockHost::new();
    host.eval_source(
        r#"(register-hook! 'on-viewport-change
             (lambda (pane range) (viewport-range pane) (log! 'trace "viewport-fired")))"#,
        &mut mock,
    )
    .unwrap();
    ed.scripting = Some(host);

    ed.queue_viewport_change(pid_b);
    // Reproduces "something else in the same batch closed the pane first":
    // a direct close here, rather than an earlier-queued timer's own
    // pane-close; the observable failure (the batch reaches a dead pane) is
    // identical either way. The buffer stays open in A throughout.
    ed.state.focus.set_for_test(&mut ed.view, pid_b);
    live_host!(ed)
        .run_command_sync(
            "pane-close",
            PaneHandle::with_pane(bid, pid_b),
            NativeCall::default(),
        )
        .expect("pane-close must succeed");
    ed.settle();

    let log = ed.state.message_log.format_for_display();
    assert!(
        !log.contains("viewport-fired"),
        "the handler must not fire for a pane that closed before the batch drained: {log:?}"
    );
    assert!(
        !log.contains("hook error"),
        "the event must be silently skipped, not surface as a hook error: {log:?}"
    );
}

/// Two handlers registered for the same event, in a
/// batch where the first raises, must both run. A plugin bug in one
/// handler must not drop every handler registered after it for the same
/// event.
///
/// `run_steel_calls` runs each call in its own Steel session, so the first
/// handler's raise leaves the second one to run.
#[test]
fn a_raising_hook_handler_does_not_drop_the_next_handler_for_the_same_event() {
    use crate::testing::MockHost;
    use hume_scripting::ScriptingHost;

    let mut ed = editor_from("-[a]>b\n");
    let mut host = ScriptingHost::for_testing(&Dirs::none());
    let mut mock = MockHost::new();
    host.eval_source(
        r#"(register-hook! 'on-buffer-save (lambda (bid) (car '())))
           (register-hook! 'on-buffer-save (lambda (bid) (log! 'trace "second-fired")))"#,
        &mut mock,
    )
    .unwrap();
    ed.scripting = Some(host);

    let bid = ed.focused_buffer_id();
    ed.queue_buffer_save(bid);
    ed.settle();

    let log = ed.state.message_log.format_for_display();
    assert!(
        log.contains("second-fired"),
        "the second handler must still run despite the first raising: {log:?}"
    );
    assert!(
        log.contains("hook error") || log.contains("steel call error"),
        "the first handler's raise must still be reported, not silently swallowed: {log:?}"
    );
}
