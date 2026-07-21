use crate::ops::MotionMode;
use hume_editing::lines::place_column;
use hume_editing::selection::{Selection, SelectionSet};
use hume_editing::text::Text;

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
    _count: usize,
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
    _count: usize,
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

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests;
