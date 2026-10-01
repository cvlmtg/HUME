//! Per-(pane, buffer) and per-pane editor state bundles.
//!
//! [`PaneBufferState`] holds all per-(pane, buffer) mutable facts: selections,
//! search cursor, and the in-progress insert session's typed-run/autoindent
//! bookkeeping. Adding a new per-(pane, buffer) field later requires changing
//! exactly one struct and one Default impl, not four parallel maps.
//!
//! [`PaneView`] groups the three per-pane maps (`state`, `jumps`, `render`)
//! so callers deal with one field on [`super::EditorState`] instead of three.
//!
//! The in-progress insert/paste undo group itself is not here; see
//! [`super::edit_session::EditSession`], `EditorState::active_session`.

use hume_engine::pipeline::{BufferId, EngineView, PaneId, PanePool};
use slotmap::SecondaryMap;

use super::Editor;
use super::LayoutKey;
use super::commands::FocusedPane;
use super::search::SearchCursor;
use crate::editor::buffer::Buffer;
use crate::editor::buffer::store::BufferStore;
use hume_editing::changeset::Assoc;
use hume_editing::edit::TextChange;
use hume_editing::selection::{EditView, SelectionSet};
use hume_editing::state::EditState;
use hume_editing::text::BufferText;
use hume_editing::tracked::Tracked;
use hume_rope::cluster::ClusterStart;
use hume_rope::offset::{CharOffset, ExclusiveRange};

