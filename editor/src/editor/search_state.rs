//! Search state: direction, compiled regex, match cache, and cache-invalidation keys.
//!
//! Grouped here so that all "is a search active?" invariants live in one place
//! rather than being scattered across five independent `Editor` fields.

use crate::core::history::RevisionId;
use crate::core::selection::SelectionSet;

/// Direction for `search-forward` / `search-backward` and `search-next` / `search-prev`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SearchDirection {
    Forward,
    Backward,
}

/// All search-related state, grouped to keep the "is a search active?" invariant
/// in one place instead of scattered across five independent `Editor` fields.
pub(crate) struct SearchState {
    /// Direction of the current or last search. Set when entering Search mode;
    /// persists after confirming so live search knows which way to go.
    pub direction: SearchDirection,
    /// Snapshot of selections taken when entering Search mode.
    /// Restored on cancel; discarded on confirm.
    pub pre_search_sels: Option<SelectionSet>,
    /// Whether extend mode was active when this search was started.
    ///
    /// Captured at search-enter time (before mode becomes `Search`) so live
    /// search can extend from the pre-search anchor even though `mode` is now
    /// `Search` rather than `Extend`. Cleared with the rest of `SearchState`
    /// via [`clear`].
    pub extend: bool,
    /// Compiled regex from the last confirmed (or in-progress) search pattern.
    /// `None` until a valid pattern is typed. Reused by `search-next`/`search-prev` without recompiling.
    /// Mutate only through [`set_regex`] to keep the match cache coherent.
    pub(super) regex: Option<regex_cursor::engines::meta::Regex>,
    /// All non-overlapping matches of `regex` in the current buffer,
    /// as `(start_char, end_char_inclusive)` pairs in document order.
    /// Kept up to date by `update_search_cache`; empty when `regex` is `None`.
    pub(super) matches: Vec<(usize, usize)>,
    /// Cached `(current_1based, total)` derived from `matches` and the
    /// primary cursor position. `None` when `regex` is `None`.
    pub(super) match_count: Option<(usize, usize)>,
    /// `true` when the last `search-next`/`search-prev` jump wrapped around the buffer boundary.
    /// Read by the `SearchMatches` statusline element to show a `W` prefix.
    pub(super) wrapped: bool,

    // ── Cache-invalidation keys ───────────────────────────────────────────────
    // Stored so `update_search_cache` can skip recomputation when nothing changed.
    // Both start as sentinel values that never match real state, forcing a full
    // recompute on the very first call.

    /// Buffer revision when `matches` was last computed. Changes on any edit,
    /// undo, or redo. When this differs from `doc.revision_id()`, `matches`
    /// must be recomputed.
    pub(super) cache_revision: RevisionId,
    /// Primary cursor head position when `match_count` was last computed.
    /// When this differs from the current head, `match_count` must be recomputed
    /// (but `matches` can be reused if the revision hasn't changed).
    pub(super) cache_head: usize,
}

impl Default for SearchState {
    fn default() -> Self {
        Self {
            direction: SearchDirection::Forward,
            pre_search_sels: None,
            extend: false,
            regex: None,
            matches: Vec::new(),
            match_count: None,
            wrapped: false,
            // Sentinel values: usize::MAX can never be a real revision or cursor
            // position, so the first call to update_search_cache always recomputes.
            cache_revision: RevisionId(usize::MAX),
            cache_head: usize::MAX,
        }
    }
}

impl SearchState {
    /// Clear the active search — drops the regex and flushes the highlight cache.
    /// Direction is preserved so a future `search-next`/`search-prev` or
    /// `search-forward`/`search-backward` still knows the last-used direction.
    pub fn clear(&mut self) {
        self.pre_search_sels = None;
        self.extend = false;
        self.wrapped = false;
        self.set_regex(None);
    }

    /// Replace the regex, invalidating the match-list cache.
    ///
    /// Always call this instead of writing `self.regex = …` directly so that
    /// `update_search_cache` knows the match list must be recomputed even when
    /// the buffer revision hasn't changed (e.g. a new character was typed in
    /// the search prompt).
    pub fn set_regex(&mut self, regex: Option<regex_cursor::engines::meta::Regex>) {
        self.regex = regex;
        self.matches.clear();
        self.match_count = None;
        self.wrapped = false;
        self.cache_revision = RevisionId(usize::MAX);
        self.cache_head = usize::MAX;
    }

    pub(crate) fn matches(&self) -> &[(usize, usize)] {
        &self.matches
    }

    pub(crate) fn match_count(&self) -> Option<(usize, usize)> {
        self.match_count
    }

    pub(crate) fn wrapped(&self) -> bool {
        self.wrapped
    }
}
