mod single;
#[cfg(test)]
pub mod testing;

pub use single::{Selection, is_selection_linewise};

use crate::changeset::{Assoc, ChangeSet};
use crate::error::ValidationError;
use crate::text::Text;

/// The complete selection state for one buffer.
///
/// # Invariants
/// 1. Never empty — always at least one `Selection`.
/// 2. Selections are sorted in ascending order of `start()`.
/// 3. No two selections overlap. Adjacent selections (where one ends exactly
///    where the next begins) are merged.
///
/// Invariants 2 and 3 are enforced by [`SelectionSet::merge_overlapping`],
/// which must be called after any operation that might violate them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SelectionSet {
    /// The sorted, non-overlapping selections.
    ///
    /// `Vec` is the right choice here: in practice editors have at most dozens
    /// of selections; linear scan and sort are faster than a tree for that
    /// cardinality due to cache locality.
    selections: Vec<Selection>,

    /// Index of the "primary" selection — the one displayed in the statusline
    /// and used for operations that act on a single selection (e.g.,
    /// `cmd_keep_primary_selection`).
    primary: usize,
}

impl Default for SelectionSet {
    /// Minimal-valid state: a single collapsed cursor at offset 0.
    ///
    /// Required so `std::mem::take` produces a structurally valid `SelectionSet`
    /// (an empty vec + `primary: 0` would violate the "primary indexes into
    /// selections" invariant). Matches the stdlib pattern — `Default` is always
    /// a valid state.
    fn default() -> Self {
        Self {
            selections: vec![Selection::collapsed(0)],
            primary: 0,
        }
    }
}

impl SelectionSet {
    /// Create a set with a single selection. This is the normal starting state.
    pub fn single(sel: Selection) -> Self {
        Self {
            selections: vec![sel],
            primary: 0,
        }
    }

    /// The primary (focused) selection.
    pub fn primary(&self) -> Selection {
        self.selections[self.primary]
    }

    /// The index of the primary selection within the sorted selections Vec.
    ///
    /// Useful when rebuilding a `SelectionSet` after transforming all selections
    /// and you need to preserve which one is primary.
    pub fn primary_index(&self) -> usize {
        self.primary
    }

    /// Number of selections.
    ///
    /// A `SelectionSet` is non-empty by invariant (day-one: at least one
    /// selection always exists), so `is_empty()` is intentionally absent.
    #[allow(clippy::len_without_is_empty)]
    pub fn len(&self) -> usize {
        self.selections.len()
    }

    /// Iterate over all selections in ascending `start()` order.
    pub fn iter_sorted(&self) -> impl Iterator<Item = &Selection> {
        self.selections.iter()
    }

    /// Return all selections sorted by ascending `head` order.
    ///
    /// The engine layer (`EngineView::panes`) requires selections sorted by
    /// `head`, which differs from `start()` when anchor > head (backward
    /// selections). This function allocates a scratch `Vec` and sorts it —
    /// the allocation is intentional and visible in the return type.
    pub fn iter_head_sorted(&self) -> Vec<&Selection> {
        let mut v: Vec<&Selection> = self.selections.iter().collect();
        v.sort_by_key(|s| s.head);
        v
    }

    /// Apply `f` to every selection and return a canonicalized `SelectionSet`.
    ///
    /// After applying `f` the result is sorted by `start()`, overlapping or
    /// adjacent selections are merged, and the primary is relocated by content
    /// (the mapped selection that was previously primary stays primary after
    /// the merge). The returned set always satisfies all `SelectionSet`
    /// invariants.
    ///
    /// **Iteration order:** `f` is called in ascending-`start()` order
    /// (same as [`iter_sorted`](Self::iter_sorted)).
    #[must_use]
    pub fn map<F>(self, mut f: F) -> Self
    where
        F: FnMut(Selection) -> Selection,
    {
        // Capture the primary index before consuming self so that
        // merge_overlapping_in_place picks up the right `primary_before`
        // (the mapped primary selection at that index, before sorting).
        let primary = self.primary;
        let selections = self.selections.into_iter().map(&mut f).collect();
        let mut result = Self {
            selections,
            primary,
        };
        result.merge_overlapping_in_place();
        result
    }

