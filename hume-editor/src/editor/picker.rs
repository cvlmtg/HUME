//! Fuzzy-picker data store. Sibling of `CompletionSession`
//! (`editor/lsp/completion.rs`), not a generalization of it: item shape,
//! query origin, accept semantics, lifetime, scale, and scroll model all
//! differ between the two, so a shared abstract core would be parameterized
//! over six axes for two call sites — not worth it unless the bodies
//! converge later. Mirrors completion's `rank_scratch` reuse and
//! reset-on-rerank patterns.
//!
//! Wired onto `EditorState.picker`; opened through the [`open_picker`] free
//! fn below (Steel's `picker!` builtin, `hume-scripting`'s `ui::picker`) and
//! driven per-frame by `Editor::sync_picker_view` and per-key by
//! `Editor::handle_picker_key` (`editor/mappings/mod.rs`).

use std::cmp::Reverse;
use std::sync::atomic::{AtomicU64, Ordering};

use hume_platform::process::line_source::SpawnedLineSource;
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
/// `picker!`/`picker-push!`/`picker-close!`; this module has no
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
    /// Label painted before the query in the input line, e.g. `"files: "`.
    /// Empty by default — an empty prompt renders identically to no prompt
    /// at all.
    prompt: String,
    /// Stale-push guard: `push` is a no-op unless the caller's token matches.
    token: u64,
    /// The streaming external-command source attached via
    /// `picker-source-spawn!`, if any. Owning it
    /// here — rather than in some separate registry — is what makes
    /// kill-on-close/replace automatic: `SpawnedLineSource::drop` kills the
    /// child, and this field is dropped whenever the session itself is
    /// (`close_picker`'s `take()`, `open_picker`'s replace).
    source: Option<SpawnedLineSource>,
    /// Set by `picker!`'s `#:pending` for a caller whose results arrive via
    /// `spawn-async!` rather than `picker-source-spawn!` — the latter
    /// already has its own "still populating" signal (`source.is_some()`),
    /// so this flag only exists for the shape that has no `source` to ask.
    /// Cleared by the first `push` that actually applies (matching token),
    /// even an empty batch — a clean `git status`, say, still means the job
    /// is done. See [`is_pending`](Self::is_pending).
    pending: bool,
}

static NEXT_TOKEN: AtomicU64 = AtomicU64::new(1);

impl PickerSession {
    /// Opens empty — the caller's initial item list (from `picker!`) arrives
    /// through the same `push` path as any later batch: open empty, then
    /// attach a source.
    pub(crate) fn new(on_select: SteelVal, prompt: String, pending: bool) -> Self {
        Self {
            items: Vec::new(),
            query: String::new(),
            filtered: Vec::new(),
            rank_scratch: Vec::new(),
            matcher: FuzzyMatcher::new(),
            selected: 0,
            scroll: 0,
            on_select,
            prompt,
            token: NEXT_TOKEN.fetch_add(1, Ordering::Relaxed),
            source: None,
            pending,
        }
    }

    pub(crate) fn token(&self) -> u64 {
        self.token
    }

    pub(crate) fn prompt(&self) -> &str {
        &self.prompt
    }

    /// Whether results are still arriving: either `picker!`'s `#:pending`
    /// hasn't been cleared by a matching `push` yet, or a streaming
    /// `picker-source-spawn!` source is still attached (cleared once its
    /// reader disconnects and the caller consumes it via `take_source`).
    pub(crate) fn is_pending(&self) -> bool {
        self.pending || self.source.is_some()
    }

    /// Seeds the initial item list `picker!` was given. An empty seed is
    /// not a batch arrival — nothing has come back yet, so `#:pending` must
    /// survive it; a non-empty one goes through `push`, which clears
    /// `pending` because a populated list needs no "still arriving" marker.
    pub(crate) fn seed(&mut self, items: Vec<PickerItem>) {
        if !items.is_empty() {
            self.push(self.token, items);
        }
    }

    /// Appends `items` and reranks, but only if `token` matches this
    /// session's token. A mismatch is expected-normal (a late batch from a
    /// picker the user already closed or replaced) — silent no-op, not an
    /// error. Returns whether the push was applied.
    pub(crate) fn push(&mut self, token: u64, items: Vec<PickerItem>) -> bool {
        if token != self.token {
            return false;
        }
        self.pending = false;
        self.items.extend(items);
        self.rerank();
        true
    }

    /// Attaches a spawned streaming source, replacing (and thereby killing,
    /// via `SpawnedLineSource::drop`) any source already attached — a second
    /// `picker-source-spawn!` on the same session is a re-spawn, not a
    /// second concurrent source. This is also the semantics a future
    /// live-requery feature (re-running the source per query change) would
    /// reuse as-is.
    pub(crate) fn attach_source(&mut self, source: SpawnedLineSource) {
        self.source = Some(source);
    }

    pub(crate) fn source_mut(&mut self) -> Option<&mut SpawnedLineSource> {
        self.source.as_mut()
    }

    /// Takes the source out (e.g. once its reader has disconnected and the
    /// caller wants to consume it via `SpawnedLineSource::finish`).
    pub(crate) fn take_source(&mut self) -> Option<SpawnedLineSource> {
        self.source.take()
    }

    #[cfg(all(test, unix))]
    pub(crate) fn has_source(&self) -> bool {
        self.source.is_some()
    }

