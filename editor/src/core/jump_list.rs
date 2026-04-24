//! Jump list — a navigable history of cursor positions before large movements.
//!
//! Records the cursor position (as a full [`SelectionSet`]) before "jump"
//! commands like `goto-first-line`, `goto-last-line`, `search-next`,
//! `search-prev`, page scroll, and any motion that crosses more than
//! `EditorSettings::jump_line_threshold` lines. The user navigates the
//! history with `jump-backward` and `jump-forward`.
//!
//! Internally this is a [`VecDeque<JumpEntry>`] with a cursor index, capped
//! at `EditorSettings::jump_list_capacity`. When the user navigates backward
//! and then makes a new jump, forward history is truncated — matching
//! Vim/Helix semantics.

use std::collections::VecDeque;

use engine::pipeline::BufferId;

use crate::core::selection::{Selection, SelectionSet};
use crate::core::text::Text;

/// Default capacity — kept here so tests can construct jump lists without
/// importing `EditorSettings`.
#[cfg(test)]
pub(crate) const DEFAULT_JUMP_LIST_CAPACITY: usize = 100;

/// A single saved cursor position in the jump list.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct JumpEntry {
    /// Buffer this position belongs to — needed for cross-buffer Ctrl+O/I.
    pub buffer_id: BufferId,
    /// Full selection state at the moment of the jump.
    pub selections: SelectionSet,
    /// Line number of the primary selection's head — cached for O(1) dedup.
    pub primary_line: usize,
}

impl JumpEntry {
    /// Build a jump entry from the current selection state, deriving
    /// `primary_line` from the buffer so callers don't have to.
    pub(crate) fn new(selections: SelectionSet, buf: &Text, buffer_id: BufferId) -> Self {
        let primary_line = buf.char_to_line(selections.primary().head);
        Self {
            buffer_id,
            selections,
            primary_line,
        }
    }

    /// Build a jump entry from a pre-motion snapshot.
    ///
    /// Used at call sites that capture the cursor *before* a motion runs, so
    /// `primary_line` is already known and no buffer reference is needed.
    pub(crate) fn from_pre_motion(
        pre_primary: Selection,
        primary_line: usize,
        buffer_id: BufferId,
    ) -> Self {
        Self {
            buffer_id,
            selections: SelectionSet::single(pre_primary),
            primary_line,
        }
    }
}

/// Navigable history of cursor positions before large movements.
///
/// `cursor` indexes into `entries`. When `cursor == entries.len()`, the user
/// is "at the present" — no backward navigation is active. Navigating backward
/// decrements cursor; navigating forward increments it. A new `push` truncates
/// any forward history (entries after cursor) before appending.
#[derive(Debug)]
pub(crate) struct JumpList {
    entries: VecDeque<JumpEntry>,
    /// Current position. `cursor == entries.len()` means "at the present".
    cursor: usize,
    /// Maximum number of entries. Oldest entry is dropped when exceeded.
    capacity: usize,
}

impl JumpList {
    /// Create a new jump list with the given capacity limit.
    pub(crate) fn new(capacity: usize) -> Self {
        Self {
            entries: VecDeque::new(),
            cursor: 0,
            capacity,
        }
    }

    /// Record a jump. Truncates forward history, deduplicates against the last
    /// entry by line number, and caps the list at `self.capacity`.
    pub(crate) fn push(&mut self, entry: JumpEntry) {
        self.entries.truncate(self.cursor);

        // Deduplicate: same line AND same buffer — cross-buffer same-line entries are distinct.
        if let Some(last) = self
            .entries
            .back_mut()
            .filter(|l| l.primary_line == entry.primary_line && l.buffer_id == entry.buffer_id)
        {
            *last = entry;
            return;
        }

        self.entries.push_back(entry);

        if self.entries.len() > self.capacity {
            self.entries.pop_front();
        }

        self.cursor = self.entries.len();
    }

    /// Remove all entries for `id`. Adjusts the cursor so its relative position
    /// in the remaining entries is preserved; clamps to `entries.len()` if the
    /// cursor falls past the end (which means "at the present").
    pub(crate) fn prune_buffer(&mut self, id: BufferId) {
        let removed_before = self
            .entries
            .iter()
            .take(self.cursor)
            .filter(|e| e.buffer_id == id)
            .count();
        self.entries.retain(|e| e.buffer_id != id);
        self.cursor = self
            .cursor
            .saturating_sub(removed_before)
            .min(self.entries.len());
    }

