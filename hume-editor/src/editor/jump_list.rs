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

use hume_engine::pipeline::{BufferId, PaneId};
use slotmap::SecondaryMap;

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

        // Deduplicate against the immediately preceding entry only, by (line,
        // buffer) — cross-buffer same-line entries are distinct.
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

    /// Remap every entry for `buf_id` through an edit, keeping stored
    /// positions pointing at the same text rather than the same offset, and
    /// merge any two adjacent entries an edit has newly collapsed onto one
    /// line.
    ///
    /// Entries for any other buffer are left untouched — the jump list is
    /// cross-buffer (that's what makes cross-buffer Ctrl+O work), so a remap
    /// triggered by an edit in one buffer must not touch another buffer's
    /// entries. This holds for the merge too: two untouched entries always
    /// have equal pre- and post-edit lines, which the merge condition below
    /// requires to *differ* — so it can never fire between them.
    ///
    /// Runs `PosMapCursor` once per entry rather than batching every entry's
    /// position through one shared cursor (the "batch `PosMapCursor`, never
    /// per-position" rule `docs/LSP.md` sets for diagnostics/decorations):
    /// unlike a `SelectionSet`, entries are not sorted relative to one
    /// another (`backward`/`push` interleave buffers and lines freely), so a
    /// forward-only cursor can't walk them in one pass without first sorting
    /// and later scattering results back. Sound for a single keystroke's
    /// changeset (a handful of ops against `jump-list-capacity`'s default of
    /// 100 entries); a changeset with many ops (a multi-cursor edit, `:%s`,
    /// an LSP whole-document format) pays `O(entries × ops)` here, the case
    /// the batched form would instead win. `Selection`s *within* one entry
    /// are already sorted, so that inner mapping
    /// (`SelectionSet::translate_in_place_with`) does share one cursor across
    /// them.
    ///
    /// `edits` must be `cs.edited_old_ranges()` — the caller computes it once
    /// and passes it to every pane's list, rather than each list rebuilding
    /// the same `Vec` from `cs`.
    ///
    /// `text_pre`/`text_post` must be the buffer text immediately before and
    /// after the edit — `text_pre` for `translate_in_place_with`'s own
    /// sticky-column invalidation, `text_post` to recompute `primary_line`,
    /// which is a cached line index rather than an offset and so can't be
    /// mapped through `cs` directly.
    ///
    /// Merging runs in the same pass as the remap, via the write-index
    /// compaction `SelectionSet::merge_overlapping_in_place` uses: entries
    /// are moved (`VecDeque::swap`), never cloned, and the common case — no
    /// merge — costs one self-swap per entry, no allocation. The merge
    /// condition is deliberately narrower than "same slot": `backward()`
    /// deliberately appends a same-line entry without dedup (so `forward()`
    /// can still return to it, e.g. two search matches on one line), so a
    /// pre-existing same-slot pair must survive here. Only a pair whose lines
    /// *differed* before this edit and *match* after it is a collision this
    /// edit actually created — that's the one case worth merging, keeping the
    /// newer entry, matching `push`'s own `*last = entry`. The cursor is
    /// adjusted exactly as `prune_buffer` adjusts it for a removal: by how
    /// many merged-away entries had an original index before it.
    pub(crate) fn translate_in_place(
        &mut self,
        buf_id: BufferId,
        edits: &[(usize, usize)],
        cs: &ChangeSet,
        text_pre: &BufferText,
        text_post: &BufferText,
    ) {
        let mut write = 0usize;
        let mut removed_before_cursor = 0usize;
        // (buffer_id, pre-remap line, post-remap line, original index) of the
        // entry currently kept at slot `write - 1`.
        let mut last: Option<(BufferId, usize, usize, usize)> = None;

        for read in 0..self.entries.len() {
            let bid = self.entries[read].buffer_id;
            let pre_line = self.entries[read].primary_line;
            if bid == buf_id {
                let entry = &mut self.entries[read];
                entry
                    .selections
                    .translate_in_place_with(edits, cs, text_pre);
                entry.primary_line = text_post.char_to_line(entry.selections.primary().head());
            }
            let post_line = self.entries[read].primary_line;

            if let Some((lbid, lpre, lpost, lread)) = last
                && lbid == bid
                && lpost == post_line
                && lpre != pre_line
            {
                // A collision this edit just created — overwrite the older
                // entry's slot with this one.
                write -= 1;
                removed_before_cursor += usize::from(lread < self.cursor);
            }
            self.entries.swap(write, read);
            last = Some((bid, pre_line, post_line, read));
            write += 1;
        }
        self.entries.truncate(write);
        self.cursor = self
            .cursor
            .saturating_sub(removed_before_cursor)
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

// ── JumpLists ────────────────────────────────────────────────────────────────

/// Every pane's [`JumpList`], keyed by `PaneId`.
///
/// A newtype rather than a bare `SecondaryMap` so "do X to every pane's jump
/// list" — remap through an edit, drop a buffer's entries, apply a capacity
/// change — is one named method instead of a `for jumps in
/// …values_mut() { … }` loop hand-written at each call site.
#[derive(Debug, Clone, Default)]
pub(crate) struct JumpLists(SecondaryMap<PaneId, JumpList>);

impl JumpLists {
    pub(crate) fn insert(&mut self, pid: PaneId, list: JumpList) {
        self.0.insert(pid, list);
    }

    pub(crate) fn remove(&mut self, pid: PaneId) {
        self.0.remove(pid);
    }

    /// Test-only: production seeding always goes through [`Self::insert`]
    /// unconditionally (`open_pane`, `Editor::new`), never guarded by a
    /// presence check — only the `switch_focused_pane` test choke-point
    /// lazily seeds a pane it didn't create through the normal path.
    #[cfg(test)]
    pub(crate) fn contains_key(&self, pid: PaneId) -> bool {
        self.0.contains_key(pid)
    }

    /// Remap every pane's jump-list entries for `buf_id` through `cs` — the
    /// per-edit propagation step `doc_ops::finish_edit` and
    /// `reload_buffer_in_place` both call.
    ///
    /// Unlike sibling-pane selection propagation, this does **not** filter by
    /// which panes currently view `buf_id` — a pane's jump list holds entries
    /// for buffers that pane isn't showing right now (that's what makes
    /// cross-buffer Ctrl+O work), so every pane's list must be checked,
    /// including the focused one (its own live cursor isn't a jump-list
    /// entry, so nothing is mapped twice).
    ///
    /// `edits` must be `cs.edited_old_ranges()` — computed once by the
    /// caller and shared across every pane's list; see
    /// [`JumpList::translate_in_place`].
    pub(crate) fn translate(
        &mut self,
        buf_id: BufferId,
        edits: &[(usize, usize)],
        cs: &ChangeSet,
        text_pre: &BufferText,
        text_post: &BufferText,
    ) {
        for jumps in self.0.values_mut() {
            jumps.translate_in_place(buf_id, edits, cs, text_pre, text_post);
        }
    }

    /// Drop every pane's entries for `id` — used when `id`'s content was
    /// replaced wholesale (a full `Buffer` swap, or `set_view_content`'s
    /// history-resetting in-place replace) rather than edited: there is no
    /// `ChangeSet` to remap through, and same-buffer-id survival alone isn't
    /// enough, since the new content shares nothing but its id with the old.
    pub(crate) fn prune_buffer(&mut self, id: BufferId) {
        for jumps in self.0.values_mut() {
            jumps.prune_buffer(id);
        }
    }

    /// Apply a `jump-list-capacity` change to every pane's list — takes
    /// effect on each list's next `push`, per `JumpList::set_capacity`.
    pub(crate) fn set_capacity(&mut self, capacity: usize) {
        for jumps in self.0.values_mut() {
            jumps.set_capacity(capacity);
        }
    }
}

impl std::ops::Index<PaneId> for JumpLists {
    type Output = JumpList;
    fn index(&self, pid: PaneId) -> &JumpList {
        &self.0[pid]
    }
}

impl std::ops::IndexMut<PaneId> for JumpLists {
    fn index_mut(&mut self, pid: PaneId) -> &mut JumpList {
        &mut self.0[pid]
    }
}

// ── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests;
