//! Text with payloads attached to parts of it.

use ropey::RopeSlice;

use crate::cluster::ClusterRange;
use crate::column::ByteCol;
use crate::grapheme::{next_str_boundary, prev_str_boundary};
use crate::offset::{CharOffset, ExclusiveRange};
use crate::width::ELLIPSIS;

/// A string and the parts of it that carry a payload: byte ranges sorted by
/// start, non-overlapping, each starting and ending on a cluster boundary of
/// the text. The text and its spans are one value, so no holder can pair
/// the spans with another text.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SpannedText<T> {
    text: String,
    spans: Vec<(ExclusiveRange<ByteCol>, T)>,
}

impl<T> SpannedText<T> {
    /// `text` with no span.
    pub fn plain(text: String) -> Self {
        Self {
            text,
            spans: Vec::new(),
        }
    }

    /// `text` with `spans`, each a `(start, end, payload)` of char offsets
    /// into `text`. This is the one validator for a payload attached to a
    /// part of a string: it rejects an empty span, a span ending past the
    /// text, and two spans that share a char.
    ///
    /// An edge inside a grapheme cluster widens the span to the whole
    /// cluster. Two spans that widen into the same cluster overlap and are
    /// rejected: one character takes one payload.
    pub fn new(text: String, spans: Vec<(usize, usize, T)>) -> Result<Self, SpanError> {
        // `RopeSlice::from` scans all of `text`, which no span needs here.
        if spans.is_empty() {
            return Ok(Self::plain(text));
        }
        let mut spans: Vec<_> = spans.into_iter().enumerate().collect();
        spans.sort_by_key(|(_, (start, _, _))| *start);

        let slice = RopeSlice::from(text.as_str());
        let len = slice.len_chars();

        let mut prev_end = 0usize;
        let mut prev = (0, 0);
        let mut out = Vec::with_capacity(spans.len());
        for (index, (start, end, payload)) in spans {
            let fail = |kind| Err(SpanError { index, kind });
            if start >= end {
                return fail(SpanErrorKind::Empty { start, end });
            }
            if end > len {
                return fail(SpanErrorKind::PastEnd { end, len });
            }
            let chars = ClusterRange::covering(
                slice,
                ExclusiveRange::new(CharOffset::new(start), CharOffset::new(end)),
            )
            .expect("a non-empty range within the text covers a cluster")
            .chars();
            let start_byte = slice.char_to_byte(chars.start.index());
            let end_byte = slice.char_to_byte(chars.end.index());
            if start_byte < prev_end {
                let (other_start, other_end) = prev;
                return fail(SpanErrorKind::Overlap {
                    start,
                    end,
                    other_start,
                    other_end,
                });
            }
            prev_end = end_byte;
            prev = (start, end);
            out.push((
                ExclusiveRange::new(ByteCol::new(start_byte), ByteCol::new(end_byte)),
                payload,
            ));
        }

        Ok(Self { text, spans: out })
    }

    /// The same text and ranges, each payload replaced by `f` of it.
    pub fn map_payload<U>(self, mut f: impl FnMut(T) -> U) -> SpannedText<U> {
        SpannedText {
            text: self.text,
            spans: self
                .spans
                .into_iter()
                .map(|(range, payload)| (range, f(payload)))
                .collect(),
        }
    }

    /// The part of the text in `kept`, a byte range starting and ending on
    /// cluster boundaries. Each span is clipped to `kept` and moves with the
    /// text, and a span left empty is dropped.
    pub(crate) fn slice(mut self, kept: ExclusiveRange<ByteCol>) -> Self {
        let (start, end) = (kept.start.index(), kept.end.index());
        debug_assert!(self.text.is_char_boundary(start) && self.text.is_char_boundary(end));
        self.text.truncate(end);
        self.text.drain(..start);
        let spans = self
            .spans
            .into_iter()
            .filter_map(|(range, payload)| {
                let span_start = range.start.index().max(start);
                let span_end = range.end.index().min(end);
                (span_start < span_end).then(|| {
                    let range = ExclusiveRange::new(
                        ByteCol::new(span_start - start),
                        ByteCol::new(span_end - start),
                    );
                    (range, payload)
                })
            })
            .collect();
        Self {
            text: self.text,
            spans,
        }
    }

