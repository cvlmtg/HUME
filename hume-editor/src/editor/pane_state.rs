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

use hume_engine::pipeline::{BufferId, PaneId};
use slotmap::SecondaryMap;

use super::Editor;
use super::search::SearchCursor;
use crate::editor::buffer::Buffer;
use crate::editor::buffer::store::BufferStore;
use hume_editing::changeset::ChangeSet;
use hume_editing::selection::SelectionSet;
use hume_editing::text::Text;

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
    pub text_snapshot: Text,
    /// Selection state at group open — stored in the history revision so
    /// undo restores the cursor to its pre-insert position.
    pub pre_sels: SelectionSet,
    /// Running composition of all forward ChangeSets applied since the group
    /// opened. `None` until the first keystroke (empty session = no revision
    /// recorded on commit).
    pub cs: Option<ChangeSet>,
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
    /// Anchors of the run typed during the open insert session — one per
    /// selection, sorted, kept in post-edit coordinates by
    /// `apply_doc_edit_grouped`. `Some` from the moment the session's entry
    /// command positions the cursor (`pin_insert_anchors`) until
    /// `end_insert_session` consumes it on exit, for every insert entry
    /// (`i`/`a`/`o`/`O`/`A`/`I`/`c`/…), not just `c`.
    pub pinned_anchors: Option<Vec<usize>>,
    /// Whether `end_insert_session` should select the typed span (rather than
    /// just stash it for `mii`) on exit. Set only by `cmd_change`, gated on
    /// the `select-changed-text` setting. Lives here (not on `InsertSession`)
    /// because dot-repeat replay never creates an `InsertSession` — see
    /// `begin_insert_session`'s replay-signal guard — so a flag needed at
    /// exit must survive on state that isn't cleared by that guard.
    pub select_on_exit: bool,
    /// Whether the open insert session was entered via a ring-capturing kill
    /// (bare or `"k`-prefixed `c` — an explicit-register change writes no
    /// stamp and must not set this). Set only by `cmd_change`, for the same
    /// reason `select_on_exit` lives here rather than on `InsertSession`.
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
pub(crate) fn fresh_from_buf(buf: &Buffer) -> PaneBufferState {
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
pub(crate) fn ensure<'a>(
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

// ── PaneTransient ────────────────────────────────────────────────────────────

/// Per-pane-only transient state (not keyed by buffer).
///
/// Stored in `Editor.pane_transient: SecondaryMap<PaneId, PaneTransient>`.
/// Flat on each pane because this state is associated with the pane's current
/// mode, not with any particular buffer. For example `pre_search_sels` is the
/// state to restore if the user cancels Search mode — it belongs to the pane
/// that entered Search mode, independent of which buffer that pane is viewing.
#[derive(Default)]
pub(crate) struct PaneTransient {
    /// Snapshot of selections taken when this pane entered Search mode.
    /// Restored on cancel; discarded on confirm. `None` when not in Search mode.
    pub pre_search_sels: Option<SelectionSet>,
    /// Snapshot of selections taken when this pane entered Select mode.
    /// Restored on cancel; discarded on confirm.
    pub pre_select_sels: Option<SelectionSet>,
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
/// inlay-hint/virtual-line handles, bundled in [`crate::ui::PaneRenderHandles`]
/// since `build_pane` always allocates and `drop_pane_state` always drops them
/// together) so `EditorState` exposes one field instead of four. The map
/// types and keying are unchanged; NLL still allows simultaneous mutable borrows
/// of different fields (e.g. `panes.state` and `panes.jumps` in
/// `buffer::lifecycle::switch_to_buffer_with_jump`).
pub(crate) struct PaneView {
    pub(crate) state: SecondaryMap<PaneId, SecondaryMap<BufferId, PaneBufferState>>,
    pub(crate) transient: SecondaryMap<PaneId, PaneTransient>,
    pub(crate) jumps: SecondaryMap<PaneId, super::jump_list::JumpList>,
    pub(crate) render: SecondaryMap<PaneId, crate::ui::PaneRenderHandles>,
}

impl Editor {
    // ── Pane-state accessors ──────────────────────────────────────────────────

    /// The focused pane's effective wrap mode: pane override → buffer
    /// override → global default (see `commands::effective_wrap_mode`).
    pub(crate) fn focused_wrap_mode(&self) -> hume_engine::pane::WrapMode {
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
    pub(crate) fn set_focused_wrap_override(&mut self, mode: hume_engine::pane::WrapMode) {
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
            self.viewport_mut().horizontal_offset = 0;
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
    /// `top_row_offset`, by contrast, is left alone here on purpose: it
    /// addresses a row inside `top_line`'s whole visual block (`before` +
    /// content rows + `after`) in *either* wrap mode (`scroll::set_top`
    /// writes it unconditionally) — a mode change can leave it past the new
    /// block's row count (off→on starts a narrower block; on→on width/style
    /// changes can shrink it), and that out-of-range case is exactly what
    /// `scroll::clamp_viewport_top` repairs once per pane per frame, so
    /// there's no need to throw the address away here. What clamping
    /// *cannot* catch: only a `content`-side change (not this function)
    /// grows the block, so an offset that addressed an `after` row in
    /// no-wrap can still be in range once wrapping grows `content` —
    /// landing on a wrap row of the line's own text instead of the virtual
    /// row it used to point at. Silent, not a bug this function fixes.
    pub(crate) fn toggle_focused_wrap(&mut self) -> bool {
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
        self.viewport_mut().horizontal_offset = 0;
        now_wrapping
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests;
