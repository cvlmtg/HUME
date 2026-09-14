//! Fuzzy-picker data store. Sibling of `CompletionSession`
//! (`editor/lsp/completion.rs`), not a generalization of it: item shape,
//! query origin, accept semantics, lifetime, scale, and scroll model all
//! differ between the two, so a shared abstract core would be parameterized
//! over six axes for two call sites — not worth it unless the bodies
//! converge later. Mirrors completion's `rank_scratch` reuse and
//! reset-on-rerank patterns.
//!
//! Wired onto `EditorState.picker`; opened through the [`open_picker`] free
//! fn below, via [`PickerSession::new`] (Steel's `picker!` builtin,
//! `hume-scripting`'s `ui::picker`) or [`PickerSession::new_live`]
//! (`live-picker!`, `ui::live_picker`) — and driven per-frame by
//! `Editor::sync_picker_view` and per-key by `Editor::handle_picker_key`
//! (`editor/mappings/mod.rs`).

use std::cmp::Reverse;
use std::sync::atomic::{AtomicU64, Ordering};

use hume_engine::types::TruncateEnd;
use hume_platform::process::line_source::SpawnedLineSource;
use hume_scripting::host::{LivePickerOpts, PickerOpts};
use steel::rvals::SteelVal;
use termina::event::KeyEvent;

use super::fuzzy::{FuzzyMatcher, FuzzyProfile};
use super::keymap::canonical;

/// One row in a picker: a display string shown to the user and an opaque
/// payload handed back to `on_select` verbatim. Rust never interprets
/// `payload` — mirrors the drawer's "rows are pre-formatted display
/// strings" contract.
pub(in crate::editor) struct PickerItem {
    pub(in crate::editor) display: String,
    pub(in crate::editor) payload: SteelVal,
}

/// `UiHost`'s wire shape for a batch of items, converted — `open_picker` and
/// `picker_feed` in `EditorHostImpl` each need this same conversion before
/// handing a batch to the store; kept here rather than duplicated at each
/// call site because `hume-scripting`'s `UiHost` trait cannot name
/// `PickerItem`, an `hume-editor`-private type.
pub(in crate::editor) fn picker_items(items: Vec<(String, SteelVal)>) -> Vec<PickerItem> {
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
    /// Set at attach time for a live requery's source: its first `push`
    /// replaces `items` wholesale instead of appending, so the previous
    /// pattern's rows stay on screen until this source actually has
    /// something to show. Scoped to the source, not the session, so a
    /// batch still queued from the *outgoing* source — killed by a
    /// respawn but not yet drained — can never carry this flag; only the
    /// source actually attached when a batch arrives can.
    supersedes_rows: bool,
}

/// Whether the query drives the local fuzzy filter (`picker!`) or an
/// external source (`live-picker!`). Its `Live`-ness is read directly
/// wherever "is this session live" matters — no separate bool duplicating
/// it, and no `Option` whose `None` arm silently means something structural.
enum PickerMode {
    Filter,
    /// `insert_char`/`pop_grapheme` fire `on_query_change` with the new
    /// query instead of the query driving the local fuzzy filter — see
    /// `rebuild_filtered`'s doc for why a live session's ranking is always
    /// the identity permutation over `items`.
    Live {
        on_query_change: SteelVal,
    },
}

impl PickerMode {
    fn is_live(&self) -> bool {
        matches!(self, PickerMode::Live { .. })
    }

    /// Cheap `Rc` clone of the query-change callback, or `None` for
    /// `Filter` — the return shape `insert_char`/`pop_grapheme` hand
    /// straight to their caller.
    fn on_query_change(&self) -> Option<SteelVal> {
        match self {
            PickerMode::Filter => None,
            PickerMode::Live { on_query_change } => Some(on_query_change.clone()),
        }
    }
}

