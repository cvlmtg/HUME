mod fit;
mod resolver;
mod single;
mod view;

pub(crate) use fit::{assert_fits, assert_positions, check_fit};
pub(crate) use resolver::Resolver;
pub use single::{Facing, Selection, StickyDisplayCol};
pub use view::{EditView, LineSpan, SelectionView};

use hume_rope::cluster::ClusterStart;
use hume_rope::offset::{CharOffset, ExclusiveRange};

use crate::changeset::Assoc;
use crate::edit::TextChange;

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
        let primary_before = self.selections[self.primary];
        self.selections.sort_by_key(|s| s.start());
        self.merge_sorted(primary_before);
    }

    /// [`Self::merge_overlapping`] for selections already sorted by first
    /// cluster; `primary_before` is the selection `primary` indexes.
    fn merge_sorted(&mut self, primary_before: Selection) {
        if self.selections.len() <= 1 {
            self.primary = 0;
            return;
        }

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

    /// This set carried through `change`. Each end maps past text inserted
    /// at it and lands on the cluster of the new text holding it; selections
    /// the change folds together merge. A sticky column survives only when
    /// the change left the head's line alone. Returns the primary head in the
    /// new text.
    ///
    /// # Panics
    /// Panics if this set was not computed for `change`'s old text: carrying
    /// it would land its positions wherever the change happens to map them.
    pub fn translate(&mut self, change: &TextChange<'_>) -> ClusterStart {
        crate::selection::assert_fits(change.before(), self);
        let mut line_edits = LineEdits::new(change.edited_old_ranges(), change.before());
        let mut resolver = Resolver::new(change);
        for sel in &mut self.selections {
            let sel_before = if line_edits.leave_line_alone(sel.head().offset()) {
                *sel
            } else {
                sel.without_sticky()
            };
            *sel = resolver.carry(sel_before, Assoc::After);
        }
        self.version = change.after().version();
        self.merge_sorted(self.selections[self.primary]);
        self.selections[self.primary].head()
    }
}

/// Whether a change's edits touch a head's line, asked of heads in
/// increasing order so one walk over the edits serves a whole selection set.
pub(crate) struct LineEdits<'a> {
    edits: &'a [ExclusiveRange<CharOffset>],
    before: &'a BufferText,
    next: usize,
}

impl<'a> LineEdits<'a> {
    /// `edits` are a change's edited ranges of `before`, in order.
    pub(crate) fn new(edits: &'a [ExclusiveRange<CharOffset>], before: &'a BufferText) -> Self {
        Self {
            edits,
            before,
            next: 0,
        }
    }

    /// Whether none of the edits touches `head`'s line.
    pub(crate) fn leave_line_alone(&mut self, head: CharOffset) -> bool {
        let line = self.before.lines().range(self.before.char_to_line(head));
        let (line_start, line_end) = (line.start().offset(), line.end().offset());
        // An edit ending before this line touches no later head's line
        // either. An insertion at `line_start` still touches this line.
        while let Some(&edit) = self.edits.get(self.next) {
            let fully_before = if edit.start == edit.end {
                edit.end < line_start
            } else {
                edit.end <= line_start
            };
            if !fully_before {
                break;
            }
            self.next += 1;
        }
        self.edits
            .get(self.next)
            .is_none_or(|edit| edit.start >= line_end)
    }
}

#[cfg(test)]
mod tests;
