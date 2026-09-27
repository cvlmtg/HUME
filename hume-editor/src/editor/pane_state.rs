//! Per-(pane, buffer) and per-pane editor state bundles.
//!
//! [`PaneBufferState`] holds all per-(pane, buffer) mutable facts: selections,
//! search cursor, and the in-progress insert session's typed-run/autoindent
//! bookkeeping. Adding a new per-(pane, buffer) field later requires changing
//! exactly one struct and one Default impl — not four parallel maps.
//!
//! [`PaneView`] groups the three per-pane maps — `state`, `jumps`, `render` —
//! so callers deal with one field on [`super::EditorState`] instead of three.
//!
//! The in-progress insert/paste undo group itself is not here — see
//! [`super::edit_session::EditSession`], `EditorState::active_session`.

use hume_engine::pipeline::{BufferId, EngineView, PaneId, PanePool};
use slotmap::SecondaryMap;

use super::Editor;
use super::LayoutKey;
use super::commands::FocusedPane;
use super::search::SearchCursor;
use crate::editor::buffer::Buffer;
use crate::editor::buffer::store::BufferStore;
use hume_editing::selection::{Selection, SelectionSet};
use hume_rope::offset::{CharOffset, ExclusiveRange};

/// The span typed since an open insert session's entry command positioned the
/// cursor — one (anchor, end) pair per selection, index-paired and always the
/// same length (both `Vec`s are seeded together by `begin_typed_run` and
/// remapped together by `apply_doc_edit_grouped`, which is the only writer
/// after seeding).
pub(crate) struct TypedRun {
    /// Start of each selection's typed span, kept in post-edit coordinates
    /// with `Assoc::Before` — a keystroke exactly at the anchor is typed
    /// content, so the anchor must stay left of it.
    pub anchors: Vec<CharOffset>,
    /// Exclusive end of each typed span, kept with `Assoc::After` (opposite
    /// of `anchors`) so it tracks what was written rather than where the
    /// cursor happens to sit: a real keystroke at the run's end pushes it
    /// forward, an auto-paired closer pushes it past both inserted chars, and
    /// a skip-close (which edits nothing) leaves it where it was.
    pub ends: Vec<CharOffset>,
}

