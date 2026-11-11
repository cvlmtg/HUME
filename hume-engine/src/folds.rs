//! Folded regions: which buffer lines a closed fold hides.
//!
//! A [`Fold`] is one region a grammar marks as foldable. [`HiddenLines`] is
//! the union of the lines a pane's closed folds hide, the one input the
//! display-line map steps over.

use std::sync::Arc;

use hume_rope::line::ContentLine;
use hume_rope::offset::InclusiveRange;

/// A foldable region: `first ..= last`, of which a closed fold hides every
/// line after `first`, except `last` when `shows_last` (a line holding only
/// the region's closing delimiter).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Fold {
    first: ContentLine,
    last: ContentLine,
    shows_last: bool,
}

impl Fold {
    /// The region `first ..= last`, `None` when closing it would hide no line.
    pub fn new(first: ContentLine, last: ContentLine, shows_last: bool) -> Option<Self> {
        let fold = Self {
            first,
            last,
            shows_last,
        };
        (fold.last_hidden() > first).then_some(fold)
    }

    /// The line a closed fold keeps showing in place of the region.
    pub fn first(self) -> ContentLine {
        self.first
    }

    /// The region's last line.
    pub fn last(self) -> ContentLine {
        self.last
    }

    /// Whether a closed fold keeps showing [`Fold::last`].
    pub fn shows_last(self) -> bool {
        self.shows_last
    }

    /// The lines a closed fold hides.
    pub fn hidden(self) -> InclusiveRange<ContentLine> {
        InclusiveRange::new(self.first.advance(1), self.last_hidden())
    }

    /// Whether `line` lies in the region, first and last line included.
    pub fn spans(self, line: ContentLine) -> bool {
        InclusiveRange::new(self.first, self.last).contains(line)
    }

    fn last_hidden(self) -> ContentLine {
        match self.shows_last {
            true => self.last.retreat_saturating(1),
            false => self.last,
        }
    }
}

/// The lines a set of closed folds hides: sorted ranges, apart from each
/// other by at least one shown line. Line 0 is never hidden, since every
/// fold shows its first line.
#[derive(Clone, Default, PartialEq, Eq, Debug)]
pub struct HiddenLines(Arc<[InclusiveRange<ContentLine>]>);

impl HiddenLines {
    /// The lines `folds` hide, closed all at once.
    pub fn from_folds(folds: impl IntoIterator<Item = Fold>) -> Self {
        let mut hidden: Vec<_> = folds.into_iter().map(Fold::hidden).collect();
        hidden.sort_unstable_by_key(|r| r.start);
        let mut merged: Vec<InclusiveRange<ContentLine>> = Vec::with_capacity(hidden.len());
        for range in hidden {
            match merged.last_mut() {
                Some(last) if range.start <= last.end.advance(1) => {
                    last.end = last.end.max(range.end);
                }
                _ => merged.push(range),
            }
        }
        Self(merged.into())
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    pub fn is_hidden(&self, line: ContentLine) -> bool {
        self.hiding(line).is_some()
    }

    /// Whether a closed fold starts at `line`: `line` is shown and the line
    /// after it is not.
    pub fn closed_at(&self, line: ContentLine) -> bool {
        let next = self.0.partition_point(|r| r.start <= line);
        self.0.get(next).is_some_and(|r| r.start == line.advance(1))
    }

    /// `line`, or the line showing in its place when a fold hides it.
    pub fn shown_at_or_before(&self, line: ContentLine) -> ContentLine {
        match self.hiding(line) {
            Some(range) => range.start.retreat_saturating(1),
            None => line,
        }
    }

    /// The first shown line after `line`. It may lie past the buffer's last
    /// line, as `line.advance(1)` would.
    pub fn next_shown(&self, line: ContentLine) -> ContentLine {
        self.line_after_shown(line, 1)
    }

    /// The last shown line before `line`, `None` from line 0.
    pub fn prev_shown(&self, line: ContentLine) -> Option<ContentLine> {
        (line.index() > 0).then(|| self.shown_at_or_before(line.retreat_saturating(1)))
    }

    /// How many shown lines lie in `from .. to`.
    pub fn shown_between(&self, from: ContentLine, to: ContentLine) -> usize {
        if to <= from {
            return 0;
        }
        let last = to.retreat_saturating(1);
        let hidden: usize = self.0[self.first_ending_at_or_after(from)..]
            .iter()
            .take_while(|r| r.start <= last)
            .map(|r| r.end.min(last).lines_since(r.start.max(from)) + 1)
            .sum();
        to.lines_since(from) - hidden
    }

    /// The line `n` shown lines past `from`, `from` itself for `n == 0`.
    /// Unbounded above, like [`HiddenLines::next_shown`].
    pub fn line_after_shown(&self, from: ContentLine, n: usize) -> ContentLine {
        if n == 0 {
            return from;
        }
        let mut line = from;
        let mut left = n;
        for range in &self.0[self.first_ending_at_or_after(from)..] {
            if range.start > line {
                let gap = range.start.lines_since(line) - 1;
                if left <= gap {
                    return line.advance(left);
                }
                left -= gap;
            }
            line = line.max(range.end);
        }
        line.advance(left)
    }

    /// The runs of shown lines in `lines`, in order.
    pub fn shown_runs(
        &self,
        lines: InclusiveRange<ContentLine>,
    ) -> impl Iterator<Item = InclusiveRange<ContentLine>> + '_ {
        let mut hidden = self.0[self.first_ending_at_or_after(lines.start)..]
            .iter()
            .peekable();
        let mut from = Some(lines.start);
        std::iter::from_fn(move || {
            let mut start = from?;
            while let Some(range) = hidden.next_if(|r| r.start <= start) {
                start = range.end.advance(1);
            }
            if start > lines.end {
                from = None;
                return None;
            }
            let end = match hidden.next() {
                Some(range) => {
                    from = Some(range.end.advance(1));
                    range.start.retreat_saturating(1).min(lines.end)
                }
                None => {
                    from = None;
                    lines.end
                }
            };
            Some(InclusiveRange::new(start, end))
        })
    }

    fn hiding(&self, line: ContentLine) -> Option<InclusiveRange<ContentLine>> {
        self.0
            .get(self.first_ending_at_or_after(line))
            .filter(|r| r.start <= line)
            .copied()
    }

    fn first_ending_at_or_after(&self, line: ContentLine) -> usize {
        self.0.partition_point(|r| r.end < line)
    }
}

#[cfg(test)]
mod tests;
