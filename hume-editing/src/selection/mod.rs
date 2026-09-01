mod single;
#[cfg(test)]
pub mod testing;

pub use single::{
    DisplayColOrigin, Selection, StickyDisplayCol, is_selection_linewise, linewise_classification,
};

use crate::changeset::{Assoc, ChangeSet, PosMapCursor};
use crate::error::ValidationError;
use crate::text::BufferText;

/// The complete selection state for one buffer.
///
/// # Invariants
/// 1. Never empty — always at least one `Selection`.
/// 2. Selections are sorted in ascending order of `start()`.
/// 3. No two selections overlap. Adjacent selections (where one ends exactly
///    where the next begins) are merged.
///
/// Invariants 2 and 3 are enforced by [`SelectionSet::merge_overlapping_in_place`],
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
    /// set, canonicalized (sorted, overlapping/adjacent selections merged) so
    /// the `SelectionSet` invariants always hold. Panics if `idx >= len()`.
    pub fn replace(mut self, idx: usize, new_sel: Selection) -> Self {
        self.selections[idx] = new_sel;
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
        assert!(idx < self.selections.len(), "remove index out of bounds");
        if self.selections.len() <= 1 {
            return self; // can't remove the only selection — no-op
        }
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
    /// Call this at every chokepoint where a `(BufferText, SelectionSet)` pair is
    /// produced: edit operations, motions, and `Transaction::apply`.
    #[inline]
    pub fn debug_assert_valid(&self, text: &BufferText) {
        let buf_len = text.len_chars();
        debug_assert!(
            buf_len > 0,
            "BufferText must have at least 1 char (the structural \\n)"
        );
        debug_assert!(
            text.char_at(buf_len - 1) == Some('\n'),
            "BufferText must end with structural '\\n', but last char is {:?}",
            text.char_at(buf_len - 1),
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
    /// Merged selections get `sticky_display_col: None` regardless of their pre-merge values
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
                        // sel is backward: the merged head goes to the union's
                        // start, which is last.start() (sorted by start, so
                        // last.start() <= sel.start() == sel.head).
                        last.head = last.start();
                        last.anchor = sel.end();
                    } else {
                        last.anchor = last.start();
                        last.head = sel.end();
                    }
                    // Merged — reset sticky_display_col since neither side's column is valid.
                    last.sticky_display_col = None;
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
    /// - Maps `anchor` and `head` through the changeset.
    /// - Resets `sticky_display_col` to `None` if the edit touched the head's
    ///   pre-edit line (the display column is stale when the line's content
    ///   changed).
    /// - After all selections are mapped, calls `merge_overlapping_in_place` so
    ///   the no-overlap invariant is restored (a deletion spanning multiple
    ///   selections can collapse them).
    ///
    /// `text_pre` must be the buffer text **before** the edit — the pre-edit line
    /// map is needed to identify which line each head resided on before mapping.
    ///
    /// Runs in O(selections + ops) rather than O(selections × ops): selections
    /// are sorted and non-overlapping, so both the line-touch check and the
    /// position mapping walk their respective changeset data with a single
    /// forward-only cursor shared across all selections, instead of
    /// re-scanning the whole changeset per selection.
    ///
    /// Thin wrapper over [`Self::translate_in_place_with`] for a caller
    /// translating a single `SelectionSet` — see that method for a caller
    /// translating many.
    pub fn translate_in_place(&mut self, cs: &ChangeSet, text_pre: &BufferText) {
        self.translate_in_place_with(&cs.edited_old_ranges(), cs, text_pre);
    }

    /// Same as [`Self::translate_in_place`], but takes `cs`'s edited ranges
    /// precomputed by the caller — for translating many independent
    /// `SelectionSet`s through the same `ChangeSet` (e.g. one jump-list entry
    /// per pane), so [`ChangeSet::edited_old_ranges`]'s `Vec` build is paid
    /// once rather than once per `SelectionSet`. `edits` must be
    /// `cs.edited_old_ranges()` — passing ranges from a different changeset
    /// silently mis-maps every selection.
    pub fn translate_in_place_with(
        &mut self,
        edits: &[(usize, usize)],
        cs: &ChangeSet,
        text_pre: &BufferText,
    ) {
        let mut edit_idx = 0usize;
        let mut mapper = PosMapCursor::new(cs.ops());

        for sel in &mut self.selections {
            let pre_line = text_pre.char_to_line(sel.head);
            let line_start = text_pre.line_to_char(pre_line);
            let line_end = crate::lines::line_end_exclusive(text_pre, pre_line);

            // Drop edits that end entirely before this line — heads (and thus
            // pre-edit lines) strictly increase across selections in a sorted,
            // non-overlapping SelectionSet, so a dropped edit can never touch
            // this or any later selection's line. A point range (Insert) at
            // exactly `line_start` still counts as touching, so it uses a
            // strict `<` rather than `<=`.
            while edit_idx < edits.len() {
                let (start, end) = edits[edit_idx];
                let fully_before = if start == end {
                    end < line_start
                } else {
                    end <= line_start
                };
                if fully_before {
                    edit_idx += 1;
                } else {
                    break;
                }
            }
            // The first remaining edit (if any) touches this line iff it
            // starts before `line_end` — anything surviving the skip above
            // already ends at or after `line_start`, so `start < line_end`
            // alone implies overlap (proof: for a range, that's exactly the
            // half-open overlap test; for a point, `start == end` already
            // means `line_start <= start` from the skip, so `start < line_end`
            // gives `line_start <= start < line_end`).
            if edit_idx < edits.len() && edits[edit_idx].0 < line_end {
                sel.sticky_display_col = None;
            }

            let forward = sel.anchor <= sel.head;
            let lo = mapper.map(sel.start(), Assoc::After);
            let hi = mapper.map(sel.end(), Assoc::After);
            if forward {
                sel.anchor = lo;
                sel.head = hi;
            } else {
                sel.anchor = hi;
                sel.head = lo;
            }
        }
        if self.selections.len() > 1 {
            self.merge_overlapping_in_place();
        }
    }
}

// ── Tests ──────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests;
