use hume_editing::grapheme::next_grapheme_boundary;
use hume_editing::selection::{Selection, SelectionSet};
use hume_editing::text::Text;

use crate::ops::MotionMode;

mod argument;
mod bracket;
mod line;
mod quote;
mod word;

pub(crate) use argument::{cmd_around_argument, cmd_inner_argument};
pub(crate) use bracket::{
    cmd_around_angle, cmd_around_brace, cmd_around_bracket, cmd_around_paren, cmd_inner_angle,
    cmd_inner_brace, cmd_inner_bracket, cmd_inner_paren,
};
pub(crate) use line::{cmd_around_line, cmd_inner_line};
pub(crate) use quote::{
    cmd_around_backtick, cmd_around_double_quote, cmd_around_single_quote, cmd_inner_backtick,
    cmd_inner_double_quote, cmd_inner_single_quote,
};
pub(crate) use word::{
    apply_nearest_word_result, cmd_around_uppercase_word, cmd_around_word,
    cmd_inner_uppercase_word, cmd_inner_word, cmd_select_uppercase_word_around,
    cmd_select_word_around, cmd_select_word_nearest_on_line, expand_word_unit, inner_word_impl,
    nearest_word_on_line, word_unit_at,
};

// ── Text object framework ──────────────────────────────────────────────────────

/// Apply a text object to every selection in the set.
///
/// Unlike motions, which map a single cursor position to a new position, a
/// text object maps a cursor position to a *range* — the region to select.
/// `text_object` returns `Some((start, end))` as an inclusive char-offset pair,
/// or `None` if no match exists (e.g., cursor not inside any bracket pair).
///
/// On `None`, the existing selection is preserved — `mi(` when not inside parens
/// is a no-op. On `Some`, the selection is replaced with a
/// forward selection anchored at `start` and with head at `end`.
///
/// Uses `map` (which always merges) so that multiple cursors landing on the
/// same range (e.g., both cursors inside the same bracket pair) are merged.
pub(crate) fn apply_text_object(
    buf: &Text,
    sels: SelectionSet,
    text_object: impl Fn(&Text, usize) -> Option<(usize, usize)>,
) -> SelectionSet {
    let result = sels.map(|sel| match text_object(buf, sel.head()) {
        Some((start, end)) => Selection::new(start, end),
        None => sel,
    });
    result.debug_assert_valid(buf);
    result
}

/// Apply a text object in extend mode: union the matched range with the current selection.
///
/// On match, the result spans `min(sel.start(), start)` to `max(sel.end(), end)`,
/// preserving the direction of the original selection. On no-match, the selection
/// is unchanged.
///
/// Two-pass strategy for outward growth:
/// 1. Try `text_object(buf, sel.head)`. If the result is *larger* than the current
///    selection, use it — this handles the initial extend-from-cursor case.
/// 2. If the result is a subset (union doesn't grow), retry from the position just
///    past `sel.end()`. For bracket/quote text objects this escapes the current pair
///    and causes the search to find the next enclosing pair instead.
pub(crate) fn apply_text_object_extend(
    buf: &Text,
    sels: SelectionSet,
    text_object: impl Fn(&Text, usize) -> Option<(usize, usize)>,
) -> SelectionSet {
    let result = sels.map(|sel| {
        let forward = sel.anchor() <= sel.head();

        // First try from head (correct for initial extend from a cursor).
        if let Some((start, end)) = text_object(buf, sel.head()) {
            let new_start = sel.start().min(start);
            let new_end = sel.end().max(end);
            if new_start != sel.start() || new_end != sel.end() {
                return Selection::directed(new_start, new_end, forward);
            }
        }

        // Result was a subset (no growth). Retry from one past the selection end so
        // bracket/quote searches find the enclosing pair rather than the current one.
        let past_end = next_grapheme_boundary(buf, sel.end());
        if past_end < buf.len_chars()
            && let Some((start, end)) = text_object(buf, past_end)
        {
            let new_start = sel.start().min(start);
            let new_end = sel.end().max(end);
            return Selection::directed(new_start, new_end, forward);
        }

        sel
    });
    result.debug_assert_valid(buf);
    result
}

/// Dispatch to [`apply_text_object`] or [`apply_text_object_extend`] based on `mode`.
#[inline]
fn apply_text_object_by_mode(
    buf: &Text,
    sels: SelectionSet,
    mode: MotionMode,
    f: impl Fn(&Text, usize) -> Option<(usize, usize)>,
) -> SelectionSet {
    match mode {
        MotionMode::Move => apply_text_object(buf, sels, f),
        MotionMode::Extend => apply_text_object_extend(buf, sels, f),
    }
}

#[cfg(test)]
mod tests;
