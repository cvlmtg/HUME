//! Fuzzy-picker data store. Sibling of `CompletionSession`
//! (`editor/lsp/completion.rs`), not a generalization of it — see
//! `docs/FUZZY-FINDERS.md`'s "Why not one shared session type" note. Mirrors
//! completion's `rank_scratch` reuse and reset-on-rerank patterns.
//!
//! Wired onto `EditorState.picker`; opened through `Editor::open_picker`
//! below (tests today, B4's `picker!` builtin later) and driven per-frame by
//! `Editor::sync_picker_view` and per-key by `Editor::handle_picker_key`
//! (`editor/mappings/mod.rs`).

use std::cmp::Reverse;
use std::sync::atomic::{AtomicU64, Ordering};

use steel::rvals::SteelVal;
use unicode_segmentation::UnicodeSegmentation;

use super::fuzzy::{FuzzyMatcher, FuzzyPattern};

/// One row in a picker: a display string shown to the user and an opaque
/// payload handed back to `on_select` verbatim. Rust never interprets
/// `payload` — mirrors the drawer's "rows are pre-formatted display
/// strings" contract.
pub(crate) struct PickerItem {
    pub(crate) display: String,
    pub(crate) payload: SteelVal,
}

/// Rust-side store for one open picker: items, query, ranked indices,
/// selection, scroll, and a stale-push guard token. Steel drives it through
/// `picker!`/`picker-push!`/`picker-close!` (B4); this module has no
/// Steel-facing surface of its own.
pub(crate) struct PickerSession {
    /// Append-only while the picker is open.
    items: Vec<PickerItem>,
    query: String,
    /// Ranked indices into `items`, rebuilt on every rerank.
    filtered: Vec<u32>,
    /// Reused scoring buffer — `(score, item index)` — cleared, never
    /// reallocated, across reranks.
    rank_scratch: Vec<(u32, u32)>,
    /// One instance per session; owns nucleo's reusable scoring buffers.
    matcher: FuzzyMatcher,
    /// Index into `filtered`. `0` whenever `filtered` is empty.
    selected: usize,
    /// First visible row index into `filtered`.
    scroll: usize,
    on_select: SteelVal,
    /// Stale-push guard: `push` is a no-op unless the caller's token matches.
    #[allow(dead_code)] // read by `push`/`token`, both awaiting B4's picker!/picker-push!
    token: u64,
}

#[allow(dead_code)] // read by `PickerSession::new`, awaiting B4's picker! production caller
static NEXT_TOKEN: AtomicU64 = AtomicU64::new(1);

impl PickerSession {
    /// Opens empty — the caller's initial item list (from `picker!`) arrives
    /// through the same `push` path as any later batch, matching B6's "open
    /// empty, then attach source" composition.
    #[allow(dead_code)] // production caller is B4's `picker!` builtin
    pub(crate) fn new(on_select: SteelVal) -> Self {
        Self {
            items: Vec::new(),
            query: String::new(),
            filtered: Vec::new(),
            rank_scratch: Vec::new(),
            matcher: FuzzyMatcher::new(),
            selected: 0,
            scroll: 0,
            on_select,
            token: NEXT_TOKEN.fetch_add(1, Ordering::Relaxed),
        }
    }

    #[allow(dead_code)] // production caller is B4's `picker-push!` builtin
    pub(crate) fn token(&self) -> u64 {
        self.token
    }

    /// Appends `items` and reranks, but only if `token` matches this
    /// session's token. A mismatch is expected-normal (a late batch from a
    /// picker the user already closed or replaced) — silent no-op, not an
    /// error. Returns whether the push was applied.
    #[allow(dead_code)] // production caller is B4's `picker-push!` builtin / B5's spawned source
    pub(crate) fn push(&mut self, token: u64, items: Vec<PickerItem>) -> bool {
        if token != self.token {
            return false;
        }
        self.items.extend(items);
        self.rerank();
        true
    }

    /// Appends one `char` to the query and reranks. Key events deliver
    /// printable input one `char` at a time, including combining marks,
    /// which simply extend the trailing grapheme cluster.
    pub(crate) fn insert_char(&mut self, ch: char) {
        self.query.push(ch);
        self.rerank();
    }

    /// Removes the trailing grapheme cluster (not merely the last `char`) so
    /// that precomposed accents and ZWJ/modifier emoji sequences are deleted
    /// as one unit, then reranks. Returns `false` without effect when the
    /// query is already empty, so callers can give backspace-on-empty its
    /// own meaning (mirrors the minibuffer's `EmptiedByBackspace` /
    /// `BackspaceOnEmpty` distinction).
    pub(crate) fn pop_grapheme(&mut self) -> bool {
        let boundary = match self.query.grapheme_indices(true).next_back() {
            Some((i, _)) => i,
            None => return false,
        };
        self.query.truncate(boundary);
        self.rerank();
        true
    }

