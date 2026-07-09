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

    /// The focused pane's wrap mode. `Pane::wrap_mode` is the SSOT (a view
    /// property, not a document one — two panes on the same buffer may wrap
    /// differently); this is the raw (unresolved sentinel) value.
    pub(crate) fn focused_wrap_mode(&self) -> hume_engine::pane::WrapMode {
        self.view.panes[self.state.focused_pane_id].wrap_mode
    }

    /// Apply `mode` as the focused pane's wrap mode — the shared path behind
    /// both `:wrap` and `:set pane wrap-mode=…`.
    ///
    /// Setting a wrapping mode also updates `saved_wrap_mode` (the restore
    /// target for a future `:wrap` toggle-on) and, on an off→on transition,
    /// zeroes horizontal scroll (meaningless once wrapped). Setting
    /// `WrapMode::None` stashes the pane's current wrap mode into
    /// `saved_wrap_mode` first, preserving the toggle invariant that it's
    /// never `None`.
    pub(crate) fn apply_focused_wrap_mode(&mut self, mode: hume_engine::pane::WrapMode) {
        use hume_engine::pane::WrapMode;
        let now_wrapping = mode.is_wrapping();
        let pane = &mut self.view.panes[self.state.focused_pane_id];
        let was_wrapping = pane.wrap_mode.is_wrapping();
        let mode_changed = mode != pane.wrap_mode;
        if now_wrapping {
            pane.wrap_mode = mode;
            pane.saved_wrap_mode = mode;
        } else {
            if was_wrapping {
                pane.saved_wrap_mode = pane.wrap_mode;
            }
            pane.wrap_mode = WrapMode::None;
        }
        // Any actual mode change invalidates the sub-row scroll state:
        // off→on starts wrapping fresh; on→off leaves non-wrap rendering with
        // no sub-row concept (nothing in unwrapped scrolling ever clears a
        // stale `top_row_offset`, and the renderer forwards it verbatim as
        // `top_skip_rows` regardless of wrap mode); on→on width/style changes
        // can leave a sub-row offset past the new line's row count.
        if mode_changed {
            let vp = self.viewport_mut();
            vp.horizontal_offset = 0;
            vp.top_row_offset = 0;
        }
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pane_buffer_state_default_is_valid() {
        use hume_editing::selection::Selection;
        let state = PaneBufferState::default();
        assert_eq!(state.selections.primary(), Selection::collapsed(0));
        assert!(state.edit_group.is_none());
        assert!(state.paste_group.is_none());
        assert!(state.search_cursor.match_count.is_none());
    }

    #[test]
    fn pane_transient_default_is_empty() {
        let t = PaneTransient::default();
        assert!(t.pre_search_sels.is_none());
        assert!(t.pre_select_sels.is_none());
        assert!(!t.search_extend);
    }

    #[test]
    fn fresh_from_buf_seeds_initial_sels() {
        use crate::editor::buffer::Buffer;
        let buf = Buffer::scratch();
        let expected = buf.initial_sels();
        let state = fresh_from_buf(&buf);
        assert_eq!(state.selections, expected);
        assert!(state.edit_group.is_none());
        assert!(state.paste_group.is_none());
        assert!(state.search_cursor.match_count.is_none());
    }
}