    /// Replace the selection at `idx` with `new_sel` and return the updated
    /// set. Panics if `idx >= len()`.
    pub fn replace(mut self, idx: usize, new_sel: Selection) -> Self {
        self.selections[idx] = new_sel;
        self
    }

    /// Merge overlapping or adjacent selections and sort by position.
    ///
    /// After this call:
    /// - Selections are sorted ascending by `start()`.
    /// - No two selections overlap or touch (adjacent = same offset).
    /// - Cursor positions (head) are preserved as best as possible: the merged
    ///   selection keeps the head of whichever original selection had the
    ///   greater `end()` (the "rightmost extent wins").
    ///
    /// The primary index is updated to point at the merged selection that
    /// contained the original primary.
    ///
    /// This is the consuming form of [`merge_overlapping_in_place`][Self::merge_overlapping_in_place];
    /// both share one implementation so their merge semantics (including the
    /// `horiz` reset on merged selections) can never drift apart.
    #[must_use]
    pub fn merge_overlapping(mut self) -> Self {
        self.merge_overlapping_in_place();
        self
    }

    /// Build a `SelectionSet` from a non-empty `Vec<Selection>`, with
    /// `primary` pointing at the given index.
    ///
    /// The input is automatically sorted and merged so the output always
    /// satisfies the `SelectionSet` invariants (sorted, non-overlapping,
    /// non-empty). The `primary` is interpreted as an index into the
    /// *input* vec; after sort+merge, the primary is relocated to the
    /// compacted slot that contains that selection's range.
    ///
    /// # Panics
    /// Panics if `selections` is empty or `primary >= selections.len()`.
    pub fn from_vec(selections: Vec<Selection>, primary: usize) -> Self {
        assert!(!selections.is_empty(), "SelectionSet must not be empty");
        assert!(primary < selections.len(), "primary index out of bounds");
        let mut result = Self {
            selections,
            primary,
        };
        result.merge_overlapping_in_place();
        result
    }

    /// Build a `SelectionSet` from a raw `Vec<Selection>` **without**
    /// sorting or merging.
    ///
    /// **For tests only.** Use this when a test deliberately needs to construct
    /// an out-of-order or overlapping set to exercise downstream merge /
    /// propagation logic. Production code must use [`from_vec`](Self::from_vec).
    ///
    /// `#[cfg(test)]` cannot be used here because the function is called from
    /// cross-crate tests (the `editor` test suite); making it conditionally
    /// compiled would hide it from those callers. The name makes the intent clear.
    ///
    /// # Panics
    /// Panics if `selections` is empty or `primary >= selections.len()`.
    pub fn from_vec_unchecked(selections: Vec<Selection>, primary: usize) -> Self {
        assert!(!selections.is_empty(), "SelectionSet must not be empty");
        assert!(primary < selections.len(), "primary index out of bounds");
        Self {
            selections,
            primary,
        }
    }

    // ── Selection-set manipulation ────────────────────────────────────────────

    /// Return a new set containing only the primary selection.
    ///
    /// All other selections are dropped. The primary index resets to 0.
    pub fn keep_primary(self) -> Self {
        let primary = self.selections[self.primary];
        Self {
            selections: vec![primary],
            primary: 0,
        }
    }

    /// Remove the selection at `idx` and return the updated set.
    ///
    /// If `idx` is the primary, the new primary becomes the next selection
    /// in document order, wrapping around to the first if the removed
    /// selection was the last. If `len() == 1`, returns `self` unchanged — you cannot
    /// remove the only selection. Panics if `idx >= len()`.
    pub fn remove(mut self, idx: usize) -> Self {
        if self.selections.len() <= 1 {
            return self; // can't remove the only selection — no-op
        }
        assert!(idx < self.selections.len(), "remove index out of bounds");
        self.selections.remove(idx);
        let new_len = self.selections.len();
        self.primary = if idx < self.primary {
            self.primary - 1
        } else if idx == self.primary {
            idx % new_len
        } else {
            self.primary
        };
        self
    }