/// The span typed since an open insert session's entry command positioned the
/// cursor: one (anchor, end) pair per selection, index-paired and always the
/// same length (both `Vec`s are seeded together by `begin_typed_run` and
/// carried together by [`PaneBufferState::carry`], the only writer after
/// seeding).
pub(crate) struct TypedRun {
    /// Start of each selection's typed span, kept in post-edit coordinates
    /// with `Assoc::Before`: a keystroke exactly at the anchor is typed
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
/// Every seed site goes through [`fresh_from_buf`], which starts from the
/// buffer's initial selections.
pub(crate) struct PaneBufferState {
    /// This pane's selections for this buffer. Private: every write goes
    /// through [`PaneBufferState::set_selections`], [`PaneBufferState::store`]
    /// or [`PaneBufferState::carry`]; the first two raise
    /// [`PaneBufferState::reveal_pending`] (see that field's own doc).
    selections: SelectionSet,
    /// Per-pane cursor through the buffer's shared match list.
    pub search_cursor: SearchCursor,
    /// The open insert session's typed span, carried through every text
    /// change by [`PaneBufferState::carry`]. `Some` from the moment the session's entry
    /// command positions the cursor (`begin_typed_run`) until
    /// `tear_down_insert` consumes it on exit, for every insert entry
    /// (`i`/`a`/`o`/`O`/`A`/`I`/`c`/…).
    pub typed_run: Option<Tracked<TypedRun>>,
    /// Per-selection range of leading whitespace *this* insert session
    /// auto-inserted (`o`/`O`/Enter's auto-indent), carried the same way
    /// `typed_run` is.
    /// `None` when nothing has armed a record yet this session (`i`/`I`/`c`
    /// entry, or before the first Enter/`o`/`O`).
    ///
    /// Read on exit (`hume_ops::edit::owned_blank_indent`) to decide whether
    /// the cursor's current blank line is whitespace *this session itself*
    /// inserted, rather than pre-existing or hand-typed whitespace. The
    /// vacate-on-exit trim must never touch the latter. A positional record
    /// rather than a bool ("is some trim pending") so ownership is re-derived
    /// from the buffer at exit instead of relying on every cursor-motion key
    /// handler remembering to invalidate a flag: a motion off this line
    /// leaves the record pointing at a line the cursor no longer occupies,
    /// which the containment check rejects on its own.
    pub autoindent: Option<Tracked<Vec<ExclusiveRange<CharOffset>>>>,
    /// Set by `begin_typed_run` from its `ExitCursor` parameter for `a`/`A`/
    /// `o`/`O` entry (never for `i`/`I`/`c`). Decides where an *empty* typed
    /// run's cursor lands on exit: step one grapheme back (so `a<Esc>` is a
    /// round trip) rather than staying put. Lives here, beside the typed run
    /// it decides about, so a session `replay_dot` re-enters sets and reads
    /// it exactly as a live one does.
    pub step_back_on_exit: bool,
    /// Whether the open insert session was entered via a ring-capturing kill
    /// (bare or `"k`-prefixed `c`; an explicit-register change writes no
    /// stamp and must not set this). Set only by `cmd_change`; lives here for
    /// the same reason `step_back_on_exit` does. Read by `tear_down_insert`:
    /// every keystroke typed during the session bumps `BufferStore::
    /// edit_seq`, so the `PasteStamp` `cmd_change` wrote (pointing at the
    /// just-replaced text) goes stale by the time the session closes,
    /// and refreshing its `seq` here is what keeps `c <text> <Esc> p` reading
    /// the kill ring instead of the clipboard.
    pub kill_opened_session: bool,
    /// A fact worth re-settling the viewport for happened since the last
    /// frame handled one, raised at the source, not inferred from state.
    ///
    /// Two writers: the selection funnel [`PaneBufferState::set_selections`]/
    /// [`PaneBufferState::store`] (set whenever a write actually moves *this
    /// pane's own* primary head; a `commands::scroll_view` that couldn't
    /// carry a selection past a virtual block, say, leaves this `false` for
    /// that write), and `doc_ops::finish_edit`, set for every real edit this
    /// pane makes regardless of whether it moved the head (`r` replacing the
    /// character under an unmoved cursor still deserves a reveal). A sibling
    /// pane's edit does *not* raise this: [`PaneBufferState::carry`]
    /// only remaps the position, leaving the reveal decision for such
    /// external changes to `frame.rs`'s [`PaneBufferState::last_layout_key`]/
    /// [`PaneBufferState::parked`] comparison instead, which is gated on this
    /// pane's own parked state rather than firing unconditionally.
    ///
    /// Read and cleared every frame by `frame.rs`'s scroll step, alongside
    /// that comparison: either one being true means the vertical
    /// `Viewport::reveal` correction runs this frame; both false means
    /// `cursor::content_pos` re-derives the caret's current position
    /// without moving the viewport: the same "hidden caret until an
    /// ordinary motion resyncs the view" behavior a scroll that parks a
    /// selection behind a virtual block always could produce.
    pub reveal_pending: bool,
    /// This pane's [`LayoutKey`] as of the last frame that read one:
    /// `frame.rs`'s scroll step's own memo, compared against a fresh
    /// `EditorState::layout_key(pane)` every frame to derive reveals for
    /// every non-selection source at once: a resize, a wrap-mode pin or
    /// toggle, a buffer switch (the very first read for a `(pane, buffer)`
    /// pair is `None`, so it always differs), a decoration-generation
    /// change (inlay hints, EOL text, virtual lines, signs), and any edit to
    /// the buffer at all (its text generation), including one made through a sibling
    /// pane. A changed key reveals only when [`PaneBufferState::parked`] is
    /// `false`: a pane parked behind an unfollowable scroll must not snap
    /// back onto its cursor just because something changed elsewhere in the
    /// buffer it happens to be viewing. `pub(in crate::editor)`, matching
    /// `reveal_pending`'s own visibility: `frame.rs` is its only reader, and
    /// it is a pure memo with no invariant to funnel through a narrower API.
    ///
    /// Not reset on a buffer switch: a pane revisiting a
    /// buffer it showed before, with every layout input still identical to
    /// what it read last time, finds a matching key and stays quiet,
    /// coherent with the parked-view model this whole mechanism serves
    /// (switching away and back is not itself a change), and the
    /// counterpart to `reveal_pending` needing no reset either (see
    /// `frame.rs`'s prune-cache doc: both die with the pane's own
    /// `SecondaryMap` entry, same as everything else on this struct).
    pub(in crate::editor) last_layout_key: Option<LayoutKey>,
    /// Whether this pane's cursor sat outside the scroll-margin band as of the
    /// last frame `frame.rs`'s scroll step settled it: `Viewport::settled_row`
    /// returning `None`, the case a wheel or `Ctrl-d` scroll leaves behind
    /// when `carry` can't fully follow it (a virtual block too tall for the
    /// band, a document edge). Gates whether a [`PaneBufferState::last_layout_key`]
    /// change reveals this pane; see that field's own doc. Written and read
    /// only there, same visibility and same "dies with the pane's own entry"
    /// lifetime as `last_layout_key`, and for the same revisit reason left
    /// unreset on a buffer switch: a pane revisiting a buffer finds its own
    /// prior parked state, not a fresh unparked one.
    pub(in crate::editor) parked: bool,
}

impl PaneBufferState {
    /// The stored selections, for a caller that keeps or hands them on
    /// whole; reading them takes a text ([`Self::view`]).
    pub(crate) fn selections(&self) -> &SelectionSet {
        &self.selections
    }