    /// Replaces the query wholesale and reranks.
    #[allow(dead_code)] // production caller is B4/B5's live-requery replace path (Q-B5)
    pub(crate) fn set_query(&mut self, query: String) {
        self.query = query;
        self.rerank();
    }

    /// Moves `selected` by `delta`, saturating at both ends of `filtered`
    /// with no wraparound, then clamps `scroll` so `selected` stays inside
    /// the `visible_rows`-tall window (same formula as
    /// `clamp_drawer_scroll`). No-op when `filtered` is empty or
    /// `visible_rows` is `0`. Page moves are simply `delta = ±visible_rows`.
    pub(crate) fn move_selection(&mut self, delta: isize, visible_rows: usize) {
        if self.filtered.is_empty() {
            return;
        }
        let max = self.filtered.len() - 1;
        self.selected = (self.selected as isize + delta).clamp(0, max as isize) as usize;

        if visible_rows == 0 {
            return;
        }
        if self.selected >= self.scroll + visible_rows {
            self.scroll = self.selected + 1 - visible_rows;
        } else if self.selected < self.scroll {
            self.scroll = self.selected;
        }
        debug_assert!(self.scroll <= self.selected && self.selected < self.scroll + visible_rows);
    }

    pub(crate) fn query(&self) -> &str {
        &self.query
    }

    pub(crate) fn selected(&self) -> usize {
        self.selected
    }

    pub(crate) fn scroll(&self) -> usize {
        self.scroll
    }

    /// Number of items currently ranked (i.e. matching the query).
    pub(crate) fn matched_len(&self) -> usize {
        self.filtered.len()
    }

    /// Total number of items ever pushed, regardless of the current query.
    pub(crate) fn total_len(&self) -> usize {
        self.items.len()
    }

    /// Display strings of up to `rows` items starting at `scroll`, in ranked
    /// order — the window B3 paints. The selected row's on-screen position
    /// is `selected - scroll`.
    pub(crate) fn window(&self, rows: usize) -> impl Iterator<Item = &str> + '_ {
        self.filtered
            .iter()
            .skip(self.scroll)
            .take(rows)
            .map(|&idx| self.items[idx as usize].display.as_str())
    }

    /// Payload of the currently selected item, or `None` when nothing
    /// matches.
    pub(crate) fn selected_payload(&self) -> Option<&SteelVal> {
        self.filtered
            .get(self.selected)
            .map(|&idx| &self.items[idx as usize].payload)
    }

    /// Cheap `Rc` clone — B3/B4 fire this via `queue_steel_call` on
    /// accept/dismiss; the store itself never invokes it.
    pub(crate) fn on_select(&self) -> &SteelVal {
        &self.on_select
    }

    /// The only place ranking happens; every mutator above routes through
    /// this. Resets `selected`/`scroll` to `0` on every rerank — per the
    /// design doc, a stale selection surviving a rerank is worse than
    /// landing back on the top row.
    fn rerank(&mut self) {
        if self.query.is_empty() {
            // Insertion order by construction — avoids relying on nucleo's
            // (undocumented) all-equal-score behavior on an empty pattern,
            // and skips scoring entirely on the dominant streaming-ingest
            // path (empty query while a spawned source drains batches).
            self.filtered.clear();
            self.filtered.extend(0..self.items.len() as u32);
        } else {
            let pattern = FuzzyPattern::parse(&self.query);
            self.rank_scratch.clear();
            for (idx, item) in self.items.iter().enumerate() {
                if let Some(score) = self.matcher.score(&pattern, &item.display) {
                    self.rank_scratch.push((score, idx as u32));
                }
            }
            // Score descending, tie-break by ascending insertion index —
            // the key is unique (index), so the ordering is deterministic
            // despite `sort_unstable_by_key`, and ties preserve the
            // streamed-source's relative order.
            self.rank_scratch
                .sort_unstable_by_key(|&(score, idx)| (Reverse(score), idx));
            self.filtered.clear();
            self.filtered
                .extend(self.rank_scratch.iter().map(|&(_, idx)| idx));
        }
        debug_assert!(self.filtered.len() <= self.items.len());
        self.selected = 0;
        self.scroll = 0;
    }
}

