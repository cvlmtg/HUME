//! Per-(pane, buffer) and per-pane editor state bundles.
//!
//! [`PaneBufferState`] holds all per-(pane, buffer) mutable facts: selections,
//! search cursor, and the in-progress edit group. Adding a new per-(pane, buffer)
//! field later requires changing exactly one struct and one Default impl —
//! not four parallel maps.
//!
//! [`PaneTransient`] holds per-pane-only transient state (search / select mode
//! snapshots) that is not keyed by buffer.
//!
//! [`PaneView`] groups the three per-pane maps — `state`, `transient`, `jumps` —
//! so callers deal with one field on [`super::EditorState`] instead of three.
//!
//! [`EditGroup`] is the in-progress insert-session accumulator. It is stored on
//! [`PaneBufferState`] rather than [`crate::editor::buffer::Buffer`] so that
//! the focus-switch-Normal-only invariant can be maintained without
//! per-buffer group bookkeeping (at most one pane is ever in Insert).

use hume_engine::pipeline::{BufferId, EngineView, PaneId};
use slotmap::SecondaryMap;

use super::Editor;
use super::search::SearchCursor;
use crate::editor::buffer::Buffer;
use crate::editor::buffer::store::BufferStore;
use hume_editing::changeset::ChangeSet;
use hume_editing::selection::{Selection, SelectionSet};
use hume_editing::text::BufferText;
use hume_rope::offset::CharOffset;

// ── EditGroup ────────────────────────────────────────────────────────────────

/// Accumulated state for an in-progress insert-mode session.
///
/// Stored on [`PaneBufferState`] so it is per-(pane, buffer) rather than
/// per-buffer. The focus-switch-Normal-only invariant ensures at most one pane
/// is ever in Insert at a time, so at most one `PaneBufferState` will have
/// `Some(EditGroup)` at any moment.
pub(crate) struct EditGroup {
    /// Buffer text snapshot taken at `begin_edit_group`. Used by
    /// `commit_edit_group` to invert the composed CS and record a single
    /// history revision.
    pub text_snapshot: BufferText,
    /// Selection state at group open — stored in the history revision so
    /// undo restores the cursor to its pre-insert position.
    pub pre_sels: SelectionSet,
    /// Running composition of all forward ChangeSets applied since the group
    /// opened. `None` until the first keystroke (empty session = no revision
    /// recorded on commit).
    pub cs: Option<ChangeSet>,
}

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

// ── PaneBufferState ──────────────────────────────────────────────────────────

