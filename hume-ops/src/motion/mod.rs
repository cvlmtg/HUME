use hume_editing::selection::{Selection, SelectionView};
use hume_editing::state::EditState;
use hume_rope::cluster::ClusterStart;

use super::MotionMode;

/// Whether an f/t motion places the cursor on the found character or adjacent to it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FindKind {
    /// `find-forward` / `find-backward`: cursor lands ON the found character.
    Inclusive,
    /// `till-forward` / `till-backward`: cursor lands one grapheme before (forward) or after (backward) it.
    Exclusive,
}

// ── Motion framework ──────────────────────────────────────────────────────────

/// Apply an inner motion to every selection, repeated `count` times.
///
/// `motion` computes one new head, given the whole current selection. Most
/// motions only read the head, but a motion that needs to resolve against the
/// whole span (e.g. [`goto_matching_pair`]) can too. `apply_motion` handles
/// the anchor semantics (via `mode`) and multi-cursor bookkeeping.
///
/// Each step views the selection pinned to its *original* anchor with the
/// latest head, so a multi-step motion sees a selection shaped like its caller
/// would see it after one step, not a bare head. Every selection takes its
/// `count` steps before selections merge: "move 3 words", not "apply 1w to the
/// whole set three times", so multi-cursor selections never merge between
/// steps. Selections that converge afterwards merge.
pub(crate) fn apply_motion(
    state: EditState,
    mode: MotionMode,
    count: usize,
    motion: impl Fn(SelectionView<'_>) -> ClusterStart,
) -> EditState {
    state.map(|sel| {
        // Stop at a fixed point: every motion here is a pure function of
        // (text, selection), so once a step stops moving the head, every
        // later step returns the same head. Without this a large count does
        // O(count) work instead of O(distance moved).
        let mut step = sel;
        for _ in 0..count {
            let head = motion(step);
            if head == step.head() {
                break;
            }
            step = step.with_selection(step.selection().with_head(head));
        }
        match mode {
            MotionMode::Move => Selection::cursor(step.head()),
            MotionMode::Extend => sel.selection().with_head(step.head()),
        }
    })
}

mod matching_pair;
use matching_pair::goto_matching_pair;
mod char_move;
use char_move::{goto_first_line, goto_last_line, move_left, move_right};
mod line;
use line::{goto_first_nonblank, goto_line_end, goto_line_newline, goto_line_start};
mod word;
pub(crate) use word::prev_word_start;
pub use word::{
    cmd_select_next_uppercase_word, cmd_select_next_word, cmd_select_prev_uppercase_word,
    cmd_select_prev_word, word_runs,
};
mod paragraph;
pub(crate) use paragraph::paragraph_at;
pub use paragraph::{cmd_goto_next_paragraph, cmd_goto_prev_paragraph};
mod line_select;
pub use line_select::{cmd_select_line, cmd_select_line_backward};
mod find;
pub use find::{find_char_backward, find_char_forward};
mod object;
pub use object::apply_object_motion;

#[cfg(test)]
mod tests;

// ── Named commands (public API) ───────────────────────────────────────────────
//
// Named commands take and return an `EditState`; a motion leaves the text as
// it is.
//
// The `motion_cmd!` macro below generates each command, so the table is just
// data (name, mode, motion) with no repeated scaffolding.

/// Generate a named motion command whose motion function takes only
/// `(&BufferText, head)`, wrapped to fit `apply_motion`'s selection view:
/// ```text
/// motion_cmd!(/// doc, cmd_move_right, move_right);
/// ```
///
/// `#[allow(non_snake_case)]` is emitted unconditionally to suppress the
/// expected warning for WORD variants (`cmd_next_WORD_start` etc.) without a
/// separate macro arm.
macro_rules! motion_cmd {
    ($(#[$attr:meta])* $name:ident, $motion:expr) => {
        $(#[$attr])*
        #[allow(non_snake_case)]
        pub fn $name(state: EditState, count: usize, mode: MotionMode) -> EditState {
            apply_motion(state, mode, count, |s| $motion(s.text(), s.head()))
        }
    };
}

// ── Command table ─────────────────────────────────────────────────────────────

motion_cmd!(/// Move or extend cursors one grapheme to the right.
    cmd_move_right, move_right);
motion_cmd!(/// Move or extend cursors one grapheme to the left.
    cmd_move_left, move_left);

motion_cmd!(/// Move or extend cursors to the first character of the buffer.
    cmd_goto_first_line, goto_first_line);
motion_cmd!(/// Move or extend cursors to the first character of the last line.
    cmd_goto_last_line, goto_last_line);

motion_cmd!(/// Move or extend cursors to the start of their current line.
    cmd_goto_line_start, goto_line_start);
motion_cmd!(/// Move or extend cursors to the last non-newline character on their current line.
    cmd_goto_line_end, goto_line_end);
motion_cmd!(/// Move or extend cursors to the `\n` terminating the current line.
    cmd_goto_line_newline, goto_line_newline);
motion_cmd!(/// Move or extend cursors to the first non-blank character on their current line.
    cmd_goto_first_nonblank, goto_first_nonblank);

/// Move or extend cursors to the matching bracket or tag (`#`).
///
/// Not a `motion_cmd!`: the motion is an involution (applying it twice
/// returns to the start), so folding it `count` times the way every other
/// motion does would make an even count a no-op and an odd count identical
/// to a bare `#`. Vim's `count%` means "go to N% of the file" (a different
/// operation this motion doesn't implement), so `count` is ignored rather
/// than given a meaning nobody asked for.
pub fn cmd_goto_matching_pair(state: EditState, _count: usize, mode: MotionMode) -> EditState {
    apply_motion(state, mode, 1, goto_matching_pair)
}
