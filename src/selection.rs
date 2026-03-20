use crate::error::ValidationError;

/// A single selection range within a buffer.
///
/// Both `anchor` and `head` are **char offsets** — indices into the buffer's
/// sequence of Unicode scalar values. The cursor (the moving end that the user
/// sees blinking) is always at `head`.
///
/// When `anchor == head`, the selection covers a single character — the one at
/// index `head`. This is the smallest possible selection, not a zero-width
/// point. The cursor block sits on that character, matching Helix/Kakoune's
/// inclusive model.
///
/// `head` must always be a valid char index (`< buf.len_chars()`). Since every
/// buffer always ends with a trailing `\n`, there is always at least one
/// character to sit on — even in an "empty" buffer.
///
/// # Directional selections
///
/// - **Forward** (anchor ≤ head): the user extended towards the end of the file.
/// - **Backward** (anchor > head): the user extended towards the start.
///
/// Use `start()` / `end()` when you need the bounds irrespective of direction,
/// and `anchor` / `head` when direction matters (e.g., when extending).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) struct Selection {
    /// The stationary end of the selection. Stays put when the user extends.
    pub anchor: usize,
    /// The moving end / cursor position.
    pub head: usize,
}

impl Selection {
    /// A single-character selection at `pos` (anchor == head == pos).
    pub(crate) fn cursor(pos: usize) -> Self {
        Self { anchor: pos, head: pos }
    }

    /// A directional range from `anchor` to `head`.
    /// Passing `anchor == head` produces a single-character selection.
    pub(crate) fn new(anchor: usize, head: usize) -> Self {
        Self { anchor, head }
    }

    /// Is this a single-character selection (anchor == head)?
    pub(crate) fn is_cursor(&self) -> bool {
        self.anchor == self.head
    }

    /// The smaller of the two offsets — the start of the selected range.
    pub(crate) fn start(&self) -> usize {
        self.anchor.min(self.head)
    }

    /// The larger of the two offsets — the far end of the selected range.
    ///
    /// In the inclusive cursor model this char IS part of the selection (the
    /// cursor or anchor sits on it). This is NOT an exclusive bound.
    pub(crate) fn end(&self) -> usize {
        self.anchor.max(self.head)
    }

    /// Swap anchor and head. A forward selection becomes backward and vice
    /// versa. Useful for `flip selection` commands.
    pub(crate) fn flip(self) -> Self {
        Self { anchor: self.head, head: self.anchor }
    }

    /// Move both anchor and head by `delta` chars (positive = forward).
    ///
    /// Used when an edit *before* this selection shifts all offsets.
    pub(crate) fn shift(self, delta: isize) -> Self {
        let anchor = (self.anchor as isize + delta) as usize;
        let head = (self.head as isize + delta) as usize;
        // Catch underflow early in tests. A wrapped value will be enormous.
        debug_assert!(
            anchor <= isize::MAX as usize,
            "shift underflow on anchor: {} + {delta} wrapped",
            self.anchor
        );
        debug_assert!(
            head <= isize::MAX as usize,
            "shift underflow on head: {} + {delta} wrapped",
            self.head
        );
        Self { anchor, head }
    }
}

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
pub(crate) struct SelectionSet {
    /// The sorted, non-overlapping selections.
    ///
    /// `Vec` is the right choice here: in practice editors have at most dozens
    /// of selections; linear scan and sort are faster than a tree for that
    /// cardinality due to cache locality.
    selections: Vec<Selection>,

    /// Index of the "primary" selection — the one displayed in the status bar
    /// and used for operations that act on a single selection (e.g.,
    /// `cmd_keep_primary_selection`).
    primary: usize,
}

impl SelectionSet {
    /// Create a set with a single selection. This is the normal starting state.
    pub(crate) fn single(sel: Selection) -> Self {
        Self { selections: vec![sel], primary: 0 }
    }

    /// The primary (focused) selection.
    pub(crate) fn primary(&self) -> Selection {
        self.selections[self.primary]
    }

    /// The index of the primary selection within the sorted selections Vec.
    ///
    /// Useful when rebuilding a `SelectionSet` after transforming all selections
    /// and you need to preserve which one is primary.
    pub(crate) fn primary_index(&self) -> usize {
        self.primary
    }

    /// Number of selections.
    pub(crate) fn len(&self) -> usize {
        self.selections.len()
    }