/// All per-(pane, buffer) editor state bundled into one struct.
///
/// Stored in `EditorState.panes.state: SecondaryMap<PaneId, SecondaryMap<BufferId, PaneBufferState>>`.
/// Default initialisation is used at every seed site — callers override
/// `selections` with `buffer.initial_sels()` when seeding for the first time.
#[derive(Default)]
pub(crate) struct PaneBufferState {
    /// The focused pane's cursor / selection state for this buffer.
    pub selections: SelectionSet,
    /// Per-pane cursor through the buffer's shared match list.
    pub search_cursor: SearchCursor,
    /// Some only while this pane is in Insert mode for this buffer.
    pub edit_group: Option<EditGroup>,
    /// Open paste session: `Some` between the first `p`/`P` and the next
    /// non-cycle command. Stores the pre-paste snapshot so `[`/`]` can
    /// re-paste from the pristine state and fold all cycles into one undo step.
    pub paste_group: Option<EditGroup>,
    /// Direction the open paste session was opened with (`true` = `P`/paste-before).
    /// Meaningful only while `paste_group.is_some()`; read by `[`/`]` so cycling
    /// re-pastes in the same direction as the opening `p`/`P`.
    pub paste_before: bool,
    /// The open insert session's typed span, kept in post-edit coordinates by
    /// `apply_doc_edit_grouped`. `Some` from the moment the session's entry
    /// command positions the cursor (`begin_typed_run`) until
    /// `end_insert_session` consumes it on exit, for every insert entry
    /// (`i`/`a`/`o`/`O`/`A`/`I`/`c`/…).
    pub typed_run: Option<TypedRun>,
    /// Set by `begin_typed_run` from its `ExitCursor` parameter for `a`/`A`/
    /// `o`/`O` entry (never for `i`/`I`/`c`). Decides where an *empty* typed
    /// run's cursor lands on exit — step one grapheme back (so `a<Esc>` is a
    /// round trip) rather than staying put. Lives here rather than on
    /// `InsertSession` because dot-repeat replay never creates one — see
    /// `begin_insert_session`'s replay-signal guard — so a flag
    /// `end_insert_session` reads on exit must survive on state that isn't
    /// cleared by that guard.
    pub step_back_on_exit: bool,
    /// Whether the open insert session was entered via a ring-capturing kill
    /// (bare or `"k`-prefixed `c` — an explicit-register change writes no
    /// stamp and must not set this). Set only by `cmd_change`, for the same
    /// reason `step_back_on_exit` lives here rather than on `InsertSession`.
    /// Read by `end_insert_session`: every keystroke typed during the session
    /// bumps `BufferStore::edit_seq`, so the `PasteStamp` `cmd_change` wrote
    /// (pointing at the just-replaced text) goes stale by the time the
    /// session closes — refreshing its `seq` here is what keeps
    /// `c <text> <Esc> p` reading the kill ring instead of the clipboard.
    pub kill_opened_session: bool,
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
/// Panics if `pid` or `bid` is not a live slotmap key; that is a caller-contract
/// violation (the pane or buffer was never opened), not a recoverable error.
pub(in crate::editor) fn ensure<'a>(
    pane_state: &'a mut SecondaryMap<PaneId, SecondaryMap<BufferId, PaneBufferState>>,
    buffers: &BufferStore,
    pid: PaneId,
    bid: BufferId,
) -> &'a mut PaneBufferState {
    let inner = pane_state
        .entry(pid)
        .expect("pid must be a live PaneId")
        .or_default();
    inner
        .entry(bid)
        .expect("bid must be a live BufferId")
        .or_insert_with(|| fresh_from_buf(buffers.get(bid)))
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
    pid: PaneId,
    bid: BufferId,
    char_pos: CharOffset,
) {
    ensure(pane_state, buffers, pid, bid).selections =
        SelectionSet::single(Selection::collapsed(char_pos));
}

/// Collapse `pane_state[pid][bid]`'s selection onto a 0-based
/// `(line, grapheme_col)`, clamping the line to the buffer's last content
/// line. Shared by every caller that parks a cursor at a line/column pair —
/// a read-only view's opening position and a CLI startup position both
/// reduce to this.
pub(in crate::editor) fn park_cursor_at(
    pane_state: &mut SecondaryMap<PaneId, SecondaryMap<BufferId, PaneBufferState>>,
    buffers: &BufferStore,
    pid: PaneId,
    bid: BufferId,
    line0: hume_rope::line::ContentLine,
    grapheme_col0: hume_rope::column::GraphemeCol,
) {
    let text = buffers.get(bid).text();
    let line = hume_rope::line::ContentLine::clamped(text.rope(), line0.index());
    let char_pos = hume_editing::lines::place_grapheme_column(text, line.into(), grapheme_col0);
    write_cursor(pane_state, buffers, pid, bid, char_pos);
}

// ── PaneTransient ────────────────────────────────────────────────────────────

/// Per-pane-only transient state (not keyed by buffer).
///
/// Stored in `Editor.pane_transient: SecondaryMap<PaneId, PaneTransient>`.
/// Flat on each pane because this state is associated with the pane's current
/// mode, not with any particular buffer. For example `pre_search_sels` is the
/// state to restore if the user cancels Search mode — it belongs to the pane
/// that entered Search mode, independent of which buffer that pane is viewing.
#[derive(Default)]
pub(in crate::editor) struct PaneTransient {
    /// Snapshot of selections taken when this pane entered Search mode.
    /// Restored on cancel; discarded on confirm. `None` when not in Search mode.
    pub pre_search_sels: Option<SelectionSet>,
    /// Snapshot of selections taken when this pane entered Sift mode.
    /// Restored on cancel; discarded on confirm.
    pub pre_sift_sels: Option<SelectionSet>,
    /// Whether Extend mode was active when this pane entered Search mode.
    /// Captured so live-search can extend from the pre-search anchor even
    /// though `mode` is `Search` during the live preview.
    pub search_extend: bool,
}

// ── PaneView ──────────────────────────────────────────────────────────────────

