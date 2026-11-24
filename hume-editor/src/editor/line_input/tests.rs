use termina::event::{KeyCode, KeyEvent, Modifiers};

use super::LineInput;
use crate::editor::tests::{key, key_backspace, key_ctrl};

/// `text` with the cursor `back` graphemes before its end.
fn line(text: &str, back: usize) -> LineInput {
    let mut line = LineInput::new(text.to_owned());
    for _ in 0..back {
        line.move_left();
    }
    line
}

#[test]
fn a_char_types_at_the_cursor() {
    let mut l = line("ac", 1);
    assert!(l.edit(key('b')));
    assert_eq!((l.text(), l.cursor()), ("abc", 2));
}

#[test]
fn backspace_removes_the_whole_cluster_before_the_cursor() {
    let mut l = line("ae\u{301}x", 1);
    assert!(l.edit(key_backspace()));
    assert_eq!((l.text(), l.cursor()), ("ax", 1));
}

#[test]
fn backspace_at_the_start_changes_nothing() {
    let mut l = line("ab", 2);
    assert!(!l.edit(key_backspace()));
    assert_eq!((l.text(), l.cursor()), ("ab", 0));
}

#[test]
fn ctrl_w_removes_the_path_component_before_the_cursor() {
    let mut l = line("src/edit x", 2);
    assert!(l.edit(key_ctrl('w')));
    assert_eq!((l.text(), l.cursor()), ("src/ x", 4));
}

#[test]
fn ctrl_w_with_nothing_before_the_cursor_changes_nothing() {
    let mut l = line("ab", 2);
    assert!(!l.edit(key_ctrl('w')));
    assert_eq!((l.text(), l.cursor()), ("ab", 0));
}

#[test]
fn alt_and_ctrl_letters_type_nothing() {
    let mut l = line("ab", 0);
    assert!(!l.edit(KeyEvent::new(KeyCode::Char('x'), Modifiers::ALT)));
    assert!(!l.edit(key_ctrl('x')));
    assert_eq!((l.text(), l.cursor()), ("ab", 2));
}

#[test]
fn insert_str_types_at_the_cursor_and_an_empty_one_changes_nothing() {
    let mut l = line("ad", 1);
    assert!(!l.insert_str(""));
    assert!(l.insert_str("bc"));
    assert_eq!((l.text(), l.cursor()), ("abcd", 3));
}

#[test]
fn splice_parks_the_cursor_after_the_inserted_text() {
    let mut l = line("ab cd x", 0);
    l.splice(3..5, "efg");
    assert_eq!((l.text(), l.cursor()), ("ab efg x", 6));
}
