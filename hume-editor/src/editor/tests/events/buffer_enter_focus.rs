//! `on-buffer-enter`, `on-focus-gained` and `on-option-change`.

use super::*;

// ── OnBufferEnter / OnFocusGained ──────────────────────────────────────────────

/// `last_entered_buffer` starts `None`, so the very first `settle()` a fresh
/// `Editor` ever runs must observe a diff against it and fire
/// `on-buffer-enter` for the startup buffer, matching Vim's `BufEnter`
/// firing once on open.
///
/// Seeded with the startup buffer, the diff would find nothing new and the
/// hook would never fire.
#[test]
fn startup_buffer_fires_on_buffer_enter_on_the_first_settle() {
    use crate::testing::MockHost;
    use hume_scripting::ScriptingHost;

    let mut ed = editor_from("-[a]>b\n");
    let mut host = ScriptingHost::new(&Dirs::none());
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
/// fresh `OnBufferEnter`. An unconditional fire would mean a `stat` (via
/// its Rust reaction) and a Steel call on every idle frame.
///
/// The `last_entered_buffer` comparison in `Editor::detect_buffer_enter` is
/// what keeps it quiet.
#[test]
fn settle_with_no_focus_change_raises_no_further_on_buffer_enter() {
    use crate::testing::MockHost;
    use hume_scripting::ScriptingHost;

    let mut ed = editor_from("-[a]>b\n");
    let mut host = ScriptingHost::new(&Dirs::none());
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
/// `OnBufferEnter`, for the *final* buffer, not one per intermediate write.
/// The diff is taken against `last_entered_buffer`, not against every raw
/// write to the focused pane or `pane.buffer_id`.
#[test]
fn consecutive_switches_before_settle_coalesce_into_one_event_for_the_final_buffer() {
    use crate::editor::buffer::Buffer;
    use crate::testing::MockHost;
    use hume_scripting::ScriptingHost;

    let mut ed = editor_from("-[a]>b\n");
    let mut host = ScriptingHost::new(&Dirs::none());
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
    ed.switch_to_buffer_with_jump(FocusedPane::current(&ed.view), buf1);
    ed.switch_to_buffer_with_jump(FocusedPane::current(&ed.view), buf2);

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
/// the focused pane (a direct write via `set_for_test`, standing in for a
/// real focus change like pane-focus cycling) *and* `pane.buffer_id` (like a
/// buffer switch) before any `settle()` runs must
/// still coalesce into a single `OnBufferEnter` for wherever focus ends up
/// (`focused_buffer_id()` is one join evaluated once per pass, not two
/// independent things to diff separately).
#[test]
fn pane_focus_write_and_buffer_write_in_one_pass_coalesce_into_one_event() {
    use crate::editor::buffer::Buffer;
    use crate::testing::MockHost;
    use hume_scripting::ScriptingHost;

    let mut ed = editor_from("-[a]>b\n");
    let mut host = ScriptingHost::new(&Dirs::none());
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

    let pid_a = ed.view.focused_pane();
    let bid = ed.focused_buffer_id();
    let pid_b = open_pane_in_layout(
        &mut ed.state,
        &mut ed.view,
        bid,
        hume_engine::pipeline::Direction::Horizontal,
    );
    let buf2 = ed.open_buffer(Buffer::scratch());

    // Bare `focus` write (no settle() in between)...
    ed.state.focus.set_for_test(&mut ed.view, pid_b);
    // ...then a `pane.buffer_id` write on the pane that write just focused.
    ed.switch_to_buffer_with_jump(FocusedPane::current(&ed.view), buf2);
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
    assert_eq!(ed.view.focused_pane(), pid_b);
    assert_eq!(ed.focused_buffer_id(), buf2);
}

/// A handler that itself calls `switch-to-buffer!` re-triggers the diff on
/// the *next pass of the same `settle()` call*, not a frame later. Needs
/// the real host (`switch-to-buffer!`/`open-buffer!` are gated to
/// `Command`/`PluginActivation` mode, unavailable to `MockHost`); both are
/// legal from inside a fired hook, which runs under `Command` mode
/// (`ScriptingHost::run_steel_calls`).
///
/// A diff taken once before the drain loop would log only one "entered" and
/// pick up the handler's own switch a `settle()` later.
#[test]
fn handler_driven_switch_produces_a_second_on_buffer_enter_in_the_same_settle_call() {
    let tmp = tempfile::tempdir().unwrap();
    let other = tmp.path().join("other.txt");
    std::fs::write(&other, "b\n").unwrap();
    let other_path = steel_path(&other);

    let mut ed = editor_from("-[a]>b\n");
    let bid_before = ed.focused_buffer_id();
    let mut host = hume_scripting::ScriptingHost::new(&Dirs::none());
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
                     (switch-to-buffer! bid (open-buffer! {other_path})))))"#
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
/// `settle()`. Nothing routes through `OnBufferEnter`'s per-buffer
/// mechanism, since regaining terminal focus may be relevant to every open
/// buffer, not just the focused one.
///
/// A missing raise site, a wrong Steel name, or a non-empty `into_steel_args`
/// payload would each break this test.
#[test]
fn on_focus_gained_fires_from_handle_input_and_settle_with_no_args() {
    use crate::testing::MockHost;
    use hume_scripting::ScriptingHost;
    use termina::event::Event as TerminalEvent;

    let mut ed = editor_from("-[a]>b\n");
    let mut host = ScriptingHost::new(&Dirs::none());
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

/// `on-option-change` fires `(key value)` after `apply_global` (the single
/// write path `:set global`, `set-option!`, and `:theme` all funnel through
/// in `settings/ops.rs`) succeeds. Exercised via `:set global` here;
/// `tests/unix/lsp_inlay_feature.rs`'s
/// `setting_off_via_set_command_clears_hints_through_the_plugin_hook` covers
/// the same raise reached through the real shipped plugin's own handler.
///
/// The raise has to exist, sit after the write (and off the buffer-scoped
/// `apply_buffer` path), use the right Steel name, and pass the right pair
/// through `into_steel_args`.
#[test]
fn on_option_change_fires_key_and_value_after_a_set_global() {
    use crate::testing::MockHost;
    use hume_scripting::ScriptingHost;

    let mut ed = editor_from("-[a]>b\n");
    let mut host = ScriptingHost::new(&Dirs::none());
    let mut mock = MockHost::new();
    host.eval_source(
        r#"(register-hook! 'on-option-change (lambda (key value)
             (log! 'trace (string-append key "=" (if (equal? value #t) "on" "off")))))"#,
        &mut mock,
    )
    .unwrap();
    ed.scripting = Some(host);
    ed.settle();

    type_cmd(&mut ed, ":set global lsp-diagnostics-on-insert-line=true");
    ed.settle();

    assert!(
        ed.state
            .message_log
            .entries()
            .any(|e| e.severity == Severity::Trace && e.text == "lsp-diagnostics-on-insert-line=on"),
        "on-option-change must fire with the changed key and its new value \
         after a successful :set global"
    );
}

/// An enum option's new value reaches the hook as the string `get-option`
/// returns.
#[test]
fn on_option_change_passes_an_enum_value_as_its_name() {
    use crate::testing::MockHost;
    use hume_scripting::ScriptingHost;

    let mut ed = editor_from("-[a]>b\n");
    let mut host = ScriptingHost::new(&Dirs::none());
    let mut mock = MockHost::new();
    host.eval_source(
        r#"(register-hook! 'on-option-change (lambda (key value)
             (log! 'trace (if (equal? value "soft") "soft-name" "other"))))"#,
        &mut mock,
    )
    .unwrap();
    ed.scripting = Some(host);
    ed.settle();

    type_cmd(&mut ed, ":set global indent-style=soft");
    ed.settle();

    assert!(
        ed.state
            .message_log
            .entries()
            .any(|e| e.severity == Severity::Trace && e.text == "soft-name"),
        "the hook must receive the enum value as its name"
    );
}