/// All per-(pane, buffer) editor state bundled into one struct.
///
/// Stored in `EditorState.panes.state: SecondaryMap<PaneId, SecondaryMap<BufferId, PaneBufferState>>`.
/// Default initialisation is used at every seed site — callers override
/// `selections` with `buffer.initial_sels()` when seeding for the first time.
#[derive(Default)]
pub(crate) struct PaneBufferState {
    /// The focused pane's cursor / selection state for this buffer. Private:
    /// every write goes through [`PaneBufferState::set_selections`] or the
    /// [`PaneBufferState::take_selections`]/[`PaneBufferState::restore_selections`]
    /// pair, which is what raises [`PaneBufferState::reveal_pending`] — see
    /// that field's own doc.
    selections: SelectionSet,
    /// Per-pane cursor through the buffer's shared match list.
    pub search_cursor: SearchCursor,
    /// The open insert session's typed span, kept in post-edit coordinates by
    /// `apply_doc_edit_grouped`. `Some` from the moment the session's entry
    /// command positions the cursor (`begin_typed_run`) until
    /// `tear_down_insert` consumes it on exit, for every insert entry
    /// (`i`/`a`/`o`/`O`/`A`/`I`/`c`/…).
    pub typed_run: Option<TypedRun>,
    /// Per-selection range of leading whitespace *this* insert session
    /// auto-inserted (`o`/`O`/Enter's auto-indent), kept in post-edit
    /// coordinates by `apply_doc_edit_grouped` the same way `typed_run` is.
    /// `None` when nothing has armed a record yet this session (`i`/`I`/`c`
    /// entry, or before the first Enter/`o`/`O`).
    ///
    /// Read on exit (`hume_ops::edit::owned_blank_indent`) to decide whether
    /// the cursor's current blank line is whitespace *this session itself*
    /// inserted, rather than pre-existing or hand-typed whitespace — the
    /// vacate-on-exit trim must never touch the latter. A positional record
    /// rather than a bool ("is some trim pending") so ownership is re-derived
    /// from the buffer at exit instead of relying on every cursor-motion key
    /// handler remembering to invalidate a flag: a motion off this line
    /// leaves the record pointing at a line the cursor no longer occupies,
    /// which the containment check rejects on its own.
    pub autoindent: Option<Vec<ExclusiveRange<CharOffset>>>,
    /// Set by `begin_typed_run` from its `ExitCursor` parameter for `a`/`A`/
    /// `o`/`O` entry (never for `i`/`I`/`c`). Decides where an *empty* typed
    /// run's cursor lands on exit — step one grapheme back (so `a<Esc>` is a
    /// round trip) rather than staying put. Lives here, beside the typed run
    /// it decides about, so a session `replay_dot` re-enters sets and reads
    /// it exactly as a live one does.
    pub step_back_on_exit: bool,
    /// Whether the open insert session was entered via a ring-capturing kill
    /// (bare or `"k`-prefixed `c` — an explicit-register change writes no
    /// stamp and must not set this). Set only by `cmd_change`; lives here for
    /// the same reason `step_back_on_exit` does. Read by `tear_down_insert`:
    /// every keystroke typed during the session bumps `BufferStore::
    /// edit_seq`, so the `PasteStamp` `cmd_change` wrote (pointing at the
    /// just-replaced text) goes stale by the time the session closes —
    /// refreshing its `seq` here is what keeps `c <text> <Esc> p` reading
    /// the kill ring instead of the clipboard.
    pub kill_opened_session: bool,
    /// A fact worth re-settling the viewport for happened since the last
    /// frame handled one — raised at the source, not inferred from state.
    ///
    /// The selection funnel — [`PaneBufferState::set_selections`]/
    /// `restore_selections`/`translate_selections_in_place` — is this
    /// field's only writer: set whenever a write actually moves the primary
    /// head (not on a write that leaves it where it was — a
    /// `commands::scroll_view` that couldn't carry a selection past a
    /// virtual block, say, leaves this `false` for that write). Every
    /// non-selection source (a resize, a wrap-mode change, a buffer switch,
    /// a decoration-generation change) is folded into `frame.rs`'s scroll
    /// step instead, as a comparison against [`PaneBufferState::last_layout_key`]
    /// — see that field's own doc for why a derived comparison needs no
    /// raise site per source.
    ///
    /// Read and cleared every frame by `frame.rs`'s scroll step, alongside
    /// that comparison: either one being true means the vertical
    /// `Viewport::reveal` correction runs this frame; both false means
    /// `cursor::content_pos` re-derives the caret's current position
    /// without moving the viewport — the same "hidden caret until an
    /// ordinary motion resyncs the view" behavior a scroll that parks a
    /// selection behind a virtual block always could produce.
    pub reveal_pending: bool,
    /// This pane's [`LayoutKey`] as of the last frame that read one —
    /// `frame.rs`'s scroll step's own memo, compared against a fresh
    /// `EditorState::layout_key(pane)` every frame to derive reveals for
    /// every non-selection source at once: a resize, a wrap-mode pin or
    /// toggle, a buffer switch (the very first read for a `(pane, buffer)`
    /// pair is `None`, so it always differs), and a decoration-generation
    /// change (inlay hints, EOL text, virtual lines) all show up as *some*
    /// `LayoutKey` field changing, rather than needing their own raise site
    /// each. `pub(in crate::editor)`, matching `reveal_pending`'s own
    /// visibility: `frame.rs` is its only reader, and it is a pure memo
    /// with no invariant to funnel through a narrower API.
    ///
    /// Deliberately not reset on a buffer switch: a pane revisiting a
    /// buffer it showed before, with every layout input still identical to
    /// what it read last time, finds a matching key and stays quiet —
    /// coherent with the parked-view model this whole mechanism serves
    /// (switching away and back is not itself a change), and the
    /// counterpart to `reveal_pending` needing no reset either (see
    /// `frame.rs`'s prune-cache doc: both die with the pane's own
    /// `SecondaryMap` entry, same as everything else on this struct).
    pub(in crate::editor) last_layout_key: Option<LayoutKey>,
}

impl PaneBufferState {
    /// Read-only access to the current selections.
    pub(crate) fn selections(&self) -> &SelectionSet {
        &self.selections
    }

    /// Replace the selections outright, raising [`PaneBufferState::reveal_pending`]
    /// iff the primary head actually moved. The ordinary write path for a
    /// caller that already holds the new value (as opposed to
    /// [`PaneBufferState::take_selections`]'s destructive-read pattern).
    pub(in crate::editor) fn set_selections(&mut self, new: SelectionSet) {
        let old_head = self.selections.primary().head();
        self.restore_selections(new, old_head);
    }

