//! Jump list: a navigable history of cursor positions before large movements.
//!
//! Records the cursor position (as a full [`SelectionSet`]) before "jump"
//! commands like `goto-first-line`, `goto-last-line`, `search-next`,
//! `search-prev`, page scroll, and any motion that crosses more than
//! `EditorSettings::jump_line_threshold` lines. The user navigates the
//! history with `jump-backward` and `jump-forward`.
//!
//! Internally this is a [`VecDeque<JumpEntry>`] with a cursor index, capped
//! at `EditorSettings::jump_list_capacity`. When the user navigates backward
//! and then makes a new jump, forward history is truncated, matching
//! Vim/Helix semantics.

use std::collections::VecDeque;

use hume_engine::pipeline::{BufferId, EngineView, PaneId};
use slotmap::SecondaryMap;

use hume_editing::edit::TextChange;
use hume_editing::selection::{EditView, Selection, SelectionSet};
use hume_editing::state::EditState;
use hume_editing::text::BufferText;

use super::commands::{CommandPane, pane_view};
use super::{EditorState, Mode};

/// Default capacity, used in tests to construct jump lists without importing `EditorSettings`.
#[cfg(test)]
pub(in crate::editor::jump_list) const DEFAULT_JUMP_LIST_CAPACITY: usize = 100;

/// A single saved cursor position in the jump list.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::editor) struct JumpEntry {
    /// Buffer this position belongs to, needed for cross-buffer Ctrl-o/I.
    pub buffer_id: BufferId,
    /// Full selection state at the moment of the jump.
    pub selections: SelectionSet,
    /// Line number of the primary selection's head, cached for O(1) dedup.
    pub primary_line: hume_rope::line::ContentLine,
}

impl JumpEntry {
    /// `primary_line`'s one derivation: the primary selection's head, resolved
    /// to a line via `text`.
    fn primary_line_of(
        selections: &SelectionSet,
        text: &BufferText,
    ) -> hume_rope::line::ContentLine {
        EditView::bind(text, selections).primary().head_line()
    }

    /// The primary selection, read against `text`, the text these selections
    /// belong to.
    pub(in crate::editor) fn primary_selection(&self, text: &BufferText) -> Selection {
        EditView::bind(text, &self.selections).primary().selection()
    }

    /// Carry this entry through `change`, keeping `primary_line` in step.
    fn translate(&mut self, change: &TextChange<'_>) {
        self.selections.translate(change);
        self.primary_line = Self::primary_line_of(&self.selections, change.after());
    }

    /// Build a jump entry from selections of `text`, the buffer's current
    /// text, deriving `primary_line` from it so callers don't have to.
    pub(in crate::editor) fn new(
        selections: SelectionSet,
        text: &BufferText,
        buffer_id: BufferId,
    ) -> Self {
        let primary_line = Self::primary_line_of(&selections, text);
        Self {
            buffer_id,
            selections,
            primary_line,
        }
    }
}

/// Navigable history of cursor positions before large movements.
///
/// `cursor` indexes into `entries`. When `cursor == entries.len()`, the user
/// is "at the present": no backward navigation is active. Navigating backward
/// decrements cursor; navigating forward increments it. A new `push` truncates
/// any forward history (entries after cursor) before appending.
#[derive(Debug)]
pub(in crate::editor) struct JumpList {
    entries: VecDeque<JumpEntry>,
    /// Current position. `cursor == entries.len()` means "at the present".
    cursor: usize,
    /// Maximum number of entries. Oldest entry is dropped when exceeded.
    capacity: usize,
    /// Entries captured before a navigation that may edit their buffer, one
    /// per open [`PendingJump`], innermost last. They are carried through
    /// edits like `entries`, but belong to no history until pushed.
    pending: Vec<Option<JumpEntry>>,
}

/// A clone is a history snapshot: the pending captures belong to navigations
/// running against the original, and their handles index the original's
/// list.
impl Clone for JumpList {
    fn clone(&self) -> Self {
        Self {
            entries: self.entries.clone(),
            cursor: self.cursor,
            capacity: self.capacity,
            pending: Vec::new(),
        }
    }
}

/// A position captured before a navigation, held by its [`JumpList`] until
/// [`JumpList::end_pending`] takes it back.
#[must_use = "a pending jump stays in its list until `end_pending` takes it"]
#[derive(Debug)]
struct PendingJump(usize);

