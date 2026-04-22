//! Per-(pane, buffer) and per-pane editor state bundles.
//!
//! [`PaneBufferState`] holds all per-(pane, buffer) mutable facts: selections,
//! search cursor, and the in-progress edit group. Adding a new per-(pane, buffer)
//! field later requires changing exactly one struct and one Default impl —
//! not four parallel maps.
//!
//! [`PaneTransient`] holds per-pane-only transient state (search / select mode
//! snapshots) that is not keyed by buffer. It lives in
//! `Editor.pane_transient: SecondaryMap<PaneId, PaneTransient>`.
//!
//! [`EditGroup`] is the in-progress insert-session accumulator. It is stored on
//! [`PaneBufferState`] rather than [`crate::editor::buffer::Buffer`] so that
//! the focus-switch-Normal-only invariant can be maintained without
//! per-buffer group bookkeeping (at most one pane is ever in Insert).

use engine::pipeline::{BufferId, PaneId};
use slotmap::SecondaryMap;

use crate::core::changeset::ChangeSet;
use crate::core::search_state::SearchCursor;
use crate::core::selection::SelectionSet;
use crate::core::text::Text;
use crate::editor::buffer::Buffer;
use crate::editor::buffer_store::BufferStore;

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
/// Stored in `Editor.pane_state: SecondaryMap<PaneId, SecondaryMap<BufferId, PaneBufferState>>`.
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
        .or_insert_with(SecondaryMap::new);
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

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pane_buffer_state_default_is_valid() {
        use crate::core::selection::Selection;
        let state = PaneBufferState::default();
        assert_eq!(state.selections.primary(), Selection::collapsed(0));
        assert!(state.edit_group.is_none());
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
        assert!(state.search_cursor.match_count.is_none());
    }
}