/// The one "are results still arriving" state for a session — a single
/// signal, so `#:pending` and "is a source attached" can never disagree the
/// way two independently-tracked fields could.
enum Population {
    /// Everything the session will ever get is already in `items`.
    Complete,
    /// `#:pending` — results arrive out-of-band (`spawn-async!` +
    /// `picker-push!`), so there is no source here to ask. Cleared by the
    /// first applied `push`/`replace`, even an empty batch — a clean
    /// `git status`, say, still means the job is done.
    Awaiting,
    /// A streaming `picker-source-spawn!` source is attached; cleared on
    /// disconnect (`take_source`), explicit stop, or respawn.
    Streaming(AttachedSource),
}

/// Rust-side store for one open picker: items, query, ranked indices,
/// selection, scroll, and a stale-push-or-replace guard token. Steel drives
/// it through `picker!`/`live-picker!`/`picker-push!`/`picker-replace!`/
/// `picker-close!`;
/// this module has no Steel-facing surface of its own.
pub(in crate::editor) struct PickerSession {
    /// Append-only via `push`/`seed` — the common case, and what lets
    /// `push` preserve a selection by index (see `rerank_keeping_selection`).
    /// `replace` is the one mutator that breaks this: it clears the vec
    /// wholesale before re-extending it, for a live requery that must drop
    /// the previous pattern's rows. `push` itself can also take this path —
    /// see its `take_supersede` exception, doc'd there rather than here.
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
    /// Which end of an over-long row the panel clips — `#:truncate`, see
    /// [`TruncateEnd`].
    truncate: TruncateEnd,
    /// Identifies this session to Steel and to [`session_for_token`], the
    /// shared guard every token-scoped picker mutation checks before
    /// reaching a `&mut PickerSession` at all.
    token: u64,
    /// Whether results are still arriving, and how — see [`Population`].
    /// Owning a `Streaming` source here — rather than in some separate
    /// registry — is what makes kill-on-close/replace automatic:
    /// `SpawnedLineSource`'s `Drop` kills the child, and this field is
    /// dropped whenever the session itself is (`close_picker`'s `take()`,
    /// `open_picker`'s replace).
    population: Population,
    /// Whether the query drives the local fuzzy filter or an external
    /// source — see [`PickerMode`].
    mode: PickerMode,
    /// A live session's query changed and its requery (stop old source,
    /// debounce, spawn new one) hasn't delivered a first batch yet. Can't
    /// ride `population`: `picker-source-stop!` takes the source and
    /// resets `population` to `Complete` well before the respawn's first
    /// batch lands, and `is_pending` must keep reading "still arriving"
    /// across that whole gap. Armed by `notify_query_change`; cleared only
    /// by `replace` — not by `push`'s ordinary append path, which a batch
    /// still in flight from the *outgoing* source can also reach (see
    /// `replace`'s doc).
    requery_armed: bool,
    /// `#:actions` — extra key→proc bindings tried, in order, after every
    /// built-in picker key. A linear scan, not a map: real sessions carry a
    /// handful of entries at most, and canonicalizing each on lookup instead
    /// of hashing keeps the comparison in one place ([`canonical`]) rather
    /// than needing a `Hash`/`Eq` impl to agree with it separately.
    actions: Vec<(KeyEvent, SteelVal)>,
}

static NEXT_TOKEN: AtomicU64 = AtomicU64::new(1);

impl PickerSession {
    /// Opens empty — the caller's initial item list (from `picker!`) arrives
    /// through the same `push` path as any later batch: open empty, then
    /// attach a source.
    pub(in crate::editor) fn new(on_select: SteelVal, opts: PickerOpts) -> Self {
        let population = if opts.pending {
            Population::Awaiting
        } else {
            Population::Complete
        };
        Self::build(
            on_select,
            opts.prompt,
            opts.query,
            opts.truncate,
            opts.actions,
            population,
            PickerMode::Filter,
        )
    }

