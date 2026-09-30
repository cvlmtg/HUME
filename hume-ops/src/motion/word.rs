use super::MotionMode;
use crate::WordCtx;
use crate::word_unit::{
    IsBoundary, anchor_unit, class_at, expand_word_unit, find_word_end_from, find_word_start_from,
    is_blank_at, prev_nonblank, word_unit_at,
};
use hume_editing::grapheme::{first_cluster, graphemes_at, last_cluster, prev_cluster};
use hume_editing::selection::{Facing, Selection};
use hume_editing::state::EditState;
use hume_editing::text::BufferText;
use hume_editing::word::{CharClass, WordChars, is_uppercase_word_boundary, is_word_boundary};
use hume_rope::cluster::{ClusterBound, ClusterRange, ClusterStart};

// ── Word motions (inner) ──────────────────────────────────────────────────────

/// The start of the next word.
///
/// Pair-scan forward: stop when the category changes AND the next cluster is
/// either Eol or not Space. This skips the current word/punct, skips spaces
/// (but not newlines), and lands on the next word/punct start or on a newline.
/// With no next word it lands on the structural `\n`.
///
/// The `is_boundary` parameter is `is_word_boundary` for `w` and
/// `is_uppercase_word_boundary` for `W`. `chars` folds this buffer's extra
/// word characters into every classification (see [`WordChars::classify`]).
pub(super) fn next_word_start(
    text: &BufferText,
    head: ClusterStart,
    is_boundary: impl Fn(CharClass, CharClass) -> bool,
    chars: WordChars<'_>,
) -> ClusterStart {
    let mut prev_class = class_at(text, head, chars);
    for cluster in graphemes_at(text, head.into()).skip(1) {
        let cur_class = chars.classify(cluster.first());
        if is_boundary(prev_class, cur_class)
            && (cur_class == CharClass::Eol || cur_class != CharClass::Space)
        {
            return cluster.start();
        }
        prev_class = cur_class;
    }
    last_cluster(text)
}

/// The start of the previous word.
///
/// Two-phase backward scan: skip Space/Eol backward, then skip backward while
/// in the same category, landing on the first cluster of that group.
pub(crate) fn prev_word_start(
    text: &BufferText,
    head: ClusterStart,
    is_boundary: impl Fn(CharClass, CharClass) -> bool,
    chars: WordChars<'_>,
) -> ClusterStart {
    // Skip whitespace and line ends backward; nothing but whitespace before
    // `head` lands at the buffer start.
    let pos = prev_nonblank(text, head.into(), ClusterBound::TEXT_START)
        .unwrap_or_else(|| first_cluster(text));
    find_word_start_from(text, pos, is_boundary, chars)
}

/// Every maximal run of `Word`-class grapheme clusters in `text`, in order:
/// the whole-text counterpart to `find_word_end_from`'s single-run query.
/// Each cluster is classified by its first char, the same classification
/// `w`/`b` step by, so a run found here is what `w`/`b` would select, the
/// property `core:buffer-words`' Steel-side `split-words` builtin
/// (`hume-scripting/src/builtins/words.rs`) depends on.
pub fn word_runs(text: &BufferText, chars: WordChars<'_>) -> Vec<ClusterRange> {
    let mut runs = Vec::new();
    let mut open: Option<ClusterRange> = None;
    for cluster in graphemes_at(text, ClusterBound::TEXT_START) {
        if chars.classify(cluster.first()) == CharClass::Word {
            open = Some(open.map_or(cluster.range(), |run| run.hull(cluster.range())));
        } else if let Some(run) = open.take() {
            runs.push(run);
        }
    }
    runs.extend(open);
    runs
}

// ── Word-select helpers ───────────────────────────────────────────────────────

/// Find the next word (or WORD) from `pos` and return it.
///
/// Returns `None` when there is no next word: at the last word in the buffer
/// (no-op) or on an empty buffer.
///
/// Unlike `next_word_start`, this function crosses line boundaries: if the
/// scan lands on a newline between lines, it calls `next_word_start` a second
/// time from the newline to reach the first word on the next line.
pub(super) fn select_next_word(
    text: &BufferText,
    pos: ClusterStart,
    is_boundary: impl Fn(CharClass, CharClass) -> bool + Copy,
    chars: WordChars<'_>,
) -> Option<ClusterRange> {
    let last = last_cluster(text);
    let mut word_start = next_word_start(text, pos, is_boundary, chars);

    // Landed on a newline that is NOT the structural one: cross the line to
    // the next line's word.
    if word_start < last && class_at(text, word_start, chars) == CharClass::Eol {
        word_start = next_word_start(text, word_start, is_boundary, chars);
    }

    // The structural '\n': no next word.
    if word_start >= last || is_blank_at(text, word_start) {
        return None;
    }

    let word_end = find_word_end_from(text, word_start, is_boundary, chars);
    ClusterRange::through(text.full_slice(), word_start, word_end)
}

