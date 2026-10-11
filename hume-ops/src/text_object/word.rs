//! Word/WORD text objects (`iw`/`aw`, `iW`/`aW`) and the position-based
//! `mm`/`MM`/nearest-word-on-line family they share with visual-move.

use hume_editing::selection::{Facing, Selection, SelectionView};
use hume_editing::state::EditState;
use hume_editing::text::BufferText;
use hume_editing::word::{WordChars, blank_class, is_uppercase_word_boundary, is_word_boundary};
use hume_rope::cluster::{ClusterBound, ClusterRange, ClusterStart};

use super::apply_text_object_by_mode;
use crate::word_unit::{IsBoundary, is_blank_at, prev_nonblank, word_span_at};
use crate::{MotionMode, WhitespaceSide, WordCtx};

/// Find the nearest word within `[line_start, line_end)` from `head`.
///
/// - If `head` is on a word or punctuation char, returns its inner-word range
///   (identical to `inner_word_impl`), or the word plus its whitespace
///   bookend (identical to `word_unit_at`) when `whitespace` names a side.
/// - If `head` is on whitespace or EOL, scans left and right within the given
///   bounds to find the closest word. "Closest" is measured as the distance
///   from `head` to the nearest edge of each candidate word; ties go to the
///   previous (left) word. The winning word is then resolved the same way
///   (inner vs. around) as the direct-hit case.
/// - Returns `None` when no word exists within the bounds.
///
/// Callers supply bounds explicitly so this helper can be scoped to either a
/// buffer line (no-wrap path) or a visual sub-line (wrap path). `whitespace`
/// mirrors the effective `word-selects-whitespace` setting (see
/// `cmd_select_word_nearest_on_line` and `cmd_visual_select_word_nearest_on_line`).
pub fn nearest_word_on_line(
    text: &BufferText,
    head: ClusterStart,
    line_start: ClusterBound,
    line_end: ClusterBound,
    whitespace: Option<WhitespaceSide>,
    chars: WordChars<'_>,
) -> Option<ClusterRange> {
    let unit = |pos: ClusterStart| {
        word_span_at(
            text,
            pos,
            is_word_boundary,
            line_start,
            line_end,
            whitespace,
            chars,
        )
    };

    // Fast path: head is already on a word/punct, so delegate to inner/around unit.
    if !is_blank_at(text, head) {
        return unit(head);
    }

    // Scan LEFT within the given bounds for the first non-whitespace grapheme.
    let prev_anchor = prev_nonblank(text, head.into(), line_start);

    // Scan RIGHT within the given bounds for the first non-whitespace grapheme.
    let next_anchor = text
        .clusters()
        .graphemes_at(head.into())
        .skip(1)
        .take_while(|cluster| cluster.end() <= line_end)
        .find(|cluster| blank_class(cluster.first()).is_none())
        .map(|cluster| cluster.start());

    match (prev_anchor, next_anchor) {
        (None, None) => None,
        (Some(p), None) => unit(p),
        (None, Some(n)) => unit(n),
        (Some(p), Some(n)) => {
            // Pick the word whose nearest edge is closer to `head`; tie → prev.
            // `p` is the last cluster of the prev word's run (nearest edge = p itself).
            // `n` is the first cluster of the next word's run (nearest edge = n itself).
            let clusters_between = |from: ClusterStart, to: ClusterStart| {
                text.clusters()
                    .graphemes_at(from.into())
                    .take_while(|cluster| cluster.start() < to)
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
    sel: SelectionView<'_>,
    found: Option<ClusterRange>,
    mode: MotionMode,
) -> Selection {
    let Some(range) = found else {
        return sel.selection();
    };
    let s = match mode {
        MotionMode::Move => Selection::covering(range, Facing::Forward),
        MotionMode::Extend => sel.union(range, sel.facing()),
    };
    match sel.selection().sticky_display_col() {
        Some(sticky) => s.with_sticky(sticky),
        None => s,
    }
}

/// Select the word nearest the cursor on the same buffer line, snapping to it
/// when the cursor sits on whitespace. Preserves `sel.sticky_display_col` so
/// the sticky display column (set by `move-down` / `move-up`) survives
/// through this step.
///
/// `ctx.whitespace` mirrors the effective `word-selects-whitespace` setting:
/// with a side, the selected span includes the word's whitespace bookend
/// (matching `mm`); with `None`, only the inner word is selected.
///
/// In wrap mode, `cmd_visual_select_word_nearest_on_line` (in `editor/visual_move.rs`)
/// should be used instead. It scopes the search to the current visual sub-line,
/// preventing the snap from reaching across a wrap boundary.
///
/// In `Extend` mode the matched word range is unioned with the existing
/// selection, matching the behaviour of `inner-word` in extend mode.
pub fn cmd_select_word_nearest_on_line(
    state: EditState,
    _count: usize,
    ctx: WordCtx<'_>,
) -> EditState {
    state.map(|sel| {
        let text = sel.text();
        let line = text.lines().range(text.char_to_line(sel.anchor().offset()));
        let found = nearest_word_on_line(
            text,
            sel.anchor(),
            line.start().into(),
            line.end(),
            ctx.whitespace,
            ctx.chars,
        );
        apply_nearest_word_result(sel, found, ctx.mode)
    })
}

/// Shared dispatch for the four word-object commands below: resolves the
/// unit at each selection's position, parameterized by word class
/// (`is_boundary`: [`is_word_boundary`] or [`is_uppercase_word_boundary`])
/// and the whitespace side (`None` for `miw`/`miW`, `ctx.whitespace` for
/// `maw`/`maW`), per [`word_span_at`].
fn word_object_cmd(
    state: EditState,
    ctx: WordCtx<'_>,
    is_boundary: IsBoundary,
    whitespace: Option<WhitespaceSide>,
) -> EditState {
    apply_text_object_by_mode(state, ctx.mode, |t, pos| {
        word_span_at(
            t,
            pos,
            is_boundary,
            ClusterBound::TEXT_START,
            t.clusters().text_end(),
            whitespace,
            ctx.chars,
        )
    })
}

/// Generates one `cmd_*_word` fn delegating to [`word_object_cmd`].
/// `$around` selects with `ctx.whitespace`'s bookend; otherwise the inner word.
///
/// The command registry stores `fun` as a bare `fn` pointer (see
/// `hume-editor`'s `SelectionBody` doc), so each variant still needs its own
/// named item. Only the body is shared here, not the item itself.
macro_rules! word_object_variant {
    ($(#[$meta:meta])* $name:ident, $doc:expr, $is_boundary:expr, $around:literal) => {
        $(#[$meta])*
        #[doc = $doc]
        pub fn $name(state: EditState, _count: usize, ctx: WordCtx<'_>) -> EditState {
            let whitespace = if $around { ctx.whitespace } else { None };
            word_object_cmd(state, ctx, $is_boundary, whitespace)
        }
    };
}

word_object_variant!(
    cmd_inner_word,
    "Inner word (`mi w`): the run of same-class characters touching the cursor.",
    is_word_boundary,
    false
);
word_object_variant!(
    cmd_around_word,
    "Around word (`ma w`, and `mm`): the word under the cursor plus its \
     whitespace bookend on `ctx.whitespace`'s side, per \
     [`expand_word_unit`](crate::word_unit::expand_word_unit); the inner word \
     when `ctx.whitespace` is `None`. `Extend` unions it with the current \
     selection.",
    is_word_boundary,
    true
);
word_object_variant!(
    #[allow(non_snake_case)]
    cmd_inner_uppercase_word,
    "Inner WORD (`mi W`); see [`cmd_inner_word`].",
    is_uppercase_word_boundary,
    false
);
word_object_variant!(
    #[allow(non_snake_case)]
    cmd_around_uppercase_word,
    "Around WORD (`ma W`, and `MM`); see [`cmd_around_word`].",
    is_uppercase_word_boundary,
    true
);
