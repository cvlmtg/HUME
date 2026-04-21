use crate::core::text::Text;
use crate::core::selection::{Selection, SelectionSet};

pub(crate) use super::MotionMode;

/// Whether an f/t motion places the cursor on the found character or adjacent to it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum FindKind {
    /// `find-forward` / `find-backward`: cursor lands ON the found character.
    Inclusive,
    /// `till-forward` / `till-backward`: cursor lands one grapheme before (forward) or after (backward) it.
    Exclusive,
}

// ── Motion framework ──────────────────────────────────────────────────────────

/// Apply an inner motion to every selection in the set, repeated `count` times.
///
/// `motion` is a plain function `fn(&Text, head) -> new_head`. It knows
/// nothing about anchors or multi-cursor — it computes exactly one new
/// position from one old position. `apply_motion` handles the anchor
/// semantics (via `mode`) and multi-cursor bookkeeping.
///
/// `count` controls how many times the motion is applied per selection.
/// The motion is folded `count` times *inside* `map_and_merge` — each
/// selection independently accumulates N steps before anchor/merge logic
/// runs. This is semantically "move 3 words" (not "apply 1w to the whole
/// selection set three times"), which prevents premature merging of
/// multi-cursor selections between steps.
///
/// Uses `map_and_merge` so that selections which converge to the same position
/// after the motion are automatically merged.
pub(crate) fn apply_motion(
    buf: &Text,
    sels: SelectionSet,
    mode: MotionMode,
    count: usize,
    motion: impl Fn(&Text, usize) -> usize,
) -> SelectionSet {
    let result = sels.map_and_merge(|sel| {
        // Apply the motion `count` times, feeding each result as the next
        // input. `fold` starting from the current head position.
        let new_head = (0..count).fold(sel.head, |h, _| motion(buf, h));
        match mode {
            MotionMode::Move => Selection::collapsed(new_head),
            MotionMode::Extend => Selection::new(sel.anchor, new_head),
        }
    });
    result.debug_assert_valid(buf);
    result
}

mod char_move;
use char_move::{move_right, move_left, goto_first_line, goto_last_line};
mod line;
use line::{goto_line_start, goto_line_end, goto_first_nonblank, move_down_inner, move_up_inner};
mod word;
pub(crate) use word::{cmd_select_next_word, cmd_select_next_WORD, cmd_select_prev_word, cmd_select_prev_WORD};
mod paragraph;
use paragraph::{next_paragraph, prev_paragraph};
pub(crate) mod line_select;
pub(crate) use line_select::{cmd_select_line, cmd_select_line_backward};
pub(crate) mod find;
pub(crate) use find::{find_char_forward, find_char_backward};

#[cfg(test)]
mod tests;

// ── Named commands (public API) ───────────────────────────────────────────────
//
// Named commands follow the edit convention — `(Text, SelectionSet) ->
// (Text, SelectionSet)` — so they can be used directly with `assert_state!`
// and, eventually, the command dispatch table.
//
// Pure motions do not modify the buffer, so `buf` passes through unchanged.
//
// Every named command is structurally identical: call `apply_motion` with a
// mode and a motion function, return `(buf, new_sels)`. The `motion_cmd!`
// macro captures that skeleton so the table below is just data — name, mode,
// motion — with no repeated scaffolding.

/// Generate a named motion command.
///
/// Two arms handle the two motion shapes that exist in this codebase:
///
/// **Direct** — the motion function takes only `(&Text, head)`:
/// ```text
/// motion_cmd!(/// doc, cmd_move_right, move_right);
/// ```
///
/// **Curried** — the motion function needs an extra argument (a boundary
/// predicate or a target-column hint). The macro generates the closure
/// `|b, h| inner(b, h, arg)`:
/// ```text
/// motion_cmd!(/// doc, cmd_move_down, move_down_inner(None));
/// ```
///
/// The curried arm is listed first so that `ident(expr)` syntax is tried
/// before the bare-`expr` arm — without this ordering, `inner(arg)` would
/// match the direct arm as an expression and generate a call-site type error.
///
/// `$(#[$attr:meta])*` forwards doc comments (and any other attributes) from
/// the invocation into the generated function. In Rust, `/// text` is
/// syntactic sugar for `#[doc = "text"]`, so it is captured by `:meta`.
///
/// `#[allow(non_snake_case)]` is emitted unconditionally. It is a no-op for
/// snake_case names and suppresses the expected warning for WORD variants
/// (`cmd_next_WORD_start` etc.) without needing a separate macro arm.
macro_rules! motion_cmd {
    // Curried arm: motion needs an extra argument — generates a closure.
    ($(#[$attr:meta])* $name:ident, $inner:ident($arg:expr)) => {
        $(#[$attr])*
        #[allow(non_snake_case)]
        pub(crate) fn $name(buf: &Text, sels: SelectionSet, count: usize, mode: MotionMode) -> SelectionSet {
            apply_motion(buf, sels, mode, count, |b, h| $inner(b, h, $arg))
        }
    };
    // Direct arm: motion function takes only (&Text, head).
    ($(#[$attr:meta])* $name:ident, $motion:expr) => {
        $(#[$attr])*
        #[allow(non_snake_case)]
        pub(crate) fn $name(buf: &Text, sels: SelectionSet, count: usize, mode: MotionMode) -> SelectionSet {
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
motion_cmd!(/// Move or extend cursors to the first non-blank character on their current line.
    cmd_goto_first_nonblank, goto_first_nonblank);

// Vertical motion passes `None` as the target-column hint (no sticky column yet).
motion_cmd!(/// Move or extend cursors down one line, preserving the char-offset column.
    cmd_move_down, move_down_inner(None));
motion_cmd!(/// Move or extend cursors up one line, preserving the char-offset column.
    cmd_move_up, move_up_inner(None));
// `w` / `W` jump to the next word/WORD and select it as a fresh forward
// selection (anchor = word start, head = word end). `b` / `B` do the same
// for the previous word. This replaces Helix's "extend from current position"
// semantics: every motion re-anchors rather than growing a drag selection.
//
// `e` / `E` are removed — they were only needed to compensate for `w` landing
// on the first char of the next word. With the new model, `w` already selects
// the whole word so `e` is redundant.


// Paragraph motions.
motion_cmd!(/// Move or extend cursors to the start of the next paragraph (`]p`).
    cmd_next_paragraph, next_paragraph);
motion_cmd!(/// Move or extend cursors to the first empty line above the current paragraph (`[p`).
    cmd_prev_paragraph, prev_paragraph);

