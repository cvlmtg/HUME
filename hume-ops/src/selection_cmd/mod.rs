mod matching;

pub use matching::{
    cmd_split_selection_on_newlines, cmd_trim_selection_whitespace, sift_matches_within,
};

use super::MotionMode;
use hume_editing::selection::Selection;
use hume_editing::state::EditState;

// ── Simple selection-set commands ─────────────────────────────────────────────

/// Collapse every selection to a cursor at its `head`.
///
/// `anchor` becomes equal to `head`: the selected range shrinks to a single
/// character (the cursor position). Uses `map` (which always merges) because
/// two overlapping selections with different heads might collapse to the same
/// position and need to be merged.
pub fn cmd_collapse_selection_to_head(
    state: EditState,
    _count: usize,
    _mode: MotionMode,
) -> EditState {
    state.map(|s| s.selection().to_head())
}

/// Collapse every selection to a cursor at its `anchor`.
///
/// Mirror of [`cmd_collapse_selection_to_head`]: the cursor lands on the stationary
/// end instead of the moving end. For a forward word selection this puts the
/// cursor on the first character of the word; for a backward selection it
/// lands on the right end. Uses `map` (which always merges) for the same
/// deduplication reason as the head variant.
pub fn cmd_collapse_selection_to_anchor(
    state: EditState,
    _count: usize,
    _mode: MotionMode,
) -> EditState {
    state.map(|s| s.selection().to_anchor())
}

/// Swap `anchor` and `head` on every selection.
///
/// A forward selection (anchor ≤ head) becomes backward, and vice versa.
/// Does not change any range bounds, so overlaps cannot arise. Uses plain
/// `map` (no merge needed).
pub fn cmd_flip_selections(state: EditState, _count: usize, _mode: MotionMode) -> EditState {
    state.map(|s| s.selection().flip())
}

/// Select the entire buffer.
///
/// Replaces all selections with a single selection spanning from the first
/// character to the last (the structural trailing `\n`). Head is placed at
/// the end so the cursor sits at the bottom, consistent with Helix `%`.
pub fn cmd_select_all(state: EditState, _count: usize, _mode: MotionMode) -> EditState {
    let all = Selection::new(
        state.text().clusters().first(),
        state.text().clusters().last(),
    );
    state.with_selections(vec![all], 0)
}

/// Keep only the primary selection; drop all others.
///
/// The result is a single-selection set. This is a destructive reduction:
/// any non-primary cursors or ranges are lost.
pub fn cmd_keep_primary_selection(state: EditState, _count: usize, _mode: MotionMode) -> EditState {
    state.keep_primary()
}

/// Remove the primary selection and the `count - 1` selections after it,
/// wrapping to the start; the selection after them becomes primary. The last
/// selection left is never removed.
pub fn cmd_remove_primary_selection(
    state: EditState,
    count: usize,
    _mode: MotionMode,
) -> EditState {
    state.remove_primary(count)
}

/// Move the primary selection `count` selections forward in document order,
/// wrapping.
pub fn cmd_cycle_primary_forward(state: EditState, count: usize, _mode: MotionMode) -> EditState {
    let steps = cycle_steps(&state, count);
    state.cycle_primary(steps)
}

/// Move the primary selection `count` selections back in document order,
/// wrapping.
pub fn cmd_cycle_primary_backward(state: EditState, count: usize, _mode: MotionMode) -> EditState {
    let steps = cycle_steps(&state, count);
    state.cycle_primary(-steps)
}

/// `count` reduced to less than one lap, so it fits a signed step.
fn cycle_steps(state: &EditState, count: usize) -> isize {
    isize::try_from(count % state.view().len()).expect("less than one lap fits an isize")
}

#[cfg(test)]
mod tests;
