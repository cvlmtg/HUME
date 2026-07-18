use super::MotionMode;
use crate::ops::text_object::{expand_word_unit, word_unit_at};
use hume_editing::grapheme::{next_grapheme_boundary, prev_grapheme_boundary};
use hume_editing::selection::{Selection, SelectionSet};
use hume_editing::text::Text;
use hume_editing::word::{CharClass, classify_char, is_uppercase_word_boundary, is_word_boundary};

// ── Word motions (inner) ──────────────────────────────────────────────────────

/// Move to the start of the next word.
///
/// Pair-scan forward: stop when the category changes AND the next char is
/// either Eol or not Space. This skips the current word/punct, skips spaces
/// (but not newlines), and lands on the next word/punct start or on a newline.
///
/// The `is_boundary` parameter is `is_word_boundary` for `w` and
/// `is_uppercase_word_boundary` for `W`.
pub(super) fn next_word_start(
    buf: &Text,
    head: usize,
    is_boundary: impl Fn(CharClass, CharClass) -> bool,
) -> usize {
    let len = buf.len_chars();
    if head >= len {
        return head;
    }

    let mut pos = head;
    let mut prev_class = classify_char(buf.char_at(pos).expect("pos < len"));
    // Advance by a full grapheme cluster so we never land mid-cluster.
    // This matters for combining sequences like e + U+0301 (combining acute):
    // stepping by 1 would land on the combining codepoint, which classify_char
    // sees as Punctuation — creating a false word boundary inside the grapheme.
    pos = next_grapheme_boundary(buf, pos);

    while pos < len {
        let cur_class = classify_char(buf.char_at(pos).expect("pos < len"));
        if is_boundary(prev_class, cur_class)
            && (cur_class == CharClass::Eol || cur_class != CharClass::Space)
        {
            return pos;
        }
        prev_class = cur_class;
        pos = next_grapheme_boundary(buf, pos);
    }
    // Clamp to last valid position (the trailing \n). len - 1 is safe because
    // the buffer always has at least one character.
    pos.min(len - 1)
}

/// Move to the start of the previous word.
///
/// Two-phase backward scan: skip Space/Eol backward, then skip backward while
/// in the same category, landing on the first char of that group.
pub(crate) fn prev_word_start(
    buf: &Text,
    head: usize,
    is_boundary: impl Fn(CharClass, CharClass) -> bool,
) -> usize {
    if head == 0 {
        return 0;
    }

    // Step back by a full grapheme cluster so we never start mid-cluster.
    // For a combining sequence like "café" (e + U+0301), stepping by 1 from
    // the position after the cluster would land on the combining codepoint —
    // which classify_char treats as Punctuation, creating a false boundary.
    let mut pos = prev_grapheme_boundary(buf, head);

    // Phase 1: skip Space and Eol backward.
    loop {
        let cat = classify_char(buf.char_at(pos).expect("pos < len"));
        if cat != CharClass::Space && cat != CharClass::Eol {
            break;
        }
        if pos == 0 {
            return 0; // nothing but whitespace before — land at buffer start
        }
        pos = prev_grapheme_boundary(buf, pos);
    }

    // Phase 2: skip backward while in the same category.
    let cat = classify_char(buf.char_at(pos).expect("pos < len"));
    while pos > 0 {
        // Use prev_grapheme_boundary rather than pos - 1 so we always examine
        // the first codepoint of each grapheme cluster (the base character),
        // not a combining codepoint that may report a different class.
        let prev_pos = prev_grapheme_boundary(buf, pos);
        let prev_cat = classify_char(buf.char_at(prev_pos).expect("prev_pos < len"));
        if is_boundary(prev_cat, cat) {
            break;
        }
        pos = prev_pos;
    }

    pos
}

// ── Word-select helpers ───────────────────────────────────────────────────────

