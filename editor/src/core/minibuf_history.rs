//! Minibuffer history — bounded, in-memory recall for `:`, `/`, and `?` prompts.
//!
//! Each prompt gets its own [`History`] ring (oldest-first [`VecDeque`]) with
//! per-session navigation state (cursor + scratch). The three rings are grouped
//! in [`HistoryStore`], which lives on `Editor` and is keyed by [`HistoryKind`].
//!
//! The API is shaped for a future shada-like persistence layer: [`HistoryStore::snapshot`]
//! and [`HistoryStore::restore`] are defined but unused in v1.

use std::collections::VecDeque;

// ── HistoryKind / HistoryDir ──────────────────────────────────────────────────

/// Which minibuffer prompt a history ring belongs to.
///
/// An explicit enum (rather than a raw `char`) keeps the variant set closed,
/// exhaustively matched, and serializable to a stable key in a future env file.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum HistoryKind {
    /// `:` command-mode prompt.
    Command,
    /// `/` forward-search prompt.
    SearchForward,
    /// `?` backward-search prompt.
    SearchBackward,
}

/// Direction for [`crate::editor::Editor::recall_history`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum HistoryDir {
    Prev,
    Next,
}

// ── History ───────────────────────────────────────────────────────────────────

/// A single bounded history ring with per-session navigation state.
///
/// Entries are stored oldest-first; `back()` is always the most recent.
/// Navigation state (`cursor`, `scratch`) is reset at the start of each
/// minibuffer session and has no meaning between sessions.
#[derive(Debug)]
pub(crate) struct History {
    entries:  VecDeque<String>,
    capacity: usize,
    /// `None` = not currently navigating (at "scratch" / no Up pressed yet).
    /// `Some(i)` = `entries[i]` is currently shown in the minibuffer.
    cursor:   Option<usize>,
    /// The text that was in the minibuffer when the user first pressed Up this
    /// session — restored by Down past the newest entry.
    scratch:  Option<String>,
}

impl History {
    pub(crate) fn new(capacity: usize) -> Self {
        Self {
            entries: VecDeque::new(),
            capacity,
            cursor:  None,
            scratch: None,
        }
    }

    /// Record a submitted entry. Skips empty strings and consecutive duplicates.
    /// Always resets nav state — a confirm ends the session.
    pub(crate) fn push(&mut self, entry: String) {
        self.begin_session();
        if entry.is_empty() {
            return;
        }
        if self.entries.back().is_some_and(|last| *last == entry) {
            return;
        }
        self.entries.push_back(entry);
        if self.entries.len() > self.capacity {
            self.entries.pop_front();
        }
    }

    /// Update the capacity limit. Trims oldest entries if the ring is already
    /// over the new limit. Called when `history-capacity` is changed at runtime.
    pub(crate) fn set_capacity(&mut self, new_cap: usize) {
        self.capacity = new_cap;
        while self.entries.len() > self.capacity {
            self.entries.pop_front();
        }
    }

    /// Walk one step older. On the first call this session, stashes `current`
    /// as scratch. Returns `Some(text)` to install, `None` if no older entry exists.
    pub(crate) fn prev(&mut self, current: &str) -> Option<String> {
        if self.entries.is_empty() {
            return None;
        }
        match self.cursor {
            None => {
                self.scratch = Some(current.to_owned());
                let idx = self.entries.len() - 1;
                self.cursor = Some(idx);
                Some(self.entries[idx].clone())
            }
            Some(0) => None, // already at oldest
            Some(i) => {
                let idx = i - 1;
                self.cursor = Some(idx);
                Some(self.entries[idx].clone())
            }
        }
    }

    /// Walk one step newer. Past newest: restores scratch and exits navigation.
    /// Returns `None` if not currently navigating.
    pub(crate) fn next(&mut self) -> Option<String> {
        let i = self.cursor?;
        if i + 1 < self.entries.len() {
            let idx = i + 1;
            self.cursor = Some(idx);
            Some(self.entries[idx].clone())
        } else {
            // Past newest — restore scratch and exit navigation mode.
            let scratch = self.scratch.take().unwrap_or_default();
            self.cursor = None;
            Some(scratch)
        }
    }

