//! `pipe`'s prompt and command up to the queued run. Keys go through
//! `handle_key`, which leaves the run queued for the main loop instead of
//! running a process; the runs themselves are in `unix/pipe.rs`.

use super::*;
use crate::editor::input_stack::shell_prompt::ShellPromptLayer;
use crate::editor::keymap::BindMode;
use crate::editor::minibuf::history::HistoryKind;
use crate::editor::pipe_run::{PipeKind, WaitEvent, wait_event};
use termina::WindowSize;
use termina::event::KeyEventKind;

fn keys(ed: &mut Editor, text: &str) {
    for ch in text.chars() {
        ed.handle_key(key(ch));
    }
}

/// The queued run's command and kind, taking it off the queue.
fn take_queued(ed: &mut Editor) -> Option<(String, PipeKind)> {
    ed.state.queued_pipe.take().map(|q| (q.command, q.kind))
}

fn prompt_label(ed: &Editor) -> Option<String> {
    ed.state.minibuf().map(|mb| mb.prompt.clone())
}

#[test]
fn bar_prompts_for_a_command_and_enter_queues_a_replace() {
    let mut ed = editor_from("-[abc]>\n");
    ed.handle_key(key('|'));
    assert_eq!(prompt_label(&ed).as_deref(), Some("|"));
    assert_eq!(ed.state.mode(), Mode::Normal);
    keys(&mut ed, "sort -u");
    ed.handle_key(key_enter());
    assert!(ed.state.minibuf().is_none());
    assert_eq!(
        take_queued(&mut ed),
        Some(("sort -u".to_string(), PipeKind::Replace))
    );
}

#[test]
fn ctrl_bar_prompts_over_insert_and_queues_an_insert() {
    let mut ed = editor_from_kitty("-[abc]>\n");
    ed.handle_key(key('i'));
    ed.handle_key(key_ctrl('|'));
    assert_eq!(prompt_label(&ed).as_deref(), Some("|"));
    assert_eq!(ed.state.mode(), Mode::Insert);
    keys(&mut ed, "date");
    ed.handle_key(key_enter());
    assert!(ed.state.minibuf().is_none());
    assert_eq!(ed.state.mode(), Mode::Insert);
    assert_eq!(
        take_queued(&mut ed),
        Some(("date".to_string(), PipeKind::Insert))
    );
}

#[test]
fn escape_closes_the_prompt_and_the_insert_session_goes_on() {
    let mut ed = editor_from_kitty("-[abc]>\n");
    ed.handle_key(key('i'));
    ed.handle_key(key_ctrl('|'));
    keys(&mut ed, "date");
    ed.handle_key(key_esc());
    assert!(ed.state.minibuf().is_none());
    assert_eq!(ed.state.mode(), Mode::Insert);
    assert!(ed.state.queued_pipe.is_none());
    ed.handle_key(key('x'));
    assert_eq!(ed.doc().text().to_string(), "xabc\n");
}

#[test]
fn a_cancelled_shell_prompt_keeps_the_previous_change() {
    let mut ed = editor_from("-[a]>\n");
    ed.handle_key(key('r'));
    ed.handle_key(key('x'));
    ed.handle_key(key('|'));
    keys(&mut ed, "sort");
    ed.handle_key(key_esc());
    assert!(ed.state.queued_pipe.is_none());
    assert_eq!(
        ed.state.last_action().map(|a| a.command.as_ref()),
        Some("replace")
    );
}

#[test]
fn ctrl_bar_needs_the_kitty_protocol() {
    let mut ed = editor_from("-[abc]>\n");
    ed.handle_key(key('i'));
    ed.handle_key(key_ctrl('|'));
    assert!(ed.state.minibuf().is_none());
}

