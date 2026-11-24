use termina::event::{KeyCode, KeyEvent, Modifiers};

use hume_rope::width::CursorWindow;

use super::{LineEdit, LineInput};
use crate::editor::tests::{key, key_backspace, key_ctrl, key_left, key_right};

/// `text` with the cursor `back` graphemes before its end.
fn line(text: &str, back: usize) -> LineInput {
    let mut line = LineInput::new(text.to_owned());
    for _ in 0..back {
        line.edit(key_left());
    }
    line
}

#[test]
fn a_char_types_at_the_cursor() {
    let mut l = line("ac", 1);
    assert_eq!(l.edit(key('b')), LineEdit::Changed);
    assert_eq!((l.text(), l.cursor()), ("abc", 2));
}

#[test]
fn backspace_removes_the_whole_cluster_before_the_cursor() {
    let mut l = line("ae\u{301}x", 1);
    assert_eq!(l.edit(key_backspace()), LineEdit::Changed);
    assert_eq!((l.text(), l.cursor()), ("ax", 1));
}

#[test]
fn backspace_at_the_start_changes_nothing() {
    let mut l = line("ab", 2);
    assert_eq!(l.edit(key_backspace()), LineEdit::Ignored);
    assert_eq!((l.text(), l.cursor()), ("ab", 0));
}

#[test]
fn ctrl_w_removes_the_path_component_before_the_cursor() {
    let mut l = line("src/edit x", 2);
    assert_eq!(l.edit(key_ctrl('w')), LineEdit::Changed);
    assert_eq!((l.text(), l.cursor()), ("src/ x", 4));
}

#[test]
fn ctrl_w_with_nothing_before_the_cursor_changes_nothing() {
    let mut l = line("ab", 2);
    assert_eq!(l.edit(key_ctrl('w')), LineEdit::Ignored);
    assert_eq!((l.text(), l.cursor()), ("ab", 0));
}

#[test]
fn a_letter_with_a_modifier_other_than_shift_types_nothing() {
    let mut l = line("ab", 0);
    for modifiers in [
        Modifiers::CONTROL,
        Modifiers::ALT,
        Modifiers::SUPER,
        Modifiers::HYPER,
        Modifiers::META,
        Modifiers::SHIFT | Modifiers::SUPER,
    ] {
        assert_eq!(
            l.edit(KeyEvent::new(KeyCode::Char('x'), modifiers)),
            LineEdit::Ignored
        );
    }
    assert_eq!((l.text(), l.cursor()), ("ab", 2));
}

#[test]
fn a_letter_with_shift_or_a_lock_types() {
    let mut l = line("", 0);
    for (ch, modifiers) in [
        ('X', Modifiers::SHIFT),
        ('Y', Modifiers::CAPS_LOCK),
        ('1', Modifiers::NUM_LOCK),
    ] {
        assert_eq!(
            l.edit(KeyEvent::new(KeyCode::Char(ch), modifiers)),
            LineEdit::Changed
        );
    }
    assert_eq!((l.text(), l.cursor()), ("XY1", 3));
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

#[test]
fn left_and_right_step_the_cursor_one_cluster() {
    let mut l = line("ae\u{301}b", 0);
    assert_eq!(l.edit(key_left()), LineEdit::Moved);
    assert_eq!(l.cursor(), "ae\u{301}".len());
    assert_eq!(l.edit(key_left()), LineEdit::Moved);
    assert_eq!(l.cursor(), 1);
    assert_eq!(l.edit(key_right()), LineEdit::Moved);
    assert_eq!((l.text(), l.cursor()), ("ae\u{301}b", "ae\u{301}".len()));
}

#[test]
fn a_move_past_either_end_stays_put() {
    let mut l = line("ab", 2);
    assert_eq!(l.edit(key_left()), LineEdit::Ignored);
    assert_eq!(l.cursor(), 0);
    let mut l = line("ab", 0);
    assert_eq!(l.edit(key_right()), LineEdit::Ignored);
    assert_eq!(l.cursor(), 2);
}

fn window<'a>(
    before: &'a str,
    at: &'a str,
    after: &'a str,
    before_width: usize,
) -> CursorWindow<'a> {
    CursorWindow {
        before,
        before_width,
        at,
        after,
    }
}

#[test]
fn the_window_keeps_its_start_while_the_cursor_moves_inside_it() {
    let mut l = line("abcdefgh", 0);
    assert_eq!(l.scroll_window(4), window("fgh", "", "", 3));
    l.edit(key_left());
    l.edit(key_left());
    assert_eq!(l.scroll_window(4), window("f", "g", "h", 1));
}

#[test]
fn typing_after_moving_before_the_window_starts_it_at_the_cursor() {
    let mut l = line("abcdefgh", 0);
    l.scroll_window(4);
    for _ in 0..4 {
        l.edit(key_left());
    }
    l.edit(key('é'));
    assert_eq!(l.scroll_window(4), window("é", "e", "fg", 1));
}

#[test]
fn a_splice_over_the_window_start_starts_it_at_the_splice() {
    let mut l = line("ab cdefgh", 0);
    l.scroll_window(4);
    l.splice(3..9, "é\u{65E5}");
    assert_eq!(l.scroll_window(4), window("é\u{65E5}", "", "", 3));
}