    /// [`ELLIPSIS`] followed by this text, the marker carrying no span. A
    /// character after the marker joins it only by extending it (a
    /// combining mark, a spacing mark, a zero-width joiner): U+2026 is not a
    /// regional indicator, an extended pictograph or a prepend character.
    /// So only the first cluster of this text can change, and a span
    /// starting there widens back over the marker, as [`Self::new`] widens
    /// an edge inside a cluster.
    pub(crate) fn prepend_ellipsis(mut self) -> Self {
        let seam = ELLIPSIS.len();
        self.text.insert_str(0, ELLIPSIS);
        for (range, _) in &mut self.spans {
            *range = ExclusiveRange::new(
                range.start.advance_saturating(seam),
                range.end.advance_saturating(seam),
            );
        }
        if let Some((range, _)) = self.spans.first_mut()
            && range.start.index() == seam
            && let Some(cluster) = cluster_across(&self.text, seam)
        {
            range.start = cluster.start;
        }
        self
    }

    /// This text followed by `after`, which carries no span. When `after`
    /// extends the last cluster of this text, a span ending there widens over
    /// the joined cluster, as [`Self::new`] widens an edge inside a cluster.
    pub fn append(mut self, after: &str) -> Self {
        let seam = self.text.len();
        self.text.push_str(after);
        if let Some((range, _)) = self.spans.last_mut()
            && range.end.index() == seam
            && let Some(cluster) = cluster_across(&self.text, seam)
        {
            range.end = cluster.end;
        }
        self
    }

    pub fn text(&self) -> &str {
        &self.text
    }

    pub fn spans(&self) -> &[(ExclusiveRange<ByteCol>, T)] {
        &self.spans
    }
}

/// The cluster of `text` that starts before byte `seam` and ends after it,
/// `None` when `seam` is a cluster boundary.
fn cluster_across(text: &str, seam: usize) -> Option<ExclusiveRange<ByteCol>> {
    let start = prev_str_boundary(text, seam);
    let end = next_str_boundary(text, start);
    (start < seam && end > seam)
        .then(|| ExclusiveRange::new(ByteCol::new(start), ByteCol::new(end)))
}

/// `text` as consecutive runs: each span's text with its payload, and each
/// non-empty stretch between or after spans with `None`. `spans` are byte
/// ranges into `text`, sorted by start and non-overlapping, as
/// [`SpannedText::spans`] holds them.
pub fn runs<'a, T>(
    text: &'a str,
    spans: &'a [(ExclusiveRange<ByteCol>, T)],
) -> impl Iterator<Item = (&'a str, Option<&'a T>)> {
    let tail_start = spans.last().map_or(0, |(range, _)| range.end.index());
    let mut gap_start = 0;
    spans
        .iter()
        .flat_map(move |(range, payload)| {
            let gap = &text[gap_start..range.start.index()];
            gap_start = range.end.index();
            [(gap, None), (&text[range.as_byte_range()], Some(payload))]
        })
        .chain([(&text[tail_start..], None)])
        .filter(|(run, _)| !run.is_empty())
}

/// Why [`SpannedText::new`] rejected a span, and which one: `index` is the
/// span's place in the list the caller passed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SpanError {
    pub index: usize,
    pub kind: SpanErrorKind,
}

/// What is wrong with a rejected span. Each variant carries the span as the
/// caller passed it. The `Display` text names no noun and no index, so a
/// caller prefixes its own ("lines 2 'segments 1").
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SpanErrorKind {
    /// `start >= end`.
    Empty { start: usize, end: usize },
    /// `end` is past the text's `len` chars.
    PastEnd { end: usize, len: usize },
    /// After widening to whole clusters, the span shares a char with the
    /// span before it in start order, `(other_start, other_end)` as the
    /// caller passed it.
    Overlap {
        start: usize,
        end: usize,
        other_start: usize,
        other_end: usize,
    },
}

impl std::fmt::Display for SpanErrorKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Empty { start, end } => write!(f, "({start}, {end}) must have start < end"),
            Self::PastEnd { end, len } => {
                write!(f, "end {end} is past the text's char length {len}")
            }
            Self::Overlap {
                start,
                end,
                other_start,
                other_end,
            } => write!(
                f,
                "({start}, {end}) overlaps ({other_start}, {other_end}): they share a character"
            ),
        }
    }
}

#[cfg(test)]
mod tests;
