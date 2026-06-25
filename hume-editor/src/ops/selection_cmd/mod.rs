mod copy;
mod matching;

pub(crate) use copy::{cmd_copy_selection_on_next_line, cmd_copy_selection_on_prev_line};
pub(crate) use matching::{
    cmd_split_selection_on_newlines, cmd_trim_selection_whitespace, select_matches_within,
};

use super::MotionMode;
use hume_editing::selection::{Selection, SelectionSet};
use hume_editing::text::Text;

// ── Simple selection-set commands ─────────────────────────────────────────────

/// Collapse every selection to a cursor at its `head`.
///
/// `anchor` becomes equal to `head` — the selected range shrinks to a single
/// character (the cursor position). Uses `map` (which always merges) because
/// two overlapping selections with different heads might collapse to the same
/// position and need to be merged.
pub(crate) fn cmd_collapse_selection_to_head(
    buf: &Text,
    sels: SelectionSet,
    _count: usize,
    _mode: MotionMode,
) -> SelectionSet {
    let new_sels = sels.map(|s| Selection::collapsed(s.head()));
    new_sels.debug_assert_valid(buf);
    new_sels
}

/// Collapse every selection to a cursor at its `anchor`.
///
/// Mirror of [`cmd_collapse_selection_to_head`] — the cursor lands on the stationary
/// end instead of the moving end. For a forward word selection this puts the
/// cursor on the first character of the word; for a backward selection it
/// lands on the right end. Uses `map` (which always merges) for the same
/// deduplication reason as the head variant.
pub(crate) fn cmd_collapse_selection_to_anchor(
    buf: &Text,
    sels: SelectionSet,
    _count: usize,
    _mode: MotionMode,
) -> SelectionSet {
    let new_sels = sels.map(|s| Selection::collapsed(s.anchor()));
    new_sels.debug_assert_valid(buf);
    new_sels
}

/// Swap `anchor` and `head` on every selection.
///
/// A forward selection (anchor ≤ head) becomes backward, and vice versa.
/// Does not change any range bounds, so overlaps cannot arise — uses plain
/// `map` (no merge needed).
pub(crate) fn cmd_flip_selections(
    buf: &Text,
    sels: SelectionSet,
    _count: usize,
    _mode: MotionMode,
) -> SelectionSet {
    // `flip` only swaps anchor/head — no range change → no new overlaps.
    let new_sels = sels.map(|s| s.flip());
    new_sels.debug_assert_valid(buf);
    new_sels
}

/// Select the entire buffer.
///
/// Replaces all selections with a single selection spanning from the first
/// character to the last (the structural trailing `\n`). Head is placed at
/// the end so the cursor sits at the bottom — consistent with Helix `%`.
pub(crate) fn cmd_select_all(
    buf: &Text,
    _sels: SelectionSet,
    _count: usize,
    _mode: MotionMode,
) -> SelectionSet {
    let end = buf.len_chars().saturating_sub(1);
    let sels = SelectionSet::single(Selection::new(0, end));
    sels.debug_assert_valid(buf);
    sels
}

/// Keep only the primary selection; drop all others.
///
/// The result is a single-selection set. This is a destructive reduction —
/// any non-primary cursors or ranges are lost.
pub(crate) fn cmd_keep_primary_selection(
    buf: &Text,
    sels: SelectionSet,
    _count: usize,
    _mode: MotionMode,
) -> SelectionSet {
    let new_sels = sels.keep_primary();
    new_sels.debug_assert_valid(buf);
    new_sels
}

/// Remove the primary selection and advance the primary to the next one.
///
/// If there is only one selection, this is a no-op (the set can never be
/// empty). After removal the primary wraps to the start if it was the last
/// selection in document order.
pub(crate) fn cmd_remove_primary_selection(
    buf: &Text,
    sels: SelectionSet,
    count: usize,
    _mode: MotionMode,
) -> SelectionSet {
    let mut sels = sels;
    for _ in 0..count {
        if sels.len() <= 1 {
            break;
        }
        let idx = sels.primary_index();
        sels = sels.remove(idx);
    }
    sels.debug_assert_valid(buf);
    sels
}