/// Scan forward from the first char of a known word group, returning the
/// position of its last char.
///
/// Starts at `start` (which must be the first char of a word or punct group),
/// advances forward while `is_boundary` reports no boundary between the
/// current and next class, and stops at the first boundary or the buffer end.
/// Not always "same class": under WORD semantics (`is_uppercase_word_boundary`)
/// Word and Punctuation are merged, so this can advance across a Word→Punct
/// transition without stopping.
///
/// This is Phase 2 of `next_word_end` run from a known starting position,
/// without the initial skip-whitespace step.
pub(super) fn find_word_end_from(
    buf: &Text,
    start: usize,
    is_boundary: impl Fn(CharClass, CharClass) -> bool,
) -> usize {
    let len = buf.len_chars();
    if start >= len {
        return start.saturating_sub(1);
    }

    let cat = classify_char(buf.char_at(start).expect("start < len"));
    let mut pos = start;

    loop {
        let next_pos = next_grapheme_boundary(buf, pos);
        // `next_pos - 1` is the last codepoint of the grapheme cluster that
        // starts at `pos`. For a single-codepoint cluster (the common case)
        // this equals `pos`; for a multi-codepoint cluster such as "e\u{0301}"
        // (é = base letter + combining accent) it includes the trailing
        // combining marks that logically belong to the same grapheme.
        if next_pos >= len {
            return next_pos - 1; // grapheme-safe: next_pos is a grapheme boundary; -1 is the last codepoint of the current cluster
        }
        let next_cat = classify_char(buf.char_at(next_pos).expect("next_pos < len"));
        if is_boundary(cat, next_cat) {
            return next_pos - 1; // grapheme-safe: next_pos is a grapheme boundary; -1 is the last codepoint of the current cluster
        }
        pos = next_pos;
    }
}

/// Scan backward from a char known to be inside a word or punct group,
/// returning the position of its first char.
///
/// Mirror of [`find_word_end_from`]: steps backward by grapheme boundary
/// while `is_boundary` reports no boundary between the previous and current
/// class, stopping at the first boundary or buffer start. See
/// [`find_word_end_from`]'s doc for why this isn't always "same class".
pub(super) fn find_word_start_from(
    buf: &Text,
    pos: usize,
    is_boundary: impl Fn(CharClass, CharClass) -> bool,
) -> usize {
    let cat = classify_char(buf.char_at(pos).expect("pos < len"));
    let mut pos = pos;
    while pos > 0 {
        let prev_pos = prev_grapheme_boundary(buf, pos);
        let prev_cat = classify_char(buf.char_at(prev_pos).expect("prev_pos < len"));
        if is_boundary(prev_cat, cat) {
            break;
        }
        pos = prev_pos;
    }
    pos
}

/// The word (or WORD) containing `anchor`, or the single position `(anchor,
/// anchor)` if `anchor` sits on whitespace/newline.
///
/// This is the range that must never be split when an extend motion crosses
/// the anchor: re-deriving it fresh from the anchor's current position (never
/// carrying extra state) is what guarantees whole words stay selected even as
/// the selection direction flips back and forth.
pub(super) fn anchor_unit(
    buf: &Text,
    anchor: usize,
    is_boundary: impl Fn(CharClass, CharClass) -> bool + Copy,
) -> (usize, usize) {
    // `anchor` may be *any* valid selection endpoint, including the last
    // codepoint of a multi-codepoint grapheme cluster — that's exactly what
    // the backward-grow branch above leaves behind as the new anchor when the
    // anchor word's own last codepoint is part of one (e.g. "café" = c,a,f,e,
    // combining-acute). `classify_char` must see the cluster's leading
    // codepoint: reading an internal combining mark directly classifies it as
    // `Punctuation` (Rust's `is_alphanumeric` excludes combining marks),
    // which misreads the anchor's own class and truncates the word down to
    // just that trailing mark. Snap to the start of the cluster containing
    // `anchor` first — a no-op when `anchor` already is a cluster start.
    let anchor = prev_grapheme_boundary(buf, next_grapheme_boundary(buf, anchor));
    let cat = classify_char(buf.char_at(anchor).expect("anchor < len"));
    if cat == CharClass::Space || cat == CharClass::Eol {
        (anchor, anchor)
    } else {
        (
            find_word_start_from(buf, anchor, is_boundary),
            find_word_end_from(buf, anchor, is_boundary),
        )
    }
}

