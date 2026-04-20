//! Search state: per-buffer and per-pane tiers.
//!
//! Three-tier split:
//! - [`SearchPattern`] + [`SearchMatches`] live on `Buffer` (shared by all panes viewing it).
//! - [`SearchCursor`] lives on [`crate::editor::pane_state::PaneBufferState`] (per-pane).
//!
//! [`SearchState`] retains only the session-level interaction fields that are
//! not tied to a buffer: the current direction and whether the search was
//! started from Extend mode. Everything else (regex, matches, match count)
//! lives in the per-buffer / per-pane tier.

use std::sync::Arc;

use crate::core::history::RevisionId;

/// Direction for `search-forward` / `search-backward` and `search-next` / `search-prev`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SearchDirection {
    Forward,
    Backward,
}

// ── Per-buffer types ──────────────────────────────────────────────────────────

/// Per-buffer search pattern. Stored on `Buffer`. All panes viewing this buffer share it.
///
/// `Arc<Regex>` makes the clone needed by `update_buffer_matches` a refcount bump —
/// no deep clone, no take/put-back dance. A present `SearchPattern` is always
/// fully-valid by construction (invalid regexes are rejected at compile time and
/// leave `Buffer.search_pattern = None`).
pub(crate) struct SearchPattern {
    #[allow(dead_code)] // stored for completeness; session direction lives on SearchState
    pub direction: SearchDirection,
    pub regex: Arc<regex_cursor::engines::meta::Regex>,
    /// Raw pattern string — used as an invalidation key for `SearchMatches`.
    pub pattern_str: String,
}

/// Per-buffer match cache. Stored on `Buffer`. Invalidated by revision or pattern change.
#[derive(Default)]
pub(crate) struct SearchMatches {
    /// All non-overlapping matches as `(start_char, end_char_inclusive)` pairs,
    /// sorted in document order.
    pub matches: Vec<(usize, usize)>,
    /// Buffer revision when `matches` was last computed. `None` = never computed.
    pub cache_revision: Option<RevisionId>,
    /// Pattern string when `matches` was last computed. `None` = never computed.
    pub cache_pattern: Option<String>,
}

// ── Per-(pane, buffer) type ───────────────────────────────────────────────────

/// Per-(pane, buffer) cursor through the buffer's shared match list.
///
/// `SearchMatches` (on `Buffer`) holds the full list; `SearchCursor` holds this
/// pane's position within that list plus the cache keys needed to detect staleness.
#[derive(Default)]
pub(crate) struct SearchCursor {
    /// `(current_1based_idx, total)` derived from `SearchMatches` + primary head.
    /// `None` when no search is active.
    pub match_count: Option<(usize, usize)>,
    /// `true` when the last search-next/prev jump wrapped around the buffer boundary.
    pub wrapped: bool,
    /// Head position when `match_count` was last computed. `None` = never computed.
    pub cache_head: Option<usize>,
    /// `SearchMatches::cache_revision` value when `match_count` was last computed.
    pub cache_matches_rev: Option<RevisionId>,
    /// `SearchMatches::cache_pattern` value when `match_count` was last computed.
    pub cache_matches_pattern: Option<String>,
}

// ── Session-level interaction state ──────────────────────────────────────────

/// Session-level search state: direction only.
///
/// All other search state (regex, matches, match count) lives in the
/// per-buffer / per-pane tier: `Buffer.search_pattern`, `Buffer.search_matches`,
/// and `PaneBufferState.search_cursor`.
pub(crate) struct SearchState {
    /// Direction of the current or last search.
    pub direction: SearchDirection,
}

impl Default for SearchState {
    fn default() -> Self {
        Self { direction: SearchDirection::Forward }
    }
}
