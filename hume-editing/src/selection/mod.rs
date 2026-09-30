mod fit;
mod recorded;
mod single;
mod unbound;
mod view;

pub(crate) use fit::{assert_fits, assert_positions, check_fit};
pub use recorded::RecordedSelections;
pub use single::{Facing, Selection, StickyDisplayCol};
pub(crate) use unbound::Resolver;
pub use unbound::UnboundSelection;
pub use view::{EditView, LineSpan, SelectionView};

use hume_rope::offset::{CharOffset, ExclusiveRange};

use crate::text::{BufferText, TextVersion};

/// The selections of one pane on one buffer, tagged with the version of the
/// text they were computed for.
///
/// # Invariants
/// 1. Never empty, and `primary` indexes into it.
/// 2. Sorted by first cluster.
/// 3. No two selections share a cluster.
/// 4. Every anchor and head is a cluster start of the tagged text.
///
/// A set on its own has no reads. Pairing it with its text
/// ([`crate::state::EditState`], [`EditView::bind`]) checks the tag and is
/// the way to read it, so a set cannot be read against the wrong text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SelectionSet {
    selections: Vec<Selection>,
    primary: usize,
    version: TextVersion,
}

impl SelectionSet {
    /// The version of the text these selections were computed for.
    pub fn version(&self) -> TextVersion {
        self.version
    }

    /// A normalized set: sorted, with selections sharing a cluster merged and
    /// the primary relocated to the selection holding it.
    ///
    /// # Panics
    /// Panics if `selections` is empty or `primary` is out of range.
    pub(crate) fn from_parts(
        selections: Vec<Selection>,
        primary: usize,
        version: TextVersion,
    ) -> Self {
        assert!(!selections.is_empty(), "SelectionSet must not be empty");
        assert!(primary < selections.len(), "primary index out of bounds");
        let mut set = Self {
            selections,
            primary,
            version,
        };
        set.merge_overlapping();
        set
    }

    /// `from_parts` without sorting or merging, for tests that need an
    /// unnormalized set.
    #[cfg(test)]
    pub(crate) fn from_parts_unchecked(
        selections: Vec<Selection>,
        primary: usize,
        version: TextVersion,
    ) -> Self {
        assert!(!selections.is_empty(), "SelectionSet must not be empty");
        assert!(primary < selections.len(), "primary index out of bounds");
        Self {
            selections,
            primary,
            version,
        }
    }

    pub(crate) fn selections(&self) -> &[Selection] {
        &self.selections
    }

    pub(crate) fn primary_pos(&self) -> usize {
        self.primary
    }

    pub(crate) fn primary_selection(&self) -> Selection {
        self.selections[self.primary]
    }

    pub(crate) fn into_parts(self) -> (Vec<Selection>, usize) {
        (self.selections, self.primary)
    }

    /// Sort by first cluster and merge selections that share a cluster,
    /// keeping the primary on the merged selection that holds it. A merged
    /// selection's head is a new position, so its sticky column is cleared.
    fn merge_overlapping(&mut self) {
        if self.selections.len() <= 1 {
            self.primary = 0;
            return;
        }

        let primary_before = self.selections[self.primary];
        self.selections.sort_by_key(|s| s.start());

        let mut write = 0;
        let mut new_primary = 0;
        let holds_primary = |sel: &Selection| {
            sel.start() <= primary_before.start() && primary_before.last() <= sel.last()
        };

        for read in 1..self.selections.len() {
            let sel = self.selections[read];
            let kept = self.selections[write];

            if sel.start() <= kept.last() {
                if sel.last() > kept.last() {
                    self.selections[write] =
                        sel.with_ends(kept.start(), sel.last()).without_sticky();
                }
                if holds_primary(&self.selections[write]) {
                    new_primary = write;
                }
            } else {
                if holds_primary(&kept) {
                    new_primary = write;
                }
                write += 1;
                self.selections[write] = sel;
            }
        }

        if holds_primary(&self.selections[write]) {
            new_primary = write;
        }

        self.selections.truncate(write + 1);
        self.primary = new_primary;
    }

    /// For each selection, whether none of `edits` (a change's edited ranges
    /// of `before`) touches its head's line. Selections are sorted and
    /// non-overlapping, so heads and their lines increase and one walk over
    /// `edits` serves the whole set.
    pub(crate) fn heads_untouched(
        &self,
        edits: &[ExclusiveRange<CharOffset>],
        before: &BufferText,
    ) -> Vec<bool> {
        let mut edit_idx = 0usize;
        self.selections
            .iter()
            .map(|sel| {
                let head = sel.head().offset();
                let pre_line = before.ropey_char_to_line(head);
                let line_start = before.line_to_char(pre_line);
                let line_end = crate::lines::next_line_start(before, pre_line);
                // An edit ending before this line touches no later selection
                // either. An insertion at `line_start` still touches this line.
                while edit_idx < edits.len() {
                    let edit = edits[edit_idx];
                    let fully_before = if edit.start == edit.end {
                        edit.end < line_start
                    } else {
                        edit.end <= line_start
                    };
                    if fully_before {
                        edit_idx += 1;
                    } else {
                        break;
                    }
                }
                !(edit_idx < edits.len() && edits[edit_idx].start < line_end)
            })
            .collect()
    }
}

#[cfg(test)]
mod tests;
