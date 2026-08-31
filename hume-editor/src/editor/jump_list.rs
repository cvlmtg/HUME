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

use hume_editing::changeset::ChangeSet;
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

        // Deduplicate against the immediately preceding entry only — same
        // rule `translate_in_place`'s post-remap collapse pass applies, kept
        // as one predicate (`same_slot`) so the two call sites can't drift.
        match self
            .entries
            .back_mut()
            .filter(|l| Self::same_slot(l, &entry))
        {
            Some(last) => *last = entry,
            None => self.entries.push_back(entry),
        }

        while self.entries.len() > self.capacity {
            self.entries.pop_front();
        }

        self.cursor = self.entries.len();
    }

    /// Same line AND same buffer — cross-buffer same-line entries are distinct.
    fn same_slot(a: &JumpEntry, b: &JumpEntry) -> bool {
        a.primary_line == b.primary_line && a.buffer_id == b.buffer_id
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

    /// Remap every entry for `buf_id` through an edit, keeping stored
    /// positions pointing at the same text rather than the same offset.
    ///
    /// Entries for any other buffer are left untouched — the jump list is
    /// cross-buffer (that's what makes cross-buffer Ctrl+O work), so a remap
    /// triggered by an edit in one buffer must not touch another buffer's
    /// entries.
    ///
    /// Runs `PosMapCursor` once per entry rather than batching every entry's
    /// position through one shared cursor (the "batch `PosMapCursor`, never
    /// per-position" rule `docs/LSP.md` sets for diagnostics/decorations):
    /// unlike a `SelectionSet`, entries are not sorted relative to one
    /// another (`backward`/`push` interleave buffers and lines freely), so a
    /// forward-only cursor can't walk them in one pass without first sorting
    /// and later scattering results back — for `jump-list-capacity`'s default
    /// of 100 entries against a typical single-keystroke changeset of a
    /// handful of ops, that's more work than just re-walking the ops per
    /// entry. `Selection`s *within* one entry are already sorted, so that
    /// inner mapping (`SelectionSet::translate_in_place`) does share one
    /// cursor across them.
    ///
    /// `text_pre`/`text_post` must be the buffer text immediately before and
    /// after the edit — `text_pre` for `translate_in_place`'s own sticky-column
    /// invalidation, `text_post` to recompute `primary_line`, which is a
    /// cached line index rather than an offset and so can't be mapped through
    /// `cs` directly.
    pub(crate) fn translate_in_place(
        &mut self,
        buf_id: BufferId,
        cs: &ChangeSet,
        text_pre: &BufferText,
        text_post: &BufferText,
    ) {
        let mut any_line_moved = false;
        for entry in self.entries.iter_mut().filter(|e| e.buffer_id == buf_id) {
            entry.selections.translate_in_place(cs, text_pre);
            let new_line = text_post.char_to_line(entry.selections.primary().head());
            any_line_moved |= new_line != entry.primary_line;
            entry.primary_line = new_line;
        }
        // Two entries can only newly collide on (buffer_id, primary_line) if
        // one of them just moved — skip the collapse walk on the common case
        // of an edit that didn't cross any entry's line.
        if any_line_moved {
            self.collapse_adjacent_duplicates();
        }
    }

    /// Merge adjacent entries that now share `(buffer_id, primary_line)` —
    /// a deletion spanning multiple jump points can map them onto the same
    /// spot. Only physically adjacent entries are checked, matching `push`'s
    /// own dedup (which only ever compares against the immediately preceding
    /// entry, not the whole list), so this can't merge entries `push` itself
    /// would have kept apart.
    ///
    /// Keeps the newer (later-pushed, higher-index) entry of each run, same
    /// as `push`'s `*last = entry`. Remaps `cursor` so it still addresses the
    /// same conceptual position: a boundary is snapped to the start of the
    /// group it falls in, so `cursor == entries.len()` (the present) still
    /// maps to the new length.
    fn collapse_adjacent_duplicates(&mut self) {
        let n = self.entries.len();
        let mut group_ends = Vec::new();
        let mut merged = VecDeque::with_capacity(n);
        let mut i = 0;
        while i < n {
            let mut j = i + 1;
            while j < n && Self::same_slot(&self.entries[i], &self.entries[j]) {
                j += 1;
            }
            merged.push_back(self.entries[j - 1].clone());
            group_ends.push(j);
            i = j;
        }
        if merged.len() == self.entries.len() {
            return; // No adjacent run had more than one member — nothing to merge.
        }
        self.cursor = group_ends.partition_point(|&end| end <= self.cursor);
        self.entries = merged;
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