    /// The attached source's OS pid, for tests that verify kill-on-close
    /// against an independent liveness check rather than the handle's own
    /// state.
    #[cfg(all(test, unix))]
    pub(crate) fn source_pid_for_test(&self) -> Option<u32> {
        self.source.as_ref().map(SpawnedLineSource::pid)
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
    #[cfg(test)] // production caller is a future live-requery replace path, not yet built
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
    /// order — the window the picker panel paints. The selected row's
    /// on-screen position is `selected - scroll`.
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

    /// Cheap `Rc` clone — the accept/dismiss dispatch fires this via
    /// `queue_steel_call`; the store itself never invokes it.
    pub(crate) fn on_select(&self) -> &SteelVal {
        &self.on_select
    }

    /// The only place ranking happens; every mutator above routes through
    /// this. Resets `selected`/`scroll` to `0` on every rerank — a stale
    /// selection surviving a rerank (now pointing at a different item, or
    /// one no longer in the filtered set) is worse than landing back on the
    /// top row.
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

/// Single open chokepoint for the picker — `hume-scripting`'s `picker!`
/// builtin (`ui::picker`) calls this via `EditorHostImpl`. Allowed from any
/// mode, but one modal owner at a time, so opening a picker always closes
/// any live completion session first. Replacing an already-open picker
/// fires *its* `on_select` with `#f` before installing the new one, via
/// [`close_picker`] — the
/// exactly-once callback contract must never have a window where a session
/// can be silently dropped without firing.
///
/// Takes `state`/`lsp` rather than `&mut Editor` because its production
/// caller, `EditorHostImpl::open_picker`, holds those as disjoint borrows,
/// not a whole `Editor` — it can never reach an `&mut Editor`.
pub(crate) fn open_picker(
    state: &mut super::EditorState,
    lsp: Option<&mut super::lsp::LspState>,
    session: PickerSession,
) {
    super::lsp::completion::clear_completion_menu(state, lsp);
    close_picker(state, SteelVal::BoolV(false));
    state.config.picker = Some(session);
}

/// Single close chokepoint for the picker: ends the session (if one is
/// open) and fires its `on_select` callback exactly once with `payload`.
/// Returns whether a session was actually closed. Shared by `Esc`, `Enter`
/// (with the selected payload), `picker-close!`, and `open_picker`'s
/// replace-on-open path (LESSONS.md L2 — one chokepoint, not one copy per
/// caller).
///
/// `Editor::reset_config_state` is a second, deliberate exit from this
/// "fires exactly once" contract: its wholesale `ConfigState` rebuild drops
/// `state.config.picker` directly (never calling this function) along with
/// the `pending_work` queue this function would have pushed the callback
/// onto — the outgoing engine that owns the callback is seconds from being
/// dropped, so firing it would be observable to nothing.
pub(crate) fn close_picker(state: &mut super::EditorState, payload: SteelVal) -> bool {
    let Some(session) = state.config.picker.take() else {
        return false;
    };
    let callback = session.on_select().clone();
    state.queue_steel_call(callback, vec![payload]);
    true
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
        PickerSession::new(dummy_on_select(), String::new(), false)
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
    fn prompt_is_stored_verbatim() {
        let s = PickerSession::new(dummy_on_select(), "files: ".to_string(), false);
        assert_eq!(s.prompt(), "files: ");
    }

    #[test]
    fn not_pending_by_default() {
        assert!(!open().is_pending());
    }

    #[test]
    fn pending_flag_set_on_open_and_cleared_by_a_matching_push() {
        let mut s = PickerSession::new(dummy_on_select(), String::new(), true);
        assert!(s.is_pending());
        let token = s.token();
        assert!(s.push(token, items(&["a"])));
        assert!(!s.is_pending(), "a matching push must clear pending");
    }

    #[test]
    fn pending_flag_cleared_by_a_matching_push_even_with_an_empty_batch() {
        // A clean `git status` still means the job finished — pending must
        // not stay stuck just because there was nothing to add.
        let mut s = PickerSession::new(dummy_on_select(), String::new(), true);
        let token = s.token();
        assert!(s.push(token, items(&[])));
        assert!(!s.is_pending());
    }

    #[test]
    fn pending_flag_survives_a_stale_token_push() {
        let mut s = PickerSession::new(dummy_on_select(), String::new(), true);
        let stale = s.token().wrapping_add(1);
        assert!(!s.push(stale, items(&["x"])));
        assert!(
            s.is_pending(),
            "a rejected push must not clear pending — the real batch hasn't arrived yet"
        );
    }

    #[test]
    fn seed_with_items_clears_pending() {
        let mut s = PickerSession::new(dummy_on_select(), String::new(), true);
        s.seed(items(&["a"]));
        assert!(
            !s.is_pending(),
            "seeding real items means the list is already populated — no \"still arriving\" marker needed"
        );
        assert_eq!(window_vec(&s, 10), vec!["a"]);
    }

    #[test]
    fn empty_seed_leaves_pending_intact() {
        let mut s = PickerSession::new(dummy_on_select(), String::new(), true);
        s.seed(items(&[]));
        assert!(
            s.is_pending(),
            "an empty seed is not a batch arrival — `#:pending`'s caller intent must survive it"
        );
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
