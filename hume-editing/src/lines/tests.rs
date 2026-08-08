use super::*;
use crate::selection::testing::parse_state;

// ── is_line_start ─────────────────────────────────────────────────────────

#[test]
fn is_line_start_buffer_start() {
    // "hello\n" — char 0 is the buffer start, which is a line start.
    let (buf, _) = parse_state("-[h]>ello\n");
    assert!(is_line_start(&buf, &Selection::collapsed(0)));
}

#[test]
fn is_line_start_mid_line_is_false() {
    // "hello\n" — char 2 ('l') is not at a line start.
    let (buf, _) = parse_state("-[h]>ello\n");
    assert!(!is_line_start(&buf, &Selection::collapsed(2)));
}

#[test]
fn is_line_start_second_line_start() {
    // "hi\nbye\n" — line 1 starts at char 3 ('b').
    // h=0, i=1, \n=2, b=3, y=4, e=5, \n=6
    let (buf, _) = parse_state("-[h]>i\nbye\n");
    assert!(is_line_start(&buf, &Selection::collapsed(3)));
    // Verify a non-boundary on line 1 is false (independent oracle: char 4 = 'y').
    assert!(!is_line_start(&buf, &Selection::collapsed(4)));
}

#[test]
fn is_line_start_newline_itself_is_not_line_start() {
    // "hi\n" — the '\n' is at char 2, which is NOT the start of its line
    // (line 0 starts at char 0). This test verifies the function uses line
    // arithmetic rather than just checking the previous char.
    let (buf, _) = parse_state("-[h]>i\n");
    assert!(!is_line_start(&buf, &Selection::collapsed(2))); // '\n' at end of line 0
}