    /// `live-picker!` — always live from construction, and always opens
    /// empty: unlike `picker!`, there is no `items`/`#:pending` here for the
    /// caller to seed with, since a live session is populated entirely
    /// through its own `on_query_change` (which itself drives
    /// `picker-push!`/`picker-replace!`/`picker-source-spawn!`) —
    /// `population` starts `Complete` and only changes once a source
    /// actually attaches.
    pub(in crate::editor) fn new_live(on_select: SteelVal, opts: LivePickerOpts) -> Self {
        Self::build(
            on_select,
            opts.prompt,
            opts.query,
            opts.truncate,
            opts.actions,
            Population::Complete,
            PickerMode::Live {
                on_query_change: opts.on_query_change,
            },
        )
    }

    fn build(
        on_select: SteelVal,
        prompt: String,
        query: String,
        truncate: TruncateEnd,
        actions: Vec<(KeyEvent, SteelVal)>,
        population: Population,
        mode: PickerMode,
    ) -> Self {
        Self {
            items: Vec::new(),
            query,
            filtered: Vec::new(),
            rank_scratch: Vec::new(),
            matcher: FuzzyMatcher::new(FuzzyProfile::Picker),
            selected: 0,
            scroll: 0,
            on_select,
            prompt,
            truncate,
            token: NEXT_TOKEN.fetch_add(1, Ordering::Relaxed),
            population,
            mode,
            requery_armed: false,
            actions: actions
                .into_iter()
                .map(|(k, p)| (canonical(k), p))
                .collect(),
        }
    }

    pub(in crate::editor) fn token(&self) -> u64 {
        self.token
    }

    pub(in crate::editor) fn truncate(&self) -> TruncateEnd {
        self.truncate
    }

    pub(in crate::editor) fn prompt(&self) -> &str {
        &self.prompt
    }

    /// Whether results are still arriving — `population` is anything but
    /// `Complete`, or a live requery is armed and hasn't delivered its
    /// first batch yet (see `requery_armed`'s doc).
    pub(in crate::editor) fn is_pending(&self) -> bool {
        self.requery_armed || !matches!(self.population, Population::Complete)
    }

    /// Clears `#:pending` on an applied batch — the `Awaiting` half of
    /// "still populating" ends the moment real results (even an empty
    /// batch) land. Leaves `Streaming` untouched: a source drains lines
    /// into `push` continuously, and the source itself — not the arrival of
    /// one particular batch — is what decides when populating ends (its
    /// disconnect, handled by `take_source`).
    fn batch_arrived(&mut self) {
        if matches!(self.population, Population::Awaiting) {
            self.population = Population::Complete;
        }
    }

    /// Seeds the initial item list `picker!` was given. An empty seed is
    /// not a batch arrival — nothing has come back yet, so `#:pending` must
    /// survive it; a non-empty one goes through `push`, which clears
    /// `pending` because a populated list needs no "still arriving" marker.
    pub(in crate::editor) fn seed(&mut self, items: Vec<PickerItem>) {
        if !items.is_empty() {
            self.push(items);
        }
    }

    /// Appends `items` and reranks. No token guard here — every
    /// *token-scoped* caller (`picker-push!`, via `EditorHostImpl::picker_feed`)
    /// has already gone through [`session_for_token`] before reaching this;
    /// the other two callers, [`seed`](Self::seed) and
    /// `Editor::drain_picker_source`, hold the session directly (at
    /// construction, and via the frame's own `&mut EditorState`, respectively)
    /// and so have no token to check in the first place.
    ///
    /// Reranks via [`rerank_keeping_selection`](Self::rerank_keeping_selection),
    /// not a hard reset: a streaming source pushes once per frame, and
    /// snapping back to row 0 on every batch would make a picker the user
    /// is actively scrolling through unnavigable. Safe here specifically
    /// because `push` only ever appends — every index a pre-push
    /// `filtered` held still names the same item afterward.
    ///
    /// Exception: the attached source's first batch after a live requery
    /// (`take_supersede`) replaces `items` wholesale instead — see
    /// `AttachedSource::supersedes_rows`'s doc. That's `replace`'s job, not
    /// a hand-rolled clear-then-extend here, so it also gets `replace`'s
    /// row-0 reset.
    pub(in crate::editor) fn push(&mut self, items: Vec<PickerItem>) {
        if self.take_supersede() {
            self.replace(items);
            return;
        }
        self.batch_arrived();
        self.items.extend(items);
        self.rerank_keeping_selection();
    }

