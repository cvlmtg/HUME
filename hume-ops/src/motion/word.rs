use super::MotionMode;
use crate::WordCtx;
use crate::word_unit::{
    IsBoundary, anchor_unit, class_at, expand_word_unit, find_word_end_from, find_word_start_from,
    is_blank_at, prev_nonblank, word_unit_at,
};
use hume_editing::grapheme::graphemes_at;
use hume_editing::selection::{Selection, SelectionSet};
use hume_editing::text::BufferText;
use hume_editing::word::{CharClass, WordChars, is_uppercase_word_boundary, is_word_boundary};
use hume_rope::offset::{CharOffset, InclusiveRange};

// ── Word motions (inner) ──────────────────────────────────────────────────────

/// Move to the start of the next word.
///
/// Pair-scan forward: stop when the category changes AND the next char is
/// either Eol or not Space. This skips the current word/punct, skips spaces
/// (but not newlines), and lands on the next word/punct start or on a newline.
///
/// The `is_boundary` parameter is `is_word_boundary` for `w` and
/// `is_uppercase_word_boundary` for `W`. `chars` folds this buffer's extra
/// word characters into every classification (see [`WordChars::classify`]).
pub(super) fn next_word_start(
    text: &BufferText,
    head: CharOffset,
    is_boundary: impl Fn(CharClass, CharClass) -> bool,
    chars: WordChars<'_>,
) -> CharOffset {
    let end = text.end();
    if head >= end {
        return head;
    }

    // Whole grapheme clusters, never single chars. This matters for
    // combining sequences like e + U+0301 (combining acute): stepping by one
    // char would land on the combining codepoint, which classify_char sees as
    // Punctuation, creating a false word boundary inside the grapheme.
    let mut clusters = graphemes_at(text, head);
    let first = clusters.next().expect("head < end");
    let mut prev_class = chars.classify(first.first);
    let mut pos = first.end;
    for cluster in clusters {
        let cur_class = chars.classify(cluster.first);
        if is_boundary(prev_class, cur_class)
            && (cur_class == CharClass::Eol || cur_class != CharClass::Space)
        {
            return cluster.start;
        }
        prev_class = cur_class;
        pos = cluster.end;
    }
    // Clamp to last valid position (the trailing \n).
    pos.min(text.last_char())
}

/// Move to the start of the previous word.
///
/// Two-phase backward scan: skip Space/Eol backward, then skip backward while
/// in the same category, landing on the first char of that group.
pub(crate) fn prev_word_start(
    text: &BufferText,
    head: CharOffset,
    is_boundary: impl Fn(CharClass, CharClass) -> bool,
    chars: WordChars<'_>,
) -> CharOffset {
    // Skip whitespace and line ends backward; nothing but whitespace before
    // `head` lands at the buffer start.
    let pos = prev_nonblank(text, head, CharOffset::new(0)).unwrap_or(CharOffset::new(0));

    // Then skip backward while in the same category.
    find_word_start_from(text, pos, is_boundary, chars)
}

/// Every maximal run of `Word`-class grapheme clusters in `text`, in order:
/// the whole-text counterpart to `find_word_end_from`'s single-run query.
/// Each cluster is classified by its first char, the same classification
/// `w`/`b` step by, so a run found here is what `w`/`b` would select, the
/// property `core:buffer-words`' Steel-side `split-words` builtin
/// (`hume-scripting/src/builtins/words.rs`) depends on. A run ends on the
/// last char of its last cluster, so a trailing combining mark stays in it.
pub fn word_runs(text: &BufferText, chars: WordChars<'_>) -> Vec<InclusiveRange<CharOffset>> {
    let mut runs = Vec::new();
    let mut open: Option<InclusiveRange<CharOffset>> = None;
    for cluster in graphemes_at(text, CharOffset::new(0)) {
        if chars.classify(cluster.first) == CharClass::Word {
            open = Some(InclusiveRange::new(
                open.map_or(cluster.start, |run| run.start),
                cluster.last_char(),
            ));
        } else if let Some(run) = open.take() {
            runs.push(run);
        }
    }
    runs.extend(open);
    runs
}

// ── Word-select helpers ───────────────────────────────────────────────────────

