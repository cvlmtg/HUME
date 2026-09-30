//! Word/WORD text objects (`iw`/`aw`, `iW`/`aW`) and the position-based
//! `mm`/`MM`/nearest-word-on-line family they share with visual-move.

use hume_editing::grapheme::graphemes_at;
use hume_editing::lines::next_line_start;
use hume_editing::selection::{Selection, SelectionSet};
use hume_editing::text::BufferText;
use hume_editing::word::{WordChars, blank_class, is_uppercase_word_boundary, is_word_boundary};
use hume_rope::offset::{CharOffset, InclusiveRange};

use super::apply_text_object_by_mode;
use crate::word_unit::{IsBoundary, inner_word_impl, prev_nonblank, word_unit_at};
use crate::{MotionMode, WordCtx};

/// Find the nearest word within `[line_start, line_end_excl)` from `head`.
///
/// - If `head` is on a word or punctuation char, returns its inner-word range
///   (identical to `inner_word_impl`), or the word plus its whitespace
///   bookend (identical to `word_unit_at`) when `around` is set.
/// - If `head` is on whitespace or EOL, scans left and right within the given
///   bounds to find the closest word. "Closest" is measured as the distance
///   from `head` to the nearest edge of each candidate word; ties go to the
///   previous (left) word. The winning word is then resolved the same way
///   (inner vs. around) as the direct-hit case.
/// - Returns `None` when no word exists within the bounds.
///
/// Callers supply bounds explicitly so this helper can be scoped to either a
/// buffer line (no-wrap path) or a visual sub-line (wrap path). `around`
/// mirrors the effective `word-selects-whitespace` setting (see
/// `cmd_select_word_nearest_on_line` and `cmd_visual_select_word_nearest_on_line`).
pub fn nearest_word_on_line(
    text: &BufferText,
    head: CharOffset,
    line_start: CharOffset,
    line_end_excl: CharOffset,
    around: bool,
    chars: WordChars<'_>,
) -> Option<InclusiveRange<CharOffset>> {
    let unit = |pos: CharOffset| {
        if around {
            word_unit_at(text, pos, is_word_boundary, line_start, chars)
        } else {
            inner_word_impl(text, pos, is_word_boundary, chars)
        }
    };

    // Fast path: head is already on a word/punct, so delegate to inner/around unit.
    if blank_class(text.char_at(head)?).is_none() {
        return unit(head);
    }

    // Scan LEFT within the given bounds for the first non-whitespace grapheme.
    let prev_anchor = prev_nonblank(text, head, line_start);

    // Scan RIGHT within the given bounds for the first non-whitespace grapheme.
    let next_anchor = graphemes_at(text, head)
        .skip(1)
        .take_while(|cluster| cluster.start < line_end_excl)
        .find(|cluster| blank_class(cluster.first).is_none())
        .map(|cluster| cluster.start);

    match (prev_anchor, next_anchor) {
        (None, None) => None,
        (Some(p), None) => unit(p),
        (None, Some(n)) => unit(n),
        (Some(p), Some(n)) => {
            // Pick the word whose nearest edge is closer to `head`; tie → prev.
            // `p` is the last cluster of the prev word's run (nearest edge = p itself).
            // `n` is the first cluster of the next word's run (nearest edge = n itself).
            let clusters_between = |from, to| {
                graphemes_at(text, from)
                    .take_while(|cluster| cluster.start < to)
                    .count()
            };
            let dist_prev = clusters_between(p, head);
            let dist_next = clusters_between(head, n);
            let anchor = if dist_next < dist_prev { n } else { p };
            unit(anchor)
        }
    }
}

/// Apply the result of `nearest_word_on_line` to `sel` according to `mode`,
/// preserving `sel.sticky_display_col` throughout.
///
/// Returns `sel` unchanged when `found` is `None` (no candidate word in bounds).
pub fn apply_nearest_word_result(
    text: &BufferText,
    sel: Selection,
    found: Option<InclusiveRange<CharOffset>>,
    mode: MotionMode,
) -> Selection {
    let Some(range) = found else {
        return sel;
    };
    match mode {
        MotionMode::Move => {
            let s = Selection::from_span(range, true, text);
            match sel.sticky_display_col() {
                Some(sticky) => Selection::with_sticky_display_col(s.anchor(), s.head(), sticky),
                None => s,
            }
        }
        MotionMode::Extend => {
            let forward = sel.anchor() <= sel.head();
            let s = sel.union_span(range, forward, text);
            match sel.sticky_display_col() {
                Some(sticky) => Selection::with_sticky_display_col(s.anchor(), s.head(), sticky),
                None => s,
            }
        }
    }
}

