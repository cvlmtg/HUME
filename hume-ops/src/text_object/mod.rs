use hume_editing::grapheme::next_grapheme_boundary;
use hume_editing::selection::{Selection, SelectionSet};
use hume_editing::text::BufferText;

use crate::MotionMode;

mod argument;
mod bracket;
mod line;
mod paragraph;
mod quote;
mod word;

pub use argument::{around_argument, around_from_inner, inner_argument};
pub use bracket::{
    cmd_around_angle, cmd_around_brace, cmd_around_bracket, cmd_around_paren, cmd_inner_angle,
    cmd_inner_brace, cmd_inner_bracket, cmd_inner_paren,
};
pub use line::{cmd_around_line, cmd_inner_line};
pub use paragraph::{cmd_around_paragraph, cmd_inner_paragraph};
pub use quote::{
    cmd_around_backtick, cmd_around_double_quote, cmd_around_single_quote, cmd_inner_backtick,
    cmd_inner_double_quote, cmd_inner_single_quote,
};
pub use word::{
    apply_nearest_word_result, cmd_around_uppercase_word, cmd_around_word,
    cmd_inner_uppercase_word, cmd_inner_word, cmd_select_uppercase_word, cmd_select_word,
    cmd_select_word_nearest_on_line, expand_word_unit, inner_word_impl, nearest_word_on_line,
    word_unit_at,
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
    text: &BufferText,
    sels: SelectionSet,
    text_object: impl Fn(&BufferText, usize) -> Option<(usize, usize)>,
) -> SelectionSet {
    let result = sels.map(|sel| match text_object(text, sel.head()) {
        Some((start, end)) => Selection::new(start, end),
        None => sel,
    });
    result.debug_assert_valid(text);
    result
}

/// Apply a text object in extend mode: union the matched range with the current selection.
///
/// On match, the result spans `min(sel.start(), start)` to `max(sel.end(), end)`,
/// preserving the direction of the original selection. On no-match, the selection
/// is unchanged.
///
/// Two-pass strategy for outward growth:
/// 1. Try `text_object(text, sel.head)`. If the result is *larger* than the current
///    selection, use it — this handles the initial extend-from-cursor case.
/// 2. If the result is a subset (union doesn't grow), retry from the position just
///    past `sel.end()`. For bracket/quote text objects this escapes the current pair
///    and causes the search to find the next enclosing pair instead.
pub(crate) fn apply_text_object_extend(
    text: &BufferText,
    sels: SelectionSet,
    text_object: impl Fn(&BufferText, usize) -> Option<(usize, usize)>,
) -> SelectionSet {
    let result = sels.map(|sel| {
        let forward = sel.anchor() <= sel.head();

        // First try from head (correct for initial extend from a cursor).
        if let Some(found) = text_object(text, sel.head()) {
            let grown = sel.union_span(found, forward);
            if grown.start() != sel.start() || grown.end() != sel.end() {
                return grown;
            }
        }

        // Result was a subset (no growth). Retry from one past the selection end so
        // bracket/quote searches find the enclosing pair rather than the current one.
        let past_end = next_grapheme_boundary(text, sel.end());
        if past_end < text.len_chars()
            && let Some(found) = text_object(text, past_end)
        {
            return sel.union_span(found, forward);
        }

        sel
    });
    result.debug_assert_valid(text);
    result
}

/// Apply a text object to every selection in the set, honoring `mode`: `Move`
/// replaces each selection with the matched range; `Extend` unions the match
/// with the current selection, retrying past the selection's end when the
/// first match would not grow it (the outward-walk described above). A
/// selection with no match is preserved unchanged in both modes. The second,
/// cross-crate caller of this exact contract is `hume-editor`'s structural
/// text objects, dispatching through a tree-sitter-backed finder rather than
/// a lexical one.
#[inline]
pub fn apply_text_object_by_mode(
    text: &BufferText,
    sels: SelectionSet,
    mode: MotionMode,
    f: impl Fn(&BufferText, usize) -> Option<(usize, usize)>,
) -> SelectionSet {
    match mode {
        MotionMode::Move => apply_text_object(text, sels, f),
        MotionMode::Extend => apply_text_object_extend(text, sels, f),
    }
}

#[cfg(test)]
mod tests;