    /// Take ownership of the current selections, replacing them with the
    /// default (a single collapsed cursor at char 0) — for a caller that
    /// needs to destructively consume them (typically to feed a pure
    /// `(&BufferText, SelectionSet) -> SelectionSet` motion/edit) without a
    /// clone. The default is transient: a panic before
    /// [`PaneBufferState::restore_selections`] runs leaves it in place
    /// rather than corrupting a partially-applied result, the same
    /// infallible-closure assumption `apply_doc_motion` already documented.
    ///
    /// Pairs with `restore_selections`, which must be called with the
    /// primary head this returned before this state is next read.
    pub(in crate::editor) fn take_selections(&mut self) -> SelectionSet {
        std::mem::take(&mut self.selections)
    }

    /// Write `new` back after a [`PaneBufferState::take_selections`], raising
    /// [`PaneBufferState::reveal_pending`] iff `new`'s primary head differs
    /// from `old_head` — the head `take_selections` returned's own value,
    /// captured by the caller before transforming it. Comparing against a
    /// caller-supplied `old_head` rather than `self.selections.primary().head()`
    /// is what makes this safe to call after `take_selections` already left
    /// `self.selections` at its transient default.
    pub(in crate::editor) fn restore_selections(
        &mut self,
        new: SelectionSet,
        old_head: CharOffset,
    ) {
        if new.primary().head() != old_head {
            self.reveal_pending = true;
        }
        self.selections = new;
    }

    /// In-place remap for a sibling pane's selections after an edit another
    /// pane made to the same buffer, raising [`PaneBufferState::reveal_pending`]
    /// iff the primary head actually moved — same rule
    /// `set_selections`/`restore_selections` apply, since a sibling pane can
    /// be visible in its own split with the shifted position now out of its
    /// own view.
    pub(in crate::editor) fn translate_selections_in_place(
        &mut self,
        edits: &[hume_rope::offset::ExclusiveRange<CharOffset>],
        cs: &hume_editing::changeset::ChangeSet,
        text_pre: &hume_editing::text::BufferText,
    ) {
        let old_head = self.selections.primary().head();
        self.selections.translate_in_place_with(edits, cs, text_pre);
        if self.selections.primary().head() != old_head {
            self.reveal_pending = true;
        }
    }
}

// ── Construction helpers ──────────────────────────────────────────────────────

/// Construct a fresh [`PaneBufferState`] for `buf` — SSOT for the initial-state
/// value. All seed sites must call this rather than building the struct literal
/// directly, so that adding a new field with a non-default initialiser requires
/// only one edit here.
pub(in crate::editor) fn fresh_from_buf(buf: &Buffer) -> PaneBufferState {
    PaneBufferState {
        selections: buf.initial_sels(),
        ..PaneBufferState::default()
    }
}

/// Ensure `pane_state[pid][bid]` exists, seeding with [`fresh_from_buf`] if absent.
/// Idempotent — safe to call even if the entry was already seeded.
///
/// Panics if `pid` or `bid` is not a live key; that is a caller-contract
/// violation (the pane or buffer was never opened), not a recoverable error.
/// Trusted mint for every synchronous caller that already knows `pid` is
/// live by construction (it was just resolved, split, or opened in the same
/// call) — see [`try_ensure`] for a caller crossing an async boundary, where
/// that's no longer guaranteed.
pub(in crate::editor) fn ensure<'a>(
    pane_state: &'a mut SecondaryMap<PaneId, SecondaryMap<BufferId, PaneBufferState>>,
    buffers: &BufferStore,
    panes: &PanePool,
    pid: PaneId,
    bid: BufferId,
) -> &'a mut PaneBufferState {
    try_ensure(pane_state, buffers, panes, pid, bid).expect("pid must be a live PaneId")
}

