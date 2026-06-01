use editing::selection::{Selection, SelectionSet};
use editing::text::Text;
use editing::lines::{line_content_end, line_end_exclusive, snap_to_grapheme_boundary};
use crate::ops::MotionMode;

// ── Vertical copy ─────────────────────────────────────────────────────────────

/// Duplicate each selection one line down and add it to the selection set.
///
/// The copy preserves the column offsets of both `anchor` and `head`,
/// clamped to the length of the target line and snapped to a grapheme
/// boundary. If the target line does not exist (i.e., the selection's
/// bottommost line is the last real line), no copy is added for that
/// selection.
///
/// The primary advances to the newly added copy of the original primary. If
/// no copy was added (last-line edge case) the primary stays on the original.
pub(crate) fn cmd_copy_selection_on_next_line(
    buf: &Text,
    sels: SelectionSet,
    _mode: MotionMode,
) -> SelectionSet {
    copy_selection_vertically(buf, sels, 1)
}

/// Duplicate each selection one line up and add it to the selection set.
///
/// Mirror of [`cmd_copy_selection_on_next_line`] — shifts up instead of down.
pub(crate) fn cmd_copy_selection_on_prev_line(
    buf: &Text,
    sels: SelectionSet,
    _mode: MotionMode,
) -> SelectionSet {
    copy_selection_vertically(buf, sels, -1)
}

/// Core implementation for copy-to-next/prev-line. `direction` is `1` for
/// down and `-1` for up.
fn copy_selection_vertically(buf: &Text, sels: SelectionSet, direction: isize) -> SelectionSet {
    let primary_idx = sels.primary_index();
    // Collect originals into `all_sels`. Copies are appended below.
    let mut all_sels: Vec<Selection> = sels.iter_sorted().copied().collect();
    let original_len = all_sels.len();
    // Index in `all_sels` for the copy of the old primary, if one was added.
    let mut primary_copy_idx: Option<usize> = None;

    for i in 0..original_len {
        let sel = all_sels[i];
        let anchor_line = buf.char_to_line(sel.anchor()) as isize;
        let head_line = buf.char_to_line(sel.head()) as isize;

        // The outermost line in the copy direction determines the offset target.
        let outer_line = if direction > 0 {
            anchor_line.max(head_line) // bottommost for "down"
        } else {
            anchor_line.min(head_line) // topmost for "up"
        };
        let target_outer = outer_line + direction;

        if target_outer < 0 {
            continue; // would go before the start of the buffer
        }
        let target_outer = target_outer as usize;

        // The phantom trailing line (line_to_char == len_chars) has no content.
        if buf.line_to_char(target_outer) >= buf.len_chars() {
            continue;
        }

        // Shift each endpoint by the same delta.
        let delta = target_outer as isize - outer_line;

        let new_anchor = column_on_shifted_line(buf, sel.anchor(), anchor_line as usize, delta);
        let new_head = column_on_shifted_line(buf, sel.head(), head_line as usize, delta);

        let new_sel = Selection::new(new_anchor, new_head);

        if i == primary_idx {
            primary_copy_idx = Some(all_sels.len());
        }
        all_sels.push(new_sel);
    }

    let desired_primary = primary_copy_idx.unwrap_or(primary_idx);
    let new_set = SelectionSet::from_vec(all_sels, desired_primary);
    new_set.debug_assert_valid(buf);
    new_set
}

/// Return the position that `anchor_or_head` would land on after shifting its
/// line by `delta` lines, preserving the char-offset column and clamping to
/// the target line's content.
fn column_on_shifted_line(buf: &Text, pos: usize, pos_line: usize, delta: isize) -> usize {
    let col = pos - buf.line_to_char(pos_line);
    let target_line = (pos_line as isize + delta) as usize;
    place_column(buf, target_line, col)
}

/// Place the cursor at `col` chars from the start of `line`, clamping to the
/// last content character and snapping to a grapheme boundary.
fn place_column(buf: &Text, line: usize, col: usize) -> usize {
    let line_start = buf.line_to_char(line);
    let end_excl = line_end_exclusive(buf, line);
    let target = line_start + col;

    if target >= end_excl {
        // Column overshoots — clamp to the last content char on the line.
        line_content_end(buf, line)
    } else {
        snap_to_grapheme_boundary(buf, line_start, target)
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
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
        let sels_out = cmd_copy_selection_on_next_line(&buf, sels, MotionMode::Move);
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
        let sels_out = cmd_copy_selection_on_next_line(&buf, sels, MotionMode::Move);
        assert_eq!(sels_out.len(), 1); // no copy added
        assert_eq!(sels_out.primary().head(), 5); // cursor unchanged
    }

    #[test]
    fn copy_to_next_line_clamps_column() {
        // "hello\nhi\n" — cursor at column 4 of line 0.
        // Line 1 is "hi\n" (only 2 real chars). Should clamp to last char 'i'.
        let (buf, sels) = parse_state("hell-[o]>\nhi\n");
        let sels_out = cmd_copy_selection_on_next_line(&buf, sels, MotionMode::Move);
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
        let sels_out = cmd_copy_selection_on_next_line(&buf, sels, MotionMode::Move);
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
        let sels_out = cmd_copy_selection_on_next_line(&buf, sels, MotionMode::Move);
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
            |(buf, sels)| cmd_copy_selection_on_next_line(&buf, sels, MotionMode::Move),
            "-[hello]>\n-[world]>\n"
        );
    }

    // ── cmd_copy_selection_on_prev_line ────────────────────────────────────

    #[test]
    fn copy_cursor_to_prev_line() {
        // Cursor at column 1 of line 1 ('a' in "bar"). Copy goes to line 0.
        let (buf, sels) = parse_state("foo\nb-[a]>r\n");
        let sels_out = cmd_copy_selection_on_prev_line(&buf, sels, MotionMode::Move);
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
        let sels_out = cmd_copy_selection_on_prev_line(&buf, sels, MotionMode::Move);
        assert_eq!(sels_out.len(), 1); // no copy added
    }

    #[test]
    fn copy_to_prev_line_clamps_column() {
        // "hi\nhello\n" — cursor at column 4 of line 1 ('o').
        // Line 0 is "hi\n" (only 2 real chars). Should clamp to last char 'i'.
        // "hi\n" = offsets 0-2, "hello\n" = offsets 3-8.
        // Cursor at col 4 of line 1 = offset 3+4 = 7 ('o').
        let (buf, sels) = parse_state("hi\nhell-[o]>\n");
        let sels_out = cmd_copy_selection_on_prev_line(&buf, sels, MotionMode::Move);
        assert_eq!(sels_out.len(), 2);
        // Copy should land at last char of "hi" = 'i' at offset 1.
        assert_eq!(sels_out.primary().head(), 1);
    }
}
