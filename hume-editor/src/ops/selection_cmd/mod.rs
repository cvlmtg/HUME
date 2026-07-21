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
mod tests;
