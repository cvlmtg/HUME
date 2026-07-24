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
pub enum HistoryKind {
    /// `:` command-mode prompt.
    Command,
    /// `/` forward-search prompt.
    SearchForward,
    /// `?` backward-search prompt.
    SearchBackward,
}

/// Direction for [`crate::editor::Editor::recall_history`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HistoryDir {
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
pub struct History {
    entries: VecDeque<String>,
    capacity: usize,
    /// `None` = not currently navigating (at "scratch" / no Up pressed yet).
    /// `Some(i)` = `entries[i]` is currently shown in the minibuffer.
    cursor: Option<usize>,
    /// The text that was in the minibuffer when the user first pressed Up this
    /// session — restored by Down past the newest entry. Also doubles as the
    /// fixed prefix that `prev`/`next` filter the walk by for the rest of the
    /// session, so navigation only visits entries matching what was typed.
    scratch: Option<String>,
}

impl History {
    pub fn new(capacity: usize) -> Self {
        Self {
            entries: VecDeque::new(),
            capacity,
            cursor: None,
            scratch: None,
        }
    }

    /// Record a submitted entry. Skips empty strings and consecutive duplicates.
    /// Always resets nav state — a confirm ends the session.
    pub fn push(&mut self, entry: String) {
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
    pub fn set_capacity(&mut self, new_cap: usize) {
        self.capacity = new_cap;
        while self.entries.len() > self.capacity {
            self.entries.pop_front();
        }
    }

    /// Walk one step older, restricted to entries whose text starts with the
    /// prefix that was in the minibuffer when navigation began (stashed as
    /// `scratch` on the first call). An empty prefix matches every entry, so a
    /// fresh prompt still walks the full ring. Returns the entry to install, or
    /// `None` if no older match exists (position unchanged).
    pub fn prev(&mut self, current: &str) -> Option<String> {
        if self.entries.is_empty() {
            return None;
        }
        let first_step = self.cursor.is_none();
        let prefix = if first_step {
            current.to_owned()
        } else {
            self.scratch.clone().unwrap_or_default()
        };
        let scan_from = self.cursor.unwrap_or(self.entries.len());
        let idx = (0..scan_from)
            .rev()
            .find(|&idx| self.entries[idx].starts_with(&prefix))?;
        if first_step {
            self.scratch = Some(current.to_owned());
        }
        self.cursor = Some(idx);
        Some(self.entries[idx].clone())
    }

    /// Walk one step newer within the prefix match set. Past the newest match,
    /// restores the stashed prefix text and exits navigation. `None` if not
    /// currently navigating.
    pub fn next(&mut self) -> Option<String> {
        let i = self.cursor?;
        let prefix = self.scratch.clone().unwrap_or_default();
        match ((i + 1)..self.entries.len()).find(|&idx| self.entries[idx].starts_with(&prefix)) {
            Some(idx) => {
                self.cursor = Some(idx);
                Some(self.entries[idx].clone())
            }
            None => {
                // Past newest match — restore scratch and exit navigation mode.
                let scratch = self.scratch.take().unwrap_or_default();
                self.cursor = None;
                Some(scratch)
            }
        }
    }

    /// Demote: the user edited a recalled entry. Clears the cursor so the next
    /// `prev` re-stashes the current (now-edited) text as fresh scratch.
    pub fn demote_to_scratch(&mut self) {
        self.cursor = None;
        self.scratch = None;
    }

    /// Reset per-session nav state. Called when the minibuffer opens or closes.
    pub fn begin_session(&mut self) {
        self.cursor = None;
        self.scratch = None;
    }

    // ── Persistence hooks (unused in v1) ──────────────────────────────────────

    #[allow(dead_code)]
    pub fn entries(&self) -> &VecDeque<String> {
        &self.entries
    }

    #[allow(dead_code)]
    pub fn restore(entries: Vec<String>, capacity: usize) -> Self {
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
pub struct HistoryStore {
    command: History,
    search_f: History,
    search_b: History,
}

impl HistoryStore {
    pub fn new(capacity: usize) -> Self {
        Self {
            command: History::new(capacity),
            search_f: History::new(capacity),
            search_b: History::new(capacity),
        }
    }

    #[cfg(test)]
    pub fn get(&self, kind: HistoryKind) -> &History {
        match kind {
            HistoryKind::Command => &self.command,
            HistoryKind::SearchForward => &self.search_f,
            HistoryKind::SearchBackward => &self.search_b,
        }
    }

    pub fn get_mut(&mut self, kind: HistoryKind) -> &mut History {
        match kind {
            HistoryKind::Command => &mut self.command,
            HistoryKind::SearchForward => &mut self.search_f,
            HistoryKind::SearchBackward => &mut self.search_b,
        }
    }

    /// Map a minibuffer prompt character to its history kind.
    /// Returns `None` for prompts that have no associated history (e.g. `⫽`).
    pub fn kind_for_prompt(prompt: &str) -> Option<HistoryKind> {
        match prompt {
            ":" => Some(HistoryKind::Command),
            "/" => Some(HistoryKind::SearchForward),
            "?" => Some(HistoryKind::SearchBackward),
            _ => None,
        }
    }

    /// Reset per-session nav state on every ring. Called when any minibuffer
    /// opens or closes.
    pub fn begin_session_all(&mut self) {
        self.command.begin_session();
        self.search_f.begin_session();
        self.search_b.begin_session();
    }

    /// Update the capacity of every ring and trim stale entries.
    /// Called when the `history-capacity` setting changes at runtime.
    pub fn set_capacity(&mut self, new_cap: usize) {
        self.command.set_capacity(new_cap);
        self.search_f.set_capacity(new_cap);
        self.search_b.set_capacity(new_cap);
    }

    // ── Persistence hooks (unused in v1) ──────────────────────────────────────

    #[allow(dead_code)]
    pub fn snapshot(&self) -> Vec<(HistoryKind, Vec<String>)> {
        vec![
            (
                HistoryKind::Command,
                self.command.entries.iter().cloned().collect(),
            ),
            (
                HistoryKind::SearchForward,
                self.search_f.entries.iter().cloned().collect(),
            ),
            (
                HistoryKind::SearchBackward,
                self.search_b.entries.iter().cloned().collect(),
            ),
        ]
    }

    #[allow(dead_code)]
    pub fn restore(snapshot: Vec<(HistoryKind, Vec<String>)>, capacity: usize) -> Self {
        let mut store = Self::new(capacity);
        for (kind, entries) in snapshot {
            *store.get_mut(kind) = History::restore(entries, capacity);
        }
        store
    }
}

// ── Unit tests ────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests;