    /// Demote: the user edited a recalled entry. Clears the cursor so the next
    /// `prev` re-stashes the current (now-edited) text as fresh scratch.
    pub(crate) fn demote_to_scratch(&mut self) {
        self.cursor = None;
        self.scratch = None;
    }

    /// Reset per-session nav state. Called when the minibuffer opens or closes.
    pub(crate) fn begin_session(&mut self) {
        self.cursor = None;
        self.scratch = None;
    }

    // ── Persistence hooks (unused in v1) ──────────────────────────────────────

    #[allow(dead_code)]
    pub(crate) fn entries(&self) -> &VecDeque<String> {
        &self.entries
    }

    #[allow(dead_code)]
    pub(crate) fn restore(entries: Vec<String>, capacity: usize) -> Self {
        let mut ring = Self::new(capacity);
        for e in entries {
            ring.entries.push_back(e);
        }
        // Silently cap to capacity — the env file may have been written with
        // a higher capacity than the current setting.
        while ring.entries.len() > capacity {
            ring.entries.pop_front();
        }
        ring
    }
}

// ── HistoryStore ──────────────────────────────────────────────────────────────

/// Container for all minibuffer history rings. A single instance lives on
/// `Editor`; rings are accessed by [`HistoryKind`].
#[derive(Debug)]
pub(crate) struct HistoryStore {
    command:  History,
    search_f: History,
    search_b: History,
}

impl HistoryStore {
    pub(crate) fn new(capacity: usize) -> Self {
        Self {
            command:  History::new(capacity),
            search_f: History::new(capacity),
            search_b: History::new(capacity),
        }
    }

    #[cfg(test)]
    pub(crate) fn get(&self, kind: HistoryKind) -> &History {
        match kind {
            HistoryKind::Command       => &self.command,
            HistoryKind::SearchForward  => &self.search_f,
            HistoryKind::SearchBackward => &self.search_b,
        }
    }

    pub(crate) fn get_mut(&mut self, kind: HistoryKind) -> &mut History {
        match kind {
            HistoryKind::Command       => &mut self.command,
            HistoryKind::SearchForward  => &mut self.search_f,
            HistoryKind::SearchBackward => &mut self.search_b,
        }
    }

    /// Map a minibuffer prompt character to its history kind.
    /// Returns `None` for prompts that have no associated history (e.g. `⫽`).
    pub(crate) fn kind_for_prompt(prompt: char) -> Option<HistoryKind> {
        match prompt {
            ':' => Some(HistoryKind::Command),
            '/' => Some(HistoryKind::SearchForward),
            '?' => Some(HistoryKind::SearchBackward),
            _   => None,
        }
    }

    /// Reset per-session nav state on every ring. Called when any minibuffer
    /// opens or closes.
    pub(crate) fn begin_session_all(&mut self) {
        self.command.begin_session();
        self.search_f.begin_session();
        self.search_b.begin_session();
    }

    /// Update the capacity of every ring and trim stale entries.
    /// Called when the `history-capacity` setting changes at runtime.
    pub(crate) fn set_capacity(&mut self, new_cap: usize) {
        self.command.set_capacity(new_cap);
        self.search_f.set_capacity(new_cap);
        self.search_b.set_capacity(new_cap);
    }

    // ── Persistence hooks (unused in v1) ──────────────────────────────────────

    #[allow(dead_code)]
    pub(crate) fn snapshot(&self) -> Vec<(HistoryKind, Vec<String>)> {
        vec![
            (HistoryKind::Command,       self.command.entries.iter().cloned().collect()),
            (HistoryKind::SearchForward,  self.search_f.entries.iter().cloned().collect()),
            (HistoryKind::SearchBackward, self.search_b.entries.iter().cloned().collect()),
        ]
    }

    #[allow(dead_code)]
    pub(crate) fn restore(snapshot: Vec<(HistoryKind, Vec<String>)>, capacity: usize) -> Self {
        let mut store = Self::new(capacity);
        for (kind, entries) in snapshot {
            *store.get_mut(kind) = History::restore(entries, capacity);
        }
        store
    }
}

