//! A buffer whose text is held, as a pipe running on it holds it: every
//! text change is refused with the busy message, and selections still move.

use super::*;
use crate::editor::buffer::{DiskCheckTrigger, TextHold};
use pretty_assertions::assert_eq;

/// The busy refusal for `ed`'s focused buffer.
fn busy(ed: &Editor) -> String {
    format!("{} is busy: a pipe is running", ed.doc().display_name())
}

fn hold(ed: &mut Editor) -> TextHold {
    let bid = ed.focused_buffer_id();
    ed.state.buffers.get_mut(bid).hold_text()
}

/// Whether any message logged contains the busy message.
fn logged_busy(ed: &Editor) -> bool {
    ed.state
        .message_log
        .entries()
        .any(|e| e.text.contains(&busy(ed)))
}

/// `-[ab]> cd\n` with `ab` yanked, and a change made and undone, so undo
/// and redo each have a step.
fn editor_with_history() -> Editor {
    let mut ed = editor_from("-[ab]> cd\n");
    for k in ['y', 'd', 'u'] {
        ed.feed_key(key(k));
    }
    assert_eq!(state(&ed), "-[ab]> cd\n", "setup");
    ed
}

#[test]
fn every_normal_mode_edit_is_refused() {
    let cases = [
        ("delete", vec![key('d')]),
        ("change", vec![key('c')]),
        ("replace", vec![key('r'), key('x')]),
        ("paste", vec![key('p')]),
        ("undo", vec![key('u')]),
        ("redo", vec![key('U')]),
        ("insert", vec![key('i')]),
        ("increment", vec![key_ctrl('a')]),
        ("indent", vec![key('>')]),
        (
            "pipe",
            "|sort".chars().map(key).chain([key_enter()]).collect(),
        ),
    ];
    for (what, keys) in cases {
        let mut ed = editor_with_history();
        let _hold = hold(&mut ed);
        for k in keys {
            ed.handle_key(k);
        }
        assert_eq!(state(&ed), "-[ab]> cd\n", "{what}");
        assert_eq!(ed.state.mode(), Mode::Normal, "{what}");
        assert_eq!(status(&ed), busy(&ed), "{what}");
    }
}

#[test]
fn sort_is_refused() {
    let mut ed = editor_from("-[b\na]>\n");
    let _hold = hold(&mut ed);
    type_cmd(&mut ed, ":sort");
    assert_eq!(state(&ed), "-[b\na]>\n");
    assert_eq!(status(&ed), busy(&ed));
}

#[test]
fn a_terminal_paste_is_refused() {
    let mut ed = editor_from("-[ab]> cd\n");
    let _hold = hold(&mut ed);
    ed.feed_paste("xy");
    assert_eq!(state(&ed), "-[ab]> cd\n");
    assert_eq!(status(&ed), busy(&ed));
}

#[test]
fn typing_in_an_insert_session_opened_before_the_hold_is_refused() {
    let mut ed = editor_from("-[a]>b\n");
    ed.feed_key(key('i'));
    let _hold = hold(&mut ed);
    ed.handle_key(key('x'));
    assert_eq!(ed.doc().text().to_string(), "ab\n");
    assert_eq!(ed.state.mode(), Mode::Insert);
    assert_eq!(status(&ed), busy(&ed));
}

#[test]
fn a_line_ending_change_is_refused() {
    let mut ed = editor_from("-[ab]>\n");
    let _hold = hold(&mut ed);
    type_cmd(&mut ed, ":set buffer line-ending=crlf");
    assert_eq!(
        ed.doc().text().line_ending(),
        hume_editing::text::LineEnding::Lf
    );
    assert!(logged_busy(&ed));
}

#[test]
fn the_reload_answer_to_a_disk_change_is_refused() {
    let (mut ed, tmp) = editor_with_file("-[h]>ello\n", "hello\n");
    std::fs::write(&tmp, "HELLO!!\n").unwrap();
    let bid = ed.focused_buffer_id();
    ed.check_buffer_disk_state(bid, DiskCheckTrigger::Ambient);
    assert!(ed.state.input.confirm().is_some(), "setup: confirm is open");
    let _hold = hold(&mut ed);
    ed.handle_key(key('r'));
    assert_eq!(ed.doc().text().to_string(), "hello\n");
    assert!(logged_busy(&ed));
}

