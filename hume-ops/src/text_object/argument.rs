//! Inner/around argument (comma-separated item) text objects: function
//! arguments, array items, object fields, or any comma list inside brackets.

use hume_editing::text::BufferText;
use hume_editing::word::{CharClass, blank_class};
use hume_rope::cluster::{ClusterRange, ClusterStart};

use super::{inner_of_pair, trim_blank};
use crate::pair::{bracket_role, find_tightest_bracket_pair};

/// One comma segment, leading and trailing whitespace included: the clusters
/// from `start` up to the delimiter cluster (comma or closing bracket) at
/// `stop`. Empty when `start == stop`, as between the commas of `(a,,b)`.
#[derive(Clone, Copy)]
struct Segment {
    start: ClusterStart,
    stop: ClusterStart,
}

impl Segment {
    fn contains(self, pos: ClusterStart) -> bool {
        self.start <= pos && pos < self.stop
    }

    /// The segment's clusters, or `None` when it is empty.
    fn range(self, text: &BufferText) -> Option<ClusterRange> {
        ClusterRange::between(text.full_slice(), self.start, self.stop.into())
    }
}

/// Collect all comma-separated segments at depth 0 inside the bracket pair
/// `pair`.
///
/// Commas inside a nested bracket pair (any `BRACKET_PAIRS` type, via
/// [`bracket_role`], the same table [`find_tightest_bracket_pair`] resolves
/// `pair` against) are skipped. Returns an empty vec for adjacent brackets
/// (`()`).
fn find_comma_segments(text: &BufferText, pair: ClusterRange) -> Vec<Segment> {
    let close = pair.last();
    let Some(content_start) = inner_of_pair(text, pair).map(|inner| inner.start()) else {
        return Vec::new();
    };

    let mut segments = Vec::new();
    let mut seg_start = content_start;
    let mut depth = 0usize;

    for (i, ch) in text
        .chars_at(content_start.offset())
        .take(close.offset().chars_since(content_start.offset()))
    {
        match bracket_role(ch) {
            Some((_, true)) => depth += 1,
            Some((_, false)) => depth = depth.saturating_sub(1),
            None if ch == ',' && depth == 0 => {
                let comma = text.snap(i);
                segments.push(Segment {
                    start: seg_start,
                    stop: comma,
                });
                seg_start = text
                    .clusters()
                    .next(comma)
                    .expect("the closing bracket follows");
            }
            None => {}
        }
    }

    // Final segment: everything after the last comma, or the whole content if no commas.
    segments.push(Segment {
        start: seg_start,
        stop: close,
    });
    segments
}

/// Find which segment in `segments` contains `pos`.
///
/// If `pos` falls in a gap (e.g., on a comma between two segments), associate
/// it with the following segment, matching Helix/Kakoune behaviour.
fn which_segment(segments: &[Segment], pos: ClusterStart) -> Option<usize> {
    segments
        .iter()
        .position(|seg| seg.contains(pos))
        .or_else(|| {
            segments
                .windows(2)
                .position(|w| pos >= w[0].stop && pos < w[1].start)
                .map(|idx| idx + 1)
        })
}

/// Resolve `pos` to its enclosing bracket pair's comma segments and the
/// index of the segment containing (or, on a comma gap, following) it.
///
/// Shared prelude for [`inner_argument`] and [`around_argument`]: locate the
/// tightest bracket pair, nudge `pos` off the bracket itself when it sits on
/// one, split the content into comma segments, and resolve which segment
/// `pos` falls in. Returns the nudged `pos` too: `around_argument`'s
/// only-argument case re-enters [`inner_argument`] with it, which lets that
/// case descend into a nested bracket pair instead of trimming the segment
/// already resolved against the outer one.
fn locate_argument(
    text: &BufferText,
    pos: ClusterStart,
) -> Option<(Vec<Segment>, usize, ClusterStart)> {
    let pair = find_tightest_bracket_pair(text, pos)?;

    // Nudge: if the cursor is on a bracket itself, step into the content zone.
    let pos = if pos == pair.start() {
        text.clusters().next(pos)?
    } else if pos == pair.last() {
        text.clusters().prev(pos.into())?
    } else {
        pos
    };

    let segments = find_comma_segments(text, pair);
    if segments.is_empty() {
        return None;
    }

    let idx = which_segment(&segments, pos)?;
    Some((segments, idx, pos))
}

/// Whitespace HUME's argument separator rule treats as blank: anything
/// [`blank_class`] classifies as `Space` or `Eol`: space, tab, NBSP,
/// ideographic space, or newline. Routed through `blank_class`
/// rather than a hand-rolled char match so `m a a` agrees with `m a w` on
/// which characters count as blank.
fn is_blank(ch: char) -> bool {
    blank_class(ch).is_some()
}

