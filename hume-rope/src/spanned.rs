//! Text with payloads attached to parts of it.

use ropey::RopeSlice;

use crate::cluster::ClusterRange;
use crate::column::ByteCol;
use crate::offset::{CharOffset, ExclusiveRange};

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

    pub fn text(&self) -> &str {
        &self.text
    }

    pub fn spans(&self) -> &[(ExclusiveRange<ByteCol>, T)] {
        &self.spans
    }
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