impl super::Editor {
    /// Single open chokepoint for the picker — tests drive it directly
    /// today, B4's `picker!` builtin calls it once that Steel surface
    /// exists. Q-B7 (`docs/FUZZY-FINDERS.md`): allowed from any mode, but
    /// one modal owner at a time, so opening a picker always closes any
    /// live completion session first. Replacing an already-open picker
    /// fires *its* `on_select` with `#f` before installing the new one —
    /// the exactly-once callback contract must never have a window where a
    /// session can be silently dropped without firing.
    #[allow(dead_code)] // production caller is B4's `picker!` builtin
    pub(crate) fn open_picker(&mut self, session: PickerSession) {
        self.clear_lsp_completion();
        if let Some(old) = self.state.picker.take() {
            let callback = old.on_select().clone();
            self.queue_steel_call(callback, vec![SteelVal::BoolV(false)]);
        }
        self.state.picker = Some(session);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(display: &str) -> PickerItem {
        PickerItem {
            display: display.to_string(),
            payload: SteelVal::StringV(display.into()),
        }
    }

    fn items(displays: &[&str]) -> Vec<PickerItem> {
        displays.iter().map(|d| item(d)).collect()
    }

    fn dummy_on_select() -> SteelVal {
        SteelVal::BoolV(false)
    }

    fn open() -> PickerSession {
        PickerSession::new(dummy_on_select())
    }

    fn payload_str(v: &SteelVal) -> &str {
        match v {
            SteelVal::StringV(s) => s.as_str(),
            other => panic!("expected StringV payload, got {other:?}"),
        }
    }

    fn window_vec(s: &PickerSession, rows: usize) -> Vec<&str> {
        s.window(rows).collect()
    }

    #[test]
    fn new_session_is_empty() {
        let s = open();
        assert_eq!(s.total_len(), 0);
        assert_eq!(s.matched_len(), 0);
        assert_eq!(s.selected(), 0);
        assert!(window_vec(&s, 10).is_empty());
    }

    #[test]
    fn two_sessions_get_distinct_tokens() {
        let s1 = open();
        let s2 = open();
        assert_ne!(s1.token(), s2.token());
    }

    #[test]
    fn push_with_empty_query_keeps_insertion_order() {
        let mut s = open();
        let token = s.token();
        assert!(s.push(token, items(&["b", "a", "c"])));
        assert_eq!(window_vec(&s, 10), vec!["b", "a", "c"]);
    }

    #[test]
    fn second_push_appends_after_first() {
        let mut s = open();
        let token = s.token();
        s.push(token, items(&["a", "b"]));
        s.push(token, items(&["c", "d"]));
        assert_eq!(window_vec(&s, 10), vec!["a", "b", "c", "d"]);
    }

    #[test]
    fn stale_token_push_is_rejected() {
        let mut s = open();
        let real_token = s.token();
        let wrong_token = real_token.wrapping_add(1);
        assert!(!s.push(wrong_token, items(&["x"])));
        assert_eq!(s.total_len(), 0);
        assert!(window_vec(&s, 10).is_empty());
        assert_eq!(s.selected(), 0);
    }

    #[test]
    fn set_query_filters_non_matches() {
        let mut s = open();
        let token = s.token();
        s.push(token, items(&["foo", "bar"]));
        s.set_query("f".to_string());
        assert_eq!(window_vec(&s, 10), vec!["foo"]);
    }

    #[test]
    fn better_match_ranks_first_regardless_of_insertion() {
        let mut s = open();
        let token = s.token();
        // Scattered subsequence pushed before the boundary match.
        s.push(token, items(&["fxxbxx", "foo/bar"]));
        s.set_query("fb".to_string());
        assert_eq!(window_vec(&s, 10), vec!["foo/bar", "fxxbxx"]);
    }

    #[test]
    fn equal_scores_tie_break_by_insertion_order() {
        let mut s = open();
        let token = s.token();
        // Two score tiers (lower-scoring "fxxbxx" scattered matches, then
        // higher-scoring "foo/bar" boundary matches), pushed low-score tier
        // first — so the pre-sort array is not already in the target
        // (descending-score) order and the sort must do genuine rearranging
        // work, not just detect an already-sorted/already-reversed run and
        // leave it untouched. Within each equal-score tier, payload order
        // must still match insertion order.
        const TIER: u32 = 32;
        let mut tagged = Vec::new();
        for i in 0..TIER {
            tagged.push(PickerItem {
                display: "fxxbxx".to_string(),
                payload: SteelVal::StringV(format!("low{i}").into()),
            });
        }
        for i in 0..TIER {
            tagged.push(PickerItem {
                display: "foo/bar".to_string(),
                payload: SteelVal::StringV(format!("high{i}").into()),
            });
        }
        s.push(token, tagged);
        s.set_query("fb".to_string());
        assert_eq!(s.matched_len(), (2 * TIER) as usize);
        for i in 0..TIER {
            let payload = s.selected_payload().expect("has a match");
            assert_eq!(payload_str(payload), format!("high{i}"));
            s.move_selection(1, (2 * TIER) as usize);
        }
        for i in 0..TIER {
            let payload = s.selected_payload().expect("has a match");
            assert_eq!(payload_str(payload), format!("low{i}"));
            s.move_selection(1, (2 * TIER) as usize);
        }
    }

    #[test]
    fn push_rerank_resets_selection_and_scroll() {
        let mut s = open();
        let token = s.token();
        s.push(token, items(&["a", "b", "c", "d", "e"]));
        s.move_selection(3, 2);
        assert_ne!(s.selected(), 0);
        s.push(token, items(&["f"]));
        assert_eq!(s.selected(), 0);
        assert_eq!(s.scroll(), 0);
    }

    #[test]
    fn set_query_resets_selection_and_scroll() {
        let mut s = open();
        let token = s.token();
        s.push(token, items(&["apple", "banana", "cherry", "date"]));
        s.move_selection(2, 2);
        assert_ne!(s.selected(), 0);
        s.set_query("a".to_string());
        assert_eq!(s.selected(), 0);
        assert_eq!(s.scroll(), 0);
    }

    #[test]
    fn widening_query_restores_matches() {
        let mut s = open();
        let token = s.token();
        s.push(token, items(&["foo", "bar"]));
        s.insert_char('z');
        assert_eq!(s.matched_len(), 0);
        assert!(s.pop_grapheme());
        assert_eq!(s.matched_len(), 2);
        assert!(!s.pop_grapheme()); // query now empty; further pop is a no-op
    }

    #[test]
    fn pop_grapheme_removes_full_cluster() {
        let mut s = open();
        // "e" + combining acute accent (U+0301) forms one grapheme cluster.
        s.insert_char('e');
        s.insert_char('\u{0301}');
        assert_eq!(s.query(), "e\u{0301}");
        assert!(s.pop_grapheme());
        assert_eq!(s.query(), "");
        assert!(s.query().is_char_boundary(0));

        // ZWJ emoji sequence: family emoji built from 4 code points joined
        // by ZWJ — one pop_grapheme must remove the whole cluster.
        for ch in "👨‍👩‍👧‍👦".chars() {
            s.insert_char(ch);
        }
        assert!(s.pop_grapheme());
        assert_eq!(s.query(), "");
    }

    #[test]
    fn move_selection_is_bounded_no_wrap() {
        let mut s = open();
        let token = s.token();
        s.push(token, items(&["a", "b", "c"]));
        s.move_selection(-5, 10);
        assert_eq!(s.selected(), 0);
        s.move_selection(10, 10);
        assert_eq!(s.selected(), 2);
        // Page move past the end stays clamped.
        s.move_selection(3, 10);
        assert_eq!(s.selected(), 2);
    }

    #[test]
    fn move_selection_scrolls_to_keep_selected_visible() {
        let mut s = open();
        let token = s.token();
        s.push(
            token,
            items(&["0", "1", "2", "3", "4", "5", "6", "7", "8", "9"]),
        );
        s.move_selection(5, 3);
        assert_eq!(s.selected(), 5);
        assert_eq!(s.scroll(), 3); // 5 + 1 - 3
        s.move_selection(-4, 3);
        assert_eq!(s.selected(), 1);
        assert_eq!(s.scroll(), 1);
        // visible_rows == 0 is a documented no-op on the scroll clamp.
        let scroll_before = s.scroll();
        s.move_selection(1, 0);
        assert_eq!(s.scroll(), scroll_before);
    }

    #[test]
    fn empty_filter_result_is_safe() {
        let mut s = open();
        let token = s.token();
        s.push(token, items(&["foo", "bar"]));
        s.set_query("zzz".to_string());
        assert_eq!(s.selected(), 0);
        s.move_selection(5, 3); // must not panic
        assert_eq!(s.selected(), 0);
        assert!(window_vec(&s, 10).is_empty());
        assert!(s.selected_payload().is_none());
    }

    #[test]
    fn window_respects_scroll_and_rows() {
        let mut s = open();
        let token = s.token();
        s.push(
            token,
            items(&["0", "1", "2", "3", "4", "5", "6", "7", "8", "9"]),
        );
        s.move_selection(6, 3); // scroll becomes 4
        assert_eq!(s.scroll(), 4);
        assert_eq!(window_vec(&s, 3), vec!["4", "5", "6"]);
    }

    #[test]
    fn selected_payload_returns_top_ranked_item() {
        let mut s = open();
        let token = s.token();
        s.push(token, items(&["fxxbxx", "foo/bar"]));
        s.set_query("fb".to_string());
        assert_eq!(
            payload_str(s.selected_payload().expect("has a match")),
            "foo/bar"
        );
        s.move_selection(1, 10);
        assert_eq!(
            payload_str(s.selected_payload().expect("has a match")),
            "fxxbxx"
        );
    }
}
