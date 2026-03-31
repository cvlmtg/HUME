use regex_cursor::engines::meta::Regex;

use crate::core::buffer::Buffer;
use crate::core::grapheme::{next_grapheme_boundary, prev_grapheme_boundary};
use crate::helpers::{classify_char, line_content_end, line_end_exclusive, snap_to_grapheme_boundary, CharClass};
use crate::core::selection::{Selection, SelectionSet};
use crate::ops::search::find_matches_in_range;

// ── Simple selection-set commands ─────────────────────────────────────────────

/// Collapse every selection to a cursor at its `head`.
///
/// `anchor` becomes equal to `head` — the selected range shrinks to a single
/// character (the cursor position). Uses `map_and_merge` because two
/// overlapping selections with different heads might collapse to the same
/// position and need to be merged.
pub(crate) fn cmd_collapse_selection(buf: &Buffer, sels: SelectionSet) -> SelectionSet {
    let new_sels = sels.map_and_merge(|s| Selection::cursor(s.head));
    new_sels.debug_assert_valid(buf);
    new_sels
}

/// Swap `anchor` and `head` on every selection.
///
/// A forward selection (anchor ≤ head) becomes backward, and vice versa.
/// Does not change any range bounds, so overlaps cannot arise — uses plain
/// `map` (no merge needed).
pub(crate) fn cmd_flip_selections(buf: &Buffer, sels: SelectionSet) -> SelectionSet {
    // `flip` only swaps anchor/head — no range change → no new overlaps.
    let new_sels = sels.map(|s| s.flip());
    new_sels.debug_assert_valid(buf);
    new_sels
}

/// Keep only the primary selection; drop all others.
///
/// The result is a single-selection set. This is a destructive reduction —
/// any non-primary cursors or ranges are lost.
pub(crate) fn cmd_keep_primary_selection(buf: &Buffer, sels: SelectionSet) -> SelectionSet {
    let new_sels = sels.keep_primary();
    new_sels.debug_assert_valid(buf);
    new_sels
}

/// Remove the primary selection and advance the primary to the next one.
///
/// If there is only one selection, this is a no-op (the set can never be
/// empty). After removal the primary wraps to the start if it was the last
/// selection in document order.
pub(crate) fn cmd_remove_primary_selection(buf: &Buffer, sels: SelectionSet) -> SelectionSet {
    let idx = sels.primary_index();
    let new_sels = sels.remove(idx); // no-op when len == 1
    new_sels.debug_assert_valid(buf);
    new_sels
}

/// Move the primary selection to the next one in document order, wrapping.
pub(crate) fn cmd_cycle_primary_forward(buf: &Buffer, sels: SelectionSet) -> SelectionSet {
    let new_sels = sels.cycle_primary(1);
    new_sels.debug_assert_valid(buf);
    new_sels
}

/// Move the primary selection to the previous one in document order, wrapping.
pub(crate) fn cmd_cycle_primary_backward(buf: &Buffer, sels: SelectionSet) -> SelectionSet {
    let new_sels = sels.cycle_primary(-1);
    new_sels.debug_assert_valid(buf);
    new_sels
}

// ── Buffer-aware selection commands ───────────────────────────────────────────

/// Split each multi-line selection into one selection per line.
///
/// Single-line selections are left unchanged. For a selection spanning lines
/// L1..L2:
/// - Line L1: from the selection's start to the last non-`\n` char on L1
///   (or the `\n` itself if the line is empty).
/// - Lines L1+1..L2-1: full lines from start to last non-`\n` char.
/// - Line L2: from the line start to the selection's end.
///
/// The direction (forward/backward) of the original selection is preserved on
/// every piece. The primary becomes the first piece of the original primary.
pub(crate) fn cmd_split_selection_on_newlines(
    buf: &Buffer,
    sels: SelectionSet,
) -> SelectionSet {
    let primary_idx = sels.primary_index();
    let mut new_sels: Vec<Selection> = Vec::new();
    // Maps each old selection (by sorted index) to the first index of its
    // pieces in `new_sels`.
    let mut piece_start: Vec<usize> = Vec::new();

    for sel in sels.iter_sorted() {
        let start = sel.start();
        let end = sel.end();
        let start_line = buf.char_to_line(start);
        let end_line = buf.char_to_line(end);
        let forward = sel.anchor <= sel.head;

        let first_piece_idx = new_sels.len();

        if start_line == end_line {
            // Single-line: keep as-is.
            new_sels.push(*sel);
        } else {
            // First line piece: from selection start to end of line content.
            let first_end = line_content_end(buf, start_line);
            let sel = Selection::directed(start, first_end, forward);
            new_sels.push(sel);

            // Middle lines: full lines.
            for line in (start_line + 1)..end_line {
                let ls = buf.line_to_char(line);
                let le = line_content_end(buf, line);
                let sel = Selection::directed(ls, le, forward);
                new_sels.push(sel);
            }

            // Last line piece: from line start to selection end.
            let last_ls = buf.line_to_char(end_line);
            let sel = Selection::directed(last_ls, end, forward);
            new_sels.push(sel);
        }

        piece_start.push(first_piece_idx);
    }

    // The new primary is the first piece of the original primary.
    let new_primary = piece_start[primary_idx];
    // Split selections cover disjoint line ranges and can't overlap, so no
    // merge is needed. `from_vec` preserves the sorted order we built.
    let new_set = SelectionSet::from_vec(new_sels, new_primary);
    new_set.debug_assert_valid(buf);
    new_set
}