/// [`ensure`]'s validating counterpart — for a caller whose `pid` was
/// captured before crossing an async boundary (an LSP response, a queued
/// Steel callback) and may have since closed. `bid` still panics on a dead
/// key: every current caller reaches this only once its own generation/
/// anchor check has already proven the buffer live, so that half of the
/// contract still holds — only `pid`'s liveness crosses the boundary
/// unchecked.
///
/// Checked against `panes` (the engine's own [`PanePool`]), not inferred
/// from `pane_state.entry(pid)`: `SecondaryMap::remove` (`drop_pane_state`)
/// drops a closed pane's slot back to vacant at version 0, and
/// `SecondaryMap::entry` returns the same `Vacant` variant for that as it
/// does for a `pid` that simply never touched this map — the two are
/// indistinguishable from `pane_state` alone unless the slot has since been
/// reused by a *newer* pane (whose version the closed `pid` no longer
/// matches). `PanePool` is the actual liveness source of truth; asking it
/// first means a closed pane errors here every time, not only once its slot
/// happens to be recycled.
pub(in crate::editor) fn try_ensure<'a>(
    pane_state: &'a mut SecondaryMap<PaneId, SecondaryMap<BufferId, PaneBufferState>>,
    buffers: &BufferStore,
    panes: &PanePool,
    pid: PaneId,
    bid: BufferId,
) -> Result<&'a mut PaneBufferState, String> {
    if !panes.contains_key(pid) {
        return Err(super::commands::TargetError::PaneClosed.to_string());
    }
    // `pid` is confirmed live in `panes` above, and every `PaneId` in
    // existence is minted from that same slotmap — so `pane_state` (a
    // `SecondaryMap` over the same keyspace) can never hold a *newer*
    // version at this index than the one `pid` already carries. `entry`
    // therefore cannot return `None` here.
    let inner = pane_state
        .entry(pid)
        .expect("pid confirmed live by panes.contains_key above")
        .or_default();
    Ok(inner
        .entry(bid)
        .expect("bid must be a live BufferId")
        .or_insert_with(|| fresh_from_buf(buffers.get(bid))))
}

/// Collapse `pane_state[pid][bid]`'s selection onto `char_pos`, without
/// switching focus or recording a jump entry. The primitive every cursor
/// placement outside the focused-buffer fast path (`set_current_selections`)
/// reduces to: [`park_cursor_at`] is its line/grapheme-column convenience for
/// a caller with no char position yet, and `goto_location`
/// (`editor/lsp/edits.rs`) — whose target is already char-indexed — calls
/// this directly.
pub(in crate::editor) fn write_cursor(
    pane_state: &mut SecondaryMap<PaneId, SecondaryMap<BufferId, PaneBufferState>>,
    buffers: &BufferStore,
    panes: &PanePool,
    pid: PaneId,
    bid: BufferId,
    char_pos: CharOffset,
) {
    ensure(pane_state, buffers, panes, pid, bid)
        .set_selections(SelectionSet::single(Selection::collapsed(char_pos)));
}

/// Collapse `pane_state[pid][bid]`'s selection onto a 0-based
/// `(line, grapheme_col)`, clamping the line to the buffer's last content
/// line. Shared by every caller that parks a cursor at a line/column pair —
/// a read-only view's opening position and a CLI startup position both
/// reduce to this.
pub(in crate::editor) fn park_cursor_at(
    pane_state: &mut SecondaryMap<PaneId, SecondaryMap<BufferId, PaneBufferState>>,
    buffers: &BufferStore,
    panes: &PanePool,
    pid: PaneId,
    bid: BufferId,
    line0: hume_rope::line::ContentLine,
    grapheme_col0: hume_rope::column::GraphemeCol,
) {
    let text = buffers.get(bid).text();
    let line = line0.min(text.last_content_line());
    let char_pos = hume_editing::lines::place_grapheme_column(text, line.into(), grapheme_col0);
    write_cursor(pane_state, buffers, panes, pid, bid, char_pos);
}

/// Groups the three per-pane maps that live on [`super::EditorState`].
///
/// Bundles `state` (per-(pane,buffer) selections/groups), `jumps` (cursor
/// history), and `render` (per-pane highlight/sign/inlay-hint/virtual-line
/// handles, bundled in [`hume_decorations::PaneDecorationHandles`] since
/// `build_pane` always allocates and `drop_pane_state` always drops them
/// together) so `EditorState` exposes one field instead of three. The map
/// types and keying are unchanged; NLL still allows simultaneous mutable
/// borrows of different fields (e.g. `panes.state` and `panes.jumps` in
/// `buffer::lifecycle::switch_to_buffer_with_jump`).
#[derive(Default)]
pub(crate) struct PaneView {
    pub(in crate::editor) state: SecondaryMap<PaneId, SecondaryMap<BufferId, PaneBufferState>>,
    pub(in crate::editor) jumps: super::jump_list::JumpLists,
    pub(in crate::editor) render: SecondaryMap<PaneId, hume_decorations::PaneDecorationHandles>,
}