/// Find the previous word (or WORD) from `pos` and return it.
///
/// Returns `None` when there is no previous word: already at or before the
/// first word in the buffer (no-op).
///
/// If `pos` is inside a word, we jump to the word BEFORE the current one (not
/// the start of the current word). If `pos` is in whitespace or at the start
/// of a word, we jump to the preceding word.
pub(super) fn select_prev_word(
    text: &BufferText,
    pos: ClusterStart,
    is_boundary: impl Fn(CharClass, CharClass) -> bool + Copy,
    chars: WordChars<'_>,
) -> Option<ClusterRange> {
    prev_cluster(text, pos.into())?;

    let word_start = prev_word_start(text, pos, is_boundary, chars);
    // Whitespace here means there is no word to jump to (e.g. the buffer
    // starts with spaces).
    if is_blank_at(text, word_start) {
        return None;
    }
    let word = ClusterRange::through(
        text.full_slice(),
        word_start,
        find_word_end_from(text, word_start, is_boundary, chars),
    )?;

    // `pos` inside the word means `prev_word_start` landed on the CURRENT
    // word: one more step back.
    if word.contains(pos) {
        prev_cluster(text, word_start.into())?;
        let prev_start = prev_word_start(text, word_start, is_boundary, chars);
        if is_blank_at(text, prev_start) {
            return None;
        }
        let prev_end = find_word_end_from(text, prev_start, is_boundary, chars);
        return ClusterRange::through(text.full_slice(), prev_start, prev_end);
    }
    Some(word)
}

/// Apply a word-select motion to every selection, repeated `count` times.
///
/// `motion` returns the selected word, and each hop replaces the selection
/// with it, facing forward. `None` stops early and keeps the last selection.
/// With `around`, the final word grows by its whitespace bookend
/// ([`expand_word_unit`]), unless the loop never moved.
///
/// Forward motions search from the head. Backward motions search from the
/// selection's first cluster: `select_prev_word` detects re-landing on the
/// current word by checking whether the origin is inside it, and after a
/// first-word-on-line `around` landing the head sits in trailing whitespace
/// outside the word, which would re-select the same word on every press.
pub(super) fn apply_word_select(
    state: EditState,
    count: usize,
    around: bool,
    backward: bool,
    motion: impl Fn(&BufferText, ClusterStart) -> Option<ClusterRange>,
) -> EditState {
    let text = state.text().clone();
    state.map(|sel| {
        let mut current = sel.selection();
        let mut word = None;
        for _ in 0..count {
            let origin = if backward {
                current.anchor().min(current.head())
            } else {
                current.head()
            };
            match motion(&text, origin) {
                Some(range) => {
                    current = Selection::covering(range, Facing::Forward);
                    word = Some(range);
                }
                None => break,
            }
        }
        match word {
            Some(range) if around => Selection::covering(
                expand_word_unit(&text, range, ClusterBound::TEXT_START),
                Facing::Forward,
            ),
            _ => current,
        }
    })
}

