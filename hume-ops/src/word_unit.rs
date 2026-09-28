//! The word (or WORD) around a position: the scan primitives that every word
//! motion and word text object builds on. Position-only: nothing here knows
//! about selections or motion modes.

use hume_editing::grapheme::{
    graphemes_at, next_grapheme_boundary, prev_grapheme_boundary, snap_to_cluster_start,
};
use hume_editing::text::BufferText;
use hume_editing::word::{CharClass, WordChars, blank_class};
use hume_rope::offset::{CharOffset, InclusiveRange};

pub(crate) type IsBoundary = fn(CharClass, CharClass) -> bool;

/// Class of the char at `pos`, which must lie inside the buffer.
pub(crate) fn class_at(text: &BufferText, pos: CharOffset, chars: WordChars<'_>) -> CharClass {
    chars.classify(text.char_at(pos).expect("pos < len"))
}

/// Whether the char at `pos`, which must lie inside the buffer, is blank
/// (space, tab, NBSP, ideographic space, or newline).
pub(crate) fn is_blank_at(text: &BufferText, pos: CharOffset) -> bool {
    blank_class(text.char_at(pos).expect("pos < len")).is_some()
}

/// Start of the nearest non-blank cluster strictly before `before` and not
/// below `floor`, or `None` if every cluster in `[floor, before)` is blank.
pub(crate) fn prev_nonblank(
    text: &BufferText,
    before: CharOffset,
    floor: CharOffset,
) -> Option<CharOffset> {
    let mut pos = before;
    while pos > floor {
        pos = prev_grapheme_boundary(text, pos);
        if !is_blank_at(text, pos) {
            return Some(pos);
        }
    }
    None
}

/// Scan backward from a char known to be inside a word or punct group,
/// returning the position of its first char.
///
/// Mirror of [`find_word_end_from`]: steps backward by grapheme boundary
/// while `is_boundary` reports no boundary between the previous and current
/// class, stopping at the first boundary or buffer start. See
/// [`find_word_end_from`]'s doc for why this isn't always "same class".
pub(crate) fn find_word_start_from(
    text: &BufferText,
    pos: CharOffset,
    is_boundary: impl Fn(CharClass, CharClass) -> bool,
    chars: WordChars<'_>,
) -> CharOffset {
    let cat = class_at(text, pos, chars);
    let mut pos = pos;
    while pos > CharOffset::new(0) {
        let prev_pos = prev_grapheme_boundary(text, pos);
        if is_boundary(class_at(text, prev_pos, chars), cat) {
            break;
        }
        pos = prev_pos;
    }
    pos
}

/// Scan forward from the first char of a known word group, returning the
/// position of its last char.
///
/// Starts at `start` (which must be the first char of a word or punct group),
/// advances forward while `is_boundary` reports no boundary between the
/// current and next class, and stops at the first boundary or the buffer end.
/// Not always "same class": under WORD semantics (`is_uppercase_word_boundary`)
/// Word and Punctuation are merged, so this can advance across a Word→Punct
/// transition without stopping.
pub(crate) fn find_word_end_from(
    text: &BufferText,
    start: CharOffset,
    is_boundary: impl Fn(CharClass, CharClass) -> bool,
    chars: WordChars<'_>,
) -> CharOffset {
    let end = text.end();
    if start >= end {
        // One back from a position at or past the buffer end (`end` is at
        // least 1: every buffer holds the structural `\n`), so `retreat`
        // can't underflow here.
        return start.retreat(1);
    }

    let mut clusters = graphemes_at(text, start);
    let mut current = clusters.next().expect("start < end");
    let cat = chars.classify(current.first);
    for next in clusters {
        if is_boundary(cat, chars.classify(next.first)) {
            break;
        }
        current = next;
    }
    current.last_char()
}

/// Inner word parameterised by boundary predicate.
///
/// The run of adjacent clusters around `pos` that share its class (no
/// boundary crossing), whatever that class is, including whitespace runs and
/// EOL. `pos` may be any valid selection endpoint, including the last
/// codepoint of a multi-codepoint cluster: it is snapped to its cluster's
/// start before classifying, since a trailing combining mark alone classifies
/// as `Punctuation`. The range ends on the final cluster's last codepoint, so
/// a trailing combining mark is included.
pub fn inner_word_impl(
    text: &BufferText,
    pos: CharOffset,
    is_boundary: impl Fn(CharClass, CharClass) -> bool,
    chars: WordChars<'_>,
) -> Option<InclusiveRange<CharOffset>> {
    text.char_at(pos)?;
    let pos = snap_to_cluster_start(text, pos);
    Some(InclusiveRange::new(
        find_word_start_from(text, pos, &is_boundary, chars),
        find_word_end_from(text, pos, &is_boundary, chars),
    ))
}

