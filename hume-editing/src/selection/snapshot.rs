//! A [`SelectionSet`] as plain data, to be saved with a file and rebuilt on
//! the text the file holds when it is read back.

use hume_rope::cluster::ClusterStart;
use hume_rope::offset::CharOffset;
use serde::{Deserialize, Serialize};

use super::{Selection, SelectionSet};
use crate::text::BufferText;

/// Selections as char offsets, which outlive the text version a
/// [`SelectionSet`] is tagged with. Sticky display columns are not kept.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SelectionsSnapshot {
    /// Each selection's anchor and head, as char offsets.
    ranges: Vec<(usize, usize)>,
    primary: usize,
}

impl SelectionsSnapshot {
    pub fn of(sels: &SelectionSet) -> Self {
        let offset = |at: ClusterStart| at.offset().index();
        Self {
            ranges: sels
                .selections()
                .iter()
                .map(|s| (offset(s.anchor()), offset(s.head())))
                .collect(),
            primary: sels.primary_pos(),
        }
    }

    /// These selections on `text`, `None` unless the primary is one of them
    /// and every end starts a cluster of `text`.
    pub fn on(&self, text: &BufferText) -> Option<SelectionSet> {
        if self.primary >= self.ranges.len() {
            return None;
        }
        let at = |offset: usize| {
            let pos = CharOffset::new(offset);
            let at = (offset < text.len_chars()).then(|| text.snap(pos))?;
            (at.offset() == pos).then_some(at)
        };
        let selections = self
            .ranges
            .iter()
            .map(|&(anchor, head)| Some(Selection::new(at(anchor)?, at(head)?)))
            .collect::<Option<Vec<_>>>()?;
        Some(SelectionSet::new(text, selections, self.primary))
    }

    /// A snapshot of any ranges, for tests of what [`Self::on`] refuses.
    #[cfg(test)]
    pub(crate) fn from_ranges(ranges: Vec<(usize, usize)>, primary: usize) -> Self {
        Self { ranges, primary }
    }
}

#[cfg(test)]
mod tests;
