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
/// `motion` computes one new head position, given the whole current
/// selection — most motions only read `sel.head()`, but a motion that needs
/// to resolve against the whole span (e.g. [`goto_matching_pair`]) can too.
/// `apply_motion` handles the anchor semantics (via `mode`) and multi-cursor
/// bookkeeping.
///
/// `count` controls how many times the motion is applied per selection. Each
/// step rebuilds a `Selection` pinned to the *original* anchor with the
/// latest head, so a multi-step motion sees a selection shaped like its
/// caller would see it after one step, not a bare head. The motion is applied
/// `count` times *inside* the `map` call — each selection independently
/// accumulates N steps before anchor/merge logic runs. This is semantically
/// "move 3 words" (not "apply 1w to the whole selection set three times"),
/// which prevents premature merging of multi-cursor selections between
/// steps.
///
/// Uses `map` (which always merges) so that selections which converge to the
/// same position after the motion are automatically merged.
pub(crate) fn apply_motion(
    text: &BufferText,
    sels: SelectionSet,
    mode: MotionMode,
    count: usize,
    motion: impl Fn(&BufferText, &Selection) -> usize,
) -> SelectionSet {
    let result = sels.map(|sel| {
        // Stop at a fixed point. Every motion here is a pure function of
        // (text, selection), so once a step stops moving the head — clamped
        // at a buffer edge, or no further match for f/t — every later step
        // returns the same head. Without this a large count does O(count)
        // work instead of O(distance moved).
        let mut s = sel;
        for _ in 0..count {
            let head = motion(text, &s);
            if head == s.head() {
                break;
            }
            s = Selection::new(s.anchor(), head);
        }
        let new_head = s.head();
        match mode {
            MotionMode::Move => Selection::collapsed(new_head),
            MotionMode::Extend => Selection::new(sel.anchor(), new_head),
        }
    });
    result.debug_assert_valid(text);
    result
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
    cmd_select_prev_word,
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
// Pure motions do not modify the buffer, so `text` passes through unchanged.
//
// The `motion_cmd!` macro below generates each command, so the table is just
// data — name, mode, motion — with no repeated scaffolding.

/// Generate a named motion command whose motion function takes only
/// `(&BufferText, head)` — wrapped to fit `apply_motion`'s `&Selection` param:
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
        pub fn $name(text: &BufferText, sels: SelectionSet, count: usize, mode: MotionMode) -> SelectionSet {
            apply_motion(text, sels, mode, count, |t, s: &Selection| $motion(t, s.head()))
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
/// to a bare `#`. Vim's `count%` means "go to N% of the file" — a different
/// operation this motion doesn't implement — so `count` is ignored rather
/// than given a meaning nobody asked for.
pub fn cmd_goto_matching_pair(
    text: &BufferText,
    sels: SelectionSet,
    _count: usize,
    mode: MotionMode,
) -> SelectionSet {
    apply_motion(text, sels, mode, 1, goto_matching_pair)
}

// Paragraph motions.
motion_cmd!(/// Move or extend cursors to the start of the next paragraph (`]p`).
    cmd_next_paragraph, next_paragraph);
motion_cmd!(/// Move or extend cursors to the first empty line above the current paragraph (`[p`).
    cmd_prev_paragraph, prev_paragraph);
