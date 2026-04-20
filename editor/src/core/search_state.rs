//! Search state: per-buffer and per-pane tiers.
//!
//! Three-tier split:
//! - [`SearchPattern`] + [`SearchMatches`] live on `Buffer` (shared by all panes viewing it).
//! - [`SearchCursor`] lives on [`crate::editor::pane_state::PaneBufferState`] (per-pane).
//!
//! This file also retains the legacy [`SearchState`] monolith while the migration
//! from the single-buffer, single-pane model is in progress. `SearchState` will
//! be removed once all call sites migrate to the three-tier accessors.

use std::sync::Arc;

use crate::core::history::RevisionId;
use crate::core::selection::SelectionSet;

/// Direction for `search-forward` / `search-backward` and `search-next` / `search-prev`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SearchDirection {
    Forward,
    Backward,
}

// ── New three-tier types ──────────────────────────────────────────────────────

/// Per-buffer search pattern. Stored on `Buffer`. All panes viewing this buffer share it.
///
/// `Arc<Regex>` makes the clone needed by `update_buffer_matches` a refcount bump —
/// no deep clone, no take/put-back dance. A present `SearchPattern` is always
/// fully-valid by construction (invalid regexes are rejected at compile time and
/// leave `Buffer.search_pattern = None`).
#[allow(dead_code)] // Phase 5: replaces SearchState on Buffer
pub(crate) struct SearchPattern {
    pub direction: SearchDirection,
    pub regex: Arc<regex_cursor::engines::meta::Regex>,
    /// Raw pattern string — used as an invalidation key for `SearchMatches`.
    pub pattern_str: String,
}

/// Per-buffer match cache. Stored on `Buffer`. Invalidated by revision or pattern change.
#[derive(Default)]
#[allow(dead_code)] // Phase 5: replaces SearchState.matches on Buffer
pub(crate) struct SearchMatches {
    /// All non-overlapping matches as `(start_char, end_char_inclusive)` pairs,
    /// sorted in document order.
    pub matches: Vec<(usize, usize)>,
    /// Buffer revision when `matches` was last computed. `None` = never computed.
    pub cache_revision: Option<RevisionId>,
    /// Pattern string when `matches` was last computed. `None` = never computed.
    pub cache_pattern: Option<String>,
}

// ── Legacy monolith (kept during migration) ───────────────────────────────────

/// All search-related state, grouped to keep the "is a search active?" invariant
/// in one place instead of scattered across five independent `Editor` fields.
///
/// **Deprecated:** being replaced by the three-tier split above. Will be removed
/// once all callers migrate to `SearchPattern` / `SearchMatches` / `SearchCursor`.
pub(crate) struct SearchState {
    /// Direction of the current or last search.
    pub direction: SearchDirection,
    /// Snapshot of selections taken when entering Search mode.
    pub pre_search_sels: Option<SelectionSet>,
    /// Whether extend mode was active when this search was started.
    pub extend: bool,
    /// Compiled regex from the last confirmed (or in-progress) search pattern.
    pub(crate) regex: Option<regex_cursor::engines::meta::Regex>,
    /// All non-overlapping matches in the current buffer.
    pub(crate) matches: Vec<(usize, usize)>,
    /// Cached `(current_1based, total)`.
    pub(crate) match_count: Option<(usize, usize)>,
    /// Whether the last search-next/prev wrapped around.
    pub(crate) wrapped: bool,

    pub(crate) cache_revision: RevisionId,
    pub(crate) cache_head: usize,
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
            cache_revision: RevisionId(usize::MAX),
            cache_head: usize::MAX,
        }
    }
}

impl SearchState {
    /// Clear the active search.
    pub fn clear(&mut self) {
        self.pre_search_sels = None;
        self.extend = false;
        self.wrapped = false;
        self.set_regex(None);
    }

    /// Replace the regex, invalidating the match-list cache.
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