// ── Select matches within ────────────────────────────────────────────────────

/// Replace each selection with the regex matches found within it.
///
/// For every selection in `sels`, finds all non-overlapping matches of `regex`
/// bounded to that selection's range. Each match becomes a new forward
/// `Selection`. The new primary is the first match within the original primary
/// selection's range.
///
/// Returns `None` when no matches are found in any selection — the caller
/// should keep the original selections unchanged.
pub(crate) fn select_matches_within(
    buf: &Buffer,
    sels: &SelectionSet,
    regex: &Regex,
) -> Option<SelectionSet> {
    let primary_idx = sels.primary_index();
    let mut new_sels: Vec<Selection> = Vec::new();
    let mut new_primary = 0;

    for (i, sel) in sels.iter_sorted().enumerate() {
        let piece_start = new_sels.len();
        let matches = find_matches_in_range(buf, regex, sel.start(), sel.end_inclusive(buf));

        for (s, e) in matches {
            new_sels.push(Selection::new(s, e));
        }

        // Primary = first match within the original primary selection.
        if i == primary_idx && piece_start < new_sels.len() {
            new_primary = piece_start;
        }
    }

    if new_sels.is_empty() {
        return None;
    }

    // Matches within non-overlapping selections can't overlap each other,
    // so no merge is needed.
    let new_set = SelectionSet::from_vec(new_sels, new_primary);
    new_set.debug_assert_valid(buf);
    Some(new_set)
}

// ── Trim whitespace ──────────────────────────────────────────────────────────

/// Trim leading and trailing whitespace from every selection's range.
///
/// "Whitespace" here means space (` `), tab (`\t`), and newline (`\n`). The
/// range shrinks inward until both ends sit on non-whitespace characters. If
/// the entire selection is whitespace the selection collapses to a cursor at
/// the original `head`.
pub(crate) fn cmd_trim_selection_whitespace(
    buf: &Buffer,
    sels: SelectionSet,
) -> SelectionSet {
    let new_sels = sels.map_and_merge(|sel| {
        let mut start = sel.start();
        let end = sel.end();
        let forward = sel.anchor <= sel.head;

        // Walk forward from start, skipping whitespace (grapheme boundary steps).
        // `classify_char` is the authoritative whitespace definition for this
        // codebase — Space covers ' '/'\t', Eol covers '\n'.
        while start <= end
            && matches!(buf.char_at(start).map(classify_char), Some(CharClass::Space | CharClass::Eol))
        {
            start = next_grapheme_boundary(buf, start);
        }

        // If we consumed everything, the selection is all whitespace.
        if start > end {
            return Selection::cursor(sel.head);
        }

        // Walk backward from end, skipping whitespace (grapheme boundary steps).
        let mut new_end = end;
        while new_end > start
            && matches!(buf.char_at(new_end).map(classify_char), Some(CharClass::Space | CharClass::Eol))
        {
            new_end = prev_grapheme_boundary(buf, new_end);
        }

        Selection::directed(start, new_end, forward)
    });
    new_sels.debug_assert_valid(buf);
    new_sels
}

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
    buf: &Buffer,
    sels: SelectionSet,
) -> SelectionSet {
    copy_selection_vertically(buf, sels, 1)
}

/// Duplicate each selection one line up and add it to the selection set.
///
/// Mirror of [`cmd_copy_selection_on_next_line`] — shifts up instead of down.
pub(crate) fn cmd_copy_selection_on_prev_line(
    buf: &Buffer,
    sels: SelectionSet,
) -> SelectionSet {
    copy_selection_vertically(buf, sels, -1)
}

// ── Shared helpers ────────────────────────────────────────────────────────────

