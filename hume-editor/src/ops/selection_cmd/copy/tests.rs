use super::*;
use crate::assert_state;
use crate::testing::parse_state;
use pretty_assertions::assert_eq;

// ── cmd_copy_selection_on_next_line ────────────────────────────────────

#[test]
fn copy_cursor_to_next_line() {
    // "foo\nbar\n" — cursor at column 1 of line 0 ('o').
    // Copy should land at column 1 of line 1 ('a').
    let (buf, sels) = parse_state("f-[o]>o\nbar\n");
    let sels_out = cmd_copy_selection_on_next_line(&buf, sels, 0, MotionMode::Move);
    assert_eq!(buf.to_string(), "foo\nbar\n"); // buffer unchanged
    assert_eq!(sels_out.len(), 2);
    // Original cursor at offset 1 stays.
    // New cursor at offset 5 (line 1, col 1: 'a' is at 4, 'b' at 4...
    // "foo\n" = offsets 0-3, "bar\n" = offsets 4-7. Col 1 = offset 5.
    let heads: Vec<usize> = sels_out.iter_sorted().map(|s| s.head()).collect();
    assert!(
        heads.contains(&1),
        "original cursor should remain at col 1 of line 0"
    );
    assert!(
        heads.contains(&5),
        "new cursor should be at col 1 of line 1"
    );
    // Primary should be the new copy (the one on line 1).
    assert_eq!(sels_out.primary().head(), 5);
}

#[test]
fn copy_to_next_line_on_last_line_is_noop() {
    // Cursor on the last real line — nothing to copy to.
    let (buf, sels) = parse_state("foo\nb-[a]>r\n");
    let sels_out = cmd_copy_selection_on_next_line(&buf, sels, 0, MotionMode::Move);
    assert_eq!(sels_out.len(), 1); // no copy added
    assert_eq!(sels_out.primary().head(), 5); // cursor unchanged
}

#[test]
fn copy_to_next_line_clamps_column() {
    // "hello\nhi\n" — cursor at column 4 of line 0.
    // Line 1 is "hi\n" (only 2 real chars). Should clamp to last char 'i'.
    let (buf, sels) = parse_state("hell-[o]>\nhi\n");
    let sels_out = cmd_copy_selection_on_next_line(&buf, sels, 0, MotionMode::Move);
    assert_eq!(sels_out.len(), 2);
    // The copy should land at the last char of "hi" = offset 7.
    // "hello\n" = offsets 0-5, "hi\n" = offsets 6-8.
    // Last non-\n char = 'i' at offset 7.
    let copy = sels_out.primary();
    assert_eq!(copy.head(), 7);
}

#[test]
fn copy_next_backward_selection() {
    // Backward selection on line 0: anchor=2('o'), head=0('f') — selects "foo" (3 chars).
    // Copy down: both endpoints shift to line 1 preserving column.
    // "foo\nbar\n": f(0),o(1),o(2),\n(3),b(4),a(5),r(6),\n(7).
    // anchor col=2 → line 1 col 2 = offset 6 ('r'). head col=0 → offset 4 ('b').
    let (buf, sels) = parse_state("<[foo]-\nbar\n");
    let sels_out = cmd_copy_selection_on_next_line(&buf, sels, 0, MotionMode::Move);
    assert_eq!(sels_out.len(), 2);
    // The copy (primary) should be backward: anchor=6, head=4.
    let copy = sels_out.primary();
    assert!(
        copy.anchor() > copy.head(),
        "copy should preserve backward direction"
    );
    assert_eq!(copy.head(), 4); // 'b' at col 0 of line 1
    assert_eq!(copy.anchor(), 6); // 'r' at col 2 of line 1
}