/// Find the next word (or WORD) from `pos` and return `(word_start, word_end)`.
///
/// Returns `None` when there is no next word — at the last word in the buffer
/// (no-op) or on an empty buffer.
///
/// Unlike `next_word_start`, this function crosses line boundaries: if the
/// scan lands on a newline between lines, it calls `next_word_start` a second
/// time from the newline to reach the first word on the next line.
pub(super) fn select_next_word(
    buf: &Text,
    pos: usize,
    is_boundary: impl Fn(CharClass, CharClass) -> bool + Copy,
) -> Option<(usize, usize)> {
    let len = buf.len_chars();

    // Find the start of the next word.
    let mut word_start = next_word_start(buf, pos, is_boundary);

    // If we landed on a newline that is NOT the trailing '\n', cross the line:
    // call next_word_start again from that newline to get to the next line's word.
    if word_start < len.saturating_sub(1) {
        let cat = classify_char(buf.char_at(word_start).expect("word_start < len"));
        if cat == CharClass::Eol {
            word_start = next_word_start(buf, word_start, is_boundary);
        }
    }

    // If we've hit the trailing '\n' (last char in the buffer), there is no
    // next word — treat this as a no-op.
    if word_start >= len.saturating_sub(1) {
        return None;
    }

    // Guard: if we somehow landed on whitespace, also a no-op.
    let cat = classify_char(buf.char_at(word_start).expect("word_start < len"));
    if cat == CharClass::Space || cat == CharClass::Eol {
        return None;
    }

    let word_end = find_word_end_from(buf, word_start, is_boundary);
    Some((word_start, word_end))
}

/// Find the previous word (or WORD) from `pos` and return `(word_start, word_end)`.
///
/// Returns `None` when there is no previous word — already at or before the
/// first word in the buffer (no-op).
///
/// If `pos` is inside a word, we jump to the word BEFORE the current one (not
/// the start of the current word). If `pos` is in whitespace or at the start
/// of a word, we jump to the preceding word.
pub(super) fn select_prev_word(
    buf: &Text,
    pos: usize,
    is_boundary: impl Fn(CharClass, CharClass) -> bool + Copy,
) -> Option<(usize, usize)> {
    if pos == 0 {
        return None;
    }

    // Find the start of the word `prev_word_start` would land on.
    let word_start = prev_word_start(buf, pos, is_boundary);

    // If that position is whitespace (e.g. buffer starts with spaces), there
    // is no actual word to jump to.
    let cat = classify_char(buf.char_at(word_start).expect("word_start < len"));
    if cat == CharClass::Space || cat == CharClass::Eol {
        return None;
    }

    let word_end = find_word_end_from(buf, word_start, is_boundary);

    // If pos is within [word_start, word_end], prev_word_start landed on the
    // CURRENT word, not the previous one. We need one more step backward.
    if pos >= word_start && pos <= word_end {
        if word_start == 0 {
            return None; // already at the first word — no-op
        }
        let prev_start = prev_word_start(buf, word_start, is_boundary);
        let prev_cat = classify_char(buf.char_at(prev_start).expect("prev_start < len"));
        if prev_cat == CharClass::Space || prev_cat == CharClass::Eol {
            return None; // no word before this one
        }
        let prev_end = find_word_end_from(buf, prev_start, is_boundary);
        return Some((prev_start, prev_end));
    }

    Some((word_start, word_end))
}

/// Apply a word-select motion to every selection in the set, repeated `count` times.
///
/// Unlike `apply_motion`, `motion` returns `(word_start, word_end)` — both
/// endpoints of the selected word — rather than a single new head position.
/// The result is always a fresh forward selection `[word_start, word_end]`
/// that replaces the old selection (no anchor accumulation).
///
/// If `motion` returns `None` (no next/previous word), the iteration stops
/// early for that selection and the last selection is kept unchanged.
///
/// When `around` is set, the final word span is grown to include its
/// whitespace bookend (leading, or trailing when the word is the first
/// on its line — see [`expand_word_unit`]) once the loop is done. A
/// selection the loop never actually moved (motion returned `None` on the
/// first iteration, e.g. `w` at EOF) is left untouched — there is no word to
/// grow around.
///
/// `backward` selects which edge of the current selection each hop searches
/// from. Forward motions (`w`/`W`) always search from `head()`: a leading
/// expansion only ever moves `start`, so `head()` always sits on the found
/// word's own last char (or, for a first-word-on-line landing, on its
/// trailing whitespace) — either way `next_word_start` searches correctly
/// from there. Backward motions (`b`/`B`) need `start()` instead:
/// `select_prev_word` detects "did I land back on the word I'm already
/// sitting in" by checking whether the search origin falls inside that
/// word's bounds, and after a first-word-on-line landing `head()` sits in
/// the word's *trailing* whitespace — just outside those bounds — which
/// defeats the check and re-returns the same word every subsequent press.
/// `start()` never drifts into trailing whitespace (a leading expansion only
/// pulls it further from the found word, which keeps the check working), so
/// it's the origin that stays correct across repeated backward presses on
/// both bare and around selections.
pub(super) fn apply_word_select(
    buf: &Text,
    sels: SelectionSet,
    count: usize,
    around: bool,
    backward: bool,
    motion: impl Fn(&Text, usize) -> Option<(usize, usize)>,
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
            match motion(buf, origin) {
                Some((anchor, head)) => {
                    current = Selection::new(anchor, head);
                    moved = true;
                }
                None => break, // no more words — stop early, keep last selection
            }
        }
        if around && moved {
            let (start, end) = expand_word_unit(buf, current.start(), current.end());
            current = Selection::new(start, end);
        }
        current
    });
    result.debug_assert_valid(buf);
    result
}

