//! The disk check on buffer enter and focus gained, and after inline output.

use super::*;
use pretty_assertions::assert_eq;

// ── OnBufferEnter / OnFocusGained ──────────────────────────────────────────────

/// End to end: a picker accept switching onto a buffer
/// whose backing file changed externally must open the reload confirm. Built
/// on `tests/picker_steel.rs`'s harness: a picker's keymap binding
/// queues as a `PendingWork::Call`, drained by the next `render_to_buf`
/// (`settle()` internally), same as `on-buffer-enter`.
///
/// A disk check wired only into typed commands would miss this path: a fuzzy
/// picker never dispatches `:e`/`:b`, and the switch happens a frame later,
/// inside the drain, so nothing at the end of `handle_input` sees it either.
#[test]
fn picker_accept_onto_an_externally_changed_buffer_opens_the_reload_confirm() {
    let tmp = tempfile::tempdir().unwrap();
    let mut ed = editor_from("-[a]>bc\n");

    let target = tmp.path().join("target.md");
    std::fs::write(&target, "hi\n").unwrap();
    let path = target.to_string_lossy().into_owned();

    // Open the target buffer up front so its stored signature reflects the
    // pre-change content. The picker below switches onto this *already-open*
    // buffer without re-reading, matching a buffer-switcher picker (not a
    // file-opener, which would read the post-change content fresh and never
    // see a mismatch).
    let bid = crate::editor::tests::open_path(&mut ed, &path);
    ed.settle();

    // File changes on disk while the buffer stays open in the background.
    rewrite_externally(&target, "hi, externally changed!\n");

    let steel_target = steel_path(&target);
    let mut host = hume_scripting::ScriptingHost::for_testing(&Dirs::none());
    eval_with_real_host(
        &mut ed,
        &mut host,
        &format!(
            r#"(define-typed-command! "go" "" (lambda (pane)
                 (picker! pane (list (cons "target" {steel_target}))
                   (test-picker-keys (lambda (p) (when p (switch-to-buffer! pane (open-buffer! p))))))))"#
        ),
        tmp.path(),
    );
    ed.scripting = Some(host);

    type_cmd(&mut ed, ":go");
    assert!(ed.state.input.picker().is_some(), "sanity: picker open");

    let rect = Rect::new(0, 0, 40, 12);
    let _ = ed.render_to_buf(rect);

    // Accept: Enter queues its binding as a PendingWork::Call.
    ed.feed_key(key_enter());

    // The next render_to_buf drains the callback (switching to the stale
    // buffer) and settles: settle()'s OnBufferEnter diff must observe the
    // switch and run the disk check in the same drain.
    let _ = ed.render_to_buf(rect);

    assert_eq!(
        ed.focused_buffer_id(),
        bid,
        "sanity: picker accept switched onto the target buffer"
    );
    assert!(
        ed.state.input.confirm().is_some(),
        "picker accept onto an externally-changed buffer must open the reload confirm"
    );
}

/// A non-interactive `switch-to-buffer!` (Steel's builtin, LSP goto-
/// definition, any async callback) onto a stale buffer must open the reload
/// confirm too.
///
/// The reaction lives in `settle()`'s own diff. Gated on
/// `Editor::handle_input`'s dispatch, a switch with no interactive dispatch
/// behind it would never reach the check.
#[test]
fn non_interactive_switch_to_buffer_onto_a_stale_buffer_opens_the_reload_confirm() {
    let (mut ed, _tmp_a) = editor_with_file("-[h]>ello\n", "hello\n");
    let bid_a = ed.focused_buffer_id();

    let (tmp_b, _tmp_b_guard) = temp_file("world\n");
    let bid_b = crate::editor::tests::open_path(&mut ed, &tmp_b);
    ed.settle();
    assert_ne!(bid_a, bid_b, "setup: two distinct buffers, A still focused");

    rewrite_externally(&tmp_b, "world, externally changed!\n");

    // The non-interactive primitive: Steel's `switch-to-buffer!` and every
    // LSP goto-definition call go through exactly this, never a typed
    // command.
    ed.switch_to_buffer_with_jump(FocusedPane::current(&ed.view), bid_b);
    ed.settle();

    assert_eq!(ed.focused_buffer_id(), bid_b);
    assert!(
        ed.state.input.confirm().is_some(),
        "a non-interactive switch onto a stale buffer must still prompt"
    );
}

/// `OnFocusGained`'s reaction is `check_all_disk_state(Ambient)`: a sweep
/// over every open buffer, not just the focused one.
#[test]
fn focus_gained_sweeps_every_open_buffer_not_just_the_focused_one() {
    use termina::event::Event as TerminalEvent;

    let (mut ed, tmp_a) = editor_with_file("-[h]>ello\n", "hello\n");
    let bid_a = ed.focused_buffer_id();

    let (tmp_b, _tmp_b_guard) = temp_file("world\n");
    type_cmd(&mut ed, &format!(":e {}", tmp_b.display()));
    let bid_b = ed.focused_buffer_id();
    assert_ne!(bid_a, bid_b, "setup: B is now focused, A is not");

    // A, the non-focused buffer, changes externally.
    rewrite_externally(&tmp_a, "hello, externally changed!\n");
    let (_, warnings_before) = ed.state.message_log.totals();

    ed.handle_input(TerminalEvent::FocusIn);
    ed.settle();

    let (_, warnings_after) = ed.state.message_log.totals();
    assert_eq!(
        warnings_after,
        warnings_before + 1,
        "OnFocusGained's sweep must warn for A even though B is focused"
    );
    assert!(
        ed.state.input.confirm().is_none(),
        "A is not focused, so an ambient check only warns, never opens a confirm"
    );
}