#[test]
fn copy_next_multiple_cursors() {
    // Two cursors on line 0 at cols 1 and 2. Both get copied to line 1.
    // "foo\nbar\n": f(0),o(1),o(2),\n(3),b(4),a(5),r(6),\n(7).
    // Col 1 → offset 5 ('a'), col 2 → offset 6 ('r').
    let (buf, sels) = parse_state("f-[o]>-[o]>\nbar\n");
    let sels_out = cmd_copy_selection_on_next_line(&buf, sels, 0, MotionMode::Move);
    assert_eq!(sels_out.len(), 4); // 2 originals + 2 copies
    let heads: Vec<usize> = sels_out.iter_sorted().map(|s| s.head()).collect();
    assert!(heads.contains(&1)); // original col 1
    assert!(heads.contains(&2)); // original col 2
    assert!(heads.contains(&5)); // copy of col 1 on line 1
    assert!(heads.contains(&6)); // copy of col 2 on line 1
}

#[test]
fn copy_next_line_count_3() {
    // repeat(3, ...) copies the cursor to 3 consecutive lines below.
    // Text: "a\nb\nc\nd\ne\n". Cursor on 'a'(0).
    // After 3 copies: cursors on 'a'(0), 'b'(2), 'c'(4), 'd'(6).
    use crate::ops::edit::repeat;
    assert_state!(
        "-[a]>\nb\nc\nd\ne\n",
        |(buf, sels)| repeat(3, &buf, sels, |b, s| cmd_copy_selection_on_next_line(
            b,
            s,
            0,
            MotionMode::Move
        )),
        "-[a]>\n-[b]>\n-[c]>\n-[d]>\ne\n"
    );
}

#[test]
fn copy_next_line_range_selection() {
    // Forward range selection covering "hello" (0..4). Copy to next line:
    // anchor=6 ('w'), head=10 ('d') — selecting "world". Both selections exist.
    assert_state!(
        "-[hello]>\nworld\n",
        |(buf, sels)| cmd_copy_selection_on_next_line(&buf, sels, 0, MotionMode::Move),
        "-[hello]>\n-[world]>\n"
    );
}

// ── cmd_copy_selection_on_prev_line ────────────────────────────────────

#[test]
fn copy_cursor_to_prev_line() {
    // Cursor at column 1 of line 1 ('a' in "bar"). Copy goes to line 0.
    let (buf, sels) = parse_state("foo\nb-[a]>r\n");
    let sels_out = cmd_copy_selection_on_prev_line(&buf, sels, 0, MotionMode::Move);
    assert_eq!(sels_out.len(), 2);
    // Original at offset 5 (line 1, col 1). New at offset 1 (line 0, col 1).
    let heads: Vec<usize> = sels_out.iter_sorted().map(|s| s.head()).collect();
    assert!(heads.contains(&5), "original cursor should remain");
    assert!(
        heads.contains(&1),
        "new cursor should be at col 1 of line 0"
    );
    // Primary is the new copy (on line 0).
    assert_eq!(sels_out.primary().head(), 1);
}

#[test]
fn copy_to_prev_line_on_first_line_is_noop() {
    let (buf, sels) = parse_state("f-[o]>o\nbar\n");
    let sels_out = cmd_copy_selection_on_prev_line(&buf, sels, 0, MotionMode::Move);
    assert_eq!(sels_out.len(), 1); // no copy added
}

#[test]
fn copy_to_prev_line_clamps_column() {
    // "hi\nhello\n" — cursor at column 4 of line 1 ('o').
    // Line 0 is "hi\n" (only 2 real chars). Should clamp to last char 'i'.
    // "hi\n" = offsets 0-2, "hello\n" = offsets 3-8.
    // Cursor at col 4 of line 1 = offset 3+4 = 7 ('o').
    let (buf, sels) = parse_state("hi\nhell-[o]>\n");
    let sels_out = cmd_copy_selection_on_prev_line(&buf, sels, 0, MotionMode::Move);
    assert_eq!(sels_out.len(), 2);
    // Copy should land at last char of "hi" = 'i' at offset 1.
    assert_eq!(sels_out.primary().head(), 1);
}
