//! A mode layer's snapshot of one pane's selections, captured on entry and
//! restored on exit: the shape `SearchLayer` and `SiftLayer` both need for
//! cancel-restore, factored out so the capture/restore/take rules live in one
//! place instead of being re-derived in each layer.
//!
//! Not `edit_session.rs`: that module's `EditGroup` already has a `pre_sels`
//! field meaning something unrelated (an undo group's pre-*edit* selections,
//! not a session's pre-*entry* ones), and two unrelated `pre_sels` in one file
//! would be exactly the kind of same-name collision this codebase avoids
//! elsewhere. This type's reason to exist is the mode-layer session
//! lifecycle (`capture` on `Layer::setup`, `restore`/`take_restore` on
//! `Layer::tear_down`), which is `input_stack/` vocabulary.

use slotmap::SecondaryMap;

use hume_editing::edit::TextChange;
use hume_editing::selection::SelectionSet;
use hume_engine::pipeline::{BufferId, EngineView, PaneId};

use super::super::EditorState;
use super::super::buffer::store::BufferStore;
use super::super::pane_state::PaneBufferState;

/// `pane`'s selections as they were the moment this session opened, with the
/// buffer they were read from, plus the pane itself, never
/// `state.focus.id()`, which may have moved on since (a mouse click always
/// falls through under these layers, so focus can change while the session
/// stays open). The buffer is recorded rather than re-read from the pane,
/// because the pane may show another buffer by the time the snapshot is
/// used. `None` once a `Confirm` arm has taken it, so `Layer::tear_down`'s
/// own restore becomes a no-op instead of double-firing on top of whatever
/// the confirm already did. A text change to the buffer carries the
/// selections (see `PositionStores::carry`).
pub(in crate::editor) struct PaneSnapshot {
    pane: PaneId,
    captured: Option<(BufferId, SelectionSet)>,
}

impl PaneSnapshot {
    /// A fresh, not-yet-captured snapshot for `pane`: construction only
    /// records *where*; `capture` records *what*, once the layer actually
    /// lands (see `capture`'s own doc for why the two are split).
    pub(in crate::editor) fn new(pane: PaneId) -> Self {
        Self {
            pane,
            captured: None,
        }
    }

    pub(in crate::editor) fn pane(&self) -> PaneId {
        self.pane
    }

    /// Records `pane`'s current selections and buffer as the pre-entry
    /// snapshot. Called from `Layer::setup`, not the constructor: `setup`
    /// runs after the outgoing layer's own `tear_down` (see `Layer::setup`'s
    /// own doc), so a re-entrant `/`-search or sift captures the state the
    /// outgoing session's `tear_down` just restored (the true pre-session
    /// selections) instead of the mid-session preview a construction-time
    /// capture would have caught.
    pub(in crate::editor) fn capture(&mut self, state: &EditorState, view: &EngineView) {
        let bid = view.panes[self.pane].buffer_id;
        let sels = state.panes.state[self.pane][bid].selections().clone();
        self.captured = Some((bid, sels));
    }

    /// The captured buffer and selections, still held, for a live preview
    /// that needs to read them without ending the session
    /// (`update_live_search`, `update_live_sift`).
    pub(in crate::editor) fn selections(&self) -> Option<(BufferId, &SelectionSet)> {
        self.captured.as_ref().map(|(bid, sels)| (*bid, sels))
    }

    /// Takes the captured buffer and selections without writing them
    /// anywhere, for a `Confirm` arm that keeps the session's live-preview
    /// result instead of restoring the pre-session state, but must still
    /// empty the snapshot so the coming `tear_down` finds nothing left to
    /// restore.
    pub(in crate::editor) fn take_selections(&mut self) -> Option<(BufferId, SelectionSet)> {
        self.captured.take()
    }

    /// Writes the captured selections back into `pane`'s state for the
    /// captured buffer without consuming the snapshot.
    pub(in crate::editor) fn restore(
        &self,
        pane_state: &mut SecondaryMap<PaneId, SecondaryMap<BufferId, PaneBufferState>>,
        buffers: &BufferStore,
    ) {
        if let Some((bid, sels)) = &self.captured {
            pane_state[self.pane][*bid].set_selections(sels.clone(), buffers.get(*bid).text());
        }
    }

    /// `restore`'s consuming counterpart, for `Layer::tear_down`: writes the
    /// snapshot back (if one was ever taken) and returns the buffer it wrote
    /// into, so a caller with follow-up work scoped to that buffer (clearing
    /// a live search) doesn't have to re-derive it. `None` when the snapshot
    /// was already taken by a `Confirm` arm.
    pub(in crate::editor) fn take_restore(
        &mut self,
        pane_state: &mut SecondaryMap<PaneId, SecondaryMap<BufferId, PaneBufferState>>,
        buffers: &BufferStore,
    ) -> Option<BufferId> {
        let (bid, sels) = self.captured.take()?;
        pane_state[self.pane][bid].set_selections(sels, buffers.get(bid).text());
        Some(bid)
    }

    /// Carry the captured selections through `change` to `buffer`'s text.
    pub(in crate::editor) fn carry(&mut self, buffer: BufferId, change: &TextChange<'_>) {
        if let Some((bid, sels)) = &mut self.captured
            && *bid == buffer
        {
            sels.translate(change);
        }
    }

    /// Drop the captured selections when `buffer`'s text was replaced.
    pub(in crate::editor) fn forget(&mut self, buffer: BufferId) {
        if self
            .captured
            .as_ref()
            .is_some_and(|(bid, _)| *bid == buffer)
        {
            self.captured = None;
        }
    }
}