    /// Iterate over all selections, primary first, then others in order.
    ///
    /// Yielding the primary first means callers that care about "the main one"
    /// can `take(1)` without extra logic.
    pub(crate) fn iter(&self) -> impl Iterator<Item = &Selection> {
        // Yield primary first by chaining: [primary] ++ [all others in order].
        // This is an O(n) iterator with no allocation.
        let primary = &self.selections[self.primary];
        let before = &self.selections[..self.primary];
        let after = &self.selections[self.primary + 1..];
        std::iter::once(primary).chain(before.iter()).chain(after.iter())
    }

    /// Iterate over all selections in sorted order (not primary-first).
    pub(crate) fn iter_sorted(&self) -> impl Iterator<Item = &Selection> {
        self.selections.iter()
    }

    /// Apply `f` to every selection and return a new `SelectionSet`.
    ///
    /// The primary index is preserved. The returned set may violate the
    /// non-overlapping invariant if `f` produces overlapping results.
    ///
    /// Use this when you can guarantee that `f` is order-preserving and cannot
    /// produce overlapping selections (e.g. `|s| s.shift(delta)`). If you are
    /// not sure, use [`map_and_merge`](Self::map_and_merge) instead.
    pub(crate) fn map<F>(self, mut f: F) -> Self
    where
        F: FnMut(Selection) -> Selection,
    {
        let primary = self.primary;
        let selections = self.selections.into_iter().map(&mut f).collect();
        Self { selections, primary }
    }

    /// Apply `f` to every selection, then merge any overlapping results.
    ///
    /// This is the safe default for motions and any transform where `f` might
    /// move selections out of order or cause them to overlap (e.g. two cursors
    /// on the same line both moving to end-of-line land on the same position).
    ///
    /// Prefer plain [`map`](Self::map) only when you can prove `f` is
    /// order-preserving and overlap-free — it avoids the O(n log n) sort.
    pub(crate) fn map_and_merge<F>(self, f: F) -> Self
    where
        F: FnMut(Selection) -> Selection,
    {
        self.map(f).merge_overlapping()
    }

