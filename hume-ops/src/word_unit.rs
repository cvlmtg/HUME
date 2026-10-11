//! The word (or WORD) around a position: the scan primitives that every word
//! motion and word text object builds on. Position-only: nothing here knows
//! about selections or motion modes.

use hume_editing::text::BufferText;
use hume_editing::word::{CharClass, WordChars, blank_class};
use hume_rope::cluster::{ClusterBound, ClusterRange, ClusterStart};

use crate::WhitespaceSide;

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
    text.clusters()
        .before(before)
        .take_while(|c| c.end() > floor)
        .find(|c| blank_class(c.first()).is_none())
        .map(|c| c.start())
}

/// The first non-blank cluster from selection end `end` toward its other end
/// `other`, both included, or `None` when every cluster between them is
/// blank. A whitespace bookend lies on the outer side of its word, so this is
/// the word the bookend belongs to.
pub(crate) fn owning_word_cluster(
    text: &BufferText,
    end: ClusterStart,
    other: ClusterStart,
) -> Option<ClusterStart> {
    if end <= other {
        text.clusters()
            .graphemes_at(end.into())
            .take_while(|c| c.start() <= other)
            .find(|c| blank_class(c.first()).is_none())
            .map(|c| c.start())
    } else {
        prev_nonblank(text, text.clusters().end_of(end), other.into())
    }
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
    text.clusters()
        .before(pos.into())
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
    text.clusters()
        .graphemes_at(start.into())
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

/// Grow a word/punct `range` to include an adjacent whitespace run: the run
/// on `side` preferred, the other side's run when that one is indentation or
/// absent.
///
/// A leading run that reaches back to the start of its line (or the start of
/// the buffer) is indentation, not inter-word spacing, and must never be
/// absorbed from either side: the first word of a line takes its trailing
/// whitespace or nothing. This keeps `w`/`b`/`mm`/`maw` from ever eating
/// indentation. A trailing run stops at the line break, so it may end the
/// line.
///
/// `min_start` is a hard lower bound on the leading scan and `max_end` a
/// hard upper bound on the trailing scan, never crossed. Buffer-line callers
/// pass the text start and end (no bound beyond the buffer itself). The wrap
/// path passes the visual sub-line's bounds so a word beginning a
/// continuation display line never absorbs the inter-word space that lives
/// at the end of the previous display line, and a word filling its display
/// line never absorbs the space that starts the next one.
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
    max_end: ClusterBound,
    side: WhitespaceSide,
) -> ClusterRange {
    let run = match side {
        WhitespaceSide::Before => {
            leading_run(text, range, min_start).or_else(|| trailing_run(text, range, max_end))
        }
        WhitespaceSide::After => {
            trailing_run(text, range, max_end).or_else(|| leading_run(text, range, min_start))
        }
    };
    run.map_or(range, |run| range.hull(run))
}

/// The first cluster of the inter-word whitespace run right before `range`,
/// or `None` when there is none or it is indentation (see
/// [`expand_word_unit`]).
fn leading_run(
    text: &BufferText,
    range: ClusterRange,
    min_start: ClusterBound,
) -> Option<ClusterRange> {
    let min_start_is_bol = text
        .clusters()
        .prev(min_start)
        .is_none_or(|prev| blank_class(first_char(text, prev)) == Some(CharClass::Eol));

    // Walk back over Space clusters from the range's start. Stopping on Eol
    // means the run touches the start of the line, so it is indentation.
    let mut run_start = range.start();
    let mut hit_eol = false;
    for c in text
        .clusters()
        .before(range.start().into())
        .take_while(|c| c.end() > min_start)
    {
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

    (run_start < range.start() && !at_bol).then(|| ClusterRange::of(text.full_slice(), run_start))
}

/// The last cluster of the whitespace run right after `range`, up to the
/// line break and not past `max_end`, or `None` when the next cluster is not
/// a space.
fn trailing_run(
    text: &BufferText,
    range: ClusterRange,
    max_end: ClusterBound,
) -> Option<ClusterRange> {
    text.clusters()
        .graphemes_at(range.end())
        .take_while(|c| c.end() <= max_end && blank_class(c.first()) == Some(CharClass::Space))
        .last()
        .map(|last| last.range())
}

/// The word (or WORD) unit at `pos`: the inner word plus its whitespace
/// bookend per [`expand_word_unit`].
///
/// When `pos` sits on whitespace there is no word under the cursor: snap to
/// the word whose bookend on `side` is that run (the one right after it for
/// `Before`, right before it for `After`), else the word on the other side,
/// and expand that instead. The whitespace under the cursor is never
/// selected for its own sake; it only appears in the span when the expansion
/// re-absorbs it, so newlines and indentation never leak into the selection.
/// Returns `None` when no word is adjacent to the run (e.g. a
/// whitespace-only buffer, or indentation at the start of the buffer), and the
/// callers treat that as a no-op.
///
/// This is the shared body of `mm`/`MM` and `maw`/`maW` (position-based,
/// unlike the motion-based `w`/`b`): all four names select the same span.
/// Also used to resolve an extend selection's anchor unit when
/// `word-selects-whitespace` is not `off`.
pub fn word_unit_at(
    text: &BufferText,
    pos: ClusterStart,
    is_boundary: impl Fn(CharClass, CharClass) -> bool + Copy,
    min_start: ClusterBound,
    max_end: ClusterBound,
    side: WhitespaceSide,
    chars: WordChars<'_>,
) -> Option<ClusterRange> {
    let range = inner_word_impl(text, pos, is_boundary, chars)?;
    if !is_blank_at(text, range.start()) {
        return Some(expand_word_unit(text, range, min_start, max_end, side));
    }

    let following = text.clusters().next(range.last());
    let preceding = text.clusters().prev(range.start().into());
    let (first, second) = match side {
        WhitespaceSide::Before => (following, preceding),
        WhitespaceSide::After => (preceding, following),
    };
    let word_pos = [first, second]
        .into_iter()
        .flatten()
        .find(|&c| !is_blank_at(text, c))?;
    let range = inner_word_impl(text, word_pos, is_boundary, chars)?;
    Some(expand_word_unit(text, range, min_start, max_end, side))
}

/// The span `mm`/`maw` select at `pos`: [`word_unit_at`] with its bookend
/// on `whitespace`'s side, or [`inner_word_impl`] when `whitespace` is
/// `None`.
pub fn word_span_at(
    text: &BufferText,
    pos: ClusterStart,
    is_boundary: impl Fn(CharClass, CharClass) -> bool + Copy,
    min_start: ClusterBound,
    max_end: ClusterBound,
    whitespace: Option<WhitespaceSide>,
    chars: WordChars<'_>,
) -> Option<ClusterRange> {
    match whitespace {
        Some(side) => word_unit_at(text, pos, is_boundary, min_start, max_end, side, chars),
        None => inner_word_impl(text, pos, is_boundary, chars),
    }
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
