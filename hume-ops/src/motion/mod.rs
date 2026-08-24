use hume_editing::selection::{Selection, SelectionSet};
use hume_editing::text::BufferText;

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

/// Apply an inner motion to every selection in the set, repeated `count` times.
///
/// `motion` is a plain function `fn(&BufferText, head) -> new_head`. It knows
/// nothing about anchors or multi-cursor — it computes exactly one new
/// position from one old position. `apply_motion` handles the anchor
/// semantics (via `mode`) and multi-cursor bookkeeping.
///
/// `count` controls how many times the motion is applied per selection.
/// The motion is folded `count` times *inside* the `map` call — each selection
/// independently accumulates N steps before anchor/merge logic runs. This is
/// semantically "move 3 words" (not "apply 1w to the whole selection set three
/// times"), which prevents premature merging of multi-cursor selections between
/// steps.
///
/// Uses `map` (which always merges) so that selections which converge to the
/// same position after the motion are automatically merged.
pub(crate) fn apply_motion(
    buf: &BufferText,
    sels: SelectionSet,
    mode: MotionMode,
    count: usize,
    motion: impl Fn(&BufferText, usize) -> usize,
) -> SelectionSet {
    let result = sels.map(|sel| {
        // Apply the motion `count` times, feeding each result as the next
        // input. `fold` starting from the current head position.
        let new_head = (0..count).fold(sel.head(), |h, _| motion(buf, h));
        match mode {
            MotionMode::Move => Selection::collapsed(new_head),
            MotionMode::Extend => Selection::new(sel.anchor(), new_head),
        }
    });
    result.debug_assert_valid(buf);
    result
}

mod char_move;
use char_move::{goto_first_line, goto_last_line, move_left, move_right};
mod line;
use line::{goto_first_nonblank, goto_line_end, goto_line_newline, goto_line_start};
mod word;
pub(crate) use word::prev_word_start;
pub use word::{
    cmd_select_next_uppercase_word, cmd_select_next_uppercase_word_around, cmd_select_next_word,
    cmd_select_next_word_around, cmd_select_prev_uppercase_word,
    cmd_select_prev_uppercase_word_around, cmd_select_prev_word, cmd_select_prev_word_around,
};
mod paragraph;
use paragraph::{next_paragraph, prev_paragraph};
mod line_select;
pub use line_select::{cmd_select_line, cmd_select_line_backward};
mod find;
pub use find::{find_char_backward, find_char_forward};

#[cfg(test)]
mod tests;

// ── Named commands (public API) ───────────────────────────────────────────────
//
// Named commands follow the edit convention — `(BufferText, SelectionSet) ->
// (BufferText, SelectionSet)` — so they can be used directly with `assert_state!`
// and, eventually, the command dispatch table.
//
// Pure motions do not modify the buffer, so `buf` passes through unchanged.
//
// The `motion_cmd!` macro below generates each command, so the table is just
// data — name, mode, motion — with no repeated scaffolding.

/// Generate a named motion command whose motion function takes only
/// `(&BufferText, head)`:
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
        pub fn $name(buf: &BufferText, sels: SelectionSet, count: usize, mode: MotionMode) -> SelectionSet {
            apply_motion(buf, sels, mode, count, $motion)
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

// Paragraph motions.
motion_cmd!(/// Move or extend cursors to the start of the next paragraph (`]p`).
    cmd_next_paragraph, next_paragraph);
motion_cmd!(/// Move or extend cursors to the first empty line above the current paragraph (`[p`).
    cmd_prev_paragraph, prev_paragraph);