impl PaneView {
    /// The seeded [`PaneBufferState`] for `(pid, bid)`, or `None` when the
    /// pane has no map yet or never showed `bid`. Read-side counterpart of
    /// [`ensure`], which seeds rather than reporting absence.
    pub(in crate::editor) fn buffer_state(
        &self,
        pid: PaneId,
        bid: BufferId,
    ) -> Option<&PaneBufferState> {
        self.state.get(pid)?.get(bid)
    }
}

/// Build a new pane viewing `buffer_id`: sign column, line-number gutter,
/// bracket-match/search-match/diagnostic/extra-highlight sources, inlay-hint
/// decoration, virtual-line source, line-background tint (all from
/// [`hume_decorations::build_providers`]), and completion/hover/selection-
/// menu/LSP overlays (from [`hume_ui::register_overlays`]). Wrap mode is not
/// seeded here — the new pane starts with no override for any buffer
/// (`Pane::new`'s empty `wraps` map) and resolves it lazily on every read
/// (`commands::effective_wrap_mode`).
///
/// Returns the pane with its freshly-allocated `PaneDecorationHandles` —
/// every pane gets its own buffers (never shared), so each pane's
/// decorations come from that pane's own buffer and viewport. The caller
/// stores them in `EditorState.panes.render` keyed by the new pane's id.
///
/// The gutter column is added with its default style; `prepare_frame` syncs
/// the buffer-resolved `line-number-style` into every pane's gutter before
/// each render (see `sync_line_number_style`), so the seeded style never
/// reaches a frame.
///
/// Single source of truth for pane construction — every creation site
/// (`Editor::open`'s bootstrap pane, `commands::open_pane`) goes through
/// this, so panes render identically. `Pane::new` alone has an empty
/// `ProviderSet` (no gutter column). Sole caller of
/// `hume_decorations::build_providers`/`hume_ui::register_overlays` — the
/// two sibling calls that together populate one pane's `ProviderSet`, one
/// per crate, since decoration providers and overlay widgets live apart.
pub(in crate::editor) fn build_pane(
    registry: &mut hume_engine::theme::ScopeRegistry,
    views: &hume_ui::OverlayViews,
    buffer_id: BufferId,
) -> (
    hume_engine::pane::Pane,
    hume_decorations::PaneDecorationHandles,
) {
    // Interns the engine's own `DEFAULT_GUTTER_SCOPE` constant rather than
    // repeating the "ui.linenr" literal here — the two must resolve to the
    // same scope: `compose_gutter`'s own fallback
    // (`EngineView::default_gutter_scope`) interns that same constant, and a
    // blank sign slot / line-number cell rendering under a different
    // `ScopeId` than the row-fill fallback would silently disagree on
    // styling.
    let linenr_scope = registry.intern(hume_engine::providers::DEFAULT_GUTTER_SCOPE.0);
    let linenr_selected_scope = registry.intern(hume_engine::theme::ui_scopes::LINENR_SELECTED);

    let mut providers = hume_engine::providers::ProviderSet::new();
    let decoration_handles = hume_decorations::build_providers(&mut providers, linenr_scope);
    providers.add_gutter_column(Box::new(
        hume_engine::builtins::line_number::LineNumberColumn::new(
            linenr_scope,
            linenr_selected_scope,
        ),
    ));
    hume_ui::register_overlays(&mut providers, views);

    let pane = hume_engine::pane::Pane {
        providers,
        ..hume_engine::pane::Pane::new(buffer_id)
    };
    (pane, decoration_handles)
}

impl super::EditorState {
    /// `bid`'s state *as seen in the focused pane*, or `None` when `bid` is
    /// unseeded there — a stale id, or `bid` not open in the focused pane.
    /// `bid` is caller-supplied, so this always looks the pane state up by
    /// the explicit id rather than assuming it matches the focused buffer.
    /// Strictly focused-pane callers only — a caller with an explicit
    /// [`crate::editor::commands::CommandPane`] (not necessarily focused)
    /// reads `state.panes.state[t.pid()][t.bid(view)]` directly instead.
    pub(in crate::editor) fn focused_buffer_state(
        &self,
        bid: BufferId,
    ) -> Option<&PaneBufferState> {
        self.panes.buffer_state(self.focus.id(), bid)
    }

