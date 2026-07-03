use super::MotionMode;
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
/// advances forward while `classify_char` stays in the same class, and stops
/// when the class changes or the buffer ends.
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
/// while `classify_char` stays in the same class, stopping at the class
/// change or buffer start.
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
pub(super) fn apply_word_select(
    buf: &Text,
    sels: SelectionSet,
    count: usize,
    motion: impl Fn(&Text, usize) -> Option<(usize, usize)>,
) -> SelectionSet {
    let result = sels.map(|sel| {
        let mut current = sel;
        for _ in 0..count {
            match motion(buf, current.head()) {
                Some((anchor, head)) => current = Selection::new(anchor, head),
                None => break, // no more words — stop early, keep last selection
            }
        }
        current
    });
    result.debug_assert_valid(buf);
    result
}

/// Apply a word-select motion in extend mode: grow toward the target word if
/// it lies beyond the anchor's word, shrink toward it if the target has
/// crossed back onto or past the anchor's word, replacing the old selection
/// rather than unioning with it.
///
/// The motion origin is `sel.head()` — each press searches from wherever the
/// last press left the cursor, so repeated presses walk word by word in
/// either direction. Because a target word can only lie entirely beyond,
/// entirely behind, or exactly on the anchor's word (`anchor_unit`; words
/// never partially overlap), the anchor's word is always kept whole: crossing
/// it flips the selection's direction but never truncates it.
///
/// If `motion` returns `None`, iteration stops early and the last selection is
/// kept.
pub(super) fn apply_word_select_extend(
    buf: &Text,
    sels: SelectionSet,
    count: usize,
    is_boundary: impl Fn(CharClass, CharClass) -> bool + Copy,
    motion: impl Fn(&Text, usize) -> Option<(usize, usize)>,
) -> SelectionSet {
    let result = sels.map(|sel| {
        let mut current = sel;
        for _ in 0..count {
            match motion(buf, current.head()) {
                Some((word_start, word_end)) => {
                    let (unit_start, unit_end) = anchor_unit(buf, current.anchor(), is_boundary);
                    current = if word_start > unit_end {
                        Selection::new(unit_start, word_end) // target beyond anchor — grow forward
                    } else if word_end < unit_start {
                        Selection::new(unit_end, word_start) // target behind anchor — grow backward
                    } else {
                        Selection::new(unit_start, unit_end) // target is the anchor's own word
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

/// Select or extend to the next word (`w`): branches on `mode`.
///
/// `Move` — re-anchors on each press (fresh forward selection spanning the word).
/// `Extend` — grows toward the next word, or shrinks back toward it if a
/// prior press already extended past it (see [`apply_word_select_extend`]).
#[allow(non_snake_case)]
pub(crate) fn cmd_select_next_word(
    buf: &Text,
    sels: SelectionSet,
    count: usize,
    mode: MotionMode,
) -> SelectionSet {
    match mode {
        MotionMode::Move => apply_word_select(buf, sels, count, |b, pos| {
            select_next_word(b, pos, is_word_boundary)
        }),
        MotionMode::Extend => {
            apply_word_select_extend(buf, sels, count, is_word_boundary, |b, pos| {
                select_next_word(b, pos, is_word_boundary)
            })
        }
    }
}

/// Select or extend to the next WORD (`W`): like `w` but treats word+punct as one class.
#[allow(non_snake_case)]
pub(crate) fn cmd_select_next_uppercase_word(
    buf: &Text,
    sels: SelectionSet,
    count: usize,
    mode: MotionMode,
) -> SelectionSet {
    match mode {
        MotionMode::Move => apply_word_select(buf, sels, count, |b, pos| {
            select_next_word(b, pos, is_uppercase_word_boundary)
        }),
        MotionMode::Extend => {
            apply_word_select_extend(buf, sels, count, is_uppercase_word_boundary, |b, pos| {
                select_next_word(b, pos, is_uppercase_word_boundary)
            })
        }
    }
}

/// Select or extend to the previous word (`b`): branches on `mode`.
#[allow(non_snake_case)]
pub(crate) fn cmd_select_prev_word(
    buf: &Text,
    sels: SelectionSet,
    count: usize,
    mode: MotionMode,
) -> SelectionSet {
    match mode {
        MotionMode::Move => apply_word_select(buf, sels, count, |b, pos| {
            select_prev_word(b, pos, is_word_boundary)
        }),
        MotionMode::Extend => {
            apply_word_select_extend(buf, sels, count, is_word_boundary, |b, pos| {
                select_prev_word(b, pos, is_word_boundary)
            })
        }
    }
}

/// Select or extend to the previous WORD (`B`): like `b` but treats word+punct as one class.
#[allow(non_snake_case)]
pub(crate) fn cmd_select_prev_uppercase_word(
    buf: &Text,
    sels: SelectionSet,
    count: usize,
    mode: MotionMode,
) -> SelectionSet {
    match mode {
        MotionMode::Move => apply_word_select(buf, sels, count, |b, pos| {
            select_prev_word(b, pos, is_uppercase_word_boundary)
        }),
        MotionMode::Extend => {
            apply_word_select_extend(buf, sels, count, is_uppercase_word_boundary, |b, pos| {
                select_prev_word(b, pos, is_uppercase_word_boundary)
            })
        }
    }
}
