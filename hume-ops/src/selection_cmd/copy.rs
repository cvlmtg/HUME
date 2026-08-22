use crate::MotionMode;
use hume_editing::lines::place_char_column;
use hume_editing::selection::{Selection, SelectionSet};
use hume_editing::text::Text;

// ── Vertical copy ─────────────────────────────────────────────────────────────

/// Duplicate each selection onto each of the `count` lines below it and add
/// them to the selection set.
///
/// Each copy preserves the **char-offset** column of both `anchor` and
/// `head` (not a display column — wrong for tabs/wide chars, same narrow gap
/// `move_down_inner`/`move_up_inner` used to have before they switched to
/// `place_column`'s display-column model). Left as char-offset here because
/// `cmd_copy_selection_on_next_line`/`_prev_line` are registered directly in
/// `CommandRegistry` as bare `fn` pointers — no channel to a per-buffer
/// `tab_width` exists at that call shape, unlike the `9j`/`9k` path, which
/// isn't registered and is reached through code that already resolves
/// buffer settings.
///
/// Clamped to the length of its target line and snapped to a grapheme
/// boundary. Copying stops early once a target line doesn't exist (i.e. the
/// selection's bottommost line is the last real line) — a `count` larger than
/// the remaining lines just clamps at the last one, it doesn't wrap or error.
///
/// The primary advances to the furthest copy of the original primary. Every
/// copy's column is re-derived from the *original* selection, not the
/// previous copy — so this is not equivalent to `count` separate presses of
/// the count-1 command, which would re-clamp against each intermediate line
/// in turn. Re-deriving from the original means a single short line in the
/// middle of the run only clamps that one copy, instead of collapsing every
/// copy after it to that line's column. If no copy was added (last-line edge
/// case) the primary stays on the original.
pub fn cmd_copy_selection_on_next_line(
    buf: &Text,
    sels: SelectionSet,
    count: usize,
    _mode: MotionMode,
) -> SelectionSet {
    copy_selection_vertically(buf, sels, 1, count)
}

/// Duplicate each selection onto each of the `count` lines above it and add
/// them to the selection set.
///
/// Mirror of [`cmd_copy_selection_on_next_line`] — shifts up instead of down.
pub fn cmd_copy_selection_on_prev_line(
    buf: &Text,
    sels: SelectionSet,
    count: usize,
    _mode: MotionMode,
) -> SelectionSet {
    copy_selection_vertically(buf, sels, -1, count)
}

/// Core implementation for copy-to-next/prev-line. `direction` is `1` for
/// down and `-1` for up; `count` is how many lines in that direction to
/// duplicate onto.
fn copy_selection_vertically(
    buf: &Text,
    sels: SelectionSet,
    direction: isize,
    count: usize,
) -> SelectionSet {
    let primary_idx = sels.primary_index();
    // Collect originals into `all_sels`. Copies are appended below.
    let mut all_sels: Vec<Selection> = sels.iter_sorted().copied().collect();
    let original_len = all_sels.len();
    // Index in `all_sels` for the furthest copy of the old primary, if one was added.
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

        // Both endpoints' columns are loop-invariant — the original selection
        // never changes across copies — so compute them once instead of
        // re-deriving from the rope on every iteration.
        let anchor_col = sel.anchor() - buf.line_to_char(anchor_line as usize);
        let head_col = sel.head() - buf.line_to_char(head_line as usize);

        // Walk outward one line at a time, breaking as soon as a target line
        // falls off the buffer — every further step in that direction would
        // too, so this is O(lines available), not O(count) even for a
        // `usize::MAX` count prefix.
        let mut target_outer = outer_line;
        for _ in 0..count {
            target_outer += direction;

            if target_outer < 0 {
                break; // would go before the start of the buffer
            }
            let target_outer_usize = target_outer as usize;

            // Past the last real content line — the phantom trailing line
            // (and anything further) has no content to copy onto.
            if target_outer_usize > buf.last_content_line() {
                break;
            }

            // Shift each endpoint by the same delta, clamped to the target
            // line's content and snapped to a grapheme boundary.
            let delta = target_outer - outer_line;
            let new_anchor = place_char_column(buf, (anchor_line + delta) as usize, anchor_col);
            let new_head = place_char_column(buf, (head_line + delta) as usize, head_col);

            let new_sel = Selection::new(new_anchor, new_head);

            if i == primary_idx {
                primary_copy_idx = Some(all_sels.len());
            }
            all_sels.push(new_sel);
        }
    }

    let desired_primary = primary_copy_idx.unwrap_or(primary_idx);
    let new_set = SelectionSet::from_vec(all_sels, desired_primary);
    new_set.debug_assert_valid(buf);
    new_set
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests;