    /// Replace the selection at `idx` with `new_sel` and return the updated
    /// set. Panics if `idx >= len()`.
    pub(crate) fn replace(mut self, idx: usize, new_sel: Selection) -> Self {
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
    pub(crate) fn merge_overlapping(mut self) -> Self {
        if self.selections.len() <= 1 {
            return self;
        }

        let primary_before = self.selections[self.primary];

        // Sort by the start position first.
        // `sort_by_key` is stable, so equal-start selections keep their
        // original order — important for picking the primary correctly.
        self.selections.sort_by_key(|s| s.start());

        // In-place compaction using a read/write cursor pattern.
        //
        // Classic technique: `write` marks the last "kept" slot, `read`
        // advances through the rest. When two adjacent entries overlap we
        // merge into `selections[write]`; otherwise we bump `write` and
        // copy the new entry there. At the end, `truncate` drops the
        // leftover tail. This avoids allocating a second Vec — we reuse
        // the memory we already own.
        let mut write = 0;
        let mut new_primary = 0;

        for read in 1..self.selections.len() {
            // Copy `sel` out first — Selection is `Copy` (two `usize`
            // fields), so this is a cheap stack copy, not a heap clone.
            let sel = self.selections[read];

            // Reborrow `self.selections[write]` mutably so we can extend
            // it if there's overlap. Rust's borrow checker is happy
            // because we copied `sel` out above — we're not holding two
            // references into the same slice simultaneously.
            let last = &mut self.selections[write];

            if sel.start() <= last.end() {
                // Overlap or adjacent — extend `last` to cover `sel`.
                // Head comes from whichever selection reaches furthest —
                // this preserves the direction of the "dominant" selection.
                if sel.end() > last.end() {
                    // If `sel` was a backward selection (head < anchor), keep
                    // the backward direction on the merged result.
                    if sel.head <= sel.anchor {
                        last.head = last.start().min(sel.head);
                        last.anchor = sel.end();
                    } else {
                        last.anchor = last.start();
                        last.head = sel.end();
                    }
                }
                // Track where the primary ended up.
                if primary_before.start() >= last.start()
                    && primary_before.end() <= last.end()
                {
                    new_primary = write;
                }
            } else {
                // No overlap — finalize the current write slot, then advance.
                let done = &self.selections[write];
                if done.start() >= primary_before.start()
                    && done.end() <= primary_before.end()
                {
                    new_primary = write;
                }
                write += 1;
                // Move `sel` into the next write slot. Because Selection is
                // Copy, this is a plain assignment — no heap work.
                self.selections[write] = sel;
            }
        }

        // Check the final write slot for primary.
        let done = &self.selections[write];
        if done.start() >= primary_before.start()
            && done.end() <= primary_before.end()
        {
            new_primary = write;
        }

        // Drop everything after `write`. `truncate` adjusts the Vec's
        // length without reallocating — the capacity stays the same.
        self.selections.truncate(write + 1);

        Self { selections: self.selections, primary: new_primary }
    }

    /// Build a `SelectionSet` directly from a non-empty `Vec<Selection>`,
    /// with `primary` pointing at the given index.
    ///
    /// # Panics
    /// Panics if `selections` is empty or `primary >= selections.len()`.
    pub(crate) fn from_vec(selections: Vec<Selection>, primary: usize) -> Self {
        assert!(!selections.is_empty(), "SelectionSet must not be empty");
        assert!(primary < selections.len(), "primary index out of bounds");
        Self { selections, primary }
    }

    // ── Selection-set manipulation ────────────────────────────────────────────

    /// Return a new set containing only the primary selection.
    ///
    /// All other selections are dropped. The primary index resets to 0.
    pub(crate) fn keep_primary(self) -> Self {
        let primary = self.selections[self.primary];
        Self { selections: vec![primary], primary: 0 }
    }

    /// Remove the selection at `idx` and return the updated set.
    ///
    /// If `idx` is the primary, the new primary becomes the next selection
    /// in document order, wrapping around to the first if the removed
    /// selection was the last. If `len() == 1`, returns `self` unchanged — you cannot
    /// remove the only selection. Panics if `idx >= len()`.
    pub(crate) fn remove(mut self, idx: usize) -> Self {
        if self.selections.len() <= 1 {
            return self; // can't remove the only selection — no-op
        }
        assert!(idx < self.selections.len(), "remove index out of bounds");
        self.selections.remove(idx);
        let new_len = self.selections.len();
        self.primary = if idx < self.primary {
            // Removed a selection before the primary — primary shifts down.
            self.primary - 1
        } else if idx == self.primary {
            // Removed the primary itself — advance to the next, wrapping.
            idx % new_len
        } else {
            // Removed a selection after the primary — primary unchanged.
            self.primary
        };
        self
    }

    /// Shift the primary index by `delta`, wrapping around.
    ///
    /// `delta = 1` moves to the next selection (forward), `-1` moves to the
    /// previous (backward). Works correctly for `|delta| >= len()` too.
    pub(crate) fn cycle_primary(mut self, delta: isize) -> Self {
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
    /// Call this at every chokepoint where a `(Buffer, SelectionSet)` pair is
    /// produced: edit operations, motions, and `Transaction::apply`.
    #[inline]
    pub(crate) fn debug_assert_valid(&self, buf_len: usize) {
        debug_assert!(buf_len > 0, "Buffer must have at least 1 char (the structural \\n)");
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
    pub(crate) fn validate(&self, buf_len: usize) -> Result<(), ValidationError> {
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
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    // ── Selection ─────────────────────────────────────────────────────────────

    #[test]
    fn cursor_is_collapsed() {
        let s = Selection::cursor(5);
        assert_eq!(s.anchor, 5);
        assert_eq!(s.head, 5);
        assert!(s.is_cursor());
    }

    #[test]
    fn forward_selection_start_end() {
        let s = Selection::new(2, 7); // anchor < head → forward
        assert_eq!(s.start(), 2);
        assert_eq!(s.end(), 7);
        assert!(!s.is_cursor());
    }

    #[test]
    fn backward_selection_start_end() {
        let s = Selection::new(7, 2); // anchor > head → backward
        assert_eq!(s.start(), 2);
        assert_eq!(s.end(), 7);
    }

    #[test]
    fn flip_reverses_direction() {
        let fwd = Selection::new(2, 7);
        let bwd = fwd.flip();
        assert_eq!(bwd.anchor, 7);
        assert_eq!(bwd.head, 2);
        assert_eq!(fwd.flip().flip(), fwd); // double-flip is identity
    }

    #[test]
    fn shift_positive() {
        let s = Selection::new(2, 7);
        let shifted = s.shift(3);
        assert_eq!(shifted.anchor, 5);
        assert_eq!(shifted.head, 10);
    }

    #[test]
    fn shift_negative() {
        let s = Selection::new(5, 10);
        let shifted = s.shift(-3);
        assert_eq!(shifted.anchor, 2);
        assert_eq!(shifted.head, 7);
    }

    // ── SelectionSet ──────────────────────────────────────────────────────────

    #[test]
    fn single_selection_is_primary() {
        let s = Selection::cursor(0);
        let set = SelectionSet::single(s);
        assert_eq!(set.primary(), s);
        assert_eq!(set.len(), 1);
    }

    #[test]
    fn merge_no_overlap() {
        // Two disjoint selections — should stay separate.
        let set = SelectionSet::from_vec(
            vec![Selection::new(0, 3), Selection::new(5, 8)],
            0,
        )
        .merge_overlapping();
        assert_eq!(set.len(), 2);
    }

    #[test]
    fn merge_overlapping_selections() {
        // (anchor=0,head=5) and (anchor=3,head=8) overlap — should merge.
        let set = SelectionSet::from_vec(
            vec![Selection::new(0, 5), Selection::new(3, 8)],
            0,
        )
        .merge_overlapping();
        assert_eq!(set.len(), 1);
        assert_eq!(set.primary().start(), 0);
        assert_eq!(set.primary().end(), 8);
    }

    #[test]
    fn merge_adjacent_selections() {
        // (anchor=0,head=3) and (anchor=3,head=6) touch at offset 3 — should merge.
        let set = SelectionSet::from_vec(
            vec![Selection::new(0, 3), Selection::new(3, 6)],
            0,
        )
        .merge_overlapping();
        assert_eq!(set.len(), 1);
        assert_eq!(set.primary().start(), 0);
        assert_eq!(set.primary().end(), 6);
    }

    #[test]
    fn merge_duplicate_selections() {
        let set = SelectionSet::from_vec(
            vec![Selection::new(2, 5), Selection::new(2, 5)],
            0,
        )
        .merge_overlapping();
        assert_eq!(set.len(), 1);
    }

    #[test]
    fn merge_contained_selection() {
        // (anchor=0,head=8) fully contains (anchor=2,head=5) — should merge.
        let set = SelectionSet::from_vec(
            vec![Selection::new(0, 8), Selection::new(2, 5)],
            0,
        )
        .merge_overlapping();
        assert_eq!(set.len(), 1);
        assert_eq!(set.primary().end(), 8);
    }

    #[test]
    fn merge_idempotent() {
        let set = SelectionSet::from_vec(
            vec![Selection::new(0, 5), Selection::new(3, 8)],
            0,
        )
        .merge_overlapping();
        let set2 = set.clone().merge_overlapping();
        assert_eq!(set, set2);
    }

    #[test]
    fn merge_three_into_one() {
        let set = SelectionSet::from_vec(
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
        let set = SelectionSet::from_vec(
            vec![Selection::new(8, 3), Selection::new(10, 5)],
            0,
        )
        .merge_overlapping();
        assert_eq!(set.len(), 1);
        let s = set.primary();
        assert_eq!(s.start(), 3);
        assert_eq!(s.end(), 10);
        // Merged result should be backward (head < anchor).
        assert!(s.head < s.anchor, "merged backward selections should stay backward");
    }

    #[test]
    fn merge_sorts_unsorted_input() {
        // Pass selections out of order — merge should sort them first.
        let set = SelectionSet::from_vec(
            vec![Selection::new(5, 8), Selection::new(0, 3)],
            0,
        )
        .merge_overlapping();
        assert_eq!(set.len(), 2);
        assert_eq!(set.selections[0].start(), 0);
        assert_eq!(set.selections[1].start(), 5);
    }

    #[test]
    fn map_preserves_primary() {
        let set = SelectionSet::from_vec(
            vec![Selection::cursor(0), Selection::cursor(5)],
            1, // primary is the second one
        );
        let shifted = set.map(|s| s.shift(1));
        assert_eq!(shifted.primary().head, 6); // was 5, shifted by 1
    }

    #[test]
    fn replace_updates_selection() {
        let set = SelectionSet::from_vec(
            vec![Selection::cursor(0), Selection::cursor(5)],
            0,
        );
        let updated = set.replace(1, Selection::cursor(10));
        assert_eq!(updated.selections[1].head, 10);
    }

    // ── map_and_merge ────────────────────────────────────────────────────────

    #[test]
    fn map_and_merge_collapses_to_same_position() {
        // Two cursors at different positions that a motion maps to the same
        // spot — e.g. "go to end of line" when both are on the same line.
        let set = SelectionSet::from_vec(
            vec![Selection::cursor(2), Selection::cursor(7)],
            0,
        );
        let merged = set.map_and_merge(|_| Selection::cursor(10));
        assert_eq!(merged.len(), 1);
        assert_eq!(merged.primary().head, 10);
    }

    #[test]
    fn map_and_merge_reorders_reversed_positions() {
        // A motion that reverses the order: cursor at 2 maps to 8, cursor
        // at 7 maps to 1. After merge the result should be sorted [1, 8].
        let set = SelectionSet::from_vec(
            vec![Selection::cursor(2), Selection::cursor(7)],
            1, // primary is the second one (at 7)
        );
        let merged = set.map_and_merge(|s| {
            if s.head == 2 {
                Selection::cursor(8)
            } else {
                Selection::cursor(1)
            }
        });
        assert_eq!(merged.len(), 2);
        // Sorted by position: first at 1, second at 8.
        assert_eq!(merged.selections[0].head, 1);
        assert_eq!(merged.selections[1].head, 8);
        // Primary was the cursor at 7 → mapped to 1 → now at index 0.
        assert_eq!(merged.primary().head, 1);
    }

    // ── keep_primary ─────────────────────────────────────────────────────────

    #[test]
    fn keep_primary_drops_others() {
        let set = SelectionSet::from_vec(
            vec![Selection::cursor(0), Selection::cursor(5), Selection::cursor(10)],
            1, // primary is the middle one
        );
        let kept = set.keep_primary();
        assert_eq!(kept.len(), 1);
        assert_eq!(kept.primary().head, 5);
        assert_eq!(kept.primary_index(), 0);
    }

    #[test]
    fn keep_primary_single_is_noop() {
        let set = SelectionSet::single(Selection::cursor(3));
        let kept = set.clone().keep_primary();
        assert_eq!(kept, set);
    }

    // ── remove ───────────────────────────────────────────────────────────────

    #[test]
    fn remove_before_primary_shifts_primary_down() {
        let set = SelectionSet::from_vec(
            vec![Selection::cursor(0), Selection::cursor(5), Selection::cursor(10)],
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
            vec![Selection::cursor(0), Selection::cursor(5), Selection::cursor(10)],
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
            vec![Selection::cursor(0), Selection::cursor(5), Selection::cursor(10)],
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
            vec![Selection::cursor(0), Selection::cursor(5), Selection::cursor(10)],
            0, // primary is the first one
        );
        let result = set.remove(2); // remove last
        assert_eq!(result.len(), 2);
        assert_eq!(result.primary().head, 0);
        assert_eq!(result.primary_index(), 0);
    }

    #[test]
    fn remove_single_is_noop() {
        let set = SelectionSet::single(Selection::cursor(0));
        let result = set.clone().remove(0);
        assert_eq!(result, set); // unchanged — can't remove the only selection
    }

    // ── cycle_primary ─────────────────────────────────────────────────────────

    #[test]
    fn cycle_primary_forward() {
        let set = SelectionSet::from_vec(
            vec![Selection::cursor(0), Selection::cursor(5), Selection::cursor(10)],
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
            vec![Selection::cursor(0), Selection::cursor(5), Selection::cursor(10)],
            2,
        );
        let cycled = set.cycle_primary(1);
        assert_eq!(cycled.primary().head, 0); // wraps back to start
    }

    #[test]
    fn cycle_primary_backward() {
        let set = SelectionSet::from_vec(
            vec![Selection::cursor(0), Selection::cursor(5), Selection::cursor(10)],
            2,
        );
        let cycled = set.cycle_primary(-1);
        assert_eq!(cycled.primary().head, 5);
    }

    #[test]
    fn cycle_primary_backward_wraps() {
        let set = SelectionSet::from_vec(
            vec![Selection::cursor(0), Selection::cursor(5), Selection::cursor(10)],
            0,
        );
        let cycled = set.cycle_primary(-1);
        assert_eq!(cycled.primary().head, 10); // wraps to end
    }

    #[test]
    fn cycle_primary_single_is_noop() {
        let set = SelectionSet::single(Selection::cursor(5));
        let cycled = set.clone().cycle_primary(1);
        assert_eq!(cycled, set);
    }


    #[test]
    fn map_and_merge_overlapping_ranges() {
        // Two non-overlapping selections that a motion causes to overlap.
        let set = SelectionSet::from_vec(
            vec![Selection::new(0, 3), Selection::new(5, 8)],
            0,
        );
        // Shift both left by 3 — first becomes (0,0) clamped, second (2,5).
        // In practice the first wraps, so let's do a simpler overlap:
        // map both to the same range.
        let merged = set.map_and_merge(|_| Selection::new(2, 5));
        assert_eq!(merged.len(), 1);
        assert_eq!(merged.primary().start(), 2);
        assert_eq!(merged.primary().end(), 5);
    }
}
