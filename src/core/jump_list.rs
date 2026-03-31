//! Jump list — a navigable history of cursor positions before large movements.
//!
//! Records the cursor position (as a full [`SelectionSet`]) before "jump"
//! commands like `goto-first-line`, `goto-last-line`, `search-next`,
//! `search-prev`, page scroll, and any motion that crosses
//! more than [`JUMP_LINE_THRESHOLD`] lines. The user navigates the history with
//! `jump-backward` and `jump-forward`.
//!
//! Internally this is a `Vec<JumpEntry>` with a cursor index, capped at
//! [`JUMP_LIST_CAPACITY`]. When the user navigates backward and then makes a
//! new jump, forward history is truncated — matching Vim/Helix semantics.

use crate::core::selection::SelectionSet;

/// Maximum number of entries kept in the jump list.
const JUMP_LIST_CAPACITY: usize = 100;

/// Motions that move the primary cursor more than this many lines are
/// automatically recorded as jumps.
pub(crate) const JUMP_LINE_THRESHOLD: usize = 5;

/// A single saved cursor position in the jump list.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct JumpEntry {
    /// Full selection state at the moment of the jump.
    pub selections: SelectionSet,
    /// Line number of the primary selection's head — cached for O(1) dedup.
    pub primary_line: usize,
}

/// Navigable history of cursor positions before large movements.
///
/// `cursor` indexes into `entries`. When `cursor == entries.len()`, the user
/// is "at the present" — no backward navigation is active. Navigating backward
/// decrements cursor; navigating forward increments it. A new `push` truncates
/// any forward history (entries after cursor) before appending.
#[derive(Debug)]
pub(crate) struct JumpList {
    entries: Vec<JumpEntry>,
    /// Current position. `cursor == entries.len()` means "at the present".
    cursor: usize,
}

impl JumpList {
    pub(crate) fn new() -> Self {
        Self { entries: Vec::new(), cursor: 0 }
    }

    /// Record a jump. Truncates forward history, deduplicates against the last
    /// entry by line number, and caps the list at [`JUMP_LIST_CAPACITY`].
    pub(crate) fn push(&mut self, entry: JumpEntry) {
        // Truncate any forward history.
        self.entries.truncate(self.cursor);

        // Deduplicate: if the last entry is on the same line, replace it.
        if let Some(last) = self.entries.last_mut().filter(|l| l.primary_line == entry.primary_line) {
            *last = entry;
            // cursor already == entries.len() after truncate
            return;
        }

        self.entries.push(entry);

        // Cap at capacity by dropping the oldest entry.
        if self.entries.len() > JUMP_LIST_CAPACITY {
            self.entries.remove(0);
        }

        self.cursor = self.entries.len();
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
            self.entries.push(current);
            // Point at the saved entry; the decrement below skips past it to
            // the most recent real jump.
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

}

// ── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::selection::{Selection, SelectionSet};

    /// Helper: build a JumpEntry with a cursor at `char_pos` on `line`.
    fn entry(char_pos: usize, line: usize) -> JumpEntry {
        JumpEntry {
            selections: SelectionSet::single(Selection::cursor(char_pos)),
            primary_line: line,
        }
    }

    #[test]
    fn push_and_backward() {
        let mut jl = JumpList::new();
        jl.push(entry(0, 0));
        jl.push(entry(10, 5));
        jl.push(entry(20, 10));

        // Navigate backward through all three entries (passing a "current" for
        // the first backward call).
        let current = entry(30, 15);
        let e = jl.backward(current).unwrap();
        assert_eq!(e.primary_line, 10);

        let e = jl.backward(entry(0, 0)).unwrap(); // current ignored when not at present
        assert_eq!(e.primary_line, 5);

        let e = jl.backward(entry(0, 0)).unwrap();
        assert_eq!(e.primary_line, 0);

        // Already at oldest — returns None.
        assert!(jl.backward(entry(0, 0)).is_none());
    }

    #[test]
    fn forward_after_backward() {
        let mut jl = JumpList::new();
        jl.push(entry(0, 0));
        jl.push(entry(10, 5));
        jl.push(entry(20, 10));

        let current = entry(30, 15);
        jl.backward(current).unwrap(); // → line 10
        jl.backward(entry(0, 0)).unwrap(); // → line 5

        let e = jl.forward().unwrap();
        assert_eq!(e.primary_line, 10);

        // Forward again returns the saved "current" (line 15).
        let e = jl.forward().unwrap();
        assert_eq!(e.primary_line, 15);

        // Already at present — None.
        assert!(jl.forward().is_none());
    }

    #[test]
    fn truncation_on_new_push() {
        let mut jl = JumpList::new();
        jl.push(entry(0, 0));
        jl.push(entry(10, 5));
        jl.push(entry(20, 10));

        // Go backward two steps.
        jl.backward(entry(30, 15)).unwrap(); // → line 10
        jl.backward(entry(0, 0)).unwrap(); // → line 5

        // New jump from here — forward history (lines 10, 15) is discarded.
        // After push: entries = [line0, line5, line25], cursor = 3.
        jl.push(entry(50, 25));

        // Forward should be empty now.
        assert!(jl.forward().is_none());

        // After truncation at cursor=1 and push(25): entries = [line0, line25].
        // Backward from line 30 saves current, then returns line 25.
        let e = jl.backward(entry(60, 30)).unwrap();
        assert_eq!(e.primary_line, 25);

        let e = jl.backward(entry(0, 0)).unwrap();
        assert_eq!(e.primary_line, 0);

        assert!(jl.backward(entry(0, 0)).is_none());
    }

    #[test]
    fn capacity_cap() {
        let mut jl = JumpList::new();
        for i in 0..=JUMP_LIST_CAPACITY {
            jl.push(entry(i * 10, i));
        }
        // Should have exactly CAPACITY entries, oldest dropped.
        assert_eq!(jl.len(), JUMP_LIST_CAPACITY);
        // Oldest remaining should be line 1 (line 0 was dropped).
        // Navigate all the way back.
        let e = jl.backward(entry(9999, 9999)).unwrap();
        assert_eq!(e.primary_line, JUMP_LIST_CAPACITY);

        // Keep going back to find the oldest.
        let mut oldest = e.primary_line;
        while let Some(e) = jl.backward(entry(0, 0)) {
            oldest = e.primary_line;
        }
        assert_eq!(oldest, 1);
    }

    #[test]
    fn deduplication() {
        let mut jl = JumpList::new();
        jl.push(entry(0, 5));
        jl.push(entry(3, 5)); // same line — replaces
        assert_eq!(jl.len(), 1);

        // Push a different line, then check we can reach the deduped entry.
        jl.push(entry(20, 10));
        let e = jl.backward(entry(99, 99)).unwrap();
        assert_eq!(e.primary_line, 10);
        let e = jl.backward(entry(0, 0)).unwrap();
        assert_eq!(e.primary_line, 5);
        // The stored entry should have the updated char_pos from the dedup.
        assert_eq!(e.selections.primary().head, 3);
    }

    #[test]
    fn empty_list() {
        let mut jl = JumpList::new();
        assert!(jl.backward(entry(0, 0)).is_none());
        assert!(jl.forward().is_none());
    }

    #[test]
    fn backward_saves_current_position() {
        let mut jl = JumpList::new();
        jl.push(entry(0, 0));

        // First backward from "present" — saves current (line 10) then returns line 0.
        let e = jl.backward(entry(50, 10)).unwrap();
        assert_eq!(e.primary_line, 0);

        // Forward returns the saved current position.
        let e = jl.forward().unwrap();
        assert_eq!(e.primary_line, 10);
    }
}