/// Groups the four per-pane maps that live on [`super::EditorState`].
///
/// Bundles `state` (per-(pane,buffer) selections/groups), `transient` (search/select
/// snapshots), `jumps` (cursor history), and `render` (per-pane highlight/sign/
/// inlay-hint/virtual-line handles, bundled in
/// [`hume_decorations::PaneDecorationHandles`] since `build_pane` always
/// allocates and `drop_pane_state` always drops them together) so
/// `EditorState` exposes one field instead of four. The map types and
/// keying are unchanged; NLL still allows simultaneous mutable borrows of
/// different fields (e.g. `panes.state` and `panes.jumps` in
/// `buffer::lifecycle::switch_to_buffer_with_jump`).
#[derive(Default)]
pub(crate) struct PaneView {
    pub(in crate::editor) state: SecondaryMap<PaneId, SecondaryMap<BufferId, PaneBufferState>>,
    pub(in crate::editor) transient: SecondaryMap<PaneId, PaneTransient>,
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
/// per crate now that decoration providers and overlay widgets live apart.
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
    let linenr_selected_scope = registry.intern("ui.linenr.selected");

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
    /// `bid` is caller-supplied (e.g. from a Steel `(current-buffer)` call),
    /// so this always looks the pane state up by the explicit id rather than
    /// assuming it matches the focused buffer. Strictly focused-pane callers
    /// only — a `bid` that may be shown in a *different* pane, or in none,
    /// wants [`shown_buffer_state`](Self::shown_buffer_state) instead.
    pub(in crate::editor) fn focused_buffer_state(
        &self,
        bid: BufferId,
    ) -> Option<&PaneBufferState> {
        self.panes.buffer_state(self.focused_pane_id, bid)
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

    /// The pane currently showing `bid`, restricted to the active tab: the
    /// focused pane if it shows `bid`, else the first *active-tab* pane
    /// (`view.active_pane_ids`' own leaf order) that does, else `None` —
    /// including when `bid` is shown only in a *background* tab's pane, not
    /// just when it's paneless entirely.
    ///
    /// The active-tab restriction is for callers that use the returned id as
    /// "the pane to act on" rather than merely "some cursor to read" —
    /// currently only `lsp/introspect.rs`'s `viewport_range`, since a
    /// background-tab pane's viewport can be stale: active-tab panes are
    /// the only ones `sync_viewport_dims` resizes per frame, so an inactive
    /// tab's pane reflects whatever geometry was current whenever its tab
    /// was last on screen, not the current terminal. A cursor reader wants
    /// `pane_with_buffer` below instead — its selections stay live no matter
    /// which tab is active.
    pub(in crate::editor) fn pane_showing_buffer(
        &self,
        view: &EngineView,
        bid: BufferId,
    ) -> Option<PaneId> {
        if view
            .panes
            .get(self.focused_pane_id)
            .is_some_and(|p| p.buffer_id == bid)
        {
            return Some(self.focused_pane_id);
        }
        // `.get` rather than indexing: `active_pane_ids()` can transiently
        // include `take_live`'s placeholder id if a panic unwinds between
        // `take_live` and `install_live` — skip it like `EngineView::render`
        // already does, rather than panicking one step earlier.
        view.active_pane_ids()
            .into_iter()
            .find(|&pid| view.panes.get(pid).is_some_and(|p| p.buffer_id == bid))
    }

    /// The pane whose *cursor state* should answer for `bid`: same as
    /// `pane_showing_buffer`, but falls further back to any pane in the pool
    /// showing `bid` — including a background tab's — rather than giving up.
    /// A pane's selections are live regardless of which tab is active; only
    /// its viewport is tied to on-screen geometry (see `pane_showing_buffer`'s
    /// doc), so this wider fallback is safe exactly for callers that never
    /// read one. `shown_buffer_state` is the only caller.
    fn pane_with_buffer(&self, view: &EngineView, bid: BufferId) -> Option<PaneId> {
        if let Some(pid) = self.pane_showing_buffer(view, bid) {
            return Some(pid);
        }
        // Cursor read only, not a viewport — see this fn's own doc.
        view.panes
            .every_pane_across_all_tabs()
            .find(|(_, p)| p.buffer_id == bid)
            .map(|(pid, _)| pid)
    }

    /// `bid`'s state as seen in the pane currently showing it, or `None`
    /// when no pane shows `bid`.
    ///
    /// A `PaneBufferState` outlives the pane's visit to `bid` — that's what
    /// restores your cursor when you switch back to a buffer — so scanning
    /// the *seeded* maps for "any pane that ever showed `bid`" can answer
    /// with the cursor of a pane that moved on long ago. Resolving against
    /// `EngineView`'s live `pane.buffer_id` instead (via `pane_with_buffer`)
    /// is what makes one `bid` mean one cursor across every surface that
    /// asks: `symbol-under-cursor`, `selections-linewise?`, and the
    /// `lsp-*-params` builders all read through this rather than
    /// `focused_buffer_state`, since a caller-supplied `bid` (a Steel
    /// `(current-buffer)` snapshot, or one carried across a debounce or an
    /// async LSP round-trip) may no longer be the buffer the focused pane
    /// shows, or may be shown in a pane belonging to a different tab —
    /// active or not, since none of these callers read a viewport.
    pub(in crate::editor) fn shown_buffer_state(
        &self,
        view: &EngineView,
        bid: BufferId,
    ) -> Option<&PaneBufferState> {
        self.panes
            .buffer_state(self.pane_with_buffer(view, bid)?, bid)
    }
}

impl Editor {
    // ── Pane-state accessors ──────────────────────────────────────────────────

