//! Every position the editor stores against a buffer's text, borrowed as one
//! value. Each `Buffer` method that changes text takes this and carries the
//! stores through the change itself, so no text change can leave one behind.
//!
//! Stores that are only valid for the text they were computed against (the
//! search match cache, the last insertion, completion and dot-repeat anchors)
//! are not here: they are `Tracked`, and read as absent once the text moves.

use hume_editing::edit::TextChange;
use hume_engine::pipeline::{BufferId, PaneId};
use slotmap::SecondaryMap;

use super::input_stack::InputStack;
use super::jump_list::JumpLists;
use super::pane_state::{PaneBufferState, PaneView};

pub(crate) struct PositionStores<'a> {
    pub(in crate::editor) panes:
        &'a mut SecondaryMap<PaneId, SecondaryMap<BufferId, PaneBufferState>>,
    pub(in crate::editor) jumps: &'a mut JumpLists,
    input: &'a mut InputStack,
}

impl<'a> PositionStores<'a> {
    pub(in crate::editor) fn new(panes: &'a mut PaneView, input: &'a mut InputStack) -> Self {
        Self {
            panes: &mut panes.state,
            jumps: &mut panes.jumps,
            input,
        }
    }

    /// Carry every position stored for `buffer` through `change`: the
    /// selections of each pane that has shown the buffer, whether or not it
    /// shows it now, every jump list, and every open prompt's snapshot.
    pub(in crate::editor) fn carry(&mut self, buffer: BufferId, change: &TextChange<'_>) {
        for buffers in self.panes.values_mut() {
            if let Some(state) = buffers.get_mut(buffer) {
                state.carry(change);
            }
        }
        self.jumps.translate(buffer, change);
        for snapshot in self.input.snapshots_mut() {
            snapshot.carry(buffer, change);
        }
    }

    /// Reset every position stored for `buffer`, whose text was replaced
    /// with no change to carry them through: each pane that holds state for
    /// it starts over from `fresh`, and jump entries and prompt snapshots for
    /// it are dropped.
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
        self.jumps.prune_buffer(buffer);
        for snapshot in self.input.snapshots_mut() {
            snapshot.forget(buffer);
        }
    }
}

/// Empty stores for a test that changes a detached `Buffer`, one no pane,
/// jump list or prompt refers to.
#[cfg(test)]
#[derive(Default)]
pub(crate) struct DetachedStores {
    panes: PaneView,
    input: InputStack,
}

#[cfg(test)]
impl DetachedStores {
    pub(crate) fn stores(&mut self) -> PositionStores<'_> {
        PositionStores::new(&mut self.panes, &mut self.input)
    }
}