/// Apply a word-select motion in extend mode: grow toward the target word if
/// it lies beyond the anchor's unit, shrink toward it if it has crossed back
/// onto or past that unit. Replaces the old selection rather than unioning.
///
/// The origin is the head, so repeated presses walk word by word. With
/// `around`, the anchor's unit is [`word_unit_at`] (leading whitespace
/// included) instead of [`anchor_unit`], and a backward-growing target's head
/// is expanded the same way. Comparisons use the target's raw bounds against
/// the expanded anchor unit, so the one-space overlap between adjacent units
/// is never double-counted.
///
/// A target lies wholly beyond, behind, or on the anchor's unit, so that unit
/// is always kept whole: crossing it flips direction without truncating.
/// `None` from `motion` stops early and keeps the last selection.
pub(super) fn apply_word_select_extend(
    state: EditState,
    count: usize,
    around: bool,
    is_boundary: impl Fn(CharClass, CharClass) -> bool + Copy,
    chars: WordChars<'_>,
    motion: impl Fn(&BufferText, ClusterStart) -> Option<ClusterRange>,
) -> EditState {
    let text = state.text().clone();
    state.map(|sel| {
        let mut current = sel.selection();
        for _ in 0..count {
            let Some(target) = motion(&text, current.head()) else {
                break;
            };
            // `word_unit_at` returns `None` when the anchor sits on
            // whitespace with no adjacent word (e.g. indentation at the very
            // start of the buffer): the bare anchor cluster stands in, as
            // `anchor_unit` yields there.
            let unit = around
                .then(|| {
                    word_unit_at(
                        &text,
                        current.anchor(),
                        is_boundary,
                        ClusterBound::TEXT_START,
                        chars,
                    )
                })
                .flatten()
                .unwrap_or_else(|| anchor_unit(&text, current.anchor(), is_boundary, chars));
            current = if target.start() > unit.last() {
                Selection::covering(unit.hull(target), Facing::Forward)
            } else if target.last() < unit.start() {
                let head = if around {
                    expand_word_unit(&text, target, ClusterBound::TEXT_START)
                } else {
                    target
                };
                Selection::covering(head.hull(unit), Facing::Backward)
            } else {
                Selection::covering(unit, Facing::Forward)
            };
        }
        current
    })
}

type SelectWord = fn(&BufferText, ClusterStart, IsBoundary, WordChars<'_>) -> Option<ClusterRange>;

/// Shared dispatch for the four word-select commands below: branches on
/// `ctx.mode` (fresh re-anchor for `Move`, grow/shrink for `Extend`, see
/// [`apply_word_select`]/[`apply_word_select_extend`]), parameterized by
/// direction (`select_word`: [`select_next_word`] or [`select_prev_word`])
/// and word class (`is_boundary`: [`is_word_boundary`] or
/// [`is_uppercase_word_boundary`]).
///
/// `backward` only affects the `Move` arm's search origin (see
/// [`apply_word_select`]'s doc); `Extend`'s chaining always uses the head and
/// has no analogous asymmetry. `ctx.around` affects both arms identically:
/// a plain field read deciding whether whitespace is included in the unit.
fn word_select_cmd(
    state: EditState,
    count: usize,
    ctx: WordCtx<'_>,
    backward: bool,
    is_boundary: IsBoundary,
    select_word: SelectWord,
) -> EditState {
    match ctx.mode {
        MotionMode::Move => apply_word_select(state, count, ctx.around, backward, |b, pos| {
            select_word(b, pos, is_boundary, ctx.chars)
        }),
        MotionMode::Extend => apply_word_select_extend(
            state,
            count,
            ctx.around,
            is_boundary,
            ctx.chars,
            |b, pos| select_word(b, pos, is_boundary, ctx.chars),
        ),
    }
}

/// Generates one `cmd_select_*` fn delegating to [`word_select_cmd`].
///
/// The command registry stores `fun` as a bare `fn` pointer (see
/// `SelectionBody`'s doc), so each variant still needs its own named item.
/// Only the body is shared here, not the item itself.
macro_rules! word_select_variant {
    ($name:ident, $doc:expr, $backward:expr, $is_boundary:expr, $select_word:expr) => {
        #[doc = $doc]
        pub fn $name(state: EditState, count: usize, ctx: WordCtx<'_>) -> EditState {
            word_select_cmd(state, count, ctx, $backward, $is_boundary, $select_word)
        }
    };
}

word_select_variant!(
    cmd_select_next_word,
    "Select or extend to the next word (`w`), covering its whitespace \
     bookend in both modes when `ctx.around` is set (`word-selects-whitespace`).",
    false,
    is_word_boundary,
    select_next_word
);
word_select_variant!(
    cmd_select_next_uppercase_word,
    "Select or extend to the next WORD (`W`): like `w` but treats \
     word+punct as one class.",
    false,
    is_uppercase_word_boundary,
    select_next_word
);
word_select_variant!(
    cmd_select_prev_word,
    "Select or extend to the previous word (`b`), covering its whitespace \
     bookend in both modes when `ctx.around` is set. See [`cmd_select_next_word`].",
    true,
    is_word_boundary,
    select_prev_word
);
word_select_variant!(
    cmd_select_prev_uppercase_word,
    "Select or extend to the previous WORD (`B`): like `b` but treats \
     word+punct as one class.",
    true,
    is_uppercase_word_boundary,
    select_prev_word
);
