//! Minibuffer history: bounded, in-memory recall for the `:`, `/`, `?`, `|`
//! and `⫽` prompts.
//!
//! Each prompt gets its own [`History`] ring (oldest-first [`VecDeque`]).
//! The rings are grouped in [`HistoryStore`], which lives on `Editor` and is
//! keyed by [`HistoryKind`]. A `MiniBuffer` names its ring as it opens,
//! records in it, and holds its own [`HistoryWalk`] through it, so a ring
//! holds only what was typed and confirmed at its prompt and a walk ends
//! with the prompt.
//!
use std::collections::VecDeque;

// ── HistoryKind ───────────────────────────────────────────────────────────────

/// Which minibuffer prompt a history ring belongs to.
///
/// An explicit ENUM (rather than a raw `char`) keeps the variant set closed,
/// exhaustively matched, and serializable as the key of a saved session's
/// rings.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub(in crate::editor) enum HistoryKind {
    /// `:` command-mode prompt.
    Command,
    /// `/` forward-search prompt.
    SearchForward,
    /// `?` backward-search prompt.
    SearchBackward,
    /// `|` shell-command prompt.
    Pipe,
    /// `⫽` sift-within prompt.
    Sift,
}

impl HistoryKind {
    /// Every kind. A new kind fails to compile in the match below; it then
    /// joins the list, whose length `entries_by_kind`'s array repeats.
    pub(in crate::editor) const ALL: [Self; 5] = {
        let all = [
            Self::Command,
            Self::SearchForward,
            Self::SearchBackward,
            Self::Pipe,
            Self::Sift,
        ];
        match all[0] {
            Self::Command
            | Self::SearchForward
            | Self::SearchBackward
            | Self::Pipe
            | Self::Sift => all,
        }
    };
}

/// A single bounded history ring.
///
/// Entries are stored oldest-first; `back()` is always the most recent.
#[derive(Debug)]
pub(in crate::editor) struct History {
    entries: VecDeque<String>,
    capacity: usize,
}

/// One prompt's walk through its ring, from the first Up until an edit, a
/// Down past the newest match, or the prompt closing.
#[derive(Debug, Clone)]
pub(super) struct HistoryWalk {
    /// The ring index of the entry the prompt shows.
    cursor: usize,
    /// The line as it was at the first Up: the prefix every step filters
    /// by, restored by Down past the newest match.
    scratch: String,
}

impl History {
    /// `capacity == 0` is a silent black hole (every `push` immediately
    /// evicts what it just pushed) rather than a documented "unlimited",
    /// unlike `undo-levels`, where `0` means exactly that. The settings
    /// parser (`UsizeNonzero`) already rejects `0` for `prompt-history-levels`
    /// before it can reach here; this just makes the trap loud if that
    /// guard is ever bypassed (a test constructing a `History` directly).
    pub(in crate::editor) fn new(capacity: usize) -> Self {
        debug_assert!(capacity > 0, "History capacity must be non-zero");
        Self {
            entries: VecDeque::new(),
            capacity,
        }
    }

    /// Record a submitted entry. Skips blank strings and consecutive
    /// duplicates. Caps the ring at `self.capacity` with a `while`, not an `if`, so a
    /// `set_capacity` shrink of any size converges to the new cap in this
    /// one call rather than one entry per push.
    pub(in crate::editor) fn push(&mut self, entry: String) {
        let is_duplicate = self.entries.back().is_some_and(|last| *last == entry);
        if !entry.trim().is_empty() && !is_duplicate {
            self.entries.push_back(entry);
        }
        while self.entries.len() > self.capacity {
            self.entries.pop_front();
        }
    }

    /// Update the capacity limit. Takes effect on the *next* `push`, not
    /// immediately, matching Vim's `undolevels` semantics (see
    /// `hume_editing::history::UndoTree::set_undo_levels`): lowering the cap
    /// does not retroactively trim existing entries. Called when
    /// `prompt-history-levels` is changed at runtime.
    pub(in crate::editor) fn set_capacity(&mut self, new_cap: usize) {
        debug_assert!(new_cap > 0, "History capacity must be non-zero");
        self.capacity = new_cap;
    }