    /// The selections bound to `text`, the buffer's current text.
    pub(crate) fn view<'a>(&'a self, text: &'a BufferText) -> EditView<'a> {
        EditView::bind(text, &self.selections)
    }

    /// The selections paired with `text`, the buffer's current text, as the
    /// input to a command.
    pub(in crate::editor) fn state(&self, text: &BufferText) -> EditState {
        EditState::bind(text, self.selections.clone())
    }

    /// Store a command's resulting selections, raising
    /// [`PaneBufferState::reveal_pending`] iff the primary head moved.
    pub(in crate::editor) fn store(&mut self, state: EditState) {
        let moved = state.view().primary().head() != self.view(state.text()).primary().head();
        self.reveal_pending |= moved;
        self.selections = state.into_selections();
    }

    /// Replace the selections outright with `new`, a set for `text`, the
    /// buffer's current text, raising [`PaneBufferState::reveal_pending`] iff
    /// the primary head actually moved.
    pub(in crate::editor) fn set_selections(&mut self, new: SelectionSet, text: &BufferText) {
        let moved = EditView::bind(text, &new).primary().head() != self.view(text).primary().head();
        self.reveal_pending |= moved;
        self.selections = new;
    }

    /// Replace this pane's selections with those of an edit it made, and
    /// raise a reveal: the buffer under the pane changed whether or not the
    /// primary head moved.
    pub(in crate::editor) fn set_selections_after_edit(&mut self, new: SelectionSet) {
        self.selections = new;
        self.reveal_pending = true;
    }

    /// Carry this pane's selections for the buffer through `change`. Raises
    /// no reveal of its own: a text change moves the text generation, which
    /// `frame.rs`'s scroll step already reads off `EditorState::layout_key`
    /// and reveals for, gated on [`PaneBufferState::parked`] like every other
    /// external change (see that field's own doc for why a parked pane must
    /// not snap back just because the head it can't currently see also
    /// moved). The pane that made an edit raises its own in `finish_edit`.
    pub(in crate::editor) fn carry_selections(&mut self, change: &TextChange<'_>) {
        self.selections.translate(change);
    }