/// Apply a word-select motion in extend mode: grow toward the target word if
/// it lies beyond the anchor's unit, shrink toward it if the target has
/// crossed back onto or past the anchor's unit, replacing the old selection
/// rather than unioning with it.
///
/// The motion origin is `sel.head()` — each press searches from wherever the
/// last press left the cursor, so repeated presses walk word by word in
/// either direction.
///
/// When `around` is set, the anchor's unit is resolved via [`word_unit_at`]
/// (leading whitespace included, same rule as [`expand_word_unit`]) instead
/// of the bare [`anchor_unit`], and a backward-growing target's `head` is
/// expanded the same way — so a backward extend can end on the target
/// word's leading whitespace. Comparisons still use the target's *raw* word
/// bounds against the *expanded* anchor unit: adjacent units can overlap by
/// one space (e.g. "one two" → "one " and " two"), but since only the
/// anchor's own unit is ever expanded for the comparison, that overlap never
/// causes a position to be double-counted. A forward-growing target never
/// needs expanding — its `head` already lands on its own last char, which is
/// exactly why `around` on `Move` needed the reversion this replaces:
/// leading units end at the word, not in trailing whitespace.
///
/// Because a target unit can only lie entirely beyond, entirely behind, or
/// exactly on the anchor's unit (units never partially overlap once the
/// anchor's own unit is fixed), the anchor's unit is always kept whole:
/// crossing it flips the selection's direction but never truncates it.
///
/// If `motion` returns `None`, iteration stops early and the last selection is
/// kept.
pub(super) fn apply_word_select_extend(
    buf: &Text,
    sels: SelectionSet,
    count: usize,
    around: bool,
    is_boundary: impl Fn(CharClass, CharClass) -> bool + Copy,
    motion: impl Fn(&Text, usize) -> Option<(usize, usize)>,
) -> SelectionSet {
    let result = sels.map(|sel| {
        let mut current = sel;
        for _ in 0..count {
            match motion(buf, current.head()) {
                Some((word_start, word_end)) => {
                    // `word_unit_at` returns `None` when the anchor sits on
                    // whitespace with no adjacent word (e.g. indentation at
                    // the very start of the buffer) — fall back to the bare
                    // whitespace position, same as `anchor_unit` yields there.
                    let (unit_start, unit_end) = if around
                        && let Some(unit) = word_unit_at(buf, current.anchor(), is_boundary)
                    {
                        unit
                    } else {
                        anchor_unit(buf, current.anchor(), is_boundary)
                    };
                    current = if word_start > unit_end {
                        Selection::new(unit_start, word_end) // target beyond anchor — grow forward
                    } else if word_end < unit_start {
                        let head = if around {
                            expand_word_unit(buf, word_start, word_end).0
                        } else {
                            word_start
                        };
                        Selection::new(unit_end, head) // target behind anchor — grow backward
                    } else {
                        Selection::new(unit_start, unit_end) // target is the anchor's own unit
                    };
                }
                None => break,
            }
        }
        current
    });
    result.debug_assert_valid(buf);
    result
}

type IsBoundary = fn(CharClass, CharClass) -> bool;
type SelectWord = fn(&Text, usize, IsBoundary) -> Option<(usize, usize)>;

/// Shared dispatch for the eight word-select commands below: branches on
/// `mode` (fresh re-anchor for `Move`, grow/shrink for `Extend` — see
/// [`apply_word_select`]/[`apply_word_select_extend`]), parameterized by
/// direction (`select_word`: [`select_next_word`] or [`select_prev_word`])
/// and word class (`is_boundary`: [`is_word_boundary`] or
/// [`is_uppercase_word_boundary`]).
///
/// `backward` only affects the `Move` arm's search origin (see
/// [`apply_word_select`]'s doc); `Extend`'s chaining always uses `head()` and
/// has no analogous asymmetry. `around` affects both arms identically —
/// whether whitespace is included in the unit.
#[allow(clippy::too_many_arguments)]
fn word_select_cmd(
    buf: &Text,
    sels: SelectionSet,
    count: usize,
    mode: MotionMode,
    around: bool,
    backward: bool,
    is_boundary: IsBoundary,
    select_word: SelectWord,
) -> SelectionSet {
    match mode {
        MotionMode::Move => apply_word_select(buf, sels, count, around, backward, |b, pos| {
            select_word(b, pos, is_boundary)
        }),
        MotionMode::Extend => {
            apply_word_select_extend(buf, sels, count, around, is_boundary, |b, pos| {
                select_word(b, pos, is_boundary)
            })
        }
    }
}