/// Select the word nearest the cursor on the same buffer line, snapping to it
/// when the cursor sits on whitespace. Preserves `sel.sticky_display_col` so
/// the sticky display column (set by `move-down` / `move-up`) survives
/// through this step.
///
/// `around` mirrors the effective `word-selects-whitespace` setting: when set,
/// the selected span includes the word's whitespace bookend (matching `mm`);
/// when unset, only the inner word is selected.
///
/// In wrap mode, `cmd_visual_select_word_nearest_on_line` (in `editor/visual_move.rs`)
/// should be used instead. It scopes the search to the current visual sub-line,
/// preventing the snap from reaching across a wrap boundary.
///
/// In `Extend` mode the matched word range is unioned with the existing
/// selection, matching the behaviour of `inner-word` in extend mode.
pub fn cmd_select_word_nearest_on_line(
    text: &BufferText,
    sels: SelectionSet,
    _count: usize,
    ctx: WordCtx<'_>,
) -> SelectionSet {
    let result = sels.map(|sel| {
        let line = text.char_to_line(sel.anchor());
        let line_start = text.line_to_char(line.into());
        let line_end_excl = next_line_start(text, line.into());
        let found = nearest_word_on_line(
            text,
            sel.anchor(),
            line_start,
            line_end_excl,
            ctx.around,
            ctx.chars,
        );
        apply_nearest_word_result(text, sel, found, ctx.mode)
    });
    result.debug_assert_valid(text);
    result
}

type WordUnitFn =
    fn(&BufferText, CharOffset, IsBoundary, WordChars<'_>) -> Option<InclusiveRange<CharOffset>>;

/// [`word_unit_at`] with `min_start` pinned to `0`: the shape every
/// text-object command below needs, as opposed to the sticky-column motion
/// path, which passes a nonzero visual-line floor.
fn around_unit(
    text: &BufferText,
    pos: CharOffset,
    is_boundary: IsBoundary,
    chars: WordChars<'_>,
) -> Option<InclusiveRange<CharOffset>> {
    word_unit_at(text, pos, is_boundary, CharOffset::new(0), chars)
}

/// Shared dispatch for the four word-object commands below: resolves the
/// unit at each selection's position, parameterized by word class
/// (`is_boundary`: [`is_word_boundary`] or [`is_uppercase_word_boundary`])
/// and inner-vs-around (`word_unit`: [`inner_word_impl`] or [`around_unit`]).
fn word_object_cmd(
    text: &BufferText,
    sels: SelectionSet,
    ctx: WordCtx<'_>,
    is_boundary: IsBoundary,
    word_unit: WordUnitFn,
) -> SelectionSet {
    apply_text_object_by_mode(text, sels, ctx.mode, |b, pos| {
        word_unit(b, pos, is_boundary, ctx.chars)
    })
}

/// Generates one `cmd_*_word` fn delegating to [`word_object_cmd`].
///
/// The command registry stores `fun` as a bare `fn` pointer (see
/// `hume-editor`'s `SelectionBody` doc), so each variant still needs its own
/// named item. Only the body is shared here, not the item itself.
macro_rules! word_object_variant {
    ($(#[$meta:meta])* $name:ident, $doc:expr, $is_boundary:expr, $word_unit:expr) => {
        $(#[$meta])*
        #[doc = $doc]
        pub fn $name(
            text: &BufferText,
            sels: SelectionSet,
            _count: usize,
            ctx: WordCtx<'_>,
        ) -> SelectionSet {
            word_object_cmd(text, sels, ctx, $is_boundary, $word_unit)
        }
    };
}

word_object_variant!(
    cmd_inner_word,
    "Inner word (`mi w`): the run of same-class characters touching the cursor.",
    is_word_boundary,
    inner_word_impl
);
word_object_variant!(
    cmd_around_word,
    "Around word (`ma w`): same span as `mm` when `word-selects-whitespace` is \
     on (see [`cmd_select_word`]), under a separate name because it stays \
     available (ignoring `ctx.around`) regardless of that setting.",
    is_word_boundary,
    around_unit
);
word_object_variant!(
    #[allow(non_snake_case)]
    cmd_inner_uppercase_word,
    "Inner WORD (`mi W`); see [`cmd_inner_word`].",
    is_uppercase_word_boundary,
    inner_word_impl
);
word_object_variant!(
    #[allow(non_snake_case)]
    cmd_around_uppercase_word,
    "Around WORD (`ma W`); see [`cmd_around_word`].",
    is_uppercase_word_boundary,
    around_unit
);

/// Select the word under the cursor (`mm`): the inner word, or (when
/// `ctx.around`, the effective `word-selects-whitespace`, is set) the same
/// unit `maw`/[`cmd_around_word`] selects, covering its surrounding
/// whitespace per [`expand_word_unit`](crate::word_unit::expand_word_unit). Both modes use the same unit;
/// `Extend` unions it with the current selection via
/// `apply_text_object_extend`.
pub fn cmd_select_word(
    text: &BufferText,
    sels: SelectionSet,
    count: usize,
    ctx: WordCtx<'_>,
) -> SelectionSet {
    if ctx.around {
        cmd_around_word(text, sels, count, ctx)
    } else {
        cmd_inner_word(text, sels, count, ctx)
    }
}

/// Select the WORD under the cursor (`MM`); see [`cmd_select_word`].
#[allow(non_snake_case)]
pub fn cmd_select_uppercase_word(
    text: &BufferText,
    sels: SelectionSet,
    count: usize,
    ctx: WordCtx<'_>,
) -> SelectionSet {
    if ctx.around {
        cmd_around_uppercase_word(text, sels, count, ctx)
    } else {
        cmd_inner_uppercase_word(text, sels, count, ctx)
    }
}