#[test]
fn restoring_a_dump_is_refused_and_keeps_the_dump_pending() {
    let (_dir, path) = file_with_dump(Some("disk\n"), Some("crashed\n"));
    let mut ed = editor_from("-[s]>cratch\n");
    type_cmd_event(&mut ed, &format!(":e {}", path.display()));
    assert!(ed.state.input.confirm().is_some(), "setup: restore prompt");
    let _hold = hold(&mut ed);
    ed.handle_key(key('r'));
    assert_eq!(ed.doc().text().to_string(), "disk\n");
    assert!(ed.doc().pending_dump.is_some());
    assert!(logged_busy(&ed));
}

#[test]
fn a_script_edit_is_refused() {
    let tmp = tempfile::tempdir().unwrap();
    let mut ed = editor_from("-[ab]> cd\n");
    run(
        &mut ed,
        tmp.path(),
        r#"(define-command! "probe" "" (lambda (pane)
             (edit-text! pane (list (hash 'start-char 0 'end-char 1 'text "X")))))"#,
    );
    let _hold = hold(&mut ed);
    ed.execute_keymap_command("probe", Count::UNTYPED, false);
    assert_eq!(state(&ed), "-[ab]> cd\n");
    assert!(logged_busy(&ed));
}

#[test]
fn paste_text_raises_the_busy_refusal() {
    let tmp = tempfile::tempdir().unwrap();
    let mut ed = editor_from("-[ab]> cd\n");
    run(
        &mut ed,
        tmp.path(),
        r#"(define-command! "probe" "" (lambda (pane) (paste-text! pane "xy")))"#,
    );
    let _hold = hold(&mut ed);
    ed.execute_keymap_command("probe", Count::UNTYPED, false);
    assert_eq!(state(&ed), "-[ab]> cd\n");
    assert!(
        logged_busy(&ed),
        "the refusal reaches the script as an error"
    );
}

#[test]
fn walking_the_undo_history_is_refused() {
    let mut ed = editor_with_history();
    let _hold = hold(&mut ed);
    type_cmd(&mut ed, ":earlier");
    assert_eq!(state(&ed), "-[ab]> cd\n");
    assert_eq!(status(&ed), busy(&ed));
    type_cmd(&mut ed, ":later");
    assert_eq!(state(&ed), "-[ab]> cd\n");
    assert_eq!(status(&ed), busy(&ed));
}

#[test]
fn goto_revision_raises_the_busy_refusal() {
    let tmp = tempfile::tempdir().unwrap();
    let mut ed = editor_with_history();
    run(
        &mut ed,
        tmp.path(),
        r#"(define-command! "probe" "" (lambda (pane) (goto-revision! pane 1)))"#,
    );
    let _hold = hold(&mut ed);
    ed.execute_keymap_command("probe", Count::UNTYPED, false);
    assert_eq!(state(&ed), "-[ab]> cd\n");
    assert!(logged_busy(&ed));
}

#[test]
fn motions_move_a_held_buffers_selections() {
    let mut ed = editor_from("-[ab]> cd\n");
    let _hold = hold(&mut ed);
    ed.handle_key(key('w'));
    assert_ne!(state(&ed), "-[ab]> cd\n");
    assert_eq!(ed.doc().text().to_string(), "ab cd\n");
    assert!(!logged_busy(&ed) && status(&ed) != busy(&ed));
}

#[test]
fn dropping_the_hold_lets_the_same_key_edit() {
    let mut ed = editor_from("-[ab]> cd\n");
    let held = hold(&mut ed);
    ed.handle_key(key('d'));
    assert_eq!(ed.doc().text().to_string(), "ab cd\n");
    drop(held);
    ed.handle_key(key('d'));
    assert_eq!(ed.doc().text().to_string(), " cd\n");
}