/// Select or extend to the next word (`w`).
#[allow(non_snake_case)]
pub(crate) fn cmd_select_next_word(
    buf: &Text,
    sels: SelectionSet,
    count: usize,
    mode: MotionMode,
) -> SelectionSet {
    word_select_cmd(
        buf,
        sels,
        count,
        mode,
        false,
        false,
        is_word_boundary,
        select_next_word,
    )
}

/// Select or extend to the next word (`w`), covering its whitespace bookend
/// in both modes — used when `word-selects-whitespace` is on. See
/// [`cmd_select_next_word`].
#[allow(non_snake_case)]
pub(crate) fn cmd_select_next_word_around(
    buf: &Text,
    sels: SelectionSet,
    count: usize,
    mode: MotionMode,
) -> SelectionSet {
    word_select_cmd(
        buf,
        sels,
        count,
        mode,
        true,
        false,
        is_word_boundary,
        select_next_word,
    )
}

/// Select or extend to the next WORD (`W`): like `w` but treats word+punct as one class.
#[allow(non_snake_case)]
pub(crate) fn cmd_select_next_uppercase_word(
    buf: &Text,
    sels: SelectionSet,
    count: usize,
    mode: MotionMode,
) -> SelectionSet {
    word_select_cmd(
        buf,
        sels,
        count,
        mode,
        false,
        false,
        is_uppercase_word_boundary,
        select_next_word,
    )
}

/// Select or extend to the next WORD (`W`), covering its whitespace bookend
/// in both modes. See [`cmd_select_next_word_around`].
#[allow(non_snake_case)]
pub(crate) fn cmd_select_next_uppercase_word_around(
    buf: &Text,
    sels: SelectionSet,
    count: usize,
    mode: MotionMode,
) -> SelectionSet {
    word_select_cmd(
        buf,
        sels,
        count,
        mode,
        true,
        false,
        is_uppercase_word_boundary,
        select_next_word,
    )
}

/// Select or extend to the previous word (`b`).
#[allow(non_snake_case)]
pub(crate) fn cmd_select_prev_word(
    buf: &Text,
    sels: SelectionSet,
    count: usize,
    mode: MotionMode,
) -> SelectionSet {
    word_select_cmd(
        buf,
        sels,
        count,
        mode,
        false,
        true,
        is_word_boundary,
        select_prev_word,
    )
}

/// Select or extend to the previous word (`b`), covering its whitespace
/// bookend in both modes. See [`cmd_select_next_word_around`].
#[allow(non_snake_case)]
pub(crate) fn cmd_select_prev_word_around(
    buf: &Text,
    sels: SelectionSet,
    count: usize,
    mode: MotionMode,
) -> SelectionSet {
    word_select_cmd(
        buf,
        sels,
        count,
        mode,
        true,
        true,
        is_word_boundary,
        select_prev_word,
    )
}

/// Select or extend to the previous WORD (`B`): like `b` but treats word+punct as one class.
#[allow(non_snake_case)]
pub(crate) fn cmd_select_prev_uppercase_word(
    buf: &Text,
    sels: SelectionSet,
    count: usize,
    mode: MotionMode,
) -> SelectionSet {
    word_select_cmd(
        buf,
        sels,
        count,
        mode,
        false,
        true,
        is_uppercase_word_boundary,
        select_prev_word,
    )
}

/// Select or extend to the previous WORD (`B`), covering its whitespace
/// bookend in both modes. See [`cmd_select_next_word_around`].
#[allow(non_snake_case)]
pub(crate) fn cmd_select_prev_uppercase_word_around(
    buf: &Text,
    sels: SelectionSet,
    count: usize,
    mode: MotionMode,
) -> SelectionSet {
    word_select_cmd(
        buf,
        sels,
        count,
        mode,
        true,
        true,
        is_uppercase_word_boundary,
        select_prev_word,
    )
}