    /// [`focused_buffer_state`](Self::focused_buffer_state) for the buffer the
    /// focused pane is currently *showing*, where absence is a violated
    /// invariant rather than a case to handle: opening or switching to a
    /// buffer seeds its state in that pane, so an unseeded focused buffer
    /// means a pane and its state map disagree.
    ///
    /// The one lookup behind every "the cursor/search state right now" reader
    /// — `commands::current_selections`, the statusline's own accessors —
    /// which would otherwise each index `panes.state[..][..]` and each panic
    /// with slotmap's own message instead of naming what actually broke.
    pub(crate) fn focused_buffer_state_or_panic(&self, bid: BufferId) -> &PaneBufferState {
        self.focused_buffer_state(bid).expect(
            "focused pane has no seeded state for the buffer it is showing — \
             pane.buffer_id and panes.state are out of sync",
        )
    }

    /// Every pane currently showing `bid`, in the order `(buffer-panes
    /// pane)` hands back to Steel: the focused pane first (if it shows
    /// `bid`), then the rest of the active tab (`view.active_pane_ids`'s own
    /// leaf order), then every other tab
    /// (`view.panes.every_pane_across_all_tabs`'s order). Explicit
    /// enumeration, not a single-pane guess — a caller wanting just one
    /// picks via `.first()` at its own call site. Pane counts are small, so
    /// the final dedup is a linear `out.contains` rather than a `HashSet`.
    pub(in crate::editor) fn buffer_panes(&self, view: &EngineView, bid: BufferId) -> Vec<PaneId> {
        let focused = self.focus.id();
        let active = view.active_pane_ids();
        let mut out = Vec::new();
        if view.panes.get(focused).is_some_and(|p| p.buffer_id == bid) {
            out.push(focused);
        }
        out.extend(
            active
                .iter()
                .copied()
                .filter(|&pid| pid != focused && view.panes[pid].buffer_id == bid),
        );
        let rest: Vec<PaneId> = view
            .panes
            .every_pane_across_all_tabs()
            .filter(|&(pid, p)| p.buffer_id == bid && !out.contains(&pid))
            .map(|(pid, _)| pid)
            .collect();
        out.extend(rest);
        out
    }
}

impl Editor {
    // ── Pane-state accessors ──────────────────────────────────────────────────

    /// The focused pane's effective wrap mode: pane override → buffer
    /// override → global default (see `commands::effective_wrap_mode`).
    #[cfg(test)]
    pub(in crate::editor) fn focused_wrap_mode(&self) -> hume_engine::pane::WrapMode {
        self.pane_wrap_mode(self.state.focus.id())
    }

    /// `pid`'s effective wrap mode for the buffer it currently views.
    fn pane_wrap_mode(&self, pid: PaneId) -> hume_engine::pane::WrapMode {
        let pane = &self.view.panes[pid];
        let doc = self.state.buffers.get(pane.buffer_id);
        super::commands::effective_wrap_mode(doc, &self.state.settings, pane)
    }

    /// Pin `fp`'s wrap mode to `mode`, for the buffer it
    /// currently views — the write path behind `:set pane wrap-mode=…`.
    ///
    /// Always writes an explicit override, even `WrapMode::None` (an
    /// explicit "don't wrap" pin): `:set pane` is itself an explicit pane
    /// action, so from here on this pane stops following `:set buffer`/
    /// `:set global wrap-mode=…` *for this buffer* until `:wrap` or another
    /// `:set pane` changes it again — there is no command that clears the
    /// pin back to inheriting. The pin lives in `Pane::wraps`, keyed by
    /// buffer (see `WrapOverride`), so it does not follow the pane to a
    /// buffer it switches to next; switching back to this buffer restores it.
    ///
    /// `WrapOverride::saved` (the `:wrap` toggle-on restore target) is synced
    /// to this pin only when `mode` itself wraps — pinning *off* deliberately
    /// leaves it alone, so whatever `saved` already pointed at (a prior
    /// wrapping pin, or "was inheriting") survives as the toggle-on target
    /// instead of being erased by this pin.
    ///
    /// Zeroes horizontal scroll (meaningless once wrapped) on any actual
    /// change to the pane's *effective* mode — see `toggle_wrap`'s doc for
    /// the full rationale, shared by both functions.
    pub(in crate::editor) fn set_wrap_override(
        &mut self,
        fp: FocusedPane,
        mode: hume_engine::pane::WrapMode,
    ) {
        let pid = fp.pid();
        let before = self.pane_wrap_mode(pid);
        let pane = &mut self.view.panes[pid];
        let mut wrap = pane.wrap();
        wrap.mode = Some(mode);
        if mode.is_wrapping() {
            wrap.saved = Some(mode);
        }
        pane.set_wrap(wrap);
        if mode != before {
            self.view.panes[pid].viewport.reset_horizontal();
        }
    }