/// Find the next word (or WORD) from `pos` and return its span.
///
/// Returns `None` when there is no next word: at the last word in the buffer
/// (no-op) or on an empty buffer.
///
/// Unlike `next_word_start`, this function crosses line boundaries: if the
/// scan lands on a newline between lines, it calls `next_word_start` a second
/// time from the newline to reach the first word on the next line.
pub(super) fn select_next_word(
    text: &BufferText,
    pos: CharOffset,
    is_boundary: impl Fn(CharClass, CharClass) -> bool + Copy,
    chars: WordChars<'_>,
) -> Option<InclusiveRange<CharOffset>> {
    let last = text.last_char();

    // Find the start of the next word.
    let mut word_start = next_word_start(text, pos, is_boundary, chars);

    // If we landed on a newline that is NOT the trailing '\n', cross the line:
    // call next_word_start again from that newline to get to the next line's word.
    if word_start < last && class_at(text, word_start, chars) == CharClass::Eol {
        word_start = next_word_start(text, word_start, is_boundary, chars);
    }

    // If we've hit the trailing '\n' (last char in the buffer), there is no
    // next word, so treat this as a no-op.
    if word_start >= last {
        return None;
    }

    // Guard: if we somehow landed on whitespace, also a no-op.
    if is_blank_at(text, word_start) {
        return None;
    }

    let word_end = find_word_end_from(text, word_start, is_boundary, chars);
    Some(InclusiveRange::new(word_start, word_end))
}

/// Find the previous word (or WORD) from `pos` and return its span.
///
/// Returns `None` when there is no previous word: already at or before the
/// first word in the buffer (no-op).
///
/// If `pos` is inside a word, we jump to the word BEFORE the current one (not
/// the start of the current word). If `pos` is in whitespace or at the start
/// of a word, we jump to the preceding word.
pub(super) fn select_prev_word(
    text: &BufferText,
    pos: CharOffset,
    is_boundary: impl Fn(CharClass, CharClass) -> bool + Copy,
    chars: WordChars<'_>,
) -> Option<InclusiveRange<CharOffset>> {
    if pos == CharOffset::new(0) {
        return None;
    }

    // Find the start of the word `prev_word_start` would land on.
    let word_start = prev_word_start(text, pos, is_boundary, chars);

    // If that position is whitespace (e.g. buffer starts with spaces), there
    // is no actual word to jump to.
    if is_blank_at(text, word_start) {
        return None;
    }

    let word_end = find_word_end_from(text, word_start, is_boundary, chars);

    // If pos is within [word_start, word_end], prev_word_start landed on the
    // CURRENT word, not the previous one. We need one more step backward.
    if pos >= word_start && pos <= word_end {
        if word_start == CharOffset::new(0) {
            return None; // already at the first word, no-op
        }
        let prev_start = prev_word_start(text, word_start, is_boundary, chars);
        if is_blank_at(text, prev_start) {
            return None; // no word before this one
        }
        let prev_end = find_word_end_from(text, prev_start, is_boundary, chars);
        return Some(InclusiveRange::new(prev_start, prev_end));
    }

    Some(InclusiveRange::new(word_start, word_end))
}

/// Apply a word-select motion to every selection in the set, repeated `count` times.
///
/// `motion` returns the selected word's span, and each hop replaces the
/// selection with a fresh forward `[word_start, word_end]`. `None` stops early
/// and keeps the last selection. With `around`, the final span grows by its
/// whitespace bookend ([`expand_word_unit`]), unless the loop never moved.
///
/// Forward motions search from `head()`. Backward motions search from
/// `start()`: `select_prev_word` detects re-landing on the current word by
/// checking whether the origin is inside it, and after a first-word-on-line
/// `around` landing `head()` sits in trailing whitespace outside the word,
/// which would re-select the same word on every press.
pub(super) fn apply_word_select(
    text: &BufferText,
    sels: SelectionSet,
    count: usize,
    around: bool,
    backward: bool,
    motion: impl Fn(&BufferText, CharOffset) -> Option<InclusiveRange<CharOffset>>,
) -> SelectionSet {
    let result = sels.map(|sel| {
        let mut current = sel;
        let mut moved = false;
        for _ in 0..count {
            let origin = if backward {
                current.start()
            } else {
                current.head()
            };
            match motion(text, origin) {
                Some(range) => {
                    current = Selection::new(range.start, range.end);
                    moved = true;
                }
                None => break, // no more words: stop early, keep last selection
            }
        }
        if around && moved {
            let range = expand_word_unit(text, current.start(), current.end(), CharOffset::new(0));
            current = Selection::new(range.start, range.end);
        }
        current
    });
    result.debug_assert_valid(text);
    result
}