    /// The focused pane's effective wrap mode: pane override → buffer
    /// override → global default (see `commands::effective_wrap_mode`).
    pub(in crate::editor) fn focused_wrap_mode(&self) -> hume_engine::pane::WrapMode {
        let pane = &self.view.panes[self.state.focused_pane_id];
        let doc = self.state.buffers.get(pane.buffer_id);
        super::commands::effective_wrap_mode(doc, &self.state.settings, pane)
    }

    /// Pin the focused pane's wrap mode to `mode`, for the buffer it
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
    /// change to the pane's *effective* mode — see `toggle_focused_wrap`'s
    /// doc for the full rationale, shared by both functions.
    pub(in crate::editor) fn set_focused_wrap_override(
        &mut self,
        mode: hume_engine::pane::WrapMode,
    ) {
        let before = self.focused_wrap_mode();
        let pid = self.state.focused_pane_id;
        let pane = &mut self.view.panes[pid];
        let mut wrap = pane.wrap();
        wrap.mode = Some(mode);
        if mode.is_wrapping() {
            wrap.saved = Some(mode);
        }
        pane.set_wrap(wrap);
        if mode != before {
            self.viewport_mut().horizontal_offset = hume_rope::column::DisplayLineCol::new(0);
        }
    }

    /// Toggle the focused pane's wrapping on/off, for the buffer it
    /// currently views — the write path behind `:wrap`/`:toggle-soft-wrap`.
    /// Returns the new wrapping state.
    ///
    /// Turning wrapping *off* stashes the pane's current override into
    /// `WrapOverride::saved` — `None` if it was inheriting from the
    /// buffer/global setting, `Some(m)` if it was explicitly pinned to `m` —
    /// then pins the pane to `WrapMode::None`. Like `set_focused_wrap_override`,
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
    /// just belt-and-suspenders: `scroll::ensure_cursor_visible_horizontal`
    /// also zeroes it for any wrapping pane on the next frame, but only a
    /// frame later, and code reading the viewport between this call and the
    /// next render (including several existing tests) expects it already
    /// zero.
    ///
    /// `top_slot`, by contrast, is left alone here on purpose: it
    /// addresses a display line inside `top_line`'s whole visual block
    /// (`before` + content display lines + `after`) in *either* wrap mode
    /// (`scroll::set_top` writes it unconditionally) — a mode change can
    /// leave it past the new block's display-line count (off→on starts a
    /// narrower block; on→on width/style changes can shrink it), and that
    /// out-of-range case is exactly what `scroll::clamp_viewport_top`
    /// repairs once per pane per frame, so there's no need to throw the
    /// address away here. What clamping *cannot* catch: only a
    /// `content`-side change (not this function) grows the block, so a slot
    /// that addressed an `after` display line in no-wrap can still be in
    /// range once wrapping grows `content` — landing on a wrap display line
    /// of the line's own text instead of the virtual display line it used
    /// to point at. Silent, not a bug this function fixes.
    pub(in crate::editor) fn toggle_focused_wrap(&mut self) -> bool {
        use hume_engine::pane::{DEFAULT_WRAP_STYLE, WrapMode};

        let pid = self.state.focused_pane_id;
        let now_wrapping = if self.focused_wrap_mode().is_wrapping() {
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
            if !self.focused_wrap_mode().is_wrapping() {
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
        self.viewport_mut().horizontal_offset = hume_rope::column::DisplayLineCol::new(0);
        now_wrapping
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests;