/// Move the primary selection to the next one in document order, wrapping.
pub(crate) fn cmd_cycle_primary_forward(
    buf: &Text,
    sels: SelectionSet,
    _count: usize,
    _mode: MotionMode,
) -> SelectionSet {
    let new_sels = sels.cycle_primary(1);
    new_sels.debug_assert_valid(buf);
    new_sels
}

/// Move the primary selection to the previous one in document order, wrapping.
pub(crate) fn cmd_cycle_primary_backward(
    buf: &Text,
    sels: SelectionSet,
    _count: usize,
    _mode: MotionMode,
) -> SelectionSet {
    let new_sels = sels.cycle_primary(-1);
    new_sels.debug_assert_valid(buf);
    new_sels
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::assert_state;
    use crate::testing::parse_state;
    use pretty_assertions::assert_eq;

    // ── cmd_collapse_selection_to_head ─────────────────────────────────────────────

    #[test]
    fn collapse_cursor_is_noop() {
        // A cursor (anchor == head) collapsing to itself — no change.
        assert_state!(
            "-[h]>ello\n",
            |(buf, sels)| cmd_collapse_selection_to_head(&buf, sels, 0, MotionMode::Move),
            "-[h]>ello\n"
        );
    }

    #[test]
    fn collapse_forward_selection() {
        assert_state!(
            "-[hell]>o\n",
            |(buf, sels)| cmd_collapse_selection_to_head(&buf, sels, 0, MotionMode::Move),
            // head was at 'l' (offset 3)
            "hel-[l]>o\n"
        );
    }

    #[test]
    fn collapse_backward_selection() {
        // Backward: anchor=3, head=0, selects "hell" (4 chars). Collapses to cursor at head=0.
        assert_state!(
            "<[hell]-o\n",
            |(buf, sels)| cmd_collapse_selection_to_head(&buf, sels, 0, MotionMode::Move),
            "-[h]>ello\n"
        );
    }

    #[test]
    fn collapse_merges_coincident_heads() {
        // Two cursors at different positions stay separate after collapse —
        // they only merge if their heads land on the exact same position.
        let (buf, sels) = parse_state("-[h]>el-[l]>o\n");
        let result = cmd_collapse_selection_to_head(&buf, sels, 0, MotionMode::Move);
        assert_eq!(result.len(), 2); // still 2 — they don't converge
    }

    // ── cmd_flip_selections ────────────────────────────────────────────────

    #[test]
    fn flip_forward_becomes_backward() {
        // Forward: anchor=0, head=3, selects "hell". After flip: anchor=3, head=0.
        assert_state!(
            "-[hell]>o\n",
            |(buf, sels)| cmd_flip_selections(&buf, sels, 0, MotionMode::Move),
            "<[hell]-o\n"
        );
    }

    #[test]
    fn flip_backward_becomes_forward() {
        // Backward: anchor=3, head=0, selects "hell". After flip: anchor=0, head=3.
        assert_state!(
            "<[hell]-o\n",
            |(buf, sels)| cmd_flip_selections(&buf, sels, 0, MotionMode::Move),
            "-[hell]>o\n"
        );
    }

    #[test]
    fn flip_cursor_is_noop() {
        // anchor == head → flip does nothing observable.
        assert_state!(
            "-[h]>ello\n",
            |(buf, sels)| cmd_flip_selections(&buf, sels, 0, MotionMode::Move),
            "-[h]>ello\n"
        );
    }

    #[test]
    fn flip_is_involution() {
        // Flipping twice is the identity.
        assert_state!(
            "-[hell]>o\n",
            |(buf, sels)| {
                let sels = cmd_flip_selections(&buf, sels, 0, MotionMode::Move);
                cmd_flip_selections(&buf, sels, 0, MotionMode::Move)
            },
            "-[hell]>o\n"
        );
    }

    // ── cmd_select_all ─────────────────────────────────────────────────────

    #[test]
    fn select_all_spans_entire_buffer() {
        // Cursor at 'e'; after select-all, anchor=0 head=last char ('\n').
        assert_state!(
            "h-[e]>llo\n",
            |(buf, sels)| cmd_select_all(&buf, sels, 0, MotionMode::Move),
            "-[hello\n]>"
        );
    }

    #[test]
    fn select_all_multi_line() {
        assert_state!(
            "foo\nb-[a]>r\nbaz\n",
            |(buf, sels)| cmd_select_all(&buf, sels, 0, MotionMode::Move),
            "-[foo\nbar\nbaz\n]>"
        );
    }

    #[test]
    fn select_all_empty_buffer() {
        // Minimal buffer is just '\n'. select-all produces a cursor at 0.
        assert_state!(
            "-[\n]>",
            |(buf, sels)| cmd_select_all(&buf, sels, 0, MotionMode::Move),
            "-[\n]>"
        );
    }

    // ── cmd_keep_primary_selection ─────────────────────────────────────────

    #[test]
    fn keep_primary_drops_all_others() {
        // Three cursors; primary (first yielded by DSL) is at 0. Others dropped.
        assert_state!(
            "-[h]>el-[l]>-[o]>\n",
            |(buf, sels)| cmd_keep_primary_selection(&buf, sels, 0, MotionMode::Move),
            "-[h]>ello\n"
        );
    }

    #[test]
    fn keep_primary_single_unchanged() {
        assert_state!(
            "-[hell]>o\n",
            |(buf, sels)| cmd_keep_primary_selection(&buf, sels, 0, MotionMode::Move),
            "-[hell]>o\n"
        );
    }

    // ── cmd_remove_primary_selection ───────────────────────────────────────

    #[test]
    fn remove_primary_single_is_noop() {
        assert_state!(
            "-[h]>ello\n",
            |(buf, sels)| cmd_remove_primary_selection(&buf, sels, 1, MotionMode::Move),
            "-[h]>ello\n"
        );
    }

    #[test]
    fn remove_primary_two_selections() {
        // Two cursors at 0 and 4. Primary is first (index 0).
        // After removal: only the cursor at 4 remains, becomes primary.
        assert_state!(
            "-[h]>ell-[o]>\n",
            |(buf, sels)| cmd_remove_primary_selection(&buf, sels, 1, MotionMode::Move),
            "hell-[o]>\n"
        );
    }

    // ── cmd_cycle_primary_forward ──────────────────────────────────────────

    #[test]
    fn cycle_forward_advances_primary() {
        // Three cursors. After cycling forward, primary should be the next one.
        let (buf, sels) = parse_state("-[h]>el-[l]>o\n"); // two cursors, primary at 0
        assert_eq!(sels.primary().head(), 0);
        let sels = cmd_cycle_primary_forward(&buf, sels, 0, MotionMode::Move);
        assert_eq!(sels.primary().head(), 3);
        // Cycle again — wraps back to first.
        let sels = cmd_cycle_primary_forward(&buf, sels, 0, MotionMode::Move);
        assert_eq!(sels.primary().head(), 0);
    }

    // ── cmd_cycle_primary_backward ─────────────────────────────────────────

    #[test]
    fn cycle_backward_wraps_to_last() {
        let (buf, sels) = parse_state("-[h]>el-[l]>o\n"); // primary at 0
        let sels = cmd_cycle_primary_backward(&buf, sels, 0, MotionMode::Move);
        assert_eq!(sels.primary().head(), 3); // wraps to last
    }

    // ── cmd_collapse_selection_to_anchor ──────────────────────────────────

    #[test]
    fn collapse_to_anchor_cursor_is_noop() {
        // anchor == head → collapsing to anchor is the same as collapsing to head.
        assert_state!(
            "-[h]>ello\n",
            |(buf, sels)| cmd_collapse_selection_to_anchor(&buf, sels, 0, MotionMode::Move),
            "-[h]>ello\n"
        );
    }

    #[test]
    fn collapse_to_anchor_forward_selection() {
        // Forward: anchor=0 (h), head=3 (l). Collapse to anchor → cursor at h.
        assert_state!(
            "-[hell]>o\n",
            |(buf, sels)| cmd_collapse_selection_to_anchor(&buf, sels, 0, MotionMode::Move),
            "-[h]>ello\n"
        );
    }

    #[test]
    fn collapse_to_anchor_backward_selection() {
        // Backward: anchor=3 (l), head=0 (h). Collapse to anchor → cursor at l.
        assert_state!(
            "<[hell]-o\n",
            |(buf, sels)| cmd_collapse_selection_to_anchor(&buf, sels, 0, MotionMode::Move),
            "hel-[l]>o\n"
        );
    }

    #[test]
    fn collapse_to_anchor_merges_coincident_anchors() {
        // Two selections with different heads but the same anchor collapse to
        // the same cursor and must be merged.
        let buf = hume_editing::text::Text::from("hello\n");
        let sels = hume_editing::selection::SelectionSet::from_vec(
            vec![
                hume_editing::selection::Selection::new(0, 2), // anchor=0
                hume_editing::selection::Selection::new(0, 4), // anchor=0
            ],
            0,
        );
        let result = cmd_collapse_selection_to_anchor(&buf, sels, 0, MotionMode::Move);
        assert_eq!(result.len(), 1); // merged — both collapsed to cursor at 0
        assert_eq!(result.primary().head(), 0);
    }

    // ── additional collapse edge cases ─────────────────────────────────────

    #[test]
    fn collapse_empty_buffer() {
        assert_state!(
            "-[\n]>",
            |(buf, sels)| cmd_collapse_selection_to_head(&buf, sels, 0, MotionMode::Move),
            "-[\n]>"
        );
    }

    #[test]
    fn collapse_two_selections_same_head_merges() {
        // Two selections with different anchors but the same head collapse to
        // one cursor — map (which always merges) must reduce the count.
        let buf = hume_editing::text::Text::from("hello\n");
        let sels = hume_editing::selection::SelectionSet::from_vec(
            vec![
                hume_editing::selection::Selection::new(0, 3), // head at 3
                hume_editing::selection::Selection::new(1, 3), // head at 3
            ],
            0,
        );
        let result = cmd_collapse_selection_to_head(&buf, sels, 0, MotionMode::Move);
        assert_eq!(result.len(), 1); // merged — both collapsed to cursor at 3
        assert_eq!(result.primary().head(), 3);
    }

    // ── additional flip edge cases ─────────────────────────────────────────

    #[test]
    fn flip_multiple_selections() {
        // Two forward selections both flip to backward.
        assert_state!(
            "-[hell]>o -[worl]>d\n",
            |(buf, sels)| cmd_flip_selections(&buf, sels, 0, MotionMode::Move),
            "<[hell]-o <[worl]-d\n"
        );
    }

    // ── additional keep_primary edge cases ─────────────────────────────────

    #[test]
    fn keep_primary_when_primary_is_not_first() {
        // Cycle primary to the second cursor, then keep — should keep that one.
        let (buf, sels) = parse_state("-[h]>el-[l]>o\n"); // primary at index 0 (head=0)
        let sels = cmd_cycle_primary_forward(&buf, sels, 0, MotionMode::Move); // primary now at index 1 (head=3)
        let sels_out = cmd_keep_primary_selection(&buf, sels, 0, MotionMode::Move);
        assert_eq!(sels_out.len(), 1);
        assert_eq!(sels_out.primary().head(), 3); // kept the second one
    }

    // ── additional remove_primary edge cases ───────────────────────────────

    #[test]
    fn remove_primary_at_end_wraps_to_first() {
        // Three cursors at 0, 3, 6. Cycle to last, then remove — should wrap
        // to the first remaining cursor (index 0 of the new set).
        let (buf, sels) = parse_state("-[h]>el-[l]>o-[\n]>"); // 3 cursors, primary at 0
        let sels = cmd_cycle_primary_backward(&buf, sels, 0, MotionMode::Move); // primary at last (head=6)
        let sels_out = cmd_remove_primary_selection(&buf, sels, 1, MotionMode::Move);
        assert_eq!(sels_out.len(), 2);
        assert_eq!(sels_out.primary().head(), 0); // wrapped to first
    }
}
