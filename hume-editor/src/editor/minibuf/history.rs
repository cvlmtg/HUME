//! Minibuffer history — bounded, in-memory recall for `:`, `/`, and `?` prompts.
//!
//! Each prompt gets its own [`History`] ring (oldest-first [`VecDeque`]) with
//! per-session navigation state (cursor + scratch). The three rings are grouped
//! in [`HistoryStore`], which lives on `Editor` and is keyed by [`HistoryKind`].
//!
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
    /// `capacity == 0` is a silent black hole (every `push` immediately
    /// evicts what it just pushed) rather than a documented "unlimited" —
    /// unlike `undo-levels`, where `0` means exactly that. The settings
    /// parser (`usize_nonzero`) already rejects `0` for `history-capacity`
    /// before it can reach here; this just makes the trap loud if that
    /// guard is ever bypassed (a test constructing a `History` directly).
    pub fn new(capacity: usize) -> Self {
        debug_assert!(capacity > 0, "History capacity must be non-zero");
        Self {
            entries: VecDeque::new(),
            capacity,
            cursor: None,
            scratch: None,
        }
    }

    /// Record a submitted entry. Skips empty strings and consecutive
    /// duplicates. Always resets nav state — a confirm ends the session.
    /// Caps the ring at `self.capacity` with a `while`, not an `if`, so a
    /// `set_capacity` shrink of any size converges to the new cap in this
    /// one call rather than one entry per push.
    pub fn push(&mut self, entry: String) {
        self.begin_session();
        let is_duplicate = self.entries.back().is_some_and(|last| *last == entry);
        if !entry.is_empty() && !is_duplicate {
            self.entries.push_back(entry);
        }
        while self.entries.len() > self.capacity {
            self.entries.pop_front();
        }
    }

    /// Update the capacity limit. Takes effect on the *next* `push`, not
    /// immediately — matching Vim's `undolevels` semantics (see
    /// `hume_editing::history::UndoTree::set_undo_levels`): lowering the cap
    /// does not retroactively trim existing entries. Called when
    /// `history-capacity` is changed at runtime. No `cursor`/`scratch`
    /// adjustment needed here, since no entries are removed by this call —
    /// a mid-navigation `cursor` stays valid until `push`'s own `while` trim
    /// runs on the next confirm.
    pub fn set_capacity(&mut self, new_cap: usize) {
        debug_assert!(new_cap > 0, "History capacity must be non-zero");
        self.capacity = new_cap;
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

    #[allow(dead_code)]
    pub fn entries(&self) -> &VecDeque<String> {
        &self.entries
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

    /// Update the capacity of every ring — see `History::set_capacity` for
    /// why this doesn't trim. Called when the `history-capacity` setting
    /// changes at runtime.
    pub fn set_capacity(&mut self, new_cap: usize) {
        self.command.set_capacity(new_cap);
        self.search_f.set_capacity(new_cap);
        self.search_b.set_capacity(new_cap);
    }
}

// ── Unit tests ────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests;