#[test]
fn the_prompt_recalls_its_own_history_only() {
    let mut ed = editor_from("-[abc]>\n");
    ed.handle_key(key('|'));
    keys(&mut ed, "sort");
    ed.handle_key(key_enter());
    take_queued(&mut ed);

    ed.handle_key(key('|'));
    ed.handle_key(key_up());
    assert_eq!(minibuf_input(&ed), "sort");
    ed.handle_key(key_esc());

    ed.handle_key(key(':'));
    ed.handle_key(key_up());
    assert_eq!(minibuf_input(&ed), "");
    assert!(
        ed.state
            .history
            .get(HistoryKind::Command)
            .entries()
            .is_empty()
    );
}

#[test]
fn a_blank_command_queues_nothing() {
    let mut ed = editor_from("-[abc]>\n");
    ed.handle_key(key('|'));
    keys(&mut ed, "  ");
    ed.handle_key(key_enter());
    assert!(ed.state.queued_pipe.is_none());
    assert_eq!(state(&ed), "-[abc]>\n");
}

#[test]
fn a_read_only_buffer_refuses_before_anything_runs() {
    let mut ed = editor_with_read_only_view("abc\n", "view");
    ed.handle_key(key('|'));
    keys(&mut ed, "sort");
    ed.handle_key(key_enter());
    assert!(ed.state.queued_pipe.is_none());
    assert_eq!(status(&ed), "Buffer is read-only");
}

#[test]
fn the_prompt_does_not_open_over_another_minibuffer() {
    let mut ed = editor_from("-[abc]>\n");
    let f2 = KeyEvent::new(KeyCode::Function(2), Modifiers::NONE);
    ed.state
        .config
        .keymap
        .bind_user_with_extend(BindMode::Command, &[f2], "pipe".into(), false);
    ed.handle_key(key(':'));
    ed.handle_key(f2);
    assert_eq!(prompt_label(&ed).as_deref(), Some(":"));
    assert!(has_error(&ed, "a minibuffer is already open"));
}

#[test]
fn call_refuses_pipe() {
    let mut ed = editor_from("-[abc]>\n");
    attach_steel(
        &mut ed,
        r#"(define-command! "f2-body" "" (lambda (pane) (call! "pipe" pane "sort")))"#,
    );
    let f2 = KeyEvent::new(KeyCode::Function(2), Modifiers::NONE);
    ed.state
        .config
        .keymap
        .bind_user_with_extend(BindMode::Normal, &[f2], "f2-body".into(), false);
    ed.handle_key(f2);
    assert!(has_error(&ed, "'pipe': runs only from a key"));
    assert!(ed.state.queued_pipe.is_none());
}

#[test]
fn a_run_waits_only_for_ctrl_c_and_resizes() {
    assert!(matches!(
        wait_event(&TerminalEvent::Key(key_ctrl('c'))),
        Some(WaitEvent::Interrupt)
    ));
    let size = WindowSize {
        cols: 80,
        rows: 24,
        pixel_width: None,
        pixel_height: None,
    };
    assert!(matches!(
        wait_event(&TerminalEvent::WindowResized(size)),
        Some(WaitEvent::Resized)
    ));
    let mut release = key_ctrl('c');
    release.kind = KeyEventKind::Release;
    for left_queued in [
        TerminalEvent::Key(release),
        TerminalEvent::Key(key('u')),
        TerminalEvent::FocusIn,
    ] {
        assert!(
            wait_event(&left_queued).is_none(),
            "{left_queued:?} stays queued for after the run"
        );
    }
}

#[test]
fn a_script_prompt_closes_the_shell_prompt() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let mut ed = editor_from("-[abc]>\n");
    run(
        &mut ed,
        tmp.path(),
        r#"(define-typed-command! "arm" "" (lambda (pane)
             (after! 0 (lambda () (prompt! pane "x: " (lambda (s) (void)))))))"#,
    );
    type_cmd(&mut ed, ":arm");
    ed.handle_key(key('|'));
    keys(&mut ed, "sort");
    ed.settle();
    assert_eq!(prompt_label(&ed).as_deref(), Some("x: "));
    assert!(ed.state.input.ref_of::<ShellPromptLayer>().is_none());
    ed.handle_key(key_esc());
    assert!(ed.state.minibuf().is_none());
    assert!(ed.state.queued_pipe.is_none());
}