    /// Shift the primary index by `delta`, wrapping around.
    ///
    /// `delta = 1` moves to the next selection (forward), `-1` moves to the
    /// previous (backward). Works correctly for `|delta| >= len()` too.
    pub fn cycle_primary(mut self, delta: isize) -> Self {
        let len = self.selections.len() as isize;
        // `rem_euclid` gives a non-negative result even for negative `delta`,
        // so we never underflow into a huge `usize` value.
        self.primary = ((self.primary as isize + delta).rem_euclid(len)) as usize;
        self
    }

    /// Assert (in debug builds) that every selection's `head` and `anchor`
    /// are within bounds for a buffer of `buf_len` chars.
    ///
    /// The invariant is `head < buf_len` and `anchor < buf_len` — selections
    /// are zero-indexed and must not point past the last character (the
    /// structural trailing `\n`).
    ///
    /// Call this at every chokepoint where a `(Text, SelectionSet)` pair is
    /// produced: edit operations, motions, and `Transaction::apply`.
    #[inline]
    pub fn debug_assert_valid(&self, buf: &Text) {
        let buf_len = buf.len_chars();
        debug_assert!(
            buf_len > 0,
            "Text must have at least 1 char (the structural \\n)"
        );
        debug_assert!(
            buf.char_at(buf_len - 1) == Some('\n'),
            "Text must end with structural '\\n', but last char is {:?}",
            buf.char_at(buf_len - 1),
        );
        for (i, sel) in self.selections.iter().enumerate() {
            debug_assert!(
                sel.head < buf_len,
                "Selection {i}: head {} >= buf_len {buf_len} — cursor is past the end of the buffer",
                sel.head,
            );
            debug_assert!(
                sel.anchor < buf_len,
                "Selection {i}: anchor {} >= buf_len {buf_len} — anchor is past the end of the buffer",
                sel.anchor,
            );
        }
    }

    /// Validate that every selection's `head` and `anchor` are in bounds for
    /// a buffer of `buf_len` chars. Returns `Err` with a descriptive error if
    /// any position is out of range.
    ///
    /// Unlike [`debug_assert_valid`][Self::debug_assert_valid], this check
    /// runs in all builds — including release. Call it at the trust boundary
    /// where plugin-constructed [`Transaction`][crate::transaction::Transaction]s
    /// enter the system.
    pub fn validate(&self, buf_len: usize) -> Result<(), ValidationError> {
        if buf_len == 0 {
            return Err(ValidationError::EmptyBuffer);
        }
        for (index, sel) in self.selections.iter().enumerate() {
            if sel.head >= buf_len {
                return Err(ValidationError::SelectionOutOfBounds {
                    index,
                    field: "head",
                    value: sel.head,
                    buf_len,
                });
            }
            if sel.anchor >= buf_len {
                return Err(ValidationError::SelectionOutOfBounds {
                    index,
                    field: "anchor",
                    value: sel.anchor,
                    buf_len,
                });
            }
        }
        Ok(())
    }

    // ── In-place propagation ──────────────────────────────────────────────────

    /// Merge overlapping or adjacent selections in place, updating `primary`.
    ///
    /// Merged selections get `horiz: None` regardless of their pre-merge values
    /// because the merged `head` is semantically a new position — the column it
    /// corresponds to was never latched by a vertical motion.
    pub fn merge_overlapping_in_place(&mut self) {
        if self.selections.len() <= 1 {
            return;
        }

        let primary_before = self.selections[self.primary];
        self.selections.sort_by_key(|s| s.start());

        let mut write = 0;
        let mut new_primary = 0;

        for read in 1..self.selections.len() {
            let sel = self.selections[read];
            let last = &mut self.selections[write];

            if sel.start() <= last.end() {
                if sel.end() > last.end() {
                    if sel.head <= sel.anchor {
                        last.head = last.start().min(sel.head);
                        last.anchor = sel.end();
                    } else {
                        last.anchor = last.start();
                        last.head = sel.end();
                    }
                    // Merged — reset horiz since neither side's column is valid.
                    last.horiz = None;
                }
                if primary_before.start() >= last.start() && primary_before.end() <= last.end() {
                    new_primary = write;
                }
            } else {
                let done = &self.selections[write];
                if done.start() >= primary_before.start() && done.end() <= primary_before.end() {
                    new_primary = write;
                }
                write += 1;
                self.selections[write] = sel;
            }
        }

        let done = &self.selections[write];
        if done.start() >= primary_before.start() && done.end() <= primary_before.end() {
            new_primary = write;
        }

        self.selections.truncate(write + 1);
        self.primary = new_primary;
    }

