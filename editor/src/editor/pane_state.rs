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

use crate::core::changeset::ChangeSet;
use crate::core::selection::SelectionSet;
use crate::core::text::Text;

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

use crate::core::search_state::SearchCursor;

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
}
