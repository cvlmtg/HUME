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
use hume_scripting::host::PickerOpts;
use steel::rvals::SteelVal;

use super::fuzzy::{FuzzyMatcher, FuzzyPattern};

/// One row in a picker: a display string shown to the user and an opaque
/// payload handed back to `on_select` verbatim. Rust never interprets
/// `payload` — mirrors the drawer's "rows are pre-formatted display
/// strings" contract.
pub(crate) struct PickerItem {
    pub(crate) display: String,
    pub(crate) payload: SteelVal,
}

/// `UiHost`'s wire shape for a batch of items, converted — `open_picker`,
/// `picker_push`, and `picker_replace` in `EditorHostImpl` each need this
/// same conversion before handing a batch to the store; kept here rather
/// than duplicated at each call site because `hume-scripting`'s `UiHost`
/// trait cannot name `PickerItem`, an `hume-editor`-private type.
pub(crate) fn picker_items(items: Vec<(String, SteelVal)>) -> Vec<PickerItem> {
    items
        .into_iter()
        .map(|(display, payload)| PickerItem { display, payload })
        .collect()
}

/// A streaming source bundled with the exit-code allowlist it was attached
/// with. One field instead of two on `PickerSession` so a respawn (a second
/// `attach_source` replacing this field) can never separate a source from
/// the codes that decide whether its own exit is a failure.
struct AttachedSource {
    source: SpawnedLineSource,
    ok_exit_codes: Vec<i32>,
}

/// Rust-side store for one open picker: items, query, ranked indices,
/// selection, scroll, and a stale-push-or-replace guard token. Steel drives
/// it through `picker!`/`picker-push!`/`picker-replace!`/`picker-close!`;
/// this module has no Steel-facing surface of its own.
pub(crate) struct PickerSession {
    /// Append-only via `push`/`seed` — the common case, and what lets
    /// `push` preserve a selection by index (see `rerank_keeping_selection`).
    /// `replace` is the one mutator that breaks this: it clears the vec
    /// wholesale before re-extending it, for a live requery that must drop
    /// the previous pattern's rows.
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
    /// Stale-push/-replace guard: `push` and `replace` are both a no-op
    /// unless the caller's token matches.
    token: u64,
    /// The streaming external-command source attached via
    /// `picker-source-spawn!`, if any, together with its exit-code
    /// allowlist. Owning it here — rather than in some separate registry —
    /// is what makes kill-on-close/replace automatic: `SpawnedLineSource`'s
    /// `Drop` kills the child, and this field is dropped whenever the
    /// session itself is (`close_picker`'s `take()`, `open_picker`'s
    /// replace).
    source: Option<AttachedSource>,
    /// Set by `picker!`'s `#:pending` for a caller whose results arrive via
    /// `spawn-async!` rather than `picker-source-spawn!` — the latter
    /// already has its own "still populating" signal (`source.is_some()`),
    /// so this flag only exists for the shape that has no `source` to ask.
    /// Cleared by the first `push`/`replace` that actually applies (matching
    /// token), even an empty batch — a clean `git status`, say, still means
    /// the job is done. See [`is_pending`](Self::is_pending).
    pending: bool,
    /// `#:on-query-change` — `Some` makes this session live: `insert_char`/
    /// `pop_grapheme` fire it with the new query (`Editor::queue_query_change`)
    /// instead of the query driving the local fuzzy filter. Its `Some`-ness
    /// is read directly wherever "is this session live" matters — no
    /// separate bool duplicating it.
    on_query_change: Option<SteelVal>,
}

static NEXT_TOKEN: AtomicU64 = AtomicU64::new(1);