    /// Propagate a `ChangeSet` through all selections in place.
    ///
    /// This is the non-acting-pane propagation primitive. For each selection:
    /// - Maps `anchor` and `head` through `cs.map_pos(_, Assoc::After)`.
    /// - Resets `horiz` to `None` if the edit touched the head's pre-edit line
    ///   (the display column is stale when the line's content changed).
    /// - After all selections are mapped, calls `merge_overlapping_in_place` so
    ///   the no-overlap invariant is restored (a deletion spanning multiple
    ///   selections can collapse them).
    ///
    /// `buf_pre` must be the buffer text **before** the edit — the pre-edit line
    /// map is needed to identify which line each head resided on before mapping.
    pub fn translate_in_place(&mut self, cs: &ChangeSet, buf_pre: &Text) {
        for sel in &mut self.selections {
            let pre_line = buf_pre.char_to_line(sel.head);
            sel.anchor = cs.map_pos(sel.anchor, Assoc::After);
            sel.head = cs.map_pos(sel.head, Assoc::After);
            if cs.touches_line(buf_pre, pre_line) {
                sel.horiz = None;
            }
        }
        if self.selections.len() > 1 {
            self.merge_overlapping_in_place();
        }
    }
}

// ── Tests ──────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    // ── SelectionSet ──────────────────────────────────────────────────────────

    #[test]
    fn single_selection_is_primary() {
        let s = Selection::collapsed(0);
        let set = SelectionSet::single(s);
        assert_eq!(set.primary(), s);
        assert_eq!(set.len(), 1);
    }

    #[test]
    fn merge_no_overlap() {
        // Two disjoint selections — should stay separate.
        let set =
            SelectionSet::from_vec_unchecked(vec![Selection::new(0, 3), Selection::new(5, 8)], 0)
                .merge_overlapping();
        assert_eq!(set.len(), 2);
    }

    #[test]
    fn merge_overlapping_selections() {
        // (anchor=0,head=5) and (anchor=3,head=8) overlap — should merge.
        let set =
            SelectionSet::from_vec_unchecked(vec![Selection::new(0, 5), Selection::new(3, 8)], 0)
                .merge_overlapping();
        assert_eq!(set.len(), 1);
        assert_eq!(set.primary().start(), 0);
        assert_eq!(set.primary().end(), 8);
    }

    #[test]
    fn merge_adjacent_selections() {
        // (anchor=0,head=3) and (anchor=3,head=6) touch at offset 3 — should merge.
        let set =
            SelectionSet::from_vec_unchecked(vec![Selection::new(0, 3), Selection::new(3, 6)], 0)
                .merge_overlapping();
        assert_eq!(set.len(), 1);
        assert_eq!(set.primary().start(), 0);
        assert_eq!(set.primary().end(), 6);
    }

    #[test]
    fn merge_duplicate_selections() {
        let set =
            SelectionSet::from_vec_unchecked(vec![Selection::new(2, 5), Selection::new(2, 5)], 0)
                .merge_overlapping();
        assert_eq!(set.len(), 1);
    }

    #[test]
    fn merge_contained_selection() {
        // (anchor=0,head=8) fully contains (anchor=2,head=5) — should merge.
        let set =
            SelectionSet::from_vec_unchecked(vec![Selection::new(0, 8), Selection::new(2, 5)], 0)
                .merge_overlapping();
        assert_eq!(set.len(), 1);
        assert_eq!(set.primary().end(), 8);
    }

    #[test]
    fn merge_consuming_clears_horiz_on_extended_selection() {
        // Consuming merge_overlapping delegates to merge_overlapping_in_place,
        // so horiz is cleared on merged selections — same as the in-place path.
        let a = Selection::with_horiz(0, 5, 42); // horiz latched
        let b = Selection::with_horiz(3, 8, 99); // horiz latched
        let set = SelectionSet::from_vec_unchecked(vec![a, b], 0);
        // The two selections overlap → they merge into one.
        let merged = set.merge_overlapping();
        assert_eq!(merged.len(), 1);
        // The merged selection must have horiz cleared — neither side's column is valid.
        assert_eq!(merged.primary().horiz, None, "merge must clear horiz");
    }

    #[test]
    fn merge_idempotent() {
        // Start with an unmerged overlapping set so the first merge does real work,
        // then verify a second merge is a no-op.
        let set =
            SelectionSet::from_vec_unchecked(vec![Selection::new(0, 5), Selection::new(3, 8)], 0)
                .merge_overlapping();
        // The first merge must have reduced the two overlapping selections to one.
        assert_eq!(
            set.len(),
            1,
            "first merge must reduce overlapping selections"
        );
        let set2 = set.clone().merge_overlapping();
        assert_eq!(
            set, set2,
            "second merge must be a no-op on an already-merged set"
        );
    }

    #[test]
    fn merge_three_into_one() {
        let set = SelectionSet::from_vec_unchecked(
            vec![
                Selection::new(0, 4),
                Selection::new(3, 7),
                Selection::new(6, 10),
            ],
            1,
        )
        .merge_overlapping();
        assert_eq!(set.len(), 1);
        assert_eq!(set.primary().start(), 0);
        assert_eq!(set.primary().end(), 10);
    }

    #[test]
    fn merge_overlapping_backward_selections() {
        // Two backward selections that overlap: (anchor=8, head=3) and
        // (anchor=10, head=5). After sorting by start(), the merge should
        // produce a single backward selection spanning 3–10.
        let set =
            SelectionSet::from_vec_unchecked(vec![Selection::new(8, 3), Selection::new(10, 5)], 0)
                .merge_overlapping();
        assert_eq!(set.len(), 1);
        let s = set.primary();
        assert_eq!(s.start(), 3);
        assert_eq!(s.end(), 10);
        // Merged result should be backward (head < anchor).
        assert!(
            s.head < s.anchor,
            "merged backward selections should stay backward"
        );
    }

    #[test]
    fn merge_sorts_unsorted_input() {
        // Pass selections out of order — merge should sort them first.
        let set =
            SelectionSet::from_vec_unchecked(vec![Selection::new(5, 8), Selection::new(0, 3)], 0)
                .merge_overlapping();
        assert_eq!(set.len(), 2);
        assert_eq!(set.selections[0].start(), 0);
        assert_eq!(set.selections[1].start(), 5);
    }

    #[test]
    fn map_relocates_primary_by_content() {
        let set = SelectionSet::from_vec(
            vec![Selection::collapsed(0), Selection::collapsed(5)],
            1, // primary is the second one
        );
        // shift(1) is order-preserving; primary should track to its new position.
        let shifted = set.map(|s| s.shift(1));
        assert_eq!(shifted.primary().head, 6); // was 5, shifted by 1
    }

    #[test]
    fn replace_updates_selection() {
        let set = SelectionSet::from_vec(vec![Selection::collapsed(0), Selection::collapsed(5)], 0);
        let updated = set.replace(1, Selection::collapsed(10));
        assert_eq!(updated.selections[1].head, 10);
    }

    // ── map (merge semantics) ─────────────────────────────────────────────────

    #[test]
    fn map_collapses_to_same_position() {
        // Two cursors at different positions that a motion maps to the same
        // spot — e.g. "go to end of line" when both are on the same line.
        let set = SelectionSet::from_vec(vec![Selection::collapsed(2), Selection::collapsed(7)], 0);
        let merged = set.map(|_| Selection::collapsed(10));
        assert_eq!(merged.len(), 1);
        assert_eq!(merged.primary().head, 10);
    }

    #[test]
    fn map_reorders_reversed_positions() {
        // A motion that reverses the order: cursor at 2 maps to 8, cursor
        // at 7 maps to 1. After merge the result should be sorted [1, 8].
        let set = SelectionSet::from_vec(
            vec![Selection::collapsed(2), Selection::collapsed(7)],
            1, // primary is the second one (at 7)
        );
        let merged = set.map(|s| {
            if s.head == 2 {
                Selection::collapsed(8)
            } else {
                Selection::collapsed(1)
            }
        });
        assert_eq!(merged.len(), 2);
        // Sorted by position: first at 1, second at 8.
        assert_eq!(merged.selections[0].head, 1);
        assert_eq!(merged.selections[1].head, 8);
        // Primary was the cursor at 7 → mapped to 1 → now at index 0.
        assert_eq!(merged.primary().head, 1);
    }

    // ── keep_primary ──────────────────────────────────────────────────────────

    #[test]
    fn keep_primary_drops_others() {
        let set = SelectionSet::from_vec(
            vec![
                Selection::collapsed(0),
                Selection::collapsed(5),
                Selection::collapsed(10),
            ],
            1, // primary is the middle one
        );
        let kept = set.keep_primary();
        assert_eq!(kept.len(), 1);
        assert_eq!(kept.primary().head, 5);
        assert_eq!(kept.primary_index(), 0);
    }

    #[test]
    fn keep_primary_single_is_noop() {
        let set = SelectionSet::single(Selection::collapsed(3));
        let kept = set.clone().keep_primary();
        assert_eq!(kept, set);
    }

    // ── remove ───────────────────────────────────────────────────────────────

    #[test]
    fn remove_before_primary_shifts_primary_down() {
        let set = SelectionSet::from_vec(
            vec![
                Selection::collapsed(0),
                Selection::collapsed(5),
                Selection::collapsed(10),
            ],
            2, // primary is the last one
        );
        let result = set.remove(0); // remove first
        assert_eq!(result.len(), 2);
        assert_eq!(result.primary().head, 10); // primary shifted from index 2 to 1
        assert_eq!(result.primary_index(), 1);
    }

    #[test]
    fn remove_primary_advances_to_next() {
        let set = SelectionSet::from_vec(
            vec![
                Selection::collapsed(0),
                Selection::collapsed(5),
                Selection::collapsed(10),
            ],
            1, // primary is the middle one
        );
        let result = set.remove(1); // remove the primary
        assert_eq!(result.len(), 2);
        // Next in document order after index 1 is now index 1 (was 2, shifted down)
        assert_eq!(result.primary().head, 10);
    }

    #[test]
    fn remove_primary_at_end_wraps_to_first() {
        let set = SelectionSet::from_vec(
            vec![
                Selection::collapsed(0),
                Selection::collapsed(5),
                Selection::collapsed(10),
            ],
            2, // primary is the last one
        );
        let result = set.remove(2);
        assert_eq!(result.len(), 2);
        // idx=2 % new_len=2 = 0 → wraps to the first selection
        assert_eq!(result.primary().head, 0);
    }

    #[test]
    fn remove_after_primary_leaves_primary_unchanged() {
        let set = SelectionSet::from_vec(
            vec![
                Selection::collapsed(0),
                Selection::collapsed(5),
                Selection::collapsed(10),
            ],
            0, // primary is the first one
        );
        let result = set.remove(2); // remove last
        assert_eq!(result.len(), 2);
        assert_eq!(result.primary().head, 0);
        assert_eq!(result.primary_index(), 0);
    }

    #[test]
    fn remove_single_is_noop() {
        let set = SelectionSet::single(Selection::collapsed(0));
        let result = set.clone().remove(0);
        assert_eq!(result, set); // unchanged — can't remove the only selection
    }

    // ── cycle_primary ─────────────────────────────────────────────────────────

    #[test]
    fn cycle_primary_forward() {
        let set = SelectionSet::from_vec(
            vec![
                Selection::collapsed(0),
                Selection::collapsed(5),
                Selection::collapsed(10),
            ],
            0,
        );
        let cycled = set.cycle_primary(1);
        assert_eq!(cycled.primary().head, 5);
        let cycled2 = cycled.cycle_primary(1);
        assert_eq!(cycled2.primary().head, 10);
    }

    #[test]
    fn cycle_primary_forward_wraps() {
        let set = SelectionSet::from_vec(
            vec![
                Selection::collapsed(0),
                Selection::collapsed(5),
                Selection::collapsed(10),
            ],
            2,
        );
        let cycled = set.cycle_primary(1);
        assert_eq!(cycled.primary().head, 0); // wraps back to start
    }

    #[test]
    fn cycle_primary_backward() {
        let set = SelectionSet::from_vec(
            vec![
                Selection::collapsed(0),
                Selection::collapsed(5),
                Selection::collapsed(10),
            ],
            2,
        );
        let cycled = set.cycle_primary(-1);
        assert_eq!(cycled.primary().head, 5);
    }

    #[test]
    fn cycle_primary_backward_wraps() {
        let set = SelectionSet::from_vec(
            vec![
                Selection::collapsed(0),
                Selection::collapsed(5),
                Selection::collapsed(10),
            ],
            0,
        );
        let cycled = set.cycle_primary(-1);
        assert_eq!(cycled.primary().head, 10); // wraps to end
    }

    #[test]
    fn cycle_primary_single_is_noop() {
        let set = SelectionSet::single(Selection::collapsed(5));
        let cycled = set.clone().cycle_primary(1);
        assert_eq!(cycled, set);
    }

    #[test]
    fn map_overlapping_ranges() {
        // Two non-overlapping selections that a motion causes to overlap.
        let set = SelectionSet::from_vec(vec![Selection::new(0, 3), Selection::new(5, 8)], 0);
        // map both to the same range — merge fires automatically.
        let merged = set.map(|_| Selection::new(2, 5));
        assert_eq!(merged.len(), 1);
        assert_eq!(merged.primary().start(), 2);
        assert_eq!(merged.primary().end(), 5);
    }

    // ── SelectionSet::from_vec panics ─────────────────────────────────────────

    #[test]
    #[should_panic(expected = "SelectionSet must not be empty")]
    fn from_vec_empty_panics() {
        let _ = SelectionSet::from_vec(vec![], 0);
    }

    #[test]
    #[should_panic(expected = "primary index out of bounds")]
    fn from_vec_primary_out_of_bounds_panics() {
        let _ = SelectionSet::from_vec(vec![Selection::collapsed(0)], 1);
    }

    // ── iter_sorted ───────────────────────────────────────────────────────────

    #[test]
    fn iter_sorted_yields_ascending_order() {
        let set = SelectionSet::from_vec(
            vec![
                Selection::collapsed(0),
                Selection::collapsed(5),
                Selection::collapsed(10),
            ],
            2, // primary is last
        );
        let starts: Vec<usize> = set.iter_sorted().map(|s| s.start()).collect();
        assert_eq!(starts, vec![0, 5, 10]);
    }

    // ── SelectionSet::validate ────────────────────────────────────────────────

    #[test]
    fn validate_ok_for_valid_set() {
        let set = SelectionSet::from_vec(vec![Selection::collapsed(0), Selection::collapsed(3)], 0);
        assert!(set.validate(10).is_ok());
    }

    #[test]
    fn validate_err_when_buffer_is_empty() {
        let set = SelectionSet::single(Selection::collapsed(0));
        assert!(matches!(
            set.validate(0),
            Err(crate::error::ValidationError::EmptyBuffer)
        ));
    }

    #[test]
    fn validate_err_when_head_out_of_bounds() {
        // buf_len = 3, head = 5 → out of bounds
        let set = SelectionSet::single(Selection::collapsed(5));
        assert!(matches!(
            set.validate(3),
            Err(crate::error::ValidationError::SelectionOutOfBounds { field: "head", .. })
        ));
    }

    #[test]
    fn validate_err_when_anchor_out_of_bounds() {
        // anchor = 10, head = 1; buf_len = 5 → anchor out of bounds
        let set = SelectionSet::single(Selection::new(10, 1));
        assert!(matches!(
            set.validate(5),
            Err(crate::error::ValidationError::SelectionOutOfBounds {
                field: "anchor",
                ..
            })
        ));
    }

    #[test]
    fn validate_passes_when_head_is_last_valid_char() {
        // head = buf_len - 1 is the largest valid position
        let set = SelectionSet::single(Selection::collapsed(4));
        assert!(set.validate(5).is_ok());
    }
}