/// Core implementation for copy-to-next/prev-line. `direction` is `1` for
/// down and `-1` for up.
fn copy_selection_vertically(buf: &Buffer, sels: SelectionSet, direction: isize) -> SelectionSet {
    let primary_idx = sels.primary_index();
    // Collect originals into `all_sels`. Copies are appended below.
    let mut all_sels: Vec<Selection> = sels.iter_sorted().copied().collect();
    let original_len = all_sels.len();
    // Index in `all_sels` for the copy of the old primary, if one was added.
    let mut primary_copy_idx: Option<usize> = None;

    for i in 0..original_len {
        let sel = all_sels[i];
        let anchor_line = buf.char_to_line(sel.anchor) as isize;
        let head_line = buf.char_to_line(sel.head) as isize;

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

        let new_anchor = column_on_shifted_line(buf, sel.anchor, anchor_line as usize, delta);
        let new_head = column_on_shifted_line(buf, sel.head, head_line as usize, delta);

        let new_sel = Selection::new(new_anchor, new_head);

        if i == primary_idx {
            primary_copy_idx = Some(all_sels.len());
        }
        all_sels.push(new_sel);
    }

    let desired_primary = primary_copy_idx.unwrap_or(primary_idx);
    let new_set = SelectionSet::from_vec(all_sels, desired_primary).merge_overlapping();
    new_set.debug_assert_valid(buf);
    new_set
}

/// Return the position that `anchor_or_head` would land on after shifting its
/// line by `delta` lines, preserving the char-offset column and clamping to
/// the target line's content.
fn column_on_shifted_line(
    buf: &Buffer,
    pos: usize,
    pos_line: usize,
    delta: isize,
) -> usize {
    let col = pos - buf.line_to_char(pos_line);
    let target_line = (pos_line as isize + delta) as usize;
    place_column(buf, target_line, col)
}

