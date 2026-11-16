//! A set of buffer lines held as line ranges, the shape the lines a pane's
//! closed folds hide and the lines it shows raw share.

use std::sync::Arc;

use hume_rope::line::{ContentLine, join_line_ranges};
use hume_rope::offset::InclusiveRange;

/// Sorted ranges, apart from each other by at least one line not in the
/// set, so no two ranges touch or overlap.
#[derive(Clone, Default, PartialEq, Eq, Debug)]
pub struct LineSet(Arc<[InclusiveRange<ContentLine>]>);

impl LineSet {
    /// Every line `ranges` cover, merged into the set's shape.
    pub fn from_ranges(ranges: impl IntoIterator<Item = InclusiveRange<ContentLine>>) -> Self {
        Self(join_line_ranges(ranges.into_iter().map(InclusiveRange::to_exclusive)).into())
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    pub fn contains(&self, line: ContentLine) -> bool {
        self.containing(line).is_some()
    }

    /// Whether any line of `span` is in the set.
    pub fn intersects(&self, span: InclusiveRange<ContentLine>) -> bool {
        self.ending_at_or_after(span.start)
            .first()
            .is_some_and(|r| r.start <= span.end)
    }

    /// The range holding `line`.
    pub(crate) fn containing(&self, line: ContentLine) -> Option<InclusiveRange<ContentLine>> {
        self.ending_at_or_after(line)
            .first()
            .filter(|r| r.start <= line)
            .copied()
    }

    /// The ranges from the first one ending at or after `line`, in order.
    pub(crate) fn ending_at_or_after(&self, line: ContentLine) -> &[InclusiveRange<ContentLine>] {
        &self.0[self.0.partition_point(|r| r.end < line)..]
    }
}

#[cfg(test)]
mod tests;