/// Grow a word/punct span `(start, end)` to include an adjacent whitespace
/// run: leading preferred, trailing when the leading run is indentation or
/// absent.
///
/// A leading run that reaches back to the start of its line (or the start of
/// the buffer) is indentation, not inter-word spacing, and must never be
/// absorbed: the first word of a line always takes its trailing whitespace
/// instead. This keeps `w`/`b`/`mm`/`maw` from ever eating indentation.
///
/// `min_start` is a hard lower bound on the leading scan, never crossed.
/// Buffer-line callers pass `0` (no floor beyond the buffer itself). The wrap
/// path passes the visual sub-line's start so a word beginning a continuation
/// display line never absorbs the inter-word space that lives at the end of
/// the previous display line.
///
/// Reaching `min_start` only counts as indentation (blocking absorption) when
/// `min_start` is itself a genuine line start: the buffer start, or right
/// after a real newline. A wrap sub-line boundary is neither: it falls
/// mid-line, so a leading run that reaches it is ordinary inter-word spacing
/// that happens to sit at the display-line split, not indentation, and stays
/// absorbable up to that floor.
pub fn expand_word_unit(
    text: &BufferText,
    start: CharOffset,
    end: CharOffset,
    min_start: CharOffset,
) -> InclusiveRange<CharOffset> {
    let min_start_is_bol = min_start == CharOffset::new(0)
        || blank_class(
            text.char_at(prev_grapheme_boundary(text, min_start))
                .expect("min_start > 0 implies a preceding char"),
        ) == Some(CharClass::Eol);

    // Leading scan: walk back over Space graphemes from `start`. Stopping on
    // Eol means the run touches the start of the line, so it is indentation.
    let mut run_start = start;
    let mut hit_eol = false;
    while run_start > min_start {
        let prev_pos = prev_grapheme_boundary(text, run_start);
        match blank_class(text.char_at(prev_pos).expect("prev_pos < len")) {
            Some(CharClass::Space) => run_start = prev_pos,
            Some(CharClass::Eol) => {
                hit_eol = true;
                break;
            }
            _ => break,
        }
    }
    let at_bol = hit_eol || (run_start == min_start && min_start_is_bol);

    if run_start < start && !at_bol {
        return InclusiveRange::new(run_start, end);
    }

    // Trailing fallback: first word of a line, punctuation immediately
    // before, or no adjacent whitespace at all.
    let mut clusters = graphemes_at(text, end);
    let mut last = clusters.next().expect("end < len");
    for next in clusters {
        if blank_class(next.first) != Some(CharClass::Space) {
            break;
        }
        last = next;
    }
    if last.start == end {
        InclusiveRange::new(start, end)
    } else {
        InclusiveRange::new(start, last.last_char())
    }
}

/// The word (or WORD) unit at `pos`: the inner word plus its whitespace
/// bookend per [`expand_word_unit`].
///
/// When `pos` sits on whitespace there is no word under the cursor: snap to
/// the adjacent word (the one right after the run if any, else the one right
/// before it) and expand that instead. The whitespace under the cursor is
/// never selected for its own sake; it only appears in the span when the
/// expansion re-absorbs it (an inter-word space run is the following word's
/// leading run), so newlines and indentation never leak into the selection.
/// Returns `None` when no word is adjacent to the run (e.g. a
/// whitespace-only buffer, or indentation at the start of the buffer), and the
/// callers treat that as a no-op.
///
/// This is the shared body of `mm`/`MM` and `maw`/`maW` (position-based,
/// unlike the motion-based `w`/`b`): all four names select the same span.
/// Also used to resolve an extend selection's anchor unit when
/// `word-selects-whitespace` is on.
pub fn word_unit_at(
    text: &BufferText,
    pos: CharOffset,
    is_boundary: impl Fn(CharClass, CharClass) -> bool + Copy,
    min_start: CharOffset,
    chars: WordChars<'_>,
) -> Option<InclusiveRange<CharOffset>> {
    let range = inner_word_impl(text, pos, is_boundary, chars)?;
    if !is_blank_at(text, range.start) {
        return Some(expand_word_unit(text, range.start, range.end, min_start));
    }

    // On whitespace: `range` is the whitespace run. Find the word adjacent
    // to it (following preferred, preceding fallback) and expand that one by
    // the normal rule instead.
    let next_pos = next_grapheme_boundary(text, range.end);
    let word_pos = if next_pos < text.end() && !is_blank_at(text, next_pos) {
        next_pos
    } else if range.start > CharOffset::new(0) {
        let prev_pos = prev_grapheme_boundary(text, range.start);
        if is_blank_at(text, prev_pos) {
            return None;
        }
        prev_pos
    } else {
        return None;
    };
    let range = inner_word_impl(text, word_pos, is_boundary, chars)?;
    Some(expand_word_unit(text, range.start, range.end, min_start))
}

/// The word (or WORD) containing `anchor`, or the single position `(anchor,
/// anchor)` if `anchor` sits on whitespace/newline.
///
/// This is the range that must never be split when an extend motion crosses
/// the anchor: re-deriving it fresh from the anchor's current position (never
/// carrying extra state) is what guarantees whole words stay selected even as
/// the selection direction flips back and forth.
pub(crate) fn anchor_unit(
    text: &BufferText,
    anchor: CharOffset,
    is_boundary: impl Fn(CharClass, CharClass) -> bool,
    chars: WordChars<'_>,
) -> InclusiveRange<CharOffset> {
    // `anchor` may be a cluster's trailing codepoint; blank-ness is its
    // cluster's, read at the cluster start.
    let anchor = snap_to_cluster_start(text, anchor);
    if is_blank_at(text, anchor) {
        InclusiveRange::new(anchor, anchor)
    } else {
        inner_word_impl(text, anchor, is_boundary, chars).expect("anchor < len")
    }
}