    /// Carry this pane's typed-run and autoindent records through `change`.
    pub(in crate::editor) fn carry_marks(&mut self, change: &TextChange<'_>) {
        // `ChangeSet::map_ranges` maps (start, end) pairs directly, but with
        // `Assoc::After` on starts and `Assoc::Before` on ends: it shrinks a
        // range around inserted text. A typed run needs the opposite: it must
        // grow to include what was just typed, so anchors and ends are mapped
        // separately with `Assoc` reversed from what `map_ranges` would use.
        if let Some(run) = self.typed_run.as_mut() {
            run.translate(change, |run| {
                change
                    .changes()
                    .map_positions(&mut run.anchors, Assoc::Before);
                change.changes().map_positions(&mut run.ends, Assoc::After);
            });
        }
        // Shrinks each record around any edit landing at its
        // start/end; see `map_ranges`' own doc. That is what makes ownership
        // self-revoking: text typed past a record's end, or a line split
        // before its start, falls outside the mapped range without any key
        // handler needing to clear it.
        if let Some(ranges) = self.autoindent.as_mut() {
            ranges.translate(change, |ranges| change.changes().map_ranges(ranges));
        }
    }
}

impl PaneBufferState {
    /// Start this pane on `sels` instead of the buffer's initial selections,
    /// as though it had first shown the buffer on them.
    #[cfg(test)]
    pub(in crate::editor) fn seed_selections(&mut self, sels: SelectionSet) {
        self.selections = sels;
    }
}

// ── Construction helpers ──────────────────────────────────────────────────────

/// Construct a fresh [`PaneBufferState`] for `buf`: SSOT for the initial-state
/// value. All seed sites must call this rather than building the struct literal
/// directly, so that adding a new field with a non-default initialiser requires
/// only one edit here.
pub(in crate::editor) fn fresh_from_buf(buf: &Buffer) -> PaneBufferState {
    PaneBufferState {
        selections: buf.initial_sels(),
        search_cursor: SearchCursor::default(),
        typed_run: None,
        autoindent: None,
        step_back_on_exit: false,
        kill_opened_session: false,
        reveal_pending: false,
        last_layout_key: None,
        parked: false,
    }
}

/// Ensure `pane_state[pid][bid]` exists, seeding with [`fresh_from_buf`] if absent.
/// Idempotent: safe to call even if the entry was already seeded.
///
/// Panics if `pid` or `bid` is not a live key; that is a caller-contract
/// violation (the pane or buffer was never opened), not a recoverable error.
/// Trusted mint for every synchronous caller that already knows `pid` is
/// live (it was just resolved, split, or opened in the same
/// call). See [`try_ensure`] for a caller crossing an async boundary, where
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

/// [`ensure`]'s validating counterpart, for a caller whose `pid` was
/// captured before crossing an async boundary (an LSP response, a queued
/// Steel callback) and may have since closed. `bid` still panics on a dead
/// key: every current caller reaches this only once its own generation/
/// anchor check has already proven the buffer live, so that half of the
/// contract still holds; only `pid`'s liveness crosses the boundary
/// unchecked.
///
/// Checked against `panes` (the engine's own [`PanePool`]), not inferred
/// from `pane_state.entry(pid)`: `SecondaryMap::remove` (`drop_pane_state`)
/// drops a closed pane's slot back to vacant at version 0, and
/// `SecondaryMap::entry` returns the same `Vacant` variant for that as it
/// does for a `pid` that simply never touched this map. The two are
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
    // existence is minted from that same slotmap, so `pane_state` (a
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

/// Collapse `pane_state[pid][bid]`'s selection onto `pos`, without
/// switching focus or recording a jump entry. The primitive every cursor
/// placement outside the focused-buffer fast path (`set_current_selections`)
/// reduces to: [`park_cursor_at`] is its line/grapheme-column convenience for
/// a caller with no char position yet.
pub(in crate::editor) fn write_cursor(
    pane_state: &mut SecondaryMap<PaneId, SecondaryMap<BufferId, PaneBufferState>>,
    buffers: &BufferStore,
    panes: &PanePool,
    pid: PaneId,
    bid: BufferId,
    pos: ClusterStart,
) {
    let text = buffers.get(bid).text();
    ensure(pane_state, buffers, panes, pid, bid).store(EditState::with_cursor(text.clone(), pos));
}

/// Resolves a 0-based `(line, grapheme_col)` to its cluster, clamping the
/// line to `text`'s last content line.
pub(in crate::editor) fn line_grapheme_to_cluster(
    text: &hume_editing::text::BufferText,
    line0: hume_rope::line::ContentLine,
    grapheme_col0: hume_rope::column::GraphemeCol,
) -> ClusterStart {
    let line = line0.min(text.last_content_line());
    text.columns().place_grapheme(line.into(), grapheme_col0)
}

/// Collapse `pane_state[pid][bid]`'s selection onto a 0-based
/// `(line, grapheme_col)`, clamping the line to the buffer's last content
/// line. Shared by every caller that parks a cursor at a line/column pair
/// with no char position yet.
pub(in crate::editor) fn park_cursor_at(
    pane_state: &mut SecondaryMap<PaneId, SecondaryMap<BufferId, PaneBufferState>>,
    buffers: &BufferStore,
    panes: &PanePool,
    pid: PaneId,
    bid: BufferId,
    line0: hume_rope::line::ContentLine,
    grapheme_col0: hume_rope::column::GraphemeCol,
) {
    let pos = line_grapheme_to_cluster(buffers.get(bid).text(), line0, grapheme_col0);
    write_cursor(pane_state, buffers, panes, pid, bid, pos);
}

/// The pane-keyed maps that live on [`super::EditorState`], plus the
/// positions scripts track.
///
/// Bundles `state` (per-(pane,buffer) selections/groups), `jumps` (cursor
/// history), `tracked` (positions scripts asked the editor to remember,
/// keyed by token rather than pane), and `render` (per-pane
/// highlight/sign/inlay-hint/virtual-line handles, bundled in
/// [`hume_decorations::PaneDecorationHandles`] since `build_pane` always
/// allocates and `drop_pane_state` always drops them together). NLL allows
/// simultaneous mutable borrows of different fields, as `PositionStores::new`
/// takes `state`, `jumps` and `tracked` at once.
#[derive(Default)]
pub(crate) struct PaneView {
    pub(in crate::editor) state: SecondaryMap<PaneId, SecondaryMap<BufferId, PaneBufferState>>,
    pub(in crate::editor) jumps: super::jump_list::JumpLists,
    pub(in crate::editor) tracked: super::tracked_positions::TrackedPositions,
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

/// Build a new pane viewing `buffer_id`, with its decoration providers
/// ([`hume_decorations::build_providers`]) and overlays
/// ([`hume_ui::register_overlays`]). Every pane creation site goes through
/// here; `Pane::new` alone has an empty `ProviderSet`.
///
/// Returns the pane with its own `PaneDecorationHandles` (never shared
/// between panes), which the caller stores in `EditorState.panes.render`.
/// Wrap mode is not seeded: it resolves lazily via
/// `commands::effective_wrap_mode`. The gutter's seeded style never reaches a
/// frame, since `prepare_frame` syncs `line-number-style` before each render.
pub(in crate::editor) fn build_pane(
    registry: &mut hume_engine::theme::ScopeRegistry,
    views: &hume_ui::OverlayViews,
    buffer_id: BufferId,
) -> (
    hume_engine::pane::Pane,
    hume_decorations::PaneDecorationHandles,
) {
    // Interns the engine's own `DEFAULT_GUTTER_SCOPE` constant rather than
    // repeating the "ui.linenr" literal here: the two must resolve to the
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
    /// unseeded there: a stale id, or `bid` not open in the focused pane.
    /// `bid` is caller-supplied, so this always looks the pane state up by
    /// the explicit id rather than assuming it matches the focused buffer.
    /// Strictly focused-pane callers only. A caller with an explicit
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
    /// (`commands::current_selections`, the statusline's own accessors),
    /// which would otherwise each index `panes.state[..][..]` and each panic
    /// with slotmap's own message instead of naming what actually broke.
    pub(crate) fn focused_buffer_state_or_panic(&self, bid: BufferId) -> &PaneBufferState {
        self.focused_buffer_state(bid).expect(
            "focused pane has no seeded state for the buffer it is showing: \
             pane.buffer_id and panes.state are out of sync",
        )
    }

    /// Every pane currently showing `bid`, in the order `(buffer-panes
    /// pane)` hands back to Steel: the focused pane first (if it shows
    /// `bid`), then the rest of the active tab (`view.active_pane_ids`'s own
    /// leaf order), then every other tab
    /// (`view.panes.every_pane_across_all_tabs`'s order). Explicit
    /// enumeration, not a single-pane guess. A caller wanting just one
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
    /// currently views: the write path behind `:set pane wrap-mode=…`.
    ///
    /// Always writes an explicit override, even `WrapMode::None` (an
    /// explicit "don't wrap" pin): `:set pane` is itself an explicit pane
    /// action, so from here on this pane stops following `:set buffer`/
    /// `:set global wrap-mode=…` *for this buffer* until `:wrap` or another
    /// `:set pane` changes it again. There is no command that clears the
    /// pin back to inheriting. The pin lives in `Pane::wraps`, keyed by
    /// buffer (see `WrapOverride`), so it does not follow the pane to a
    /// buffer it switches to next; switching back to this buffer restores it.
    ///
    /// `WrapOverride::saved` (the `:wrap` toggle-on restore target) is synced
    /// to this pin only when `mode` itself wraps. Pinning *off*
    /// leaves it alone, so whatever `saved` already pointed at (a prior
    /// wrapping pin, or "was inheriting") survives as the toggle-on target
    /// instead of being erased by this pin.
    ///
    /// Zeroes horizontal scroll (meaningless once wrapped) on any actual
    /// change to the pane's *effective* mode. See `toggle_wrap`'s doc for
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

    /// Toggle `fp`'s wrapping for the buffer it currently views (`:wrap`,
    /// `:toggle-soft-wrap`). Returns the new wrapping state. Like
    /// `set_wrap_override`, this writes `Pane::wraps` for the current buffer
    /// only.
    ///
    /// Turning off saves the current override in `WrapOverride::saved` (`None`
    /// if inheriting) and pins `WrapMode::None`. Turning on restores that
    /// override, so an inheriting pane keeps following later `:set` changes.
    /// If the result would not wrap, it pins the global `wrap_mode` when that
    /// wraps, else `DEFAULT_WRAP_STYLE`, so `:wrap` never no-ops.
    ///
    /// Horizontal scroll is zeroed now rather than by the next frame's
    /// `Viewport::reveal_horizontal`, because callers read the viewport
    /// before then. The top's slot is left alone: the next `Viewport::top_at`
    /// clamps an out-of-range slot. A slot that pointed at an `after` virtual
    /// line can stay in range and land on a wrap display line instead.
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