    /// Navigate backward. If at the present, saves `current` first so that
    /// `forward()` can return to it. Returns the entry to restore, or `None`
    /// if the list is empty / already at the oldest entry.
    pub(crate) fn backward(&mut self, current: JumpEntry) -> Option<&JumpEntry> {
        if self.entries.is_empty() {
            return None;
        }

        // At the present: always save the current position so `jump-forward`
        // can return to it. No dedup here — unlike `push()`, the "save current"
        // path must preserve the exact return point even if it's on the same
        // line as the last recorded jump (e.g., two search matches on one line).
        if self.cursor == self.entries.len() {
            self.entries.push_back(current);
            self.cursor = self.entries.len() - 1;
        }

        if self.cursor == 0 {
            return None;
        }

        self.cursor -= 1;
        Some(&self.entries[self.cursor])
    }

    /// Navigate forward. Returns the next entry, or `None` if already at the
    /// present.
    pub(crate) fn forward(&mut self) -> Option<&JumpEntry> {
        if self.cursor + 1 >= self.entries.len() {
            return None;
        }
        self.cursor += 1;
        Some(&self.entries[self.cursor])
    }

    #[cfg(test)]
    fn len(&self) -> usize {
        self.entries.len()
    }

    /// Returns `true` if any entry in the list belongs to `id`.
    #[cfg(test)]
    pub(crate) fn entries_for_buffer(&self, id: BufferId) -> bool {
        self.entries.iter().any(|e| e.buffer_id == id)
    }
}

// ── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::selection::{Selection, SelectionSet};

    /// Helper: build a JumpEntry with a cursor at `char_pos` on `line`.
    /// Bypasses `JumpEntry::new` since unit tests don't have a Text.
    fn entry(char_pos: usize, line: usize) -> JumpEntry {
        JumpEntry {
            buffer_id: engine::pipeline::BufferId::default(),
            selections: SelectionSet::single(Selection::collapsed(char_pos)),
            primary_line: line,
        }
    }

    #[test]
    fn push_and_backward() {
        let mut jl = JumpList::new(DEFAULT_JUMP_LIST_CAPACITY);
        jl.push(entry(0, 0));
        jl.push(entry(10, 5));
        jl.push(entry(20, 10));

        let current = entry(30, 15);
        let e = jl.backward(current).unwrap();
        assert_eq!(e.primary_line, 10);

        let e = jl.backward(entry(0, 0)).unwrap();
        assert_eq!(e.primary_line, 5);

        let e = jl.backward(entry(0, 0)).unwrap();
        assert_eq!(e.primary_line, 0);

        assert!(jl.backward(entry(0, 0)).is_none());
    }

    #[test]
    fn forward_after_backward() {
        let mut jl = JumpList::new(DEFAULT_JUMP_LIST_CAPACITY);
        jl.push(entry(0, 0));
        jl.push(entry(10, 5));
        jl.push(entry(20, 10));

        let current = entry(30, 15);
        jl.backward(current).unwrap();
        jl.backward(entry(0, 0)).unwrap();

        let e = jl.forward().unwrap();
        assert_eq!(e.primary_line, 10);

        let e = jl.forward().unwrap();
        assert_eq!(e.primary_line, 15);

        assert!(jl.forward().is_none());
    }

    #[test]
    fn truncation_on_new_push() {
        let mut jl = JumpList::new(DEFAULT_JUMP_LIST_CAPACITY);
        jl.push(entry(0, 0));
        jl.push(entry(10, 5));
        jl.push(entry(20, 10));

        jl.backward(entry(30, 15)).unwrap();
        jl.backward(entry(0, 0)).unwrap();

        // New jump from here — forward history (lines 10, 15) is discarded.
        jl.push(entry(50, 25));

        assert!(jl.forward().is_none());

        let e = jl.backward(entry(60, 30)).unwrap();
        assert_eq!(e.primary_line, 25);

        let e = jl.backward(entry(0, 0)).unwrap();
        assert_eq!(e.primary_line, 0);

        assert!(jl.backward(entry(0, 0)).is_none());
    }

    #[test]
    fn capacity_cap() {
        const CAP: usize = DEFAULT_JUMP_LIST_CAPACITY;
        let mut jl = JumpList::new(CAP);
        for i in 0..=CAP {
            jl.push(entry(i * 10, i));
        }
        assert_eq!(jl.len(), CAP);

        let e = jl.backward(entry(9999, 9999)).unwrap();
        assert_eq!(e.primary_line, CAP);

        let mut oldest = e.primary_line;
        while let Some(e) = jl.backward(entry(0, 0)) {
            oldest = e.primary_line;
        }
        assert_eq!(oldest, 1);
    }

    #[test]
    fn deduplication() {
        let mut jl = JumpList::new(DEFAULT_JUMP_LIST_CAPACITY);
        jl.push(entry(0, 5));
        jl.push(entry(3, 5)); // same line — replaces
        assert_eq!(jl.len(), 1);

        jl.push(entry(20, 10));
        let e = jl.backward(entry(99, 99)).unwrap();
        assert_eq!(e.primary_line, 10);
        let e = jl.backward(entry(0, 0)).unwrap();
        assert_eq!(e.primary_line, 5);
        assert_eq!(e.selections.primary().head, 3);
    }

    #[test]
    fn empty_list() {
        let mut jl = JumpList::new(DEFAULT_JUMP_LIST_CAPACITY);
        assert!(jl.backward(entry(0, 0)).is_none());
        assert!(jl.forward().is_none());
    }

    #[test]
    fn backward_after_returning_to_present() {
        let mut jl = JumpList::new(DEFAULT_JUMP_LIST_CAPACITY);
        jl.push(entry(0, 0));

        // Go backward, then forward back to the saved "present" entry.
        jl.backward(entry(50, 10)).unwrap();
        jl.forward().unwrap();

        // Now backward again. Since cursor is at the last entry (the saved
        // "present"), not past it, the new current position is NOT saved —
        // matching Vim/Helix: the present is only captured when first entering
        // the jump list from a fresh editing state.
        let e = jl.backward(entry(80, 20)).unwrap();
        assert_eq!(
            e.primary_line, 0,
            "traverses existing history without saving new position"
        );

        // Forward returns to the previously saved "present" (line 10).
        let e = jl.forward().unwrap();
        assert_eq!(e.primary_line, 10);
        assert!(jl.forward().is_none());
    }

    #[test]
    fn backward_saves_current_position() {
        let mut jl = JumpList::new(DEFAULT_JUMP_LIST_CAPACITY);
        jl.push(entry(0, 0));

        let e = jl.backward(entry(50, 10)).unwrap();
        assert_eq!(e.primary_line, 0);

        let e = jl.forward().unwrap();
        assert_eq!(e.primary_line, 10);
    }

    // ── prune_buffer cursor-adjustment arithmetic ─────────────────────────────

    /// Helper to create a JumpEntry for a specific BufferId (for prune tests).
    fn entry_for(char_pos: usize, line: usize, bid: BufferId) -> JumpEntry {
        JumpEntry {
            buffer_id: bid,
            selections: SelectionSet::single(Selection::collapsed(char_pos)),
            primary_line: line,
        }
    }

    /// Helper: allocate two distinct real BufferIds via a temporary SlotMap.
    fn two_buffer_ids() -> (BufferId, BufferId) {
        let mut sm: slotmap::SlotMap<BufferId, ()> = slotmap::SlotMap::with_key();
        let a = sm.insert(());
        let b = sm.insert(());
        (a, b)
    }

    /// Cursor decrements by the number of pruned entries that were before it.
    #[test]
    fn prune_buffer_decrements_cursor_by_removed_before_count() {
        let (bid_a, bid_b) = two_buffer_ids();
        let mut jl = JumpList::new(DEFAULT_JUMP_LIST_CAPACITY);
        // [A:0, B:1, A:2, B:3]  cursor=4 (at present)
        jl.push(entry_for(0, 0, bid_a));
        jl.push(entry_for(1, 1, bid_b));
        jl.push(entry_for(2, 2, bid_a));
        jl.push(entry_for(3, 3, bid_b));

        // Prune A: removes indices 0 and 2 (both before cursor=4).
        // remaining = [B:1, B:3], cursor = 4 − 2 = 2 (at present of 2-entry list).
        jl.prune_buffer(bid_a);

        assert_eq!(jl.len(), 2);
        assert_eq!(
            jl.cursor, 2,
            "cursor clamped to end after removing 2 entries before it"
        );
        assert!(!jl.entries_for_buffer(bid_a));
    }

    /// When the cursor points mid-list and only entries AFTER it are pruned,
    /// the cursor position is unchanged.
    #[test]
    fn prune_buffer_leaves_cursor_unchanged_when_removed_entries_are_after() {
        let (bid_a, bid_b) = two_buffer_ids();
        let mut jl = JumpList::new(DEFAULT_JUMP_LIST_CAPACITY);
        // [B:0, B:1, A:2, A:3]  cursor=2 (mid-list, pointing at A:2)
        jl.push(entry_for(0, 0, bid_b));
        jl.push(entry_for(1, 1, bid_b));
        jl.push(entry_for(2, 2, bid_a));
        jl.push(entry_for(3, 3, bid_a));
        jl.cursor = 2; // position mid-list manually

        // Prune A: removes indices 2 and 3 (both at/after cursor=2, so 0 are before).
        // remaining = [B:0, B:1], cursor = 2 − 0 = 2, then clamped to min(2, 2) = 2.
        jl.prune_buffer(bid_a);

        assert_eq!(jl.len(), 2);
        assert_eq!(jl.cursor, 2, "cursor clamped to list len (= at present)");
    }

    /// `saturating_sub` prevents underflow: removing all entries before cursor=0 is a no-op.
    #[test]
    fn prune_buffer_saturating_sub_at_zero_cursor() {
        let (bid_a, bid_b) = two_buffer_ids();
        let mut jl = JumpList::new(DEFAULT_JUMP_LIST_CAPACITY);
        jl.push(entry_for(0, 0, bid_b));
        jl.push(entry_for(1, 1, bid_a));
        jl.cursor = 0; // at oldest entry

        // Prune A: only the entry at index 1 is removed; 0 were before cursor=0.
        jl.prune_buffer(bid_a);

        assert_eq!(jl.len(), 1);
        assert_eq!(
            jl.cursor, 0,
            "cursor stays at 0 — saturating_sub prevents underflow"
        );
    }

    /// When all entries belong to the pruned buffer, list and cursor both become 0.
    #[test]
    fn prune_buffer_all_entries_removed_resets_cursor() {
        let (bid_a, _bid_b) = two_buffer_ids();
        let mut jl = JumpList::new(DEFAULT_JUMP_LIST_CAPACITY);
        jl.push(entry_for(0, 0, bid_a));
        jl.push(entry_for(1, 1, bid_a));
        jl.push(entry_for(2, 2, bid_a));
        // cursor = 3 (at present)

        jl.prune_buffer(bid_a);

        assert_eq!(jl.len(), 0);
        assert_eq!(jl.cursor, 0, "cursor = 0 when all entries removed");
    }

    /// After pruning, backward/forward still work correctly on the remaining entries.
    #[test]
    fn prune_buffer_remaining_entries_navigable() {
        let (bid_a, bid_b) = two_buffer_ids();
        let mut jl = JumpList::new(DEFAULT_JUMP_LIST_CAPACITY);
        jl.push(entry_for(0, 0, bid_b));
        jl.push(entry_for(1, 1, bid_a));
        jl.push(entry_for(2, 2, bid_b));
        // cursor = 3

        jl.prune_buffer(bid_a);
        // remaining = [B:0, B:2], cursor = 2 (3 − 1 removed before = 2, clamped to min(2,2))

        let e = jl.backward(entry_for(99, 99, bid_b)).unwrap();
        assert_eq!(
            e.primary_line, 2,
            "backward from present lands on last remaining entry"
        );
        let e = jl.backward(entry_for(0, 0, bid_b)).unwrap();
        assert_eq!(e.primary_line, 0, "backward again reaches the oldest entry");
    }
}
