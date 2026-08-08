use super::*;
use crate::test_support::rope;

#[test]
fn char_cursor_forward_from_start() {
    // "hello\n": h0 e1 l2 l3 o4 \n5.
    let buf = rope("hello");
    let got: Vec<(usize, char)> = chars_at(&buf, 0).collect();
    assert_eq!(
        got,
        vec![(0, 'h'), (1, 'e'), (2, 'l'), (3, 'l'), (4, 'o'), (5, '\n')]
    );
}

#[test]
fn char_cursor_forward_from_middle() {
    let buf = rope("hello");
    let got: Vec<(usize, char)> = chars_at(&buf, 2).collect();
    assert_eq!(got, vec![(2, 'l'), (3, 'l'), (4, 'o'), (5, '\n')]);
}

#[test]
fn char_cursor_prev_walks_back_to_start_then_none() {
    let buf = rope("hello");
    let mut c = chars_at(&buf, 4); // positioned before 'o'
    assert_eq!(c.prev(), Some((3, 'l')));
    assert_eq!(c.prev(), Some((2, 'l')));
    assert_eq!(c.prev(), Some((1, 'e')));
    assert_eq!(c.prev(), Some((0, 'h')));
    assert_eq!(c.prev(), None);
}

#[test]
fn char_cursor_interleaved_next_prev_round_trips() {
    let buf = rope("hello");
    let mut c = chars_at(&buf, 2);
    assert_eq!(c.next(), Some((2, 'l'))); // cursor now at 3
    assert_eq!(c.prev(), Some((2, 'l'))); // back to 2 — same value
    assert_eq!(c.next(), Some((2, 'l'))); // forward again — still consistent
}

#[test]
fn char_cursor_at_eof() {
    let buf = rope("hello");
    let len = buf.len_chars();
    let mut at_eof = chars_at(&buf, len);
    assert_eq!(at_eof.next(), None);
    assert_eq!(at_eof.prev(), Some((len - 1, '\n')));
}

#[test]
fn char_cursor_yields_codepoints_not_grapheme_clusters() {
    // "caf" + e + U+0301 (combining acute, 2 codepoints) + structural \n:
    // c0 a1 f2 e3 U+0301(4) \n(5). The combining mark must come back as
    // its own char, not merged with 'e' — CharCursor is char-level.
    let buf = rope("caf\u{0065}\u{0301}");
    let got: Vec<(usize, char)> = chars_at(&buf, 3).collect();
    assert_eq!(got, vec![(3, 'e'), (4, '\u{0301}'), (5, '\n')]);
}