    /// Replaces the item list wholesale and reranks — same
    /// [`session_for_token`]-guarded and `pending`-clearing contract as
    /// `push`, but always lands on row `0` (`rerank`, not `push`'s
    /// keep-the-same-item `rerank_keeping_selection`): every index the
    /// pre-replace `filtered` held names a *different* item (or nothing)
    /// once `items` is cleared, so there is no selection worth trying to
    /// preserve. Items are otherwise append-only; this is the only way to
    /// drop stale rows — a plugin driving `picker-replace!` directly, or
    /// `drain_picker_source` clearing a live requery's stale rows on a
    /// no-results exit. Also consumes `take_supersede`: an explicit
    /// replace already *is* the swap a live requery's first batch would
    /// otherwise perform, so that later batch must append, not replace
    /// again.
    ///
    /// The one place `requery_armed` clears — see its doc. `push`'s ordinary
    /// append path deliberately does not: a batch queued from the *outgoing*
    /// source can still land there after a keystroke has armed the next
    /// requery but before the queued `picker-source-stop!` callback runs
    /// (`drain_async_sources` precedes `drain_pending_work` in
    /// `Editor::settle`), and that stale batch must not read as "the
    /// requery landed."
    pub(in crate::editor) fn replace(&mut self, items: Vec<PickerItem>) {
        self.take_supersede();
        self.batch_arrived();
        self.requery_armed = false;
        self.items = items;
        self.rerank();
    }

    /// Attaches a spawned streaming source. A source already attached is
    /// replaced (and thereby killed, via `SpawnedLineSource::drop`) rather
    /// than left running — `picker_source::spawn_source`, this method's one
    /// caller, always reports and takes the outgoing source first, so in
    /// practice this replace fires only as a safety net, never on the live
    /// outgoing source itself. `ok_exit_codes` is `drain_picker_source`'s
    /// allowlist for *this* source, e.g. `rg`'s exit `1` ("no matches").
    ///
    /// `supersedes_rows` is set only for a live session (`self.mode.is_live()`):
    /// its rows always belong to exactly one query, so the next source's
    /// first batch should replace them. A `picker!` session driving
    /// `picker-source-spawn!` itself may legitimately spawn a second source
    /// to *add* rows to an already-seeded list, so it keeps `push`'s
    /// ordinary append behavior.
    pub(super) fn attach_source(&mut self, source: SpawnedLineSource, ok_exit_codes: Vec<i32>) {
        let supersedes_rows = self.mode.is_live();
        self.population = Population::Streaming(AttachedSource {
            source,
            ok_exit_codes,
            supersedes_rows,
        });
    }

    /// Consumes the attached source's `supersedes_rows` flag, if any —
    /// `false` (and a no-op) when nothing is attached or the flag was
    /// already spent. Called only by `push` and `replace`.
    fn take_supersede(&mut self) -> bool {
        match &mut self.population {
            Population::Streaming(attached) => {
                std::mem::replace(&mut attached.supersedes_rows, false)
            }
            _ => false,
        }
    }

    pub(in crate::editor) fn source_mut(&mut self) -> Option<&mut SpawnedLineSource> {
        match &mut self.population {
            Population::Streaming(attached) => Some(&mut attached.source),
            _ => None,
        }
    }

    /// Whether the attached source (if any) would still replace `items`
    /// wholesale on its next batch — `drain_picker_source`'s check for a
    /// live requery's source that disconnected before ever delivering one,
    /// so it can clear the previous pattern's now-stale rows itself.
    pub(in crate::editor) fn source_supersedes_rows(&self) -> bool {
        self.attached()
            .is_some_and(|attached| attached.supersedes_rows)
    }

