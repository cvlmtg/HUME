//! Search state: per-buffer, per-pane, and cross-buffer tiers.
//!
//! Three-tier split:
//! - [`SearchPattern`] + [`SearchMatches`] live on `Buffer` (shared by all panes viewing it).
//! - [`SearchCursor`] lives on [`crate::editor::pane_state::PaneBufferState`] (per-pane).
//! - The last search pattern string also lives in the `'s'` register
//!   (`RegisterSet::search_register`), independent of any buffer. It seeds
//!   a fresh buffer's compiled pattern the first time `n`/`N` runs there.
//!
//! [`SearchState`] retains only the session-level interaction field that is
//! not tied to a buffer: the current direction. Everything else (regex,
//! matches, match count) lives in the per-buffer / per-pane tier above.

pub(in crate::editor) mod ops;

use std::sync::Arc;

use hume_editing::text::BufferText;
use hume_editing::tracked::Tracked;
use hume_ops::search::{SearchDirection, compile_search_input};
use hume_rope::cluster::{ClusterRange, ClusterStart};

// ── Per-buffer types ──────────────────────────────────────────────────────────

/// Per-buffer search pattern. Stored on `Buffer`. All panes viewing this buffer share it.
///
/// `Arc<Regex>` makes the clone needed by `update_buffer_matches` a refcount bump:
/// no deep clone, no take/put-back dance. A present `SearchPattern` is always
/// fully-valid by construction (invalid regexes are rejected at compile time and
/// leave `Buffer.search_pattern = None`). [`SearchPattern::compile`] is the one
/// way to build one, so every producer gets this for free.
pub(in crate::editor) struct SearchPattern {
    pub regex: Arc<regex_cursor::engines::meta::Regex>,
    /// Raw prompt input, flag prefix included (`"m/bar"`, not `"bar"`), used
    /// as an invalidation key for `SearchMatches`, and as the source of truth
    /// for [`SearchPattern::multi`] rather than a separately-stored field:
    /// keeping it in the key over-invalidates when only `multi` toggles
    /// (identical matches, new key, one extra rescan), which is cheaper than a
    /// second key for a field that never affects the match list itself.
    pub pattern_str: String,
}

impl SearchPattern {
    /// Parse `raw`'s leading flags and compile the remaining pattern. `None`
    /// on an invalid regex; the caller leaves `Buffer.search_pattern`
    /// untouched (or `None`) rather than storing a half-valid pattern.
    pub(in crate::editor) fn compile(raw: &str) -> Option<Self> {
        let (_flags, regex) = compile_search_input(raw)?;
        Some(Self {
            regex: Arc::new(regex),
            pattern_str: raw.to_string(),
        })
    }

    /// The `m` (multi) flag, read back off the raw input. See
    /// [`SearchPattern::pattern_str`]'s doc for why this isn't a stored field.
    pub(in crate::editor) fn multi(&self) -> bool {
        hume_ops::search::parse_search_input(&self.pattern_str)
            .0
            .multi
    }
}

/// Per-buffer match cache. Stored on `Buffer`. Reads as absent once the text
/// moves past the one the matches were found in.
#[derive(Default)]
pub(in crate::editor) struct SearchMatches {
    /// The pattern searched for and every non-overlapping match of it,
    /// sorted in document order, for the text they were found in. `None` =
    /// never computed.
    found: Option<(String, Tracked<Vec<ClusterRange>>)>,
}

impl SearchMatches {
    /// The pattern and its matches, when they were found in `text`.
    pub(in crate::editor) fn found(&self, text: &BufferText) -> Option<(&str, &[ClusterRange])> {
        let (pattern, matches) = self.found.as_ref()?;
        Some((pattern.as_str(), matches.get(text)?.as_slice()))
    }

    /// The matches found in `text`, or none when they were not.
    pub(in crate::editor) fn matches(&self, text: &BufferText) -> &[ClusterRange] {
        self.found(text).map_or(&[], |(_, matches)| matches)
    }

    pub(in crate::editor) fn set(
        &mut self,
        pattern: String,
        matches: Vec<ClusterRange>,
        text: &BufferText,
    ) {
        self.found = Some((pattern, Tracked::new(matches, text)));
    }
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
    /// The pattern and primary head `match_count` was computed for, in the
    /// text it was computed against. `None` = never computed.
    pub computed_for: Option<Tracked<(String, ClusterStart)>>,
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
        Self {
            direction: SearchDirection::Forward,
        }
    }
}
