//! Free functions for search-state operations.
//!
//! Extracted from `impl Editor` so callers can hold disjoint borrows of other
//! `Editor` fields while updating search state.
//!
//! `update_buffer_matches` uses direct `Buffer` field access to compare the cache
//! key by reference, avoiding `pattern_str.clone()` on the hot cache-hit path.
//! Only the cache-miss write path still clones (unavoidable: the cache must own
//! its key string).
//!
//! `update_pane_cursor` takes `buffers` and `pane_state` as separate parameters
//! so the search-matches reference (`&buffers`) and the cursor write
//! (`&mut pane_state`) are disjoint, eliminating the two-block borrow dance.

use std::sync::Arc;

use slotmap::SecondaryMap;

use hume_engine::pipeline::{BufferId, PaneId};

#[cfg(test)]
use super::SearchPattern;
use super::{SearchCursor, SearchMatches};
use crate::editor::Editor;
use crate::editor::buffer::store::BufferStore;
use crate::editor::pane_state::PaneBufferState;
use crate::ops::search::{find_all_matches, search_match_info};

/// Clear the active search state for buffer `bid`: drop the pattern,
/// reset the match cache, and reset every pane's search cursor.
pub(crate) fn clear_buffer_search(
    buffers: &mut BufferStore,
    pane_state: &mut SecondaryMap<PaneId, SecondaryMap<BufferId, PaneBufferState>>,
    bid: BufferId,
) {
    let buf = buffers.get_mut(bid);
    buf.search_pattern = None;
    buf.search_matches = SearchMatches::default();
    for buf_map in pane_state.values_mut() {
        if let Some(state) = buf_map.get_mut(bid) {
            state.search_cursor = SearchCursor::default();
        }
    }
}

/// Recompute the match list for `bid` if the pattern or revision changed.
///
/// No-op when no search is active. Cache check uses direct field access
/// to compare the pattern string by reference, avoiding `pattern_str.clone()`
/// on the common cache-hit path.
pub(crate) fn update_buffer_matches(buffers: &mut BufferStore, bid: BufferId) {
    let buf = buffers.get_mut(bid);

    let Some(sp) = buf.search_pattern.as_ref() else {
        return;
    };
    let revision = buf.revision_id();

    // Compare by reference — no clone on the hot cache-hit path.
    if buf
        .search_matches
        .cache
        .as_ref()
        .is_some_and(|(r, s)| *r == revision && s == &sp.pattern_str)
    {
        return;
    }

    // Cache miss: capture what we need, then release the search_pattern borrow
    // so we can write to search_matches.
    let regex = Arc::clone(&sp.regex);
    let pattern_str = sp.pattern_str.clone();
    // sp last used above — NLL ends the buf.search_pattern borrow here.

    let matches = {
        let text = buf.text();
        find_all_matches(text, &regex)
    };
    // text borrow ended — buf.search_matches can now be written.

    buf.search_matches.matches = matches;
    buf.search_matches.cache = Some((revision, pattern_str));
}

/// Recompute `pane_state[pid][bid].search_cursor.match_count` if stale.
///
/// Takes `buffers: &BufferStore` and `pane_state: &mut ...` as separate
/// parameters — the match-list reference and the cursor write are disjoint,
/// so no intermediate owned variable is needed.
pub(crate) fn update_pane_cursor(
    buffers: &BufferStore,
    pane_state: &mut SecondaryMap<PaneId, SecondaryMap<BufferId, PaneBufferState>>,
    pid: PaneId,
    bid: BufferId,
) {
    let head = pane_state[pid][bid].selections.primary().head();
    let sm = &buffers.get(bid).search_matches;
    let cur = &pane_state[pid][bid].search_cursor;

    if cur.cache_head == Some(head) && cur.cache_matches == sm.cache {
        return;
    }
    if sm.cache.is_none() {
        return;
    }
    let count = search_match_info(&sm.matches, head);
    // sm borrows from buffers; cursor borrows from pane_state — disjoint params.
    let cursor = &mut pane_state[pid][bid].search_cursor;
    cursor.match_count = Some(count);
    cursor.cache_head = Some(head);
    cursor.cache_matches = sm.cache.clone();
}

/// Convenience: run `update_buffer_matches` + `update_pane_cursor` for a
/// specific pane/buffer pair.
pub(crate) fn sync_search_cache(
    buffers: &mut BufferStore,
    pane_state: &mut SecondaryMap<PaneId, SecondaryMap<BufferId, PaneBufferState>>,
    pid: PaneId,
    bid: BufferId,
) {
    update_buffer_matches(buffers, bid);
    update_pane_cursor(buffers, pane_state, pid, bid);
}

impl Editor {
    /// Accessor for the focused buffer's active search pattern (used in tests).
    #[cfg(test)]
    pub(crate) fn search_pattern(&self) -> Option<&SearchPattern> {
        self.state
            .buffers
            .get(self.focused_buffer_id())
            .search_pattern
            .as_ref()
    }

    /// Accessor for the focused buffer's match cache.
    #[cfg(test)]
    pub(crate) fn search_matches(&self) -> &SearchMatches {
        &self
            .state
            .buffers
            .get(self.focused_buffer_id())
            .search_matches
    }

    /// Accessor for the focused pane's search cursor (match count, wrapped flag).
    pub(crate) fn current_search_cursor(&self) -> &SearchCursor {
        &self.state.panes.state[self.state.focused_pane_id][self.focused_buffer_id()].search_cursor
    }

    /// Recompute the match list and pane search cursor for the focused buffer,
    /// if stale. No-op when no search is active.
    pub(crate) fn sync_search_cache(&mut self) {
        let pid = self.state.focused_pane_id;
        let bid = self.focused_buffer_id();
        sync_search_cache(&mut self.state.buffers, &mut self.state.panes.state, pid, bid);
    }
}
