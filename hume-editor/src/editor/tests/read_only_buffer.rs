//! A read-only buffer: every text change is refused with the read-only
//! message, and selections still move.

use super::*;
use pretty_assertions::assert_eq;

/// `-[ab]> cd\n` with `ab` yanked, a change made and undone so undo and
/// redo each have a step, then made read-only.
fn read_only_editor() -> Editor {
    let mut ed = editor_from("-[ab]> cd\n");
    type_chars(&mut ed, "ydu");
    assert_eq!(state(&ed), "-[ab]> cd\n", "setup");
    ed.doc_mut().read_only = true;
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
        ("unindent", vec![key('<')]),
        ("join", vec![key('J')]),
        ("align", vec![key('&')]),
        ("surround", vec![key('m'), key('w'), key('(')]),
        (
            "pipe",
            "|sort".chars().map(key).chain([key_enter()]).collect(),
        ),
    ];
    for (what, keys) in cases {
        let mut ed = read_only_editor();
        for k in keys {
            ed.handle_key(k);
        }
        assert_eq!(state(&ed), "-[ab]> cd\n", "{what}");
        assert_eq!(ed.state.mode(), Mode::Normal, "{what}");
        assert_eq!(status(&ed), read_only_refusal(&ed), "{what}");
    }
}

#[test]
fn every_edit_command_is_refused() {
    for name in [
        "delete-char-forward",
        "make-text-uppercase",
        "delete-word-backward",
    ] {
        let mut ed = read_only_editor();
        call(&mut ed, name);
        assert_eq!(state(&ed), "-[ab]> cd\n", "{name}");
        assert_eq!(status(&ed), read_only_refusal(&ed), "{name}");
    }
}

#[test]
fn a_terminal_paste_is_refused() {
    let mut ed = read_only_editor();
    ed.feed_paste("xy");
    assert_eq!(state(&ed), "-[ab]> cd\n");
    assert_eq!(status(&ed), read_only_refusal(&ed));
}

#[test]
fn paste_text_raises_the_read_only_refusal() {
    let tmp = tempfile::tempdir().unwrap();
    let mut ed = read_only_editor();
    run(
        &mut ed,
        tmp.path(),
        r#"(define-command! "probe" "" (lambda (pane) (paste-text! pane "xy")))"#,
    );
    call(&mut ed, "probe");
    assert_eq!(state(&ed), "-[ab]> cd\n");
    let refusal = format!("paste-text!: {}", read_only_refusal(&ed));
    assert!(
        has_error(&ed, &refusal),
        "the refusal reaches the script as an error: {}",
        ed.state.message_log.format_for_display()
    );
}

/// A macro records on a read-only buffer, and replaying it repeats its
/// motions.
#[test]
fn a_macro_of_motions_records_and_replays() {
    let mut moved = read_only_editor();
    type_chars(&mut moved, "l");
    let after_one = state(&moved);
    type_chars(&mut moved, "l");
    let after_two = state(&moved);

    let mut ed = read_only_editor();
    type_chars(&mut ed, "QQlQ");
    assert_eq!(state(&ed), after_one, "recording runs the motion");
    type_chars(&mut ed, "qq");
    ed.drain_replay_queue();
    assert_eq!(state(&ed), after_two, "replay repeats it");
}

/// Replaying a macro that edits refuses its edit, as typing it would.
#[test]
fn replaying_an_editing_macro_is_refused() {
    let mut ed = editor_from("-[ab]> cd\n");
    type_chars(&mut ed, "QQdQ");
    ed.doc_mut().read_only = true;
    let before = state(&ed);

    type_chars(&mut ed, "qq");
    ed.drain_replay_queue();

    assert_eq!(state(&ed), before);
    assert_eq!(status(&ed), read_only_refusal(&ed));
}