    /// Shared read-only half of the `Population::Streaming` match — the
    /// `&mut` accessors (`source_mut`, `take_source`) need their own match
    /// arms to hand back a mutable borrow or move the source out, but every
    /// read-only query below is a one-liner over this.
    fn attached(&self) -> Option<&AttachedSource> {
        match &self.population {
            Population::Streaming(attached) => Some(attached),
            _ => None,
        }
    }

    /// Takes the source out along with its exit-code allowlist (e.g. once
    /// its reader has disconnected and the caller wants to consume it via
    /// `SpawnedLineSource::finish`), leaving `population` at `Complete` —
    /// except when it wasn't `Streaming` to begin with (`Awaiting`, with no
    /// source ever attached, or an already-`Complete` session): a `stop`
    /// call racing a source that was never there, or that already finished,
    /// must not fabricate a "done" transition, so the prior state is put
    /// back untouched and this returns `None`.
    pub(in crate::editor) fn take_source(&mut self) -> Option<(SpawnedLineSource, Vec<i32>)> {
        match std::mem::replace(&mut self.population, Population::Complete) {
            Population::Streaming(attached) => Some((attached.source, attached.ok_exit_codes)),
            other => {
                self.population = other;
                None
            }
        }
    }

    #[cfg(all(test, unix))]
    pub(in crate::editor) fn has_source(&self) -> bool {
        self.attached().is_some()
    }

    /// The attached source's OS pid, for tests that verify kill-on-close
    /// against an independent liveness check rather than the handle's own
    /// state.
    #[cfg(all(test, unix))]
    pub(in crate::editor) fn source_pid_for_test(&self) -> Option<u32> {
        self.attached().map(|attached| attached.source.pid())
    }

    /// Polls the attached source's own OS exit status directly, bypassing
    /// `drain_picker_source` entirely — for a test that must observe a
    /// child having already exited without also triggering the ordinary
    /// disconnect-and-report drain path it's racing against.
    #[cfg(all(test, unix))]
    pub(in crate::editor) fn source_has_exited_for_test(&self) -> bool {
        self.attached()
            .is_some_and(|attached| attached.source.has_exited())
    }

    /// Appends one `char` to the query and requeries. Key events deliver
    /// printable input one `char` at a time, including combining marks,
    /// which simply extend the trailing grapheme cluster.
    ///
    /// Returns the mode's `on_query_change` callback to fire (`None` for a
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
    pub(in crate::editor) fn insert_char(&mut self, ch: char) -> Option<SteelVal> {
        self.query.push(ch);
        self.rerank();
        self.notify_query_change()
    }

    /// Removes the trailing grapheme cluster (not merely the last `char`) so
    /// that precomposed accents and ZWJ/modifier emoji sequences are deleted
    /// as one unit, then requeries. Returns `None` without effect when the
    /// query is already empty (in addition to a non-live session, same as
    /// [`insert_char`](Self::insert_char)) — the query didn't change, so
    /// there is nothing to notify either way.
    #[must_use = "queue this via queue_steel_call, or the query-change notification is silently skipped"]
    pub(in crate::editor) fn pop_grapheme(&mut self) -> Option<SteelVal> {
        if self.query.is_empty() {
            return None;
        }
        self.query.truncate(hume_rope::grapheme::prev_str_boundary(
            &self.query,
            self.query.len(),
        ));
        self.rerank();
        self.notify_query_change()
    }

    /// Shared tail of `insert_char`/`pop_grapheme`: the mode's
    /// `on_query_change` callback, if any, and — only when there is one,
    /// i.e. only for a live session — arms `requery_armed` so `is_pending`
    /// reads "still arriving" for the whole stop/debounce/respawn gap a
    /// requery opens, not just while a source happens to be attached.
    fn notify_query_change(&mut self) -> Option<SteelVal> {
        let cb = self.mode.on_query_change()?;
        self.requery_armed = true;
        Some(cb)
    }

    /// Replaces the query wholesale and reranks — test-only: production code
    /// only ever changes the query one grapheme at a time, through
    /// `insert_char`/`pop_grapheme`.
    #[cfg(test)]
    fn set_query(&mut self, query: String) {
        self.query = query;
        self.rerank();
    }

