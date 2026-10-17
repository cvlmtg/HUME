//! Insert-mode motions that stop at the end of a line land on its line break,
//! where appended text goes. Normal mode keeps them on the last character.

use super::*;
use pretty_assertions::assert_eq;

#[test]
fn end_in_insert_mode_lands_on_the_line_break() {
    let mut ed = editor_from("-[a]>bc def\n");
    ed.handle_key(key('i'));
    ed.handle_key(key_end());
    assert_eq!(state(&ed), "abc def-[\n]>");
}

#[test]
fn text_typed_after_end_in_insert_mode_is_appended() {
    let mut ed = editor_from("-[a]>bc def\n");
    ed.handle_key(key('i'));
    ed.handle_key(key_end());
    ed.handle_key(key('X'));
    ed.handle_key(key_esc());
    assert_eq!(ed.doc().text().to_string(), "abc defX\n");
}

#[test]
fn end_in_insert_mode_moves_every_cursor_to_its_own_line_break() {
    let mut ed = editor_from("-{a}>b\n-[\n]>c-[d]>e\n");
    ed.handle_key(key('i'));
    ed.handle_key(key_end());
    assert_eq!(state(&ed), "ab-{\n}>-[\n]>cde-[\n]>");
}

#[test]
fn end_in_normal_mode_stays_on_the_last_character() {
    let mut ed = editor_from("-[a]>bc def\n");
    ed.handle_key(key_end());
    assert_eq!(state(&ed), "abc de-[f]>\n");
}

#[test]
fn up_in_insert_mode_from_a_line_end_lands_on_the_shorter_line_break() {
    let mut ed = editor_from("ab\n-[a]>bcdef\n");
    ed.handle_key(key('A'));
    ed.handle_key(key_up());
    assert_eq!(state(&ed), "ab-[\n]>abcdef\n");
}

#[test]
fn down_in_insert_mode_keeps_a_column_inside_the_text() {
    let mut ed = editor_from("-[a]>b\nabcdef\n");
    ed.handle_key(key('A'));
    ed.handle_key(key_down());
    assert_eq!(state(&ed), "ab\nab-[c]>def\n");
}

#[test]
fn up_in_normal_mode_from_a_longer_line_stays_on_the_last_character() {
    let mut ed = editor_from("ab\nabcde-[f]>\n");
    ed.handle_key(key('k'));
    assert_eq!(state(&ed), "a-[b]>\nabcdef\n");
}
