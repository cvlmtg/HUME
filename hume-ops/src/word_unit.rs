//! The word (or WORD) around a position: the scan primitives that every word
//! motion and word text object builds on. Position-only: nothing here knows
//! about selections or motion modes.

use hume_editing::grapheme::{clusters_before, graphemes_at, next_cluster, prev_cluster};
use hume_editing::text::BufferText;
use hume_editing::word::{CharClass, WordChars, blank_class};
use hume_rope::cluster::{ClusterBound, ClusterRange, ClusterStart};

pub(crate) type IsBoundary = fn(CharClass, CharClass) -> bool;

/// The char that classifies the cluster starting at `pos`.
fn first_char(text: &BufferText, pos: ClusterStart) -> char {
    text.char_at(pos.offset())
        .expect("a cluster start lies inside the text")
}

/// Class of the cluster starting at `pos`.
pub(crate) fn class_at(text: &BufferText, pos: ClusterStart, chars: WordChars<'_>) -> CharClass {
    chars.classify(first_char(text, pos))
}

/// Whether the cluster starting at `pos` is blank (space, tab, NBSP,
/// ideographic space, or newline).
pub(crate) fn is_blank_at(text: &BufferText, pos: ClusterStart) -> bool {
    blank_class(first_char(text, pos)).is_some()
}

/// The nearest non-blank cluster before `before` and not before `floor`, or
/// `None` if every cluster in between is blank.
pub(crate) fn prev_nonblank(
    text: &BufferText,
    before: ClusterBound,
    floor: ClusterBound,
) -> Option<ClusterStart> {
    clusters_before(text, before)
        .take_while(|c| c.end() > floor)
        .find(|c| blank_class(c.first()).is_none())
        .map(|c| c.start())
}

/// The first cluster of the word or punct group holding `pos`.
///
/// Mirror of [`find_word_end_from`]: steps back while `is_boundary` reports
/// no boundary between the previous and current class, stopping at the first
/// boundary or the text start. See [`find_word_end_from`]'s doc for why this
/// isn't always "same class".
pub(crate) fn find_word_start_from(
    text: &BufferText,
    pos: ClusterStart,
    is_boundary: impl Fn(CharClass, CharClass) -> bool,
    chars: WordChars<'_>,
) -> ClusterStart {
    let cat = class_at(text, pos, chars);
    clusters_before(text, pos.into())
        .take_while(|c| !is_boundary(chars.classify(c.first()), cat))
        .last()
        .map_or(pos, |c| c.start())
}

/// The last cluster of the word or punct group starting at `start`.
///
/// Advances while `is_boundary` reports no boundary between the current and
/// next class, and stops at the first boundary or the text end. Not always
/// "same class": under WORD semantics (`is_uppercase_word_boundary`) Word and
/// Punctuation are merged, so this can advance across a Word→Punct transition
/// without stopping.
pub(crate) fn find_word_end_from(
    text: &BufferText,
    start: ClusterStart,
    is_boundary: impl Fn(CharClass, CharClass) -> bool,
    chars: WordChars<'_>,
) -> ClusterStart {
    let cat = class_at(text, start, chars);
    graphemes_at(text, start.into())
        .skip(1)
        .take_while(|c| !is_boundary(cat, chars.classify(c.first())))
        .last()
        .map_or(start, |c| c.start())
}

/// The run of adjacent clusters around `pos` that share its class (no
/// boundary crossing), whatever that class is, including whitespace runs and
/// EOL. Always `Some`; the `Option` matches the other word finders.
pub fn inner_word_impl(
    text: &BufferText,
    pos: ClusterStart,
    is_boundary: impl Fn(CharClass, CharClass) -> bool,
    chars: WordChars<'_>,
) -> Option<ClusterRange> {
    ClusterRange::through(
        text.full_slice(),
        find_word_start_from(text, pos, &is_boundary, chars),
        find_word_end_from(text, pos, &is_boundary, chars),
    )
}