    /// Moves `selected` by `delta`, saturating at both ends of `filtered`
    /// with no wraparound, then clamps `scroll` so `selected` stays inside
    /// the `visible_rows`-tall window (`clamp_scroll_to_window`, shared with
    /// `clamp_drawer_scroll`). No-op when `filtered` is empty or
    /// `visible_rows` is `0`. Page moves are simply `delta = ±visible_rows`.
    pub(in crate::editor) fn move_selection(&mut self, delta: isize, visible_rows: usize) {
        if self.filtered.is_empty() {
            return;
        }
        let max = self.filtered.len() - 1;
        self.selected = (self.selected as isize + delta).clamp(0, max as isize) as usize;

        if visible_rows == 0 {
            return;
        }
        self.scroll =
            hume_ui::menu_box::clamp_scroll_to_window(self.selected, self.scroll, visible_rows);
        debug_assert!(self.scroll <= self.selected && self.selected < self.scroll + visible_rows);
    }

    pub(in crate::editor) fn query(&self) -> &str {
        &self.query
    }

    pub(in crate::editor) fn selected(&self) -> usize {
        self.selected
    }

    pub(in crate::editor) fn scroll(&self) -> usize {
        self.scroll
    }

    /// Number of items currently ranked (i.e. matching the query).
    pub(in crate::editor) fn matched_len(&self) -> usize {
        self.filtered.len()
    }

    /// Total number of items ever pushed, regardless of the current query.
    pub(in crate::editor) fn total_len(&self) -> usize {
        self.items.len()
    }

    /// Display strings of up to `rows` items starting at `scroll`, in ranked
    /// order — the window the picker panel paints. The selected row's
    /// on-screen position is `selected - scroll`.
    pub(in crate::editor) fn window(&self, rows: usize) -> impl Iterator<Item = &str> + '_ {
        self.filtered
            .iter()
            .skip(self.scroll)
            .take(rows)
            .map(|&idx| self.items[idx as usize].display.as_str())
    }

    /// Payload of the currently selected item, or `None` when nothing
    /// matches.
    pub(in crate::editor) fn selected_payload(&self) -> Option<&SteelVal> {
        self.filtered
            .get(self.selected)
            .map(|&idx| &self.items[idx as usize].payload)
    }

    /// Cheap `Rc` clone — the accept/dismiss dispatch fires this via
    /// `queue_steel_call`; the store itself never invokes it.
    pub(in crate::editor) fn on_select(&self) -> &SteelVal {
        &self.on_select
    }

    /// The `#:actions` proc bound to `key`, if any — tried only after every
    /// built-in picker key, so an entry for a key `handle_picker_key`
    /// already matches (movement, `Backspace`, `Enter`, `Escape`, query
    /// input) can never be reached from here.
    pub(in crate::editor) fn action_for(&self, key: KeyEvent) -> Option<&SteelVal> {
        let key = canonical(key);
        self.actions
            .iter()
            .find_map(|(bound, proc)| (*bound == key).then_some(proc))
    }

    /// Rebuilds `filtered` from `items`/`query` — the ranking-only half
    /// shared by [`rerank`](Self::rerank) (used directly by `replace`,
    /// `set_query`, `insert_char`, and `pop_grapheme` — a live session
    /// recomputes the same identity permutation on every keystroke rather
    /// than skip the call, since the result is a pure function of
    /// `items.len()` either way and a separate skip-path bought nothing
    /// observable) and
    /// [`rerank_keeping_selection`](Self::rerank_keeping_selection); neither
    /// touches `selected`/`scroll` itself.
    fn rebuild_filtered(&mut self) {
        // `mode.is_live()` skips the local fuzzy filter at any query,
        // separately from an empty query, which always takes this branch
        // regardless of liveness — both land here in insertion order by
        // construction: an empty query avoids relying on nucleo's
        // (undocumented) all-equal-score behavior and skips scoring
        // entirely on the dominant streaming-ingest path (empty query while
        // a spawned source drains batches); a live session skips it because
        // the query already selects what the source returns (e.g. `rg`'s
        // own regex match), so a second fuzzy pass over already-matched
        // rows would drop legitimate non-fuzzy hits — `foo.*bar`
        // fuzzy-matching almost nothing it just found.
        if self.query.is_empty() || self.mode.is_live() {
            self.filtered.clear();
            self.filtered.extend(0..self.items.len() as u32);
        } else {
            let pattern = self.matcher.parse(&self.query);
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

    /// Resets `selected`/`scroll` to `0` — used directly by `rerank`, and by
    /// `rerank_keeping_selection`'s fallback when the previously selected
    /// item didn't survive the rebuild: there is no old selection worth
    /// trying to preserve once the ranking (or the query driving it) has
    /// changed meaning.
    fn reset_cursor(&mut self) {
        self.selected = 0;
        self.scroll = 0;
    }

    /// Rebuilds `filtered` and resets the cursor — for an item-list mutation
    /// under a changed set of items (`replace`), where the previous
    /// `filtered` no longer names anything reliable.
    fn rerank(&mut self) {
        self.rebuild_filtered();
        self.reset_cursor();
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
            None => self.reset_cursor(),
        }
    }
}