/// Place the cursor at `col` chars from the start of `line`, clamping to the
/// last content character and snapping to a grapheme boundary.
fn place_column(buf: &Buffer, line: usize, col: usize) -> usize {
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

    // ── cmd_collapse_selection ─────────────────────────────────────────────

    #[test]
    fn collapse_cursor_is_noop() {
        // A cursor (anchor == head) collapsing to itself — no change.
        assert_state!("-[h]>ello\n", |(buf, sels)| cmd_collapse_selection(&buf, sels), "-[h]>ello\n");
    }

    #[test]
    fn collapse_forward_selection() {
        assert_state!(
            "-[hell]>o\n",
            |(buf, sels)| cmd_collapse_selection(&buf, sels),
            // head was at 'l' (offset 3)
            "hel-[l]>o\n"
        );
    }

    #[test]
    fn collapse_backward_selection() {
        // Backward: anchor=3, head=0, selects "hell" (4 chars). Collapses to cursor at head=0.
        assert_state!(
            "<[hell]-o\n",
            |(buf, sels)| cmd_collapse_selection(&buf, sels),
            "-[h]>ello\n"
        );
    }

    #[test]
    fn collapse_merges_coincident_heads() {
        // Two cursors at different positions stay separate after collapse —
        // they only merge if their heads land on the exact same position.
        let (buf, sels) = parse_state("-[h]>el-[l]>o\n");
        let result = cmd_collapse_selection(&buf, sels);
        assert_eq!(result.len(), 2); // still 2 — they don't converge
    }

    // ── cmd_flip_selections ────────────────────────────────────────────────

    #[test]
    fn flip_forward_becomes_backward() {
        // Forward: anchor=0, head=3, selects "hell". After flip: anchor=3, head=0.
        assert_state!(
            "-[hell]>o\n",
            |(buf, sels)| cmd_flip_selections(&buf, sels),
            "<[hell]-o\n"
        );
    }

    #[test]
    fn flip_backward_becomes_forward() {
        // Backward: anchor=3, head=0, selects "hell". After flip: anchor=0, head=3.
        assert_state!(
            "<[hell]-o\n",
            |(buf, sels)| cmd_flip_selections(&buf, sels),
            "-[hell]>o\n"
        );
    }

    #[test]
    fn flip_cursor_is_noop() {
        // anchor == head → flip does nothing observable.
        assert_state!("-[h]>ello\n", |(buf, sels)| cmd_flip_selections(&buf, sels), "-[h]>ello\n");
    }

    #[test]
    fn flip_is_involution() {
        // Flipping twice is the identity.
        assert_state!(
            "-[hell]>o\n",
            |(buf, sels)| {
                let sels = cmd_flip_selections(&buf, sels);
                cmd_flip_selections(&buf, sels)
            },
            "-[hell]>o\n"
        );
    }

    // ── cmd_keep_primary_selection ─────────────────────────────────────────

    #[test]
    fn keep_primary_drops_all_others() {
        // Three cursors; primary (first yielded by DSL) is at 0. Others dropped.
        assert_state!(
            "-[h]>el-[l]>-[o]>\n",
            |(buf, sels)| cmd_keep_primary_selection(&buf, sels),
            "-[h]>ello\n"
        );
    }

    #[test]
    fn keep_primary_single_unchanged() {
        assert_state!(
            "-[hell]>o\n",
            |(buf, sels)| cmd_keep_primary_selection(&buf, sels),
            "-[hell]>o\n"
        );
    }

    // ── cmd_remove_primary_selection ───────────────────────────────────────

    #[test]
    fn remove_primary_single_is_noop() {
        assert_state!(
            "-[h]>ello\n",
            |(buf, sels)| cmd_remove_primary_selection(&buf, sels),
            "-[h]>ello\n"
        );
    }

    #[test]
    fn remove_primary_two_selections() {
        // Two cursors at 0 and 4. Primary is first (index 0).
        // After removal: only the cursor at 4 remains, becomes primary.
        assert_state!(
            "-[h]>ell-[o]>\n",
            |(buf, sels)| cmd_remove_primary_selection(&buf, sels),
            "hell-[o]>\n"
        );
    }

    // ── cmd_cycle_primary_forward ──────────────────────────────────────────

    #[test]
    fn cycle_forward_advances_primary() {
        // Three cursors. After cycling forward, primary should be the next one.
        let (buf, sels) = parse_state("-[h]>el-[l]>o\n"); // two cursors, primary at 0
        assert_eq!(sels.primary().head, 0);
        let sels = cmd_cycle_primary_forward(&buf, sels);
        assert_eq!(sels.primary().head, 3);
        // Cycle again — wraps back to first.
        let sels = cmd_cycle_primary_forward(&buf, sels);
        assert_eq!(sels.primary().head, 0);
    }

    // ── cmd_cycle_primary_backward ─────────────────────────────────────────

    #[test]
    fn cycle_backward_wraps_to_last() {
        let (buf, sels) = parse_state("-[h]>el-[l]>o\n"); // primary at 0
        let sels = cmd_cycle_primary_backward(&buf, sels);
        assert_eq!(sels.primary().head, 3); // wraps to last
    }

    // ── cmd_split_selection_on_newlines ────────────────────────────────────

    #[test]
    fn split_single_line_is_noop() {
        assert_state!(
            "-[hell]>o\n",
            |(buf, sels)| cmd_split_selection_on_newlines(&buf, sels),
            "-[hell]>o\n"
        );
    }

    #[test]
    fn split_two_line_selection() {
        // "foo\nbar\n", selection from 'f'(0) to 'r'(6) (cross-line forward).
        // "#[foo\nba|r]#\n" → anchor=0, head=6 (cursor on 'r').
        // After split: "foo" on line 0, "bar" on line 1.
        let (buf, sels) = parse_state("-[foo\nbar]>\n");
        let sels_out = cmd_split_selection_on_newlines(&buf, sels);
        // Buffer unchanged (pure op).
        assert_eq!(buf.to_string(), "foo\nbar\n");
        // Two selections.
        assert_eq!(sels_out.len(), 2);
        let s: Vec<_> = sels_out.iter_sorted().copied().collect();
        // First: covers "foo" on line 0 (offsets 0–2).
        assert_eq!(s[0].start(), 0);
        assert_eq!(s[0].end(), 2);
        // Second: covers "bar" on line 1 (offsets 4–6).
        assert_eq!(s[1].start(), 4);
        assert_eq!(s[1].end(), 6);
        // Primary is first piece of original primary (index 0).
        assert_eq!(sels_out.primary_index(), 0);
    }

    #[test]
    fn split_three_line_selection() {
        // "a\nb\nc\n" — forward selection from 'a' to 'c'.
        let (buf, sels) = parse_state("-[a\nb\nc]>\n");
        let sels_out = cmd_split_selection_on_newlines(&buf, sels);
        assert_eq!(sels_out.len(), 3);
        let s: Vec<_> = sels_out.iter_sorted().copied().collect();
        // Line 0: just 'a' at offset 0.
        assert_eq!(s[0].start(), 0);
        assert_eq!(s[0].end(), 0);
        // Line 1: just 'b' at offset 2.
        assert_eq!(s[1].start(), 2);
        assert_eq!(s[1].end(), 2);
        // Line 2: just 'c' at offset 4.
        assert_eq!(s[2].start(), 4);
        assert_eq!(s[2].end(), 4);
    }

    #[test]
    fn split_cursor_at_newline_is_noop() {
        // A cursor sitting on a newline character is a single-line selection
        // (the \n is part of its line).
        let (buf, sels) = parse_state("foo-[\n]>bar\n");
        let sels_out = cmd_split_selection_on_newlines(&buf, sels);
        assert_eq!(sels_out.len(), 1);
        assert_eq!(sels_out.primary().head, 3); // still on \n
    }

    // ── cmd_trim_selection_whitespace ──────────────────────────────────────

    #[test]
    fn trim_leading_spaces() {
        // "  hello\n", forward selection covering the whole word + leading spaces.
        // "#[  hell|o]#\n" → anchor=0, head=6 (cursor on 'o', offsets:  (0) (1) h(2) e(3) l(4) l(5) o(6)).
        // After trim: start advances past the 2 spaces → start=2, end=6.
        let (buf, sels) = parse_state("-[  hello]>\n");
        let sels_out = cmd_trim_selection_whitespace(&buf, sels);
        assert_eq!(sels_out.primary().start(), 2); // after the two spaces
        assert_eq!(sels_out.primary().end(), 6);   // 'o' at offset 6
    }

    #[test]
    fn trim_trailing_spaces() {
        // "hello  \n", forward selection covering "hello  " (with trailing spaces).
        // "#[hello | ]#\n" → anchor=0, head=6 (cursor on second space).
        // After trim: end walks back past 2 spaces → end=4 ('o').
        let (buf, sels) = parse_state("-[hello  ]>\n");
        let sels_out = cmd_trim_selection_whitespace(&buf, sels);
        assert_eq!(sels_out.primary().start(), 0);
        assert_eq!(sels_out.primary().end(), 4); // 'o' at offset 4
    }

    #[test]
    fn trim_all_whitespace_collapses_to_cursor_at_head() {
        // Selection covering only spaces — should collapse to cursor at head.
        let (buf, sels) = parse_state("-[    ]>\n");
        let sels_out = cmd_trim_selection_whitespace(&buf, sels);
        assert!(sels_out.primary().is_cursor());
        // Head was at offset 3 (the `|` position in DSL).
        assert_eq!(sels_out.primary().head, 3);
    }

    #[test]
    fn trim_no_whitespace_is_noop() {
        assert_state!(
            "-[hell]>o\n",
            |(buf, sels)| cmd_trim_selection_whitespace(&buf, sels),
            "-[hell]>o\n"
        );
    }

    // ── cmd_copy_selection_on_next_line ────────────────────────────────────

    #[test]
    fn copy_cursor_to_next_line() {
        // "foo\nbar\n" — cursor at column 1 of line 0 ('o').
        // Copy should land at column 1 of line 1 ('a').
        let (buf, sels) = parse_state("f-[o]>o\nbar\n");
        let sels_out = cmd_copy_selection_on_next_line(&buf, sels);
        assert_eq!(buf.to_string(), "foo\nbar\n"); // buffer unchanged
        assert_eq!(sels_out.len(), 2);
        // Original cursor at offset 1 stays.
        // New cursor at offset 5 (line 1, col 1: 'a' is at 4, 'b' at 4...
        // "foo\n" = offsets 0-3, "bar\n" = offsets 4-7. Col 1 = offset 5.
        let heads: Vec<usize> = sels_out.iter_sorted().map(|s| s.head).collect();
        assert!(heads.contains(&1), "original cursor should remain at col 1 of line 0");
        assert!(heads.contains(&5), "new cursor should be at col 1 of line 1");
        // Primary should be the new copy (the one on line 1).
        assert_eq!(sels_out.primary().head, 5);
    }

    #[test]
    fn copy_to_next_line_on_last_line_is_noop() {
        // Cursor on the last real line — nothing to copy to.
        let (buf, sels) = parse_state("foo\nb-[a]>r\n");
        let sels_out = cmd_copy_selection_on_next_line(&buf, sels);
        assert_eq!(sels_out.len(), 1); // no copy added
        assert_eq!(sels_out.primary().head, 5); // cursor unchanged
    }

    #[test]
    fn copy_to_next_line_clamps_column() {
        // "hello\nhi\n" — cursor at column 4 of line 0.
        // Line 1 is "hi\n" (only 2 real chars). Should clamp to last char 'i'.
        let (buf, sels) = parse_state("hell-[o]>\nhi\n");
        let sels_out = cmd_copy_selection_on_next_line(&buf, sels);
        assert_eq!(sels_out.len(), 2);
        // The copy should land at the last char of "hi" = offset 7.
        // "hello\n" = offsets 0-5, "hi\n" = offsets 6-8.
        // Last non-\n char = 'i' at offset 7.
        let copy = sels_out.primary();
        assert_eq!(copy.head, 7);
    }

    // ── cmd_copy_selection_on_prev_line ────────────────────────────────────

    #[test]
    fn copy_cursor_to_prev_line() {
        // Cursor at column 1 of line 1 ('a' in "bar"). Copy goes to line 0.
        let (buf, sels) = parse_state("foo\nb-[a]>r\n");
        let sels_out = cmd_copy_selection_on_prev_line(&buf, sels);
        assert_eq!(sels_out.len(), 2);
        // Original at offset 5 (line 1, col 1). New at offset 1 (line 0, col 1).
        let heads: Vec<usize> = sels_out.iter_sorted().map(|s| s.head).collect();
        assert!(heads.contains(&5), "original cursor should remain");
        assert!(heads.contains(&1), "new cursor should be at col 1 of line 0");
        // Primary is the new copy (on line 0).
        assert_eq!(sels_out.primary().head, 1);
    }

    #[test]
    fn copy_to_prev_line_on_first_line_is_noop() {
        let (buf, sels) = parse_state("f-[o]>o\nbar\n");
        let sels_out = cmd_copy_selection_on_prev_line(&buf, sels);
        assert_eq!(sels_out.len(), 1); // no copy added
    }

    #[test]
    fn copy_to_prev_line_clamps_column() {
        // "hi\nhello\n" — cursor at column 4 of line 1 ('o').
        // Line 0 is "hi\n" (only 2 real chars). Should clamp to last char 'i'.
        // "hi\n" = offsets 0-2, "hello\n" = offsets 3-8.
        // Cursor at col 4 of line 1 = offset 3+4 = 7 ('o').
        let (buf, sels) = parse_state("hi\nhell-[o]>\n");
        let sels_out = cmd_copy_selection_on_prev_line(&buf, sels);
        assert_eq!(sels_out.len(), 2);
        // Copy should land at last char of "hi" = 'i' at offset 1.
        assert_eq!(sels_out.primary().head, 1);
    }

    // ── additional collapse edge cases ─────────────────────────────────────

    #[test]
    fn collapse_empty_buffer() {
        assert_state!("-[\n]>", |(buf, sels)| cmd_collapse_selection(&buf, sels), "-[\n]>");
    }

    #[test]
    fn collapse_two_selections_same_head_merges() {
        // Two selections with different anchors but the same head collapse to
        // one cursor — map_and_merge must reduce the count.
        let buf = crate::core::buffer::Buffer::from("hello\n");
        let sels = crate::core::selection::SelectionSet::from_vec(
            vec![
                crate::core::selection::Selection::new(0, 3), // head at 3
                crate::core::selection::Selection::new(1, 3), // head at 3
            ],
            0,
        );
        let result = cmd_collapse_selection(&buf, sels);
        assert_eq!(result.len(), 1); // merged — both collapsed to cursor at 3
        assert_eq!(result.primary().head, 3);
    }

    // ── additional flip edge cases ─────────────────────────────────────────

    #[test]
    fn flip_multiple_selections() {
        // Two forward selections both flip to backward.
        assert_state!(
            "-[hell]>o -[worl]>d\n",
            |(buf, sels)| cmd_flip_selections(&buf, sels),
            "<[hell]-o <[worl]-d\n"
        );
    }

    // ── additional keep_primary edge cases ─────────────────────────────────

    #[test]
    fn keep_primary_when_primary_is_not_first() {
        // Cycle primary to the second cursor, then keep — should keep that one.
        let (buf, sels) = parse_state("-[h]>el-[l]>o\n"); // primary at index 0 (head=0)
        let sels = cmd_cycle_primary_forward(&buf, sels); // primary now at index 1 (head=3)
        let sels_out = cmd_keep_primary_selection(&buf, sels);
        assert_eq!(sels_out.len(), 1);
        assert_eq!(sels_out.primary().head, 3); // kept the second one
    }

    // ── additional remove_primary edge cases ───────────────────────────────

    #[test]
    fn remove_primary_at_end_wraps_to_first() {
        // Three cursors at 0, 3, 6. Cycle to last, then remove — should wrap
        // to the first remaining cursor (index 0 of the new set).
        let (buf, sels) = parse_state("-[h]>el-[l]>o-[\n]>"); // 3 cursors, primary at 0
        let sels = cmd_cycle_primary_backward(&buf, sels); // primary at last (head=6)
        let sels_out = cmd_remove_primary_selection(&buf, sels);
        assert_eq!(sels_out.len(), 2);
        assert_eq!(sels_out.primary().head, 0); // wrapped to first
    }

    // ── additional split edge cases ────────────────────────────────────────

    #[test]
    fn split_empty_line_in_middle() {
        // "foo\n\nbar\n" — selection from 'f'(0) to 'r'(7) spans 3 lines.
        // Line 0: "foo\n", line 1: "\n" (empty), line 2: "bar\n".
        // Middle piece should be a cursor on the lone '\n' at offset 4.
        let (buf, sels) = parse_state("-[foo\n\nbar]>\n");
        let sels_out = cmd_split_selection_on_newlines(&buf, sels);
        assert_eq!(sels_out.len(), 3);
        let s: Vec<_> = sels_out.iter_sorted().copied().collect();
        // Line 0: "foo" → offsets 0–2.
        assert_eq!(s[0].start(), 0);
        assert_eq!(s[0].end(), 2);
        // Line 1: empty → cursor on '\n' at offset 4.
        assert_eq!(s[1].start(), 4);
        assert_eq!(s[1].end(), 4);
        // Line 2: "bar" → offsets 5–7.
        assert_eq!(s[2].start(), 5);
        assert_eq!(s[2].end(), 7);
    }

    #[test]
    fn split_backward_multi_line_with_empty_line_preserves_direction() {
        // "foo\n\nbar\n" — backward selection spanning 3 lines including an
        // empty one. All 3 pieces must be backward, and the empty-line piece
        // must be a cursor on the '\n'.
        let (buf, sels) = parse_state("<[foo\n\nbar]-\n");
        let sels_out = cmd_split_selection_on_newlines(&buf, sels);
        assert_eq!(sels_out.len(), 3);
        let s: Vec<_> = sels_out.iter_sorted().copied().collect();
        // All pieces must be backward (anchor >= head; cursor is anchor == head).
        assert!(s[0].anchor >= s[0].head, "line 0 should be backward");
        assert!(s[1].anchor >= s[1].head, "empty line should be cursor/backward");
        assert!(s[2].anchor >= s[2].head, "line 2 should be backward");
        // Empty line: cursor on the lone '\n' at offset 4.
        assert_eq!(s[1].head, 4);
    }

    #[test]
    fn split_backward_multi_line_preserves_direction() {
        // "foo\nbar\n" — backward selection: anchor=6('r'), head=0('f').
        // Each piece should be backward (anchor > head).
        let (buf, sels) = parse_state("<[foo\nbar]-\n");
        let sels_out = cmd_split_selection_on_newlines(&buf, sels);
        assert_eq!(sels_out.len(), 2);
        let s: Vec<_> = sels_out.iter_sorted().copied().collect();
        // Both pieces should be backward selections.
        assert!(s[0].anchor > s[0].head, "line 0 piece should be backward");
        assert!(s[1].anchor > s[1].head, "line 1 piece should be backward");
    }

    // ── additional trim edge cases ─────────────────────────────────────────

    #[test]
    fn trim_tab_characters() {
        // "\thello\t\n" — selection from tab(0) to tab(6) inclusive.
        // After trim: start=1 ('h'), end=5 ('o').
        // "\thello\t\n": \t(0),h(1),e(2),l(3),l(4),o(5),\t(6),\n(7).
        let (buf, sels) = parse_state("-[\thello]>\t\n");
        let sels_out = cmd_trim_selection_whitespace(&buf, sels);
        assert_eq!(sels_out.primary().start(), 1); // past leading tab
        assert_eq!(sels_out.primary().end(), 5);   // 'o'
    }

    #[test]
    fn trim_backward_selection_preserves_direction() {
        // Backward selection covering "  hello\n": anchor=7('\n'), head=0.
        // After trim: spans 'h'(2) to 'o'(6), still backward.
        assert_state!(
            "<[  hello\n]-",
            |(buf, sels)| cmd_trim_selection_whitespace(&buf, sels),
            "  <[hello]-\n"
        );
    }

    #[test]
    fn trim_empty_buffer_collapses() {
        // Only char is '\n' (whitespace) — all-whitespace selection collapses.
        assert_state!("-[\n]>", |(buf, sels)| cmd_trim_selection_whitespace(&buf, sels), "-[\n]>");
    }

    // ── additional copy edge cases ─────────────────────────────────────────

    #[test]
    fn copy_next_backward_selection() {
        // Backward selection on line 0: anchor=2('o'), head=0('f') — selects "foo" (3 chars).
        // Copy down: both endpoints shift to line 1 preserving column.
        // "foo\nbar\n": f(0),o(1),o(2),\n(3),b(4),a(5),r(6),\n(7).
        // anchor col=2 → line 1 col 2 = offset 6 ('r'). head col=0 → offset 4 ('b').
        let (buf, sels) = parse_state("<[foo]-\nbar\n");
        let sels_out = cmd_copy_selection_on_next_line(&buf, sels);
        assert_eq!(sels_out.len(), 2);
        // The copy (primary) should be backward: anchor=6, head=4.
        let copy = sels_out.primary();
        assert!(copy.anchor > copy.head, "copy should preserve backward direction");
        assert_eq!(copy.head, 4);   // 'b' at col 0 of line 1
        assert_eq!(copy.anchor, 6); // 'r' at col 2 of line 1
    }

    #[test]
    fn copy_next_multiple_cursors() {
        // Two cursors on line 0 at cols 1 and 2. Both get copied to line 1.
        // "foo\nbar\n": f(0),o(1),o(2),\n(3),b(4),a(5),r(6),\n(7).
        // Col 1 → offset 5 ('a'), col 2 → offset 6 ('r').
        let (buf, sels) = parse_state("f-[o]>-[o]>\nbar\n");
        let sels_out = cmd_copy_selection_on_next_line(&buf, sels);
        assert_eq!(sels_out.len(), 4); // 2 originals + 2 copies
        let heads: Vec<usize> = sels_out.iter_sorted().map(|s| s.head).collect();
        assert!(heads.contains(&1)); // original col 1
        assert!(heads.contains(&2)); // original col 2
        assert!(heads.contains(&5)); // copy of col 1 on line 1
        assert!(heads.contains(&6)); // copy of col 2 on line 1
    }

    // ── repeat (count prefix for selection commands) ───────────────────────

    #[test]
    fn copy_next_line_count_3() {
        // repeat(3, ...) copies the cursor to 3 consecutive lines below.
        // Buffer: "a\nb\nc\nd\ne\n". Cursor on 'a'(0).
        // After 3 copies: cursors on 'a'(0), 'b'(2), 'c'(4), 'd'(6).
        use crate::ops::edit::repeat;
        assert_state!(
            "-[a]>\nb\nc\nd\ne\n",
            |(buf, sels)| repeat(3, &buf, sels, cmd_copy_selection_on_next_line),
            "-[a]>\n-[b]>\n-[c]>\n-[d]>\ne\n"
        );
    }

    // ── range selection copy ──────────────────────────────────────────────────

    #[test]
    fn copy_next_line_range_selection() {
        // Forward range selection covering "hello" (0..4). Copy to next line:
        // anchor=6 ('w'), head=10 ('d') — selecting "world". Both selections exist.
        assert_state!(
            "-[hello]>\nworld\n",
            |(buf, sels)| cmd_copy_selection_on_next_line(&buf, sels),
            "-[hello]>\n-[world]>\n"
        );
    }

    // ── split_on_newlines on empty buffer ─────────────────────────────────────

    #[test]
    fn split_selection_on_newlines_empty_buffer_is_noop() {
        // Empty buffer: cursor on the single structural '\n'. The cursor's
        // start_line == end_line → single-line branch → kept as-is.
        assert_state!(
            "-[\n]>",
            |(buf, sels)| cmd_split_selection_on_newlines(&buf, sels),
            "-[\n]>"
        );
    }

    // ── select_matches_within ─────────────────────────────────────────────

    #[test]
    fn select_matches_basic() {
        // Select "ab" within a selection that spans "aababab".
        let (buf, sels) = parse_state("-[aababab]>\n");
        let regex = regex_cursor::engines::meta::Regex::new("ab").unwrap();
        let result = select_matches_within(&buf, &sels, &regex).unwrap();
        // Expect 3 selections: (1,2), (3,4), (5,6)
        assert_eq!(result.len(), 3);
        assert_eq!((result.primary().anchor, result.primary().head), (1, 2));
    }

    #[test]
    fn select_matches_no_hits_returns_none() {
        let (buf, sels) = parse_state("-[hello]>\n");
        let regex = regex_cursor::engines::meta::Regex::new("xyz").unwrap();
        assert!(select_matches_within(&buf, &sels, &regex).is_none());
    }

    #[test]
    fn select_matches_bounded_to_selection() {
        // Only matches within the selection range should be found.
        // "ab" appears at (0,1) and (4,5) in "abcdab\n", but selection
        // covers only chars 2..3 ("cd") — no matches.
        let buf = Buffer::from("abcdab\n");
        let sels = SelectionSet::single(Selection::new(2, 3));
        let regex = regex_cursor::engines::meta::Regex::new("ab").unwrap();
        assert!(select_matches_within(&buf, &sels, &regex).is_none());
    }

    #[test]
    fn select_matches_multiple_selections() {
        // Two selections, each containing one "ab".
        let buf = Buffer::from("ab cd ab\n");
        let sel0 = Selection::new(0, 1); // "ab"
        let sel1 = Selection::new(6, 7); // "ab"
        let sels = SelectionSet::from_vec(vec![sel0, sel1], 0);
        let regex = regex_cursor::engines::meta::Regex::new("ab").unwrap();
        let result = select_matches_within(&buf, &sels, &regex).unwrap();
        assert_eq!(result.len(), 2);
    }

    #[test]
    fn select_matches_backward_selection() {
        // Backward selection (anchor > head) should work identically.
        let buf = Buffer::from("aababab\n");
        let sels = SelectionSet::single(Selection::new(6, 0)); // backward
        let regex = regex_cursor::engines::meta::Regex::new("ab").unwrap();
        let result = select_matches_within(&buf, &sels, &regex).unwrap();
        assert_eq!(result.len(), 3);
        assert_eq!((result.primary().anchor, result.primary().head), (1, 2));
    }

    #[test]
    fn select_matches_single_char_match() {
        // Single-char regex matches produce cursor-sized selections.
        let (buf, sels) = parse_state("-[abc]>\n");
        let regex = regex_cursor::engines::meta::Regex::new("b").unwrap();
        let result = select_matches_within(&buf, &sels, &regex).unwrap();
        assert_eq!(result.len(), 1);
        let sel = result.primary();
        assert_eq!(sel.anchor, 1);
        assert_eq!(sel.head, 1);
        assert!(sel.is_cursor());
    }

    #[test]
    fn select_matches_combining_grapheme() {
        // "café\n" where 'é' is e + U+0301 (2 codepoints at chars 3,4).
        // Selection covers the whole word. Matching "é" should produce a
        // selection spanning both codepoints (3,4).
        let buf = Buffer::from("caf\u{0065}\u{0301}\n");
        let sels = SelectionSet::single(Selection::new(0, 4));
        let regex = regex_cursor::engines::meta::Regex::new("\u{0065}\u{0301}").unwrap();
        let result = select_matches_within(&buf, &sels, &regex).unwrap();
        assert_eq!(result.len(), 1);
        assert_eq!((result.primary().anchor, result.primary().head), (3, 4));
    }
}
