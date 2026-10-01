//! Every position the editor stores against a buffer's text, borrowed as one
//! value. Each `Buffer` method that changes text takes this and carries the
//! stores through the change itself, so no text change can leave one behind.
//!
//! Stores that are only valid for the text they were computed against (the
//! search match cache, the completion menu anchor) are not here: they are
//! `Tracked`, and read as absent once the text moves. Dot-repeat's origin
//! chains text versions.

use hume_decorations::DecorationStores;
use hume_editing::edit::TextChange;
use hume_editing::tracked::Tracked;
use hume_engine::pipeline::{BufferId, PaneId};
use hume_rope::cluster::ClusterRange;
use slotmap::SecondaryMap;

use super::input_stack::InputStack;
use super::jump_list::JumpLists;
use super::lsp::diagnostics::DiagnosticsStore;
use super::pane_state::{PaneBufferState, PaneView};
use super::tracked_positions::TrackedPositions;

/// The positions each buffer carries that no pane owns.
#[derive(Default)]
pub(crate) struct BufferPositions {
    /// Every language server's diagnostics.
    pub(in crate::editor) diagnostics: DiagnosticsStore,
    /// The clusters typed during each buffer's most recently completed
    /// insert session, one range per selection that typed something, sorted
    /// by start, for `mii` (`select-last-insertion`). A range a change
    /// deletes outright drops out, and the record with it once none is left.
    pub(in crate::editor) last_inserts: SecondaryMap<BufferId, Tracked<Vec<ClusterRange>>>,
}

impl BufferPositions {
    /// Carry `buffer`'s diagnostics and last insertion through `change`.
    fn carry(&mut self, buffer: BufferId, change: &TextChange<'_>) {
        self.diagnostics.remap_through(buffer, change.changes());
        let Some(last) = self.last_inserts.get_mut(buffer) else {
            return;
        };
        last.translate(change, |ranges| {
            let mut chars: Vec<_> = ranges.iter().map(|range| range.chars()).collect();
            change.changes().map_ranges(&mut chars);
            ranges.clear();
            ranges.extend(
                chars
                    .into_iter()
                    .filter_map(|range| change.after().covering(range)),
            );
        });
        if last.get(change.after()).is_some_and(Vec::is_empty) {
            self.last_inserts.remove(buffer);
        }
    }
}

pub(crate) struct PositionStores<'a> {
    pub(in crate::editor) panes:
        &'a mut SecondaryMap<PaneId, SecondaryMap<BufferId, PaneBufferState>>,
    jumps: &'a mut JumpLists,
    tracked: &'a mut TrackedPositions,
    input: &'a mut InputStack,
    buffers: &'a mut BufferPositions,
    decorations: &'a mut DecorationStores,
}

impl<'a> PositionStores<'a> {
    pub(in crate::editor) fn new(
        panes: &'a mut PaneView,
        input: &'a mut InputStack,
        buffers: &'a mut BufferPositions,
        decorations: &'a mut DecorationStores,
    ) -> Self {
        Self {
            panes: &mut panes.state,
            jumps: &mut panes.jumps,
            tracked: &mut panes.tracked,
            input,
            buffers,
            decorations,
        }
    }

    /// Carry every position stored for `buffer` through `change`: the
    /// selections of each pane that has shown the buffer, whether or not it
    /// shows it now, every jump list, every position a script is tracking,
    /// every open prompt's snapshot, the open completion session, the
    /// buffer's diagnostics and decorations, and its last insertion.
    ///
    /// `acting` is the pane whose caller replaces its selections with the
    /// edit's own once this returns, so a real change does not carry them. An
    /// identity change carries them too: it only retags them for the relabeled
    /// text.
    pub(in crate::editor) fn carry(
        &mut self,
        buffer: BufferId,
        change: &TextChange<'_>,
        acting: Option<PaneId>,
    ) {
        let acting = acting.filter(|_| !change.changes().is_identity());
        for (pane, buffers) in self.panes.iter_mut() {
            if let Some(state) = buffers.get_mut(buffer) {
                if Some(pane) != acting {
                    state.carry_selections(change);
                }
                state.carry_marks(change);
            }
        }
        self.jumps.translate(buffer, change);
        self.tracked.carry(buffer, change);
        for snapshot in self.input.snapshots_mut() {
            snapshot.carry(buffer, change);
        }
        if let Some(session) = self.input.buffer_completion_mut() {
            session.carry(buffer, change);
        }
        self.buffers.carry(buffer, change);
        self.decorations.remap_through(buffer, change.changes());
    }

    /// Reset every position stored for `buffer`, whose text was replaced
    /// with no change to carry them through: each pane that holds state for
    /// it starts over from `fresh`, and every other position stored for it
    /// is dropped (an open completion session on it is marked dead).
    pub(in crate::editor) fn reset(
        &mut self,
        buffer: BufferId,
        fresh: impl Fn() -> PaneBufferState,
    ) {
        for buffers in self.panes.values_mut() {
            if let Some(state) = buffers.get_mut(buffer) {
                *state = fresh();
            }
        }
        self.drop_shared(buffer);
    }

    /// Forget every position stored for `buffer`, which was closed:
    /// each pane's state for it is removed, and what `reset` drops beyond the
    /// panes goes with it.
    pub(in crate::editor) fn forget_buffer(&mut self, buffer: BufferId) {
        for buffers in self.panes.values_mut() {
            buffers.remove(buffer);
        }
        self.drop_shared(buffer);
    }

    /// The stores that are not a pane's own state for `buffer`.
    fn drop_shared(&mut self, buffer: BufferId) {
        self.jumps.prune_buffer(buffer);
        self.tracked.prune_buffer(buffer);
        for snapshot in self.input.snapshots_mut() {
            snapshot.forget(buffer);
        }
        if let Some(session) = self.input.buffer_completion_mut() {
            session.forget(buffer);
        }
        self.buffers.diagnostics.remove_buffer(buffer);
        self.decorations.remove_buffer(buffer);
        self.buffers.last_inserts.remove(buffer);
    }
}

/// Empty stores for a test that changes a detached `Buffer`, one no pane,
/// jump list or prompt refers to.
#[cfg(test)]
#[derive(Default)]
pub(crate) struct DetachedStores {
    panes: PaneView,
    input: InputStack,
    buffers: BufferPositions,
    decorations: DecorationStores,
}

#[cfg(test)]
impl DetachedStores {
    pub(crate) fn stores(&mut self) -> PositionStores<'_> {
        PositionStores::new(
            &mut self.panes,
            &mut self.input,
            &mut self.buffers,
            &mut self.decorations,
        )
    }

    /// Stores with one pane holding `selections` of `buf`, that pane, and the
    /// id `buf` goes by in them.
    pub(crate) fn with_pane(
        buf: &super::buffer::Buffer,
        selections: hume_editing::selection::SelectionSet,
    ) -> (Self, PaneId, BufferId) {
        let pane = slotmap::SlotMap::<PaneId, ()>::with_key().insert(());
        let buffer = slotmap::SlotMap::<BufferId, ()>::with_key().insert(());
        let mut state = super::pane_state::fresh_from_buf(buf);
        state.set_selections(selections, buf.text());
        let mut stores = Self::default();
        stores.panes.state.insert(pane, SecondaryMap::new());
        stores.panes.state[pane].insert(buffer, state);
        (stores, pane, buffer)
    }

    /// `pane`'s selections for `buffer`.
    pub(crate) fn selections(
        &self,
        pane: PaneId,
        buffer: BufferId,
    ) -> hume_editing::selection::SelectionSet {
        self.panes.state[pane][buffer].selections().clone()
    }
}