/// Apply a word-select motion in extend mode: grow toward the target word if
/// it lies beyond the anchor's unit, shrink toward it if it has crossed back
/// onto or past that unit. Replaces the old selection rather than unioning.
///
/// The origin is `sel.head()`, so repeated presses walk word by word. With
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
    text: &BufferText,
    sels: SelectionSet,
    count: usize,
    around: bool,
    is_boundary: impl Fn(CharClass, CharClass) -> bool + Copy,
    chars: WordChars<'_>,
    motion: impl Fn(&BufferText, CharOffset) -> Option<InclusiveRange<CharOffset>>,
) -> SelectionSet {
    let result = sels.map(|sel| {
        let mut current = sel;
        for _ in 0..count {
            match motion(text, current.head()) {
                Some(target) => {
                    // `word_unit_at` returns `None` when the anchor sits on
                    // whitespace with no adjacent word (e.g. indentation at
                    // the very start of the buffer). Fall back to the bare
                    // whitespace position, same as `anchor_unit` yields there.
                    let unit = if around
                        && let Some(unit) = word_unit_at(
                            text,
                            current.anchor(),
                            is_boundary,
                            CharOffset::new(0),
                            chars,
                        ) {
                        unit
                    } else {
                        anchor_unit(text, current.anchor(), is_boundary, chars)
                    };
                    current = if target.start > unit.end {
                        Selection::new(unit.start, target.end) // target beyond anchor: grow forward
                    } else if target.end < unit.start {
                        let head = if around {
                            expand_word_unit(text, target.start, target.end, CharOffset::new(0))
                                .start
                        } else {
                            target.start
                        };
                        Selection::new(unit.end, head) // target behind anchor: grow backward
                    } else {
                        Selection::new(unit.start, unit.end) // target is the anchor's own unit
                    };
                }
                None => break,
            }
        }
        current
    });
    result.debug_assert_valid(text);
    result
}

type SelectWord =
    fn(&BufferText, CharOffset, IsBoundary, WordChars<'_>) -> Option<InclusiveRange<CharOffset>>;

/// Shared dispatch for the four word-select commands below: branches on
/// `ctx.mode` (fresh re-anchor for `Move`, grow/shrink for `Extend`, see
/// [`apply_word_select`]/[`apply_word_select_extend`]), parameterized by
/// direction (`select_word`: [`select_next_word`] or [`select_prev_word`])
/// and word class (`is_boundary`: [`is_word_boundary`] or
/// [`is_uppercase_word_boundary`]).
///
/// `backward` only affects the `Move` arm's search origin (see
/// [`apply_word_select`]'s doc); `Extend`'s chaining always uses `head()` and
/// has no analogous asymmetry. `ctx.around` affects both arms identically:
/// a plain field read deciding whether whitespace is included in the unit.
fn word_select_cmd(
    text: &BufferText,
    sels: SelectionSet,
    count: usize,
    ctx: WordCtx<'_>,
    backward: bool,
    is_boundary: IsBoundary,
    select_word: SelectWord,
) -> SelectionSet {
    match ctx.mode {
        MotionMode::Move => apply_word_select(text, sels, count, ctx.around, backward, |b, pos| {
            select_word(b, pos, is_boundary, ctx.chars)
        }),
        MotionMode::Extend => apply_word_select_extend(
            text,
            sels,
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
        pub fn $name(
            text: &BufferText,
            sels: SelectionSet,
            count: usize,
            ctx: WordCtx<'_>,
        ) -> SelectionSet {
            word_select_cmd(
                text,
                sels,
                count,
                ctx,
                $backward,
                $is_boundary,
                $select_word,
            )
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