    /// Toggle `fp`'s wrapping on/off, for the buffer it
    /// currently views — the write path behind `:wrap`/`:toggle-soft-wrap`.
    /// Returns the new wrapping state.
    ///
    /// Turning wrapping *off* stashes the pane's current override into
    /// `WrapOverride::saved` — `None` if it was inheriting from the
    /// buffer/global setting, `Some(m)` if it was explicitly pinned to `m` —
    /// then pins the pane to `WrapMode::None`. Like `set_wrap_override`,
    /// this writes to `Pane::wraps` keyed by the current buffer, so it does
    /// not follow the pane to a buffer it switches to next.
    ///
    /// Turning wrapping back *on* restores that provenance rather than a
    /// frozen resolved value: a pane that was inheriting goes back to
    /// inheriting, so it keeps following later `:set buffer`/`:set global`
    /// changes instead of getting stuck on whatever style happened to be
    /// active at toggle-off time; a pane that was explicitly pinned goes
    /// back to that exact pin. If restoring "inheriting" wouldn't actually
    /// wrap (the buffer/global setting is `none`, or this pane has never
    /// wrapped before), falls back to pinning the *configured* global style
    /// (`EditorSettings::wrap_mode`) if that wraps, else `DEFAULT_WRAP_STYLE`
    /// — `:wrap` must always visibly wrap, never silently no-op, but must
    /// not override a deliberate global `none` with a style the user never
    /// asked for.
    ///
    /// Toggling always flips whether the pane is actually wrapping (off→on
    /// is guaranteed to end up wrapping, by the fallback above; on→off
    /// always ends at `WrapMode::None`), so horizontal scroll — meaningless
    /// once wrapped — is unconditionally zeroed. This is a real write, not
    /// just belt-and-suspenders: `Viewport::reveal_horizontal`
    /// also zeroes it for any wrapping pane on the next frame, but only a
    /// frame later, and code reading the viewport between this call and the
    /// next render (including several existing tests) expects it already
    /// zero.
    ///
    /// The top's slot, by contrast, is left alone here on purpose: it
    /// addresses a display line inside the top's line's whole visual block
    /// (`before` + content display lines + `after`) in *either* wrap mode —
    /// a mode change can leave it past the new block's display-line count
    /// (off→on starts a narrower block; on→on width/style changes can
    /// shrink it), and that out-of-range case is exactly what the next
    /// `Viewport::top_at` read repairs, so there's no need
    /// to throw the address away here. What clamping *cannot* catch: only a
    /// `content`-side change (not this function) grows the block, so a slot
    /// that addressed an `after` display line in no-wrap can still be in
    /// range once wrapping grows `content` — landing on a wrap display line
    /// of the line's own text instead of the virtual display line it used
    /// to point at. Silent, not a bug this function fixes.
    pub(in crate::editor) fn toggle_wrap(&mut self, fp: FocusedPane) -> bool {
        use hume_engine::pane::{DEFAULT_WRAP_STYLE, WrapMode};

        let pid = fp.pid();
        let now_wrapping = if self.pane_wrap_mode(pid).is_wrapping() {
            let pane = &mut self.view.panes[pid];
            let mut wrap = pane.wrap();
            wrap.saved = wrap.mode;
            wrap.mode = Some(WrapMode::None);
            pane.set_wrap(wrap);
            false
        } else {
            let pane = &mut self.view.panes[pid];
            let mut wrap = pane.wrap();
            wrap.mode = wrap.saved;
            pane.set_wrap(wrap);
            if !self.pane_wrap_mode(pid).is_wrapping() {
                let global = self.state.settings.wrap_mode;
                let fallback = if global.is_wrapping() {
                    global
                } else {
                    DEFAULT_WRAP_STYLE
                };
                let pane = &mut self.view.panes[pid];
                let mut wrap = pane.wrap();
                wrap.mode = Some(fallback);
                pane.set_wrap(wrap);
            }
            true
        };
        self.view.panes[pid].viewport.reset_horizontal();
        now_wrapping
    }
}

#[cfg(test)]
mod tests;