/// The open picker's session, but only if its token is `token` — the shared
/// guard for every token-scoped picker mutation (`picker-push!`,
/// `picker-replace!`, `picker-source-spawn!`, `picker-source-stop!`, a
/// scoped `picker-close!`). A mismatch, or no picker open at all, is
/// expected-normal — a late callback racing a picker the user already
/// closed or replaced — so callers treat `None` as a silent no-op, never an
/// error; none of `PickerSession`'s own mutators re-check the token
/// themselves once a caller has reached one through here.
pub(in crate::editor) fn session_for_token(
    state: &mut super::EditorState,
    token: u64,
) -> Option<&mut PickerSession> {
    state
        .config
        .picker
        .as_mut()
        .filter(|session| session.token() == token)
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
pub(in crate::editor) fn open_picker(
    state: &mut super::EditorState,
    lsp: Option<&mut super::lsp::LspState>,
    session: PickerSession,
) {
    super::lsp::completion::clear_completion_menu(state, lsp);
    close_picker(state, SteelVal::BoolV(false));
    state.config.picker = Some(session);
}

/// Single close chokepoint for the picker: ends the session (if one is
/// open) and fires exactly one callback with `payload` — `on_select` unless
/// `callback` overrides it. Shared by `Esc`, `Enter` (with the selected
/// payload), a bound `#:actions` key, `picker-close!`, and `open_picker`'s
/// replace-on-open path — one chokepoint, not one copy per caller.
///
/// `Editor::reset_config_state` is a second, deliberate exit from this
/// "fires exactly once" contract: its wholesale `ConfigState` rebuild drops
/// `state.config.picker` directly (never calling this function) along with
/// the `pending_work` queue this function would have pushed the callback
/// onto — the outgoing engine that owns the callback is seconds from being
/// dropped, so firing it would be observable to nothing.
pub(in crate::editor) fn close_picker_with(
    state: &mut super::EditorState,
    callback: Option<SteelVal>,
    payload: SteelVal,
) {
    let Some(session) = state.config.picker.take() else {
        return;
    };
    let callback = callback.unwrap_or_else(|| session.on_select().clone());
    state.queue_steel_call(callback, vec![payload]);
}

/// `close_picker_with`'s common case: fire `on_select` itself.
pub(in crate::editor) fn close_picker(state: &mut super::EditorState, payload: SteelVal) {
    close_picker_with(state, None, payload);
}

/// One `PickerItem` from a display string, its own payload — shared by this
/// module's own tests and `tests/unix/picker_source.rs`, which spawns real
/// child processes and so can't live in this (non-unix-gated) module.
#[cfg(test)]
pub(in crate::editor) fn item(display: &str) -> PickerItem {
    PickerItem {
        display: display.to_string(),
        payload: SteelVal::StringV(display.into()),
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests;