// ── Unit tests ────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn h(capacity: usize) -> History {
        History::new(capacity)
    }

    fn store(capacity: usize) -> HistoryStore {
        HistoryStore::new(capacity)
    }

    // ── History ───────────────────────────────────────────────────────────────

    #[test]
    fn push_and_prev_walks_back() {
        let mut h = h(10);
        h.push("a".into());
        h.push("b".into());
        h.push("c".into());
        assert_eq!(h.prev(""), Some("c".into()));
        assert_eq!(h.prev(""), Some("b".into()));
        assert_eq!(h.prev(""), Some("a".into()));
        assert_eq!(h.prev(""), None); // at oldest
    }

    #[test]
    fn next_after_prev_walks_forward_then_scratch() {
        let mut h = h(10);
        h.push("a".into());
        h.push("b".into());
        h.push("c".into());
        // Walk to oldest.
        h.prev("");
        h.prev("");
        h.prev("");
        // Walk back forward.
        assert_eq!(h.next(), Some("b".into()));
        assert_eq!(h.next(), Some("c".into()));
        // Past newest restores scratch ("").
        assert_eq!(h.next(), Some("".into()));
    }

    #[test]
    fn next_returns_none_when_not_navigating() {
        let mut h = h(10);
        h.push("a".into());
        assert_eq!(h.next(), None);
    }

    #[test]
    fn prev_stashes_scratch_on_first_call() {
        let mut ring = History::new(10);
        ring.push("x".into());
        // First prev: stashes "typed" as scratch.
        assert_eq!(ring.prev("typed"), Some("x".into()));
        assert_eq!(ring.scratch, Some("typed".into()));
        // Navigating forward past newest restores scratch.
        assert_eq!(ring.next(), Some("typed".into()));
        assert_eq!(ring.cursor, None);
    }

    #[test]
    fn consecutive_duplicate_push_is_skipped() {
        let mut h = h(10);
        h.push("w".into());
        h.push("w".into());
        assert_eq!(h.entries.len(), 1);
    }

    #[test]
    fn non_consecutive_duplicate_is_kept() {
        let mut h = h(10);
        h.push("w".into());
        h.push("q".into());
        h.push("w".into());
        assert_eq!(h.entries.len(), 3);
    }

    #[test]
    fn capacity_evicts_oldest() {
        let mut h = h(3);
        h.push("a".into());
        h.push("b".into());
        h.push("c".into());
        h.push("d".into()); // evicts "a"
        assert_eq!(h.entries.len(), 3);
        assert_eq!(h.prev(""), Some("d".into()));
        assert_eq!(h.prev(""), Some("c".into()));
        assert_eq!(h.prev(""), Some("b".into()));
        assert_eq!(h.prev(""), None);
    }

    #[test]
    fn prev_on_empty_history_returns_none() {
        let mut h = h(10);
        assert_eq!(h.prev("x"), None);
        // Scratch should NOT have been set — there was nothing to navigate to.
        assert!(h.scratch.is_none());
    }

    #[test]
    fn prev_at_oldest_returns_none_keeps_position() {
        let mut h = h(10);
        h.push("a".into());
        h.prev(""); // lands on "a" (oldest)
        assert_eq!(h.cursor, Some(0));
        assert_eq!(h.prev(""), None); // still at oldest
        assert_eq!(h.cursor, Some(0)); // position unchanged
    }

    #[test]
    fn demote_to_scratch_clears_navigation() {
        let mut h = h(10);
        h.push("a".into());
        h.push("b".into());
        h.prev(""); // cursor = Some(1) = "b"
        h.demote_to_scratch();
        assert_eq!(h.cursor, None);
        assert_eq!(h.scratch, None);
        // Next prev re-stashes current text.
        assert_eq!(h.prev("edited"), Some("b".into()));
        assert_eq!(h.scratch, Some("edited".into()));
    }

    #[test]
    fn empty_push_is_ignored() {
        let mut h = h(10);
        h.push(String::new());
        assert_eq!(h.entries.len(), 0);
    }

    #[test]
    fn begin_session_resets_nav_but_keeps_entries() {
        let mut h = h(10);
        h.push("a".into());
        h.prev(""); // cursor = Some(0)
        h.begin_session();
        assert_eq!(h.cursor, None);
        assert_eq!(h.scratch, None);
        assert_eq!(h.entries.len(), 1); // entry still there
        // Can navigate again in the new session.
        assert_eq!(h.prev(""), Some("a".into()));
    }

    #[test]
    fn restore_round_trips_entries() {
        let original: Vec<String> = vec!["a".into(), "b".into(), "c".into()];
        let h = History::restore(original.clone(), 10);
        assert_eq!(h.entries.iter().cloned().collect::<Vec<_>>(), original);
    }

    #[test]
    fn restore_caps_to_capacity() {
        let entries: Vec<String> = (0..10).map(|i| i.to_string()).collect();
        let h = History::restore(entries, 3);
        assert_eq!(h.entries.len(), 3);
        // Most-recent entries are kept.
        assert_eq!(h.entries.back().map(|s| s.as_str()), Some("9"));
    }

    // ── HistoryStore ──────────────────────────────────────────────────────────

    #[test]
    fn kind_for_prompt_maps_colon_slash_question() {
        assert_eq!(HistoryStore::kind_for_prompt(':'), Some(HistoryKind::Command));
        assert_eq!(HistoryStore::kind_for_prompt('/'), Some(HistoryKind::SearchForward));
        assert_eq!(HistoryStore::kind_for_prompt('?'), Some(HistoryKind::SearchBackward));
        assert_eq!(HistoryStore::kind_for_prompt('⫽'), None);
        assert_eq!(HistoryStore::kind_for_prompt('x'), None);
    }

    #[test]
    fn snapshot_and_restore_round_trips_entries() {
        let mut s = store(10);
        s.get_mut(HistoryKind::Command).push("w".into());
        s.get_mut(HistoryKind::SearchForward).push("foo".into());
        s.get_mut(HistoryKind::SearchBackward).push("bar".into());

        let snap = s.snapshot();
        let restored = HistoryStore::restore(snap, 10);

        assert_eq!(
            restored.get(HistoryKind::Command).entries().iter().cloned().collect::<Vec<_>>(),
            vec!["w"],
        );
        assert_eq!(
            restored.get(HistoryKind::SearchForward).entries().iter().cloned().collect::<Vec<_>>(),
            vec!["foo"],
        );
        assert_eq!(
            restored.get(HistoryKind::SearchBackward).entries().iter().cloned().collect::<Vec<_>>(),
            vec!["bar"],
        );
    }

    #[test]
    fn set_capacity_updates_limit_and_trims_oldest() {
        let mut h = h(10);
        h.push("a".into());
        h.push("b".into());
        h.push("c".into());
        assert_eq!(h.entries.len(), 3);

        // Shrink: oldest entries are trimmed to fit.
        h.set_capacity(2);
        assert_eq!(h.capacity, 2);
        assert_eq!(h.entries.len(), 2);
        assert_eq!(h.entries.back().map(|s| s.as_str()), Some("c"));
        assert_eq!(h.entries.front().map(|s| s.as_str()), Some("b"));

        // Future pushes respect the new limit.
        h.push("d".into());
        assert_eq!(h.entries.len(), 2);
        assert_eq!(h.entries.back().map(|s| s.as_str()), Some("d"));
    }

    #[test]
    fn history_store_set_capacity_updates_all_rings() {
        let mut s = store(10);
        s.get_mut(HistoryKind::Command).push("w".into());
        s.get_mut(HistoryKind::SearchForward).push("foo".into());
        s.get_mut(HistoryKind::SearchBackward).push("bar".into());

        s.set_capacity(5);

        assert_eq!(s.get(HistoryKind::Command).capacity, 5);
        assert_eq!(s.get(HistoryKind::SearchForward).capacity, 5);
        assert_eq!(s.get(HistoryKind::SearchBackward).capacity, 5);
        // Existing entries fit in 5 so none were evicted.
        assert_eq!(s.get(HistoryKind::Command).entries().len(), 1);
    }

    #[test]
    fn begin_session_all_resets_all_rings() {
        let mut s = store(10);
        s.get_mut(HistoryKind::Command).push("w".into());
        s.get_mut(HistoryKind::SearchForward).push("foo".into());
        s.get_mut(HistoryKind::Command).prev("");
        s.get_mut(HistoryKind::SearchForward).prev("");
        assert!(s.get(HistoryKind::Command).cursor.is_some());
        assert!(s.get(HistoryKind::SearchForward).cursor.is_some());

        s.begin_session_all();

        assert!(s.get(HistoryKind::Command).cursor.is_none());
        assert!(s.get(HistoryKind::SearchForward).cursor.is_none());
    }
}
