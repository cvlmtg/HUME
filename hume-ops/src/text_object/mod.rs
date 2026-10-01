use hume_editing::selection::{Facing, Selection};
use hume_editing::state::EditState;
use hume_editing::text::BufferText;
use hume_editing::word::blank_class;
use hume_rope::cluster::{ClusterRange, ClusterStart};
use hume_rope::grapheme::Cluster;

use crate::MotionMode;

mod argument;
mod bracket;
mod line;
mod paragraph;
mod quote;
mod word;

pub use crate::word_unit::{expand_word_unit, inner_word_impl, word_unit_at};
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
    cmd_select_word_nearest_on_line, nearest_word_on_line,
};

// ── Text object framework ──────────────────────────────────────────────────────

/// Apply a text object to every selection in the set.
///
/// Unlike motions, which map a single cursor position to a new position, a
/// text object maps a cursor position to a *range*: the region to select.
/// `text_object` returns `Some(range)`, or `None` if no match exists (e.g.,
/// cursor not inside any bracket pair).
///
/// On `None`, the existing selection is preserved: `mi(` when not inside parens
/// is a no-op. On `Some`, the selection is replaced with a forward selection
/// covering the range.
///
/// Uses `map` (which always merges) so that multiple cursors landing on the
/// same range (e.g., both cursors inside the same bracket pair) are merged.
fn apply_text_object(
    state: EditState,
    text_object: impl Fn(&BufferText, ClusterStart) -> Option<ClusterRange>,
) -> EditState {
    state.map(|sel| match text_object(sel.text(), sel.head()) {
        Some(range) => Selection::covering(range, Facing::Forward),
        None => sel.selection(),
    })
}

/// Apply a text object in extend mode: union the matched range with the current selection.
///
/// On match, the result covers both the selection and the range, keeping the
/// selection's facing. On no-match, the selection is unchanged.
///
/// Two-pass strategy for outward growth:
/// 1. Try `text_object(text, sel.head)`. If the result is *larger* than the current
///    selection, use it. This handles the initial extend-from-cursor case.
/// 2. If the result is a subset (union doesn't grow), retry from the cluster just
///    past the selection. For bracket/quote text objects this escapes the current
///    pair and causes the search to find the next enclosing pair instead.
fn apply_text_object_extend(
    state: EditState,
    text_object: impl Fn(&BufferText, ClusterStart) -> Option<ClusterRange>,
) -> EditState {
    state.map(|sel| {
        let text = sel.text();

        // First try from head (correct for initial extend from a cursor).
        if let Some(found) = text_object(text, sel.head()) {
            let grown = sel.union(found, sel.facing());
            if sel.with_selection(grown).covered() != sel.covered() {
                return grown;
            }
        }

        // Result was a subset (no growth). Retry from the cluster past the
        // selection so bracket/quote searches find the enclosing pair rather
        // than the current one.
        text.clusters()
            .next(sel.last())
            .and_then(|past| text_object(text, past))
            .map_or(sel.selection(), |found| sel.union(found, sel.facing()))
    })
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
    state: EditState,
    mode: MotionMode,
    f: impl Fn(&BufferText, ClusterStart) -> Option<ClusterRange>,
) -> EditState {
    match mode {
        MotionMode::Move => apply_text_object(state, f),
        MotionMode::Extend => apply_text_object_extend(state, f),
    }
}

#[cfg(test)]
mod tests;

/// The clusters strictly between a delimiter pair's first and last cluster,
/// or `None` when the delimiters are adjacent.
pub(super) fn inner_of_pair(text: &BufferText, pair: ClusterRange) -> Option<ClusterRange> {
    let start = text.clusters().next(pair.start())?;
    ClusterRange::between(text.full_slice(), start, pair.last().into())
}

/// Shrinks `range` inward until both ends sit on non-blank clusters (per
/// [`blank_class`]). `None` if the whole range is blank.
pub(crate) fn trim_blank(text: &BufferText, range: ClusterRange) -> Option<ClusterRange> {
    let is_content = |c: &Cluster| blank_class(c.first()).is_none();
    let first = text
        .clusters()
        .graphemes_at(range.start().into())
        .take_while(|c| c.end() <= range.end())
        .find(is_content)?;
    let last = text
        .clusters()
        .before(range.end())
        .take_while(|c| c.start() >= first.start())
        .find(is_content)?;
    ClusterRange::through(text.full_slice(), first.start(), last.start())
}
