//! A mode layer's snapshot of one pane's selections, captured on entry and
//! restored on exit — the shape `SearchLayer` and `SiftLayer` both need for
//! cancel-restore, factored out so the capture/restore/take rules live in one
//! place instead of being re-derived in each layer.
//!
//! Not `pane_state.rs`: that module's `EditGroup` already has a `pre_sels`
//! field meaning something unrelated (an undo group's pre-*edit* selections,
//! not a session's pre-*entry* ones) — two unrelated `pre_sels` in one file
//! would be exactly the kind of same-name collision this codebase avoids
//! elsewhere. This type's reason to exist is the mode-layer session
//! lifecycle (`capture` on `Layer::setup`, `restore`/`take_restore` on
//! `Layer::tear_down`), which is `input_stack/` vocabulary.

use slotmap::SecondaryMap;

use hume_editing::selection::SelectionSet;
use hume_engine::pipeline::{BufferId, EngineView, PaneId};

use super::super::EditorState;
use super::super::commands;
use super::super::pane_state::PaneBufferState;

/// `pane`'s selections as they were the moment this session opened, plus the
/// pane itself — never `state.focus.id()`, which may have moved on since (a
/// mouse click always falls through under these layers, so focus can change
/// while the session stays open). `None` once a `Confirm` arm has taken it,
/// so `Layer::tear_down`'s own restore becomes a no-op instead of
/// double-firing on top of whatever the confirm already did.
pub(in crate::editor) struct PaneSnapshot {
    pane: PaneId,
    pre_sels: Option<SelectionSet>,
}

impl PaneSnapshot {
    /// A fresh, not-yet-captured snapshot for `pane` — construction only
    /// records *where*; `capture` records *what*, once the layer actually
    /// lands (see `capture`'s own doc for why the two are split).
    pub(in crate::editor) fn new(pane: PaneId) -> Self {
        Self {
            pane,
            pre_sels: None,
        }
    }

    /// The buffer `pane` currently shows.
    pub(in crate::editor) fn buffer_id(&self, view: &EngineView) -> BufferId {
        view.panes[self.pane].buffer_id
    }

    /// Records `pane`'s current selections as the pre-entry snapshot. Called
    /// from `Layer::setup`, not the constructor: `setup` runs after the
    /// outgoing layer's own `tear_down` (see `Layer::setup`'s own doc), so a
    /// re-entrant `/`-search or sift captures the state the outgoing
    /// session's `tear_down` just restored — the true pre-session
    /// selections — instead of the mid-session preview a construction-time
    /// capture would have caught.
    pub(in crate::editor) fn capture(&mut self, state: &EditorState, view: &EngineView) {
        self.pre_sels = Some(commands::current_selections(state, view).clone());
    }

    /// The captured selections, still held — for a live preview that needs
    /// to read them without ending the session (`update_live_search`,
    /// `update_live_sift`).
    pub(in crate::editor) fn selections(&self) -> Option<&SelectionSet> {
        self.pre_sels.as_ref()
    }

    /// Takes the captured selections without writing them anywhere — for a
    /// `Confirm` arm that keeps the session's live-preview result instead of
    /// restoring the pre-session state, but must still empty the snapshot so
    /// the coming `tear_down` finds nothing left to restore.
    pub(in crate::editor) fn take_selections(&mut self) -> Option<SelectionSet> {
        self.pre_sels.take()
    }

    /// Writes the captured selections back into `pane` without consuming the
    /// snapshot — always targets `pane`, not whatever's currently focused.
    pub(in crate::editor) fn restore(
        &self,
        pane_state: &mut SecondaryMap<PaneId, SecondaryMap<BufferId, PaneBufferState>>,
        view: &EngineView,
    ) {
        let Some(sels) = self.pre_sels.clone() else {
            return;
        };
        let bid = self.buffer_id(view);
        pane_state[self.pane][bid].set_selections(sels);
    }

    /// `restore`'s consuming counterpart, for `Layer::tear_down`: writes the
    /// snapshot back (if one was ever taken) and returns the buffer it wrote
    /// into, so a caller with follow-up work scoped to that buffer (clearing
    /// a live search) doesn't have to re-derive it. `None` when the snapshot
    /// was already taken by a `Confirm` arm.
    pub(in crate::editor) fn take_restore(
        &mut self,
        pane_state: &mut SecondaryMap<PaneId, SecondaryMap<BufferId, PaneBufferState>>,
        view: &EngineView,
    ) -> Option<BufferId> {
        let sels = self.pre_sels.take()?;
        let bid = self.buffer_id(view);
        pane_state[self.pane][bid].set_selections(sels);
        Some(bid)
    }
}