impl PickerSession {
    /// Opens empty — the caller's initial item list (from `picker!`) arrives
    /// through the same `push` path as any later batch: open empty, then
    /// attach a source. `opts.query` is assigned directly rather than
    /// through `set_query`: with `items` still empty, a rerank has nothing
    /// to do.
    pub(crate) fn new(on_select: SteelVal, opts: PickerOpts) -> Self {
        Self {
            items: Vec::new(),
            query: opts.query,
            filtered: Vec::new(),
            rank_scratch: Vec::new(),
            matcher: FuzzyMatcher::new(),
            selected: 0,
            scroll: 0,
            on_select,
            prompt: opts.prompt,
            token: NEXT_TOKEN.fetch_add(1, Ordering::Relaxed),
            source: None,
            pending: opts.pending,
            on_query_change: opts.on_query_change,
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
    ///
    /// Reranks via [`rerank_keeping_selection`](Self::rerank_keeping_selection),
    /// not a hard reset: a streaming source pushes once per frame, and
    /// snapping back to row 0 on every batch would make a picker the user
    /// is actively scrolling through unnavigable. Safe here specifically
    /// because `push` only ever appends — every index a pre-push
    /// `filtered` held still names the same item afterward.
    pub(crate) fn push(&mut self, token: u64, items: Vec<PickerItem>) -> bool {
        if token != self.token {
            return false;
        }
        self.pending = false;
        self.items.extend(items);
        self.rerank_keeping_selection();
        true
    }

    /// Replaces the item list wholesale and reranks — same token-guard and
    /// `pending`-clearing contract as `push`, but always lands on row `0`
    /// (`rerank`, not `push`'s keep-the-same-item `rerank_keeping_selection`):
    /// every index the pre-replace `filtered` held names a *different* item
    /// (or nothing) once `items` is cleared, so there is no selection worth
    /// trying to preserve. Items are otherwise append-only; this is the only
    /// way to drop stale rows. The requery half of a live source: a caller
    /// with `#:on-query-change` clears the previous pattern's rows before
    /// spawning the new search.
    pub(crate) fn replace(&mut self, token: u64, items: Vec<PickerItem>) -> bool {
        if token != self.token {
            return false;
        }
        self.pending = false;
        self.items = items;
        self.rerank();
        true
    }

    /// Attaches a spawned streaming source. A source already attached is
    /// replaced (and thereby killed, via `SpawnedLineSource::drop`) rather
    /// than left running — `picker_source::spawn_source`, this method's one
    /// caller, always reports and takes the outgoing source first, so in
    /// practice this replace fires only as a safety net, never on the live
    /// outgoing source itself. `ok_exit_codes` is `drain_picker_source`'s
    /// allowlist for *this* source, e.g. `rg`'s exit `1` ("no matches").
    pub(super) fn attach_source(&mut self, source: SpawnedLineSource, ok_exit_codes: Vec<i32>) {
        self.source = Some(AttachedSource {
            source,
            ok_exit_codes,
        });
    }

    pub(crate) fn source_mut(&mut self) -> Option<&mut SpawnedLineSource> {
        self.source.as_mut().map(|attached| &mut attached.source)
    }

    /// Takes the source out along with its exit-code allowlist (e.g. once
    /// its reader has disconnected and the caller wants to consume it via
    /// `SpawnedLineSource::finish`).
    pub(crate) fn take_source(&mut self) -> Option<(SpawnedLineSource, Vec<i32>)> {
        self.source.take().map(|a| (a.source, a.ok_exit_codes))
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
        self.source.as_ref().map(|a| a.source.pid())
    }

    /// Polls the attached source's own OS exit status directly, bypassing
    /// `drain_picker_source` entirely — for a test that must observe a
    /// child having already exited without also triggering the ordinary
    /// disconnect-and-report drain path it's racing against.
    #[cfg(all(test, unix))]
    pub(crate) fn source_has_exited_for_test(&self) -> bool {
        self.source.as_ref().is_some_and(|a| a.source.has_exited())
    }

    /// Appends one `char` to the query and reranks. Key events deliver
    /// printable input one `char` at a time, including combining marks,
    /// which simply extend the trailing grapheme cluster.
    ///
    /// Returns the `#:on-query-change` callback to fire (`None` for a
    /// non-live session) — the caller, not this method, queues it via
    /// `queue_steel_call` (see `handle_picker_key`), since firing a Steel
    /// callback needs `&mut EditorState`, which a pure data store
    /// deliberately has no access to. Bundling the mutation with the
    /// callback it produces, rather than a caller calling a separate
    /// `fire_query_change` afterward on its own, is what makes forgetting
    /// to fire it (or firing it after a mutator that shouldn't, like
    /// `set_query`) a type error instead of a silent desync between the
    /// visible query and a live source.
    #[must_use = "queue this via queue_steel_call, or the query-change notification is silently skipped"]
    pub(crate) fn insert_char(&mut self, ch: char) -> Option<SteelVal> {
        self.query.push(ch);
        self.rerank();
        self.on_query_change.clone()
    }

    /// Removes the trailing grapheme cluster (not merely the last `char`) so
    /// that precomposed accents and ZWJ/modifier emoji sequences are deleted
    /// as one unit, then reranks. Returns `None` without effect when the
    /// query is already empty (in addition to a non-live session, same as
    /// [`insert_char`](Self::insert_char)) — the query didn't change, so
    /// there is nothing to notify either way.
    #[must_use = "queue this via queue_steel_call, or the query-change notification is silently skipped"]
    pub(crate) fn pop_grapheme(&mut self) -> Option<SteelVal> {
        if self.query.is_empty() {
            return None;
        }
        self.query.truncate(hume_rope::grapheme::prev_str_boundary(
            &self.query,
            self.query.len(),
        ));
        self.rerank();
        self.on_query_change.clone()
    }

    /// Replaces the query wholesale and reranks — test-only; `new` assigns
    /// `query` directly (nothing to rerank against yet) and no other
    /// production path replaces a query outside `insert_char`/`pop_grapheme`.
    #[cfg(test)]
    fn set_query(&mut self, query: String) {
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

    /// Rebuilds `filtered` from `items`/`query` — the ranking-only half
    /// shared by [`rerank`](Self::rerank) and
    /// [`rerank_keeping_selection`](Self::rerank_keeping_selection); neither
    /// touches `selected`/`scroll` itself.
    fn rebuild_filtered(&mut self) {
        // `on_query_change.is_some()` (a live session) skips the local
        // fuzzy filter at any query, separately from an empty query, which
        // always takes this branch regardless of liveness — both land here
        // in insertion order by construction: an empty query avoids relying
        // on nucleo's (undocumented) all-equal-score behavior and skips
        // scoring entirely on the dominant streaming-ingest path (empty
        // query while a spawned source drains batches); a live session
        // skips it because the query already selects what the source
        // returns (e.g. `rg`'s own regex match), so a second fuzzy pass
        // over already-matched rows would drop legitimate non-fuzzy hits —
        // `foo.*bar` fuzzy-matching almost nothing it just found.
        if self.query.is_empty() || self.on_query_change.is_some() {
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
    }

    /// Rebuilds `filtered` and resets `selected`/`scroll` to `0` — for a
    /// query change (`set_query`/`insert_char`/`pop_grapheme`), where the
    /// ranking itself changed meaning and there is no old selection worth
    /// trying to preserve.
    fn rerank(&mut self) {
        self.rebuild_filtered();
        self.selected = 0;
        self.scroll = 0;
    }

    /// Rebuilds `filtered` like `rerank`, but tries to keep the selection on
    /// the same *item* instead of resetting it — for an item-list mutation
    /// under an unchanged query (`push`), where a source streaming in the
    /// background must not keep yanking the cursor back to the top row every
    /// frame. Falls back to row `0`, same as `rerank`, when the previously
    /// selected item is no longer in `filtered` (e.g. a query already
    /// excluded it and a later batch still doesn't match it).
    ///
    /// Only valid when every index the pre-call `filtered` held still names
    /// the same item afterward — i.e. `items` was purely appended to, never
    /// cleared or reordered. `push` is `self`'s only caller and guarantees
    /// that; a caller that clears `items` first (`replace`) uses `rerank`
    /// instead, since a same-index match there would be coincidental, not a
    /// real preserved selection.
    fn rerank_keeping_selection(&mut self) {
        let selected_item = self.filtered.get(self.selected).copied();
        self.rebuild_filtered();
        match selected_item.and_then(|item| self.filtered.iter().position(|&idx| idx == item)) {
            Some(pos) => {
                self.selected = pos;
                self.scroll = self.scroll.min(self.selected);
            }
            None => {
                self.selected = 0;
                self.scroll = 0;
            }
        }
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
/// replace-on-open path — one chokepoint, not one copy per
/// caller.
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
        PickerSession::new(dummy_on_select(), PickerOpts::default())
    }

    fn open_pending() -> PickerSession {
        PickerSession::new(
            dummy_on_select(),
            PickerOpts {
                pending: true,
                ..Default::default()
            },
        )
    }

    fn open_live() -> PickerSession {
        PickerSession::new(
            dummy_on_select(),
            PickerOpts {
                on_query_change: Some(dummy_on_select()),
                ..Default::default()
            },
        )
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
        let s = PickerSession::new(
            dummy_on_select(),
            PickerOpts {
                prompt: "files: ".to_string(),
                ..Default::default()
            },
        );
        assert_eq!(s.prompt(), "files: ");
    }

    #[test]
    fn not_pending_by_default() {
        assert!(!open().is_pending());
    }

    #[test]
    fn pending_flag_set_on_open_and_cleared_by_a_matching_push() {
        let mut s = open_pending();
        assert!(s.is_pending());
        let token = s.token();
        assert!(s.push(token, items(&["a"])));
        assert!(!s.is_pending(), "a matching push must clear pending");
    }

    #[test]
    fn pending_flag_cleared_by_a_matching_push_even_with_an_empty_batch() {
        // A clean `git status` still means the job finished — pending must
        // not stay stuck just because there was nothing to add.
        let mut s = open_pending();
        let token = s.token();
        assert!(s.push(token, items(&[])));
        assert!(!s.is_pending());
    }

    #[test]
    fn pending_flag_survives_a_stale_token_push() {
        let mut s = open_pending();
        let stale = s.token().wrapping_add(1);
        assert!(!s.push(stale, items(&["x"])));
        assert!(
            s.is_pending(),
            "a rejected push must not clear pending — the real batch hasn't arrived yet"
        );
    }

    #[test]
    fn seed_with_items_clears_pending() {
        let mut s = open_pending();
        s.seed(items(&["a"]));
        assert!(
            !s.is_pending(),
            "seeding real items means the list is already populated — no \"still arriving\" marker needed"
        );
        assert_eq!(window_vec(&s, 10), vec!["a"]);
    }

    #[test]
    fn empty_seed_leaves_pending_intact() {
        let mut s = open_pending();
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
    fn query_prefill_is_visible_and_applied_at_construction() {
        let mut s = PickerSession::new(
            dummy_on_select(),
            PickerOpts {
                query: "f".to_string(),
                ..Default::default()
            },
        );
        assert_eq!(s.query(), "f");
        // Applied through the same `rerank` a later push would use — a
        // batch arriving after open is filtered by the prefilled query
        // immediately, not just once a keystroke re-triggers ranking.
        let token = s.token();
        s.push(token, items(&["foo", "bar"]));
        assert_eq!(window_vec(&s, 10), vec!["foo"]);
    }

    #[test]
    fn replace_swaps_the_item_list_instead_of_appending() {
        let mut s = open();
        let token = s.token();
        s.push(token, items(&["a", "b"]));
        assert!(s.replace(token, items(&["c"])));
        assert_eq!(window_vec(&s, 10), vec!["c"]);
        assert_eq!(s.total_len(), 1);
    }

    #[test]
    fn stale_token_replace_is_rejected() {
        let mut s = open();
        let real_token = s.token();
        s.push(real_token, items(&["a"]));
        let wrong_token = real_token.wrapping_add(1);
        assert!(!s.replace(wrong_token, items(&["z"])));
        assert_eq!(
            window_vec(&s, 10),
            vec!["a"],
            "a stale-token replace must leave the existing items untouched"
        );
    }

    #[test]
    fn replace_clears_pending_even_with_an_empty_batch() {
        let mut s = open_pending();
        let token = s.token();
        assert!(s.replace(token, items(&[])));
        assert!(!s.is_pending());
    }

    #[test]
    fn live_session_keeps_insertion_order_regardless_of_query() {
        // See `rebuild_filtered`'s doc for why a live session must skip
        // scoring entirely, the same branch an empty query already takes.
        let mut s = open_live();
        s.set_query("zzz-does-not-fuzzy-match-anything".to_string());
        let token = s.token();
        s.push(token, items(&["b", "a", "c"]));
        assert_eq!(window_vec(&s, 10), vec!["b", "a", "c"]);
    }

    #[test]
    fn non_live_session_with_the_same_query_still_filters() {
        // Same query as above, on a non-live session — confirms the
        // insertion-order result above comes from live mode, not from the
        // query happening to fail to fuzzy-match anyway.
        let mut s = open();
        s.set_query("zzz-does-not-fuzzy-match-anything".to_string());
        let token = s.token();
        s.push(token, items(&["b", "a", "c"]));
        assert!(window_vec(&s, 10).is_empty());
    }

    #[test]
    fn pop_grapheme_on_an_empty_query_is_a_no_op_even_for_a_live_session() {
        // The `Option<SteelVal>` port replaced the old boolean assertion
        // (`assert!(!s.pop_grapheme())`) with a query-content check, which
        // can't tell "returned `None`" apart from "returned the callback" —
        // this pins the return value directly, on the one session shape
        // (`on_query_change` set) where mistaking those two would matter.
        let mut s = open_live();
        assert!(s.pop_grapheme().is_none());
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
    fn push_keeps_the_selection_on_the_same_item() {
        // A streaming source pushes once per frame — snapping the selection
        // back to row 0 on every batch would make an actively-scrolled
        // picker unnavigable, so a plain append must keep pointing at the
        // same item instead of resetting.
        let mut s = open();
        let token = s.token();
        s.push(token, items(&["a", "b", "c", "d", "e"]));
        s.move_selection(3, 2);
        assert_eq!(payload_str(s.selected_payload().expect("has a match")), "d");
        s.push(token, items(&["f"]));
        assert_eq!(
            payload_str(s.selected_payload().expect("has a match")),
            "d",
            "a push must not move the selection off the item the user had selected"
        );
    }

    #[test]
    fn replace_always_resets_selection_and_scroll() {
        // Unlike `push`, `replace` swaps in an unrelated item list — the old
        // selection's index cannot mean the same thing afterward, so it must
        // always land back on row 0, never a same-index coincidence.
        let mut s = open();
        let token = s.token();
        s.push(token, items(&["a", "b", "c", "d", "e"]));
        s.move_selection(3, 2);
        assert_ne!(s.selected(), 0);
        assert!(s.replace(token, items(&["x", "y", "z"])));
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
        let _ = s.insert_char('z');
        assert_eq!(s.matched_len(), 0);
        let _ = s.pop_grapheme();
        assert_eq!(s.query(), "", "pop_grapheme must have removed the 'z'");
        assert_eq!(s.matched_len(), 2);
        let _ = s.pop_grapheme(); // query now empty; further pop is a no-op
        assert_eq!(s.query(), "");
    }

    #[test]
    fn pop_grapheme_removes_full_cluster() {
        let mut s = open();
        // "e" + combining acute accent (U+0301) forms one grapheme cluster.
        let _ = s.insert_char('e');
        let _ = s.insert_char('\u{0301}');
        assert_eq!(s.query(), "e\u{0301}");
        let _ = s.pop_grapheme();
        assert_eq!(s.query(), "");
        assert!(s.query().is_char_boundary(0));

        // ZWJ emoji sequence: family emoji built from 4 code points joined
        // by ZWJ — one pop_grapheme must remove the whole cluster.
        for ch in "👨‍👩‍👧‍👦".chars() {
            let _ = s.insert_char(ch);
        }
        let _ = s.pop_grapheme();
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