impl JumpList {
    /// Create a new jump list with the given capacity limit.
    ///
    /// `capacity == 0` is a silent black hole (every `push` immediately
    /// evicts what it just pushed) rather than a documented "unlimited",
    /// unlike `undo-levels`, where `0` means exactly that. The settings
    /// parser (`usize_nonzero`) already rejects `0` for `jump-list-capacity`
    /// before it can reach here; this just makes the trap loud if that
    /// guard is ever bypassed (a test constructing a `JumpList` directly).
    pub(in crate::editor) fn new(capacity: usize) -> Self {
        debug_assert!(capacity > 0, "JumpList capacity must be non-zero");
        Self {
            entries: VecDeque::new(),
            cursor: 0,
            capacity,
            pending: Vec::new(),
        }
    }

    /// Change the capacity limit. Takes effect on the *next* `push`, not
    /// immediately, matching Vim's `undolevels` semantics (see
    /// `hume_editing::history::UndoTree::set_undo_levels`): lowering the cap
    /// does not retroactively trim existing entries. No cursor adjustment is
    /// needed here, since no entries are removed by this call.
    pub(in crate::editor) fn set_capacity(&mut self, capacity: usize) {
        debug_assert!(capacity > 0, "JumpList capacity must be non-zero");
        self.capacity = capacity;
    }