/// Narrower than [`is_blank`]: `Space`-classified only, no `Eol`. Used only
/// for the run trailing a separator comma in [`around_from_inner`]. A line
/// break there belongs to the *next* argument's indentation, not to this
/// one's trailing whitespace, so `foo(\n    a,\n    b\n)` around `a` eats
/// `a,` and leaves the newline.
fn is_inline_blank(ch: char) -> bool {
    blank_class(ch) == Some(CharClass::Space)
}

/// Extends `pos` forward while the cluster after it is blank, returning the
/// last cluster still covered by the run (`pos` itself if the very next
/// cluster isn't blank).
fn extend_forward_while(
    text: &BufferText,
    pos: ClusterStart,
    blank: impl Fn(char) -> bool,
) -> ClusterStart {
    text.clusters()
        .graphemes_at(pos.into())
        .skip(1)
        .take_while(|c| blank(c.first()))
        .last()
        .map_or(pos, |c| c.start())
}

/// Extends `pos` backward while the cluster before it is blank, returning
/// the first cluster of the run (`pos` itself if the preceding cluster isn't
/// blank).
fn extend_backward_while(
    text: &BufferText,
    pos: ClusterStart,
    blank: impl Fn(char) -> bool,
) -> ClusterStart {
    text.clusters()
        .before(pos.into())
        .take_while(|c| blank(c.first()))
        .last()
        .map_or(pos, |c| c.start())
}

/// Whether the cluster at `pos` is a comma.
fn is_comma(text: &BufferText, pos: ClusterStart) -> bool {
    text.char_at(pos.offset()) == Some(',')
}

/// Inner argument: the text of the comma-separated item at `pos`, with leading
/// and trailing whitespace trimmed.
///
/// Works for function arguments `foo(a, b)`, array items `[1, 2]`, object
/// fields `{x: 1, y: 2}`, and any comma-separated list inside brackets.
pub fn inner_argument(text: &BufferText, pos: ClusterStart) -> Option<ClusterRange> {
    let (segments, idx, _) = locate_argument(text, pos)?;
    trim_blank(text, segments[idx].range(text)?)
}
/// Derives an argument's "around" span from its "inner" span by locating its
/// separator comma: HUME's own rule, independent of how the inner span was
/// found (the lexical scan below, or a tree-sitter `parameter.inside`
/// capture), so `m i a`/`m a a` stay one structure-aware family rather than
/// two separate objects.
///
/// **Preceding separator first**: if the blank run immediately before
/// `start` is bounded by a comma, this argument is not first: the comma
/// and everything back to it becomes the new start, and `end` extends
/// forward over its own trailing blank run (newline-inclusive, `is_blank`,
/// a no-op for every argument but the last, which has none to eat *except*
/// the newline before a multi-line list's closing delimiter, which this
/// branch does consume). Otherwise, if the blank run immediately after `end`
/// is bounded by a comma, this argument is first: `start` extends backward
/// over blanks (reaching the opening delimiter, never a comma, since the
/// first rule would have fired otherwise) and `end` extends through the
/// comma plus its inline blank run only (space/tab, no newline, see
/// `is_inline_blank`). A line break there belongs to the *next*
/// argument's indentation. An only argument matches neither rule and is
/// returned unchanged.
pub fn around_from_inner(text: &BufferText, inner: ClusterRange) -> ClusterRange {
    let through = |first, last| {
        ClusterRange::through(text.full_slice(), first, last).expect("first precedes last")
    };
    let before = extend_backward_while(text, inner.start(), is_blank);
    if let Some(comma) = text.clusters().prev(before.into())
        && is_comma(text, comma)
    {
        return through(comma, extend_forward_while(text, inner.last(), is_blank));
    }

    let after = extend_forward_while(text, inner.last(), is_blank);
    if let Some(comma) = text.clusters().next(after)
        && is_comma(text, comma)
    {
        return through(before, extend_forward_while(text, comma, is_inline_blank));
    }

    inner
}

/// Around argument: the item plus its separator comma, so that deleting
/// around leaves a clean, properly-spaced list. See [`around_from_inner`]
/// for the separator rule itself.
pub fn around_argument(text: &BufferText, pos: ClusterStart) -> Option<ClusterRange> {
    let (segments, idx, nudged_pos) = locate_argument(text, pos)?;

    if segments.len() == 1 {
        // Only argument: no separator to eat; same as inner. A bracket-nudge
        // (`nudged_pos != pos`) re-enters inner_argument so a cursor on the
        // outer bracket of `foo((a))` still resolves to the nested pair's
        // argument, not the whole `(a)` outer segment; inner_argument's own
        // locate_argument call is what does that descent. Without a nudge,
        // inner_argument would just re-resolve the same pair and segments
        // this call already has, so trim directly instead.
        return if nudged_pos == pos {
            trim_blank(text, segments[idx].range(text)?)
        } else {
            inner_argument(text, nudged_pos)
        };
    }

    let inner = trim_blank(text, segments[idx].range(text)?)?;
    Some(around_from_inner(text, inner))
}