/// `:b <other>` onto a stale buffer must run the disk check exactly once,
/// pinning against a double-check regression from a direct call surviving
/// alongside `settle()`'s event-driven diff. A second run would
/// find the confirm already open (`confirm_permit`'s `confirm.is_none()`
/// guard blocks it) and fall through to `report_disk_state`'s warn fallback
/// instead: an extra `:messages` entry alongside the confirm.
#[test]
fn switching_onto_a_stale_buffer_checks_disk_state_exactly_once() {
    let (mut ed, _tmp_a) = editor_with_file("-[h]>ello\n", "hello\n");
    let bid_a = ed.focused_buffer_id();

    let (tmp_b, _tmp_b_guard) = temp_file("world\n");
    type_cmd(&mut ed, &format!(":e {}", tmp_b.display()));
    let bid_b = ed.focused_buffer_id();
    assert_ne!(bid_a, bid_b, "setup: two distinct buffers, B focused");

    type_cmd(&mut ed, ":b #");
    assert_eq!(ed.focused_buffer_id(), bid_a, "setup: back on A");

    rewrite_externally(&tmp_b, "world, externally changed!\n");
    let (_, warnings_before) = ed.state.message_log.totals();

    type_cmd_event(&mut ed, ":b #");
    assert_eq!(
        ed.focused_buffer_id(),
        bid_b,
        "setup: switched onto stale B"
    );

    assert!(
        ed.state.input.confirm().is_some(),
        "sanity: the switch onto B must open the reload confirm"
    );
    let (_, warnings_after) = ed.state.message_log.totals();
    assert_eq!(
        warnings_after, warnings_before,
        "the disk check must run exactly once: a second run would find the \
         confirm already open and fall back to warning instead"
    );
}

// ── Inline-output disk sweep (dispatch.rs's `OnFocusGained`) ─────────────────

/// An `#:inline-output` command that both rewrites the focused file (a
/// formatter, a `git checkout` wrapper) *and* logs its own warning (a
/// non-zero exit, a lint note) must still open the reload confirm. Its own
/// warning must not shadow the very disk change it caused.
///
/// `confirm_permit`'s message-shadow clause exists to protect the
/// `BufferEnter` case (`:qa` landing on a dirty buffer): a command's own
/// failure message must survive an *unrelated* buffer-enter check that
/// happens to run right after. The `Ambient` sweep this test exercises is
/// never unrelated (it exists because of this exact command), so it must be
/// exempt.
///
/// The `trigger != DiskCheckTrigger::BufferEnter` guard provides that
/// exemption. `message_logged_this_input` is already `true` from this
/// dispatch's own `log!` warning when the queued `OnFocusGained` reaction
/// runs in the next `settle()`, so an unconditional clause would leave only
/// a `:messages` line and no confirm.
#[test]
fn inline_output_commands_own_warning_does_not_shadow_its_own_reload_confirm() {
    use hume_core::BindMode;
    use hume_scripting::ScriptingHost;

    let (mut ed, tmp) = editor_with_file("-[h]>ello\n", "hello\n");
    // No real terminal in this harness: a frame pushed with the TUI active
    // is what makes `run_steel_command` queue `OnFocusGained` at all; only
    // actually entering the alt-screen (which this command's body never
    // reaches, since it only logs) needs a real terminal.
    ed.tui = crate::editor::tui::Tui::OnHeadless;

    let mut host = ScriptingHost::for_testing(&Dirs::none());
    {
        let mut init_host = init_host!(ed);
        host.eval_source(
            r#"(define-command! "lint-and-fix" "doc"
                 (lambda () (log! 'warning "lint: 1 issue auto-fixed")) #:inline-output #t)"#,
            &mut init_host,
        )
        .expect("eval failed");
    }
    ed.scripting = Some(host);
    ed.state.config.keymap.bind(
        BindMode::Normal,
        &[key('\\')],
        "lint-and-fix".into(),
        false,
        hume_core::ConfigOrigin::User,
    );

    // The command's own subprocess rewrote the focused file while it ran.
    rewrite_externally(&tmp, "hello, world!\n");

    ed.feed_event(key('\\'));

    assert!(
        ed.state
            .message_log
            .entries()
            .any(|e| e.text.contains("lint: 1 issue auto-fixed")),
        "sanity: the command's own warning must have logged"
    );
    assert!(
        ed.state.input.confirm().is_some(),
        "the reload confirm must open despite the command's own warning"
    );
}