    /// Walk one step older, restricted to entries whose text starts with the
    /// walk's prefix: `current` when `walk` is `None`, which starts the walk.
    /// An empty prefix matches every entry. Returns the entry to install, or
    /// `None` if no older match exists (`walk` unchanged).
    pub(super) fn prev(&self, walk: &mut Option<HistoryWalk>, current: &str) -> Option<String> {
        let (prefix, scan_from) = match walk {
            Some(w) => (w.scratch.as_str(), w.cursor),
            None => (current, self.entries.len()),
        };
        let idx = (0..scan_from)
            .rev()
            .find(|&idx| self.entries[idx].starts_with(prefix))?;
        match walk {
            Some(w) => w.cursor = idx,
            None => {
                *walk = Some(HistoryWalk {
                    cursor: idx,
                    scratch: current.to_owned(),
                });
            }
        }
        Some(self.entries[idx].clone())
    }

    /// Walk one step newer within the prefix match set. Past the newest match,
    /// ends the walk and returns its stashed prefix text. `None` if `walk` is.
    pub(super) fn next(&self, walk: &mut Option<HistoryWalk>) -> Option<String> {
        let w = walk.as_mut()?;
        let newer = ((w.cursor + 1)..self.entries.len())
            .find(|&idx| self.entries[idx].starts_with(&w.scratch));
        match newer {
            Some(idx) => {
                w.cursor = idx;
                Some(self.entries[idx].clone())
            }
            None => walk.take().map(|w| w.scratch),
        }
    }

    #[cfg(test)]
    pub(in crate::editor) fn entries(&self) -> &VecDeque<String> {
        &self.entries
    }
}

/// Container for all minibuffer history rings. A single instance lives on
/// `Editor`; rings are accessed by [`HistoryKind`].
#[derive(Debug)]
pub(crate) struct HistoryStore {
    command: History,
    search_f: History,
    search_b: History,
    pipe: History,
    sift: History,
}

impl HistoryStore {
    pub(in crate::editor) fn new(capacity: usize) -> Self {
        Self {
            command: History::new(capacity),
            search_f: History::new(capacity),
            search_b: History::new(capacity),
            pipe: History::new(capacity),
            sift: History::new(capacity),
        }
    }

    pub(in crate::editor) fn get(&self, kind: HistoryKind) -> &History {
        match kind {
            HistoryKind::Command => &self.command,
            HistoryKind::SearchForward => &self.search_f,
            HistoryKind::SearchBackward => &self.search_b,
            HistoryKind::Pipe => &self.pipe,
            HistoryKind::Sift => &self.sift,
        }
    }

    pub(in crate::editor) fn get_mut(&mut self, kind: HistoryKind) -> &mut History {
        match kind {
            HistoryKind::Command => &mut self.command,
            HistoryKind::SearchForward => &mut self.search_f,
            HistoryKind::SearchBackward => &mut self.search_b,
            HistoryKind::Pipe => &mut self.pipe,
            HistoryKind::Sift => &mut self.sift,
        }
    }

    /// Every ring. The pattern names each field, so a new ring is a compile
    /// error here until it is listed.
    fn rings_mut(&mut self) -> [&mut History; 5] {
        let Self {
            command,
            search_f,
            search_b,
            pipe,
            sift,
        } = self;
        [command, search_f, search_b, pipe, sift]
    }

    /// Every ring's entries, oldest first, with the kind it belongs to.
    pub(in crate::editor) fn entries_by_kind(&self) -> [(HistoryKind, &VecDeque<String>); 5] {
        HistoryKind::ALL.map(|kind| (kind, &self.get(kind).entries))
    }

    /// Update the capacity of every ring; see `History::set_capacity` for
    /// why this doesn't trim. Called when the `prompt-history-levels` setting
    /// changes at runtime.
    pub(in crate::editor) fn set_capacity(&mut self, new_cap: usize) {
        for ring in self.rings_mut() {
            ring.set_capacity(new_cap);
        }
    }
}

#[cfg(test)]
mod tests;