/// Grow a word/punct `range` to include an adjacent whitespace run: leading
/// preferred, trailing when the leading run is indentation or absent.
///
/// A leading run that reaches back to the start of its line (or the start of
/// the buffer) is indentation, not inter-word spacing, and must never be
/// absorbed: the first word of a line always takes its trailing whitespace
/// instead. This keeps `w`/`b`/`mm`/`maw` from ever eating indentation.
///
/// `min_start` is a hard lower bound on the leading scan, never crossed.
/// Buffer-line callers pass the text start (no floor beyond the buffer
/// itself). The wrap path passes the visual sub-line's start so a word
/// beginning a continuation display line never absorbs the inter-word space
/// that lives at the end of the previous display line.
///
/// Reaching `min_start` only counts as indentation (blocking absorption) when
/// `min_start` is itself a genuine line start: the buffer start, or right
/// after a real newline. A wrap sub-line boundary is neither: it falls
/// mid-line, so a leading run that reaches it is ordinary inter-word spacing
/// that happens to sit at the display-line split, not indentation, and stays
/// absorbable up to that floor.
pub fn expand_word_unit(
    text: &BufferText,
    range: ClusterRange,
    min_start: ClusterBound,
) -> ClusterRange {
    let min_start_is_bol = prev_cluster(text, min_start)
        .is_none_or(|prev| blank_class(first_char(text, prev)) == Some(CharClass::Eol));

    // Leading scan: walk back over Space clusters from the range's start.
    // Stopping on Eol means the run touches the start of the line, so it is
    // indentation.
    let mut run_start = range.start();
    let mut hit_eol = false;
    for c in clusters_before(text, range.start().into()).take_while(|c| c.end() > min_start) {
        match blank_class(c.first()) {
            Some(CharClass::Space) => run_start = c.start(),
            Some(CharClass::Eol) => {
                hit_eol = true;
                break;
            }
            _ => break,
        }
    }
    let at_bol = hit_eol || (ClusterBound::from(run_start) == min_start && min_start_is_bol);

    if run_start < range.start() && !at_bol {
        return range.hull(ClusterRange::of(text.full_slice(), run_start));
    }

    // Trailing fallback: first word of a line, punctuation immediately
    // before, or no adjacent whitespace at all.
    let trailing = graphemes_at(text, range.end())
        .take_while(|c| blank_class(c.first()) == Some(CharClass::Space))
        .last();
    match trailing {
        Some(last) => range.hull(last.range()),
        None => range,
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
    pos: ClusterStart,
    is_boundary: impl Fn(CharClass, CharClass) -> bool + Copy,
    min_start: ClusterBound,
    chars: WordChars<'_>,
) -> Option<ClusterRange> {
    let range = inner_word_impl(text, pos, is_boundary, chars)?;
    if !is_blank_at(text, range.start()) {
        return Some(expand_word_unit(text, range, min_start));
    }

    // On whitespace: `range` is the whitespace run. Find the word adjacent
    // to it (following preferred, preceding fallback) and expand that one by
    // the normal rule instead.
    let word_pos = match next_cluster(text, range.last()) {
        Some(next) if !is_blank_at(text, next) => next,
        _ => {
            let prev = prev_cluster(text, range.start().into())?;
            if is_blank_at(text, prev) {
                return None;
            }
            prev
        }
    };
    let range = inner_word_impl(text, word_pos, is_boundary, chars)?;
    Some(expand_word_unit(text, range, min_start))
}

/// The word (or WORD) holding `anchor`, or just `anchor`'s cluster when it
/// is whitespace or a newline.
///
/// This is the range that must never be split when an extend motion crosses
/// the anchor: re-deriving it fresh from the anchor's current position (never
/// carrying extra state) is what guarantees whole words stay selected even as
/// the selection direction flips back and forth.
pub(crate) fn anchor_unit(
    text: &BufferText,
    anchor: ClusterStart,
    is_boundary: impl Fn(CharClass, CharClass) -> bool,
    chars: WordChars<'_>,
) -> ClusterRange {
    if is_blank_at(text, anchor) {
        ClusterRange::of(text.full_slice(), anchor)
    } else {
        inner_word_impl(text, anchor, is_boundary, chars).expect("a cluster start")
    }
}