    /// Record a jump. Truncates forward history, deduplicates against the
    /// last entry by line number, and caps the list at `self.capacity`: a
    /// `while`, not an `if`, so a `set_capacity` shrink of any size converges
    /// to the new cap in this one call rather than one entry per push.
    pub(in crate::editor) fn push(&mut self, entry: JumpEntry) {
        self.entries.truncate(self.cursor);

        // Deduplicate against the immediately preceding entry only, by (line,
        // buffer); cross-buffer same-line entries are distinct.
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
    pub(in crate::editor) fn prune_buffer(&mut self, id: BufferId) {
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
        for slot in &mut self.pending {
            if slot.as_ref().is_some_and(|e| e.buffer_id == id) {
                *slot = None;
            }
        }
    }

    /// Hold `entry` until the navigation it precedes is over. The list carries
    /// it through any edit that navigation makes, so it still names the text
    /// it was captured from when [`Self::end_pending`] returns it.
    fn begin_pending(&mut self, entry: JumpEntry) -> PendingJump {
        self.pending.push(Some(entry));
        PendingJump(self.pending.len() - 1)
    }

    /// The entry `pending` held, carried through every edit since; `None` when
    /// its buffer was replaced meanwhile. Also drops any pending entry begun
    /// after it and never ended.
    fn end_pending(&mut self, pending: PendingJump) -> Option<JumpEntry> {
        let entry = self.pending.get_mut(pending.0).and_then(Option::take);
        self.pending.truncate(pending.0);
        entry
    }

    /// Remap every entry for `buf_id` through an edit, keeping stored
    /// positions pointing at the same text rather than the same offset, and
    /// merge any two adjacent entries an edit has newly collapsed onto one
    /// line. Other buffers' entries are untouched, and never merge because
    /// their lines don't change.
    ///
    /// Each entry runs its own `PosMapCursor`: entries aren't sorted relative
    /// to each other, so one shared forward cursor can't walk them. That costs
    /// `O(entries × ops)` for large changesets (`:%s`, multi-cursor, format).
    /// Selections within an entry are sorted and do share one cursor.
    ///
    /// The change's new text recomputes each entry's cached `primary_line`.
    ///
    /// Merging is write-index compaction in the same pass (swaps, no
    /// allocation). Only a pair whose lines differed before the edit and match
    /// after it merges, keeping the newer entry as `push` does. Pre-existing
    /// same-line pairs survive, since `backward()` appends them on purpose so
    /// `forward()` can return. The cursor shifts by the number of merged-away
    /// entries before it, as in `prune_buffer`.
    pub(in crate::editor) fn translate_in_place(
        &mut self,
        buf_id: BufferId,
        change: &TextChange<'_>,
    ) {
        for entry in self.pending.iter_mut().flatten() {
            if entry.buffer_id == buf_id {
                entry.translate(change);
            }
        }

        let mut write = 0usize;
        let mut removed_before_cursor = 0usize;
        // (buffer_id, pre-remap line, post-remap line, original index) of the
        // entry currently kept at slot `write - 1`.
        let mut last: Option<(
            BufferId,
            hume_rope::line::ContentLine,
            hume_rope::line::ContentLine,
            usize,
        )> = None;

        for read in 0..self.entries.len() {
            let bid = self.entries[read].buffer_id;
            let pre_line = self.entries[read].primary_line;
            if bid == buf_id {
                self.entries[read].translate(change);
            }
            let post_line = self.entries[read].primary_line;

            if let Some((lbid, lpre, lpost, lread)) = last
                && lbid == bid
                && lpost == post_line
                && lpre != pre_line
            {
                // A collision this edit just created: overwrite the older
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
    pub(in crate::editor) fn backward(&mut self, current: JumpEntry) -> Option<&JumpEntry> {
        if self.entries.is_empty() {
            return None;
        }

        // At the present: always save the current position so `jump-forward`
        // can return to it. No dedup here. Unlike `push()`, the "save current"
        // path must preserve the exact return point even if it's on the same
        // line as the last recorded jump (e.g., two search matches on one line).
        if self.cursor == self.entries.len() {
            self.entries.push_back(current);
            // Same cap enforcement as `push()`: this is the list's other
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
    pub(in crate::editor) fn forward(&mut self) -> Option<&JumpEntry> {
        if self.cursor + 1 >= self.entries.len() {
            return None;
        }
        self.cursor += 1;
        Some(&self.entries[self.cursor])
    }

    #[cfg(test)]
    pub(in crate::editor) fn len(&self) -> usize {
        self.entries.len()
    }

    /// Returns `true` if any entry in the list belongs to `id`.
    #[cfg(test)]
    pub(in crate::editor) fn entries_for_buffer(&self, id: BufferId) -> bool {
        self.entries.iter().any(|e| e.buffer_id == id)
    }
}

/// When a navigation's starting position is recorded in the pane's jump
/// list.
#[derive(Clone, Copy)]
pub(in crate::editor) enum JumpRule {
    /// The whole selection set, whenever the pane's buffer or selections
    /// changed.
    IfMoved,
    /// The primary selection only, when it changed and either `is_jump` or
    /// its line moved further than `EditorSettings::jump_line_threshold`.
    /// Selection commands are left out of this rule by their callers: a
    /// large text-object selection is a select-then-act staging step, not
    /// deliberate navigation.
    Threshold { is_jump: bool },
}

/// Snapshot `t`'s pane's current selections as a `JumpEntry`.
pub(in crate::editor) fn current_jump_entry(
    state: &EditorState,
    view: &EngineView,
    t: CommandPane,
) -> JumpEntry {
    let bid = t.bid(view);
    let sels = t.state(&state.panes.state, view).selections().clone();
    JumpEntry::new(sels, state.buffers.get(bid).text(), bid)
}

/// Runs `body`, a navigation of `t`'s pane, and records where it started in
/// the pane's jump list according to `rule`. Returns `body`'s result and
/// whether the pane moved (its buffer or, under `Threshold`, its primary
/// selection changed).
///
/// The start is held by the jump list while `body` runs, so an edit the
/// navigation makes to that buffer (leaving Insert trims auto-indent)
/// carries it and it still names the text it was captured from. `moved`
/// guards every push: `JumpList::push` truncates forward history
/// unconditionally, so a navigation that turned out to be a no-op (`:42`
/// already on line 42, `goto-first-line` already on line 1, a search confirmed
/// on the match already under the cursor) must not push at all. The whole
/// `Selection` is compared, not just its head: `select-all` from the buffer's
/// own last char moves only the anchor.
///
/// An Insert-mode typed run in the focused pane is invalidated before `body`
/// runs: it would otherwise select across text the cursor jumped away from
/// once Insert exits. The mode is read before the body, since an entry
/// command (`i`/`a`/`o`/`c`) is already back in Insert by the time its own
/// body returns and would wipe the pins `begin_typed_run` just installed.
/// A pane `body` closed has no list left to record in.
pub(in crate::editor) fn with_jump<R>(
    state: &mut EditorState,
    view: &mut EngineView,
    t: CommandPane,
    rule: JumpRule,
    body: impl FnOnce(&mut EditorState, &mut EngineView) -> R,
) -> (R, bool) {
    let pid = t.pid();
    let pre_bid = t.bid(view);
    let entry = match rule {
        JumpRule::IfMoved => current_jump_entry(state, view, t),
        JumpRule::Threshold { .. } => {
            let text = state.buffers.get(pre_bid).text();
            let selections = t.state(&state.panes.state, view).selections().clone();
            JumpEntry::new(
                EditState::bind(text, selections)
                    .keep_primary()
                    .into_selections(),
                text,
                pre_bid,
            )
        }
    };
    if state.mode() == Mode::Insert && state.focus.id() == pid {
        t.state_mut(&mut state.panes.state, view).typed_run = None;
    }
    let pending = state.panes.jumps[pid].begin_pending(entry);
    let result = body(state, view);
    let Some(t) = CommandPane::existing(view, pid) else {
        return (result, false);
    };
    let entry = state.panes.jumps[pid].end_pending(pending);
    let post_bid = t.bid(view);
    let moved = match (rule, entry) {
        (JumpRule::IfMoved, Some(entry)) => record_if_moved(state, view, t, entry),
        (JumpRule::Threshold { is_jump }, Some(entry)) => {
            let post = pane_view(state, view, t).primary();
            let moved = post_bid != pre_bid
                || entry.primary_selection(state.buffers.get(pre_bid).text()) != post.selection();
            if moved
                && (is_jump
                    || entry.primary_line.abs_diff(post.head_line())
                        > state.settings.jump_line_threshold)
            {
                state.panes.jumps[pid].push(entry);
            }
            moved
        }
        (_, None) => post_bid != pre_bid,
    };
    (result, moved)
}

/// Pushes `pre`, a position captured before some navigation however long
/// ago, when `t`'s buffer or selections have changed since. Returns whether
/// they did. [`with_jump`]'s `IfMoved` rule, and the one entry point for a
/// navigation whose start was captured outside a single call (a search
/// confirmed after many keystrokes).
pub(in crate::editor) fn record_if_moved(
    state: &mut EditorState,
    view: &EngineView,
    t: CommandPane,
    pre: JumpEntry,
) -> bool {
    let moved = pre.buffer_id != t.bid(view)
        || pre.selections != *t.state(&state.panes.state, view).selections();
    if moved {
        state.panes.jumps[t.pid()].push(pre);
    }
    moved
}

/// Every pane's [`JumpList`], keyed by `PaneId`.
///
/// A newtype rather than a bare `SecondaryMap` so "do X to every pane's jump
/// list" (remap through an edit, drop a buffer's entries, apply a capacity
/// change) is one named method instead of a `for jumps in
/// …values_mut() { … }` loop hand-written at each call site.
#[derive(Debug, Clone, Default)]
pub(in crate::editor) struct JumpLists(SecondaryMap<PaneId, JumpList>);

impl JumpLists {
    pub(in crate::editor) fn insert(&mut self, pid: PaneId, list: JumpList) {
        self.0.insert(pid, list);
    }

    pub(in crate::editor) fn remove(&mut self, pid: PaneId) {
        self.0.remove(pid);
    }

    /// Test-only: production seeding always goes through [`Self::insert`]
    /// unconditionally (`commands::pane::open_pane`, `Editor::new`), never
    /// guarded by a presence check. Only the `switch_focused_pane` test
    /// choke-point lazily seeds a pane it didn't create through the normal
    /// path.
    #[cfg(test)]
    pub(in crate::editor) fn contains_key(&self, pid: PaneId) -> bool {
        self.0.contains_key(pid)
    }

    /// Remap every pane's jump-list entries for `buf_id` through `change`:
    /// one of the stores `PositionStores::carry` carries.
    ///
    /// Unlike sibling-pane selection propagation, this does **not** filter by
    /// which panes currently view `buf_id`: a pane's jump list holds entries
    /// for buffers that pane isn't showing right now (that's what makes
    /// cross-buffer Ctrl-o work), so every pane's list must be checked,
    /// including the focused one (its own live cursor isn't a jump-list
    /// entry, so nothing is mapped twice).
    pub(in crate::editor) fn translate(&mut self, buf_id: BufferId, change: &TextChange<'_>) {
        for jumps in self.0.values_mut() {
            jumps.translate_in_place(buf_id, change);
        }
    }

    /// Drop every pane's entries for `id`, used when `id`'s content was
    /// replaced wholesale (a full `Buffer` swap, or `set_view_content`'s
    /// history-resetting in-place replace) rather than edited: there is no
    /// `ChangeSet` to remap through, and same-buffer-id survival alone isn't
    /// enough, since the new content shares nothing but its id with the old.
    pub(in crate::editor) fn prune_buffer(&mut self, id: BufferId) {
        for jumps in self.0.values_mut() {
            jumps.prune_buffer(id);
        }
    }

    /// Apply a `jump-list-capacity` change to every pane's list. Takes
    /// effect on each list's next `push`, per `JumpList::set_capacity`.
    pub(in crate::editor) fn set_capacity(&mut self, capacity: usize) {
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

#[cfg(test)]
mod tests;
