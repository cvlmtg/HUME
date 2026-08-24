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

use hume_engine::pipeline::BufferId;

use hume_editing::selection::{Selection, SelectionSet};
use hume_editing::text::BufferText;

/// Default capacity — used in tests to construct jump lists without importing `EditorSettings`.
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
    pub(crate) fn new(selections: SelectionSet, text: &BufferText, buffer_id: BufferId) -> Self {
        let primary_line = text.char_to_line(selections.primary().head());
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
#[derive(Debug, Clone)]
pub(crate) struct JumpList {
    entries: VecDeque<JumpEntry>,
    /// Current position. `cursor == entries.len()` means "at the present".
    cursor: usize,
    /// Maximum number of entries. Oldest entry is dropped when exceeded.
    capacity: usize,
}

impl JumpList {
    /// Create a new jump list with the given capacity limit.
    ///
    /// `capacity == 0` is a silent black hole (every `push` immediately
    /// evicts what it just pushed) rather than a documented "unlimited" —
    /// unlike `undo-levels`, where `0` means exactly that. The settings
    /// parser (`usize_nonzero`) already rejects `0` for `jump-list-capacity`
    /// before it can reach here; this just makes the trap loud if that
    /// guard is ever bypassed (a test constructing a `JumpList` directly).
    pub(crate) fn new(capacity: usize) -> Self {
        debug_assert!(capacity > 0, "JumpList capacity must be non-zero");
        Self {
            entries: VecDeque::new(),
            cursor: 0,
            capacity,
        }
    }

    /// Change the capacity limit. Takes effect on the *next* `push`, not
    /// immediately — matching Vim's `undolevels` semantics (see
    /// `hume_editing::history::UndoTree::set_undo_levels`): lowering the cap
    /// does not retroactively trim existing entries. No cursor adjustment is
    /// needed here, since no entries are removed by this call.
    pub(crate) fn set_capacity(&mut self, capacity: usize) {
        debug_assert!(capacity > 0, "JumpList capacity must be non-zero");
        self.capacity = capacity;
    }

    /// Record a jump. Truncates forward history, deduplicates against the
    /// last entry by line number, and caps the list at `self.capacity` — a
    /// `while`, not an `if`, so a `set_capacity` shrink of any size converges
    /// to the new cap in this one call rather than one entry per push.
    pub(crate) fn push(&mut self, entry: JumpEntry) {
        self.entries.truncate(self.cursor);

        // Deduplicate: same line AND same buffer — cross-buffer same-line entries are distinct.
        match self
            .entries
            .back_mut()
            .filter(|l| l.primary_line == entry.primary_line && l.buffer_id == entry.buffer_id)
        {
            Some(last) => *last = entry,
            None => self.entries.push_back(entry),
        }

        while self.entries.len() > self.capacity {
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
            // Same cap enforcement as `push()` — this is the list's other
            // append site, and without it a list already at capacity grows
            // to `capacity + 1` here (capacity stops being an invariant of
            // the type). `while`, matching `push`, for the same
            // shrink-converges-in-one-call reasoning.
            while self.entries.len() > self.capacity {
                self.entries.pop_front();
            }
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
    pub(crate) fn len(&self) -> usize {
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
mod tests;
