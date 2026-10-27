//! Every position the editor stores against a buffer's text, borrowed as one
//! value. Each `Buffer` method that changes text takes this and carries the
//! stores through the change itself, so no text change can leave one behind.
//!
//! Stores that are only valid for the text they were computed against (the
//! search match cache, the completion menu anchor) are not here: they are
//! `Tracked`, and read as absent once the text moves. Dot-repeat's origin
//! chains text versions.

use hume_core::{BufferId, PaneId};
use hume_decorations::DecorationStores;
use hume_editing::edit::TextChange;
use hume_editing::tracked::Tracked;
use hume_rope::cluster::ClusterRange;
use slotmap::SecondaryMap;

use super::buffer::store::BufferStore;
use super::edit_session::{self, EditSession, InsertSession, OpenSession};
use super::input_stack::{InputStack, OtherLayers};
use super::lsp::diagnostics::DiagnosticsStore;
use super::lsp::document::LspDocuments;
use super::pane_state::{PaneBufferState, PaneEntries, PaneView};
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
    /// Each buffer's language-server attachments and the changes not yet
    /// sent to them.
    pub(in crate::editor) lsp: LspDocuments,
}

impl BufferPositions {
    /// Carry `buffer`'s diagnostics and last insertion through `change`, and
    /// queue it for the buffer's language servers.
    fn carry(&mut self, buffer: BufferId, change: &TextChange<'_>) {
        self.diagnostics.remap_through(buffer, change.changes());
        self.lsp.record(buffer, change);
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
    pub(in crate::editor) panes: &'a mut PaneEntries,
    /// The open Paste session or dot-repeat placeholder.
    pub(in crate::editor) session: &'a mut Option<EditSession>,
    /// The open Insert session, whose records are carried with the rest, and
    /// which grouped edits compose into.
    insert: Option<&'a mut InsertSession>,
    tracked: &'a mut TrackedPositions,
    /// Every input layer but the Insert one, for the snapshots and the
    /// completion session they hold.
    layers: OtherLayers<'a>,
    buffers: &'a mut BufferPositions,
    decorations: &'a mut DecorationStores,
}

impl<'a> PositionStores<'a> {
    /// The stores, with `insert` the open Insert session whether or not its
    /// layer is still on `input`: the closing session of an Insert layer's
    /// teardown is not.
    pub(in crate::editor) fn new(
        panes: &'a mut PaneView,
        session: &'a mut Option<EditSession>,
        closing: Option<&'a mut InsertSession>,
        input: &'a mut InputStack,
        buffers: &'a mut BufferPositions,
        decorations: &'a mut DecorationStores,
    ) -> Self {
        let (on_stack, layers) = input.split_insert();
        debug_assert!(
            closing.is_none() || on_stack.is_none(),
            "a closing Insert session's layer has left the stack"
        );
        Self {
            panes: &mut panes.entries,
            session,
            insert: closing.or(on_stack),
            tracked: &mut panes.tracked,
            layers,
            buffers,
            decorations,
        }
    }

    /// The open session, of any kind.
    pub(in crate::editor) fn open_session(&self) -> Option<OpenSession<'_>> {
        edit_session::open_session(self.insert.as_deref(), self.session)
    }

    /// The open Insert session, when it is on `(pane, buffer)`: the session
    /// a grouped edit there composes into.
    pub(in crate::editor) fn insert_at_mut(
        &mut self,
        pane: PaneId,
        buffer: BufferId,
    ) -> Option<&mut InsertSession> {
        self.insert
            .as_deref_mut()
            .filter(|s| s.owned_by(pane, buffer))
    }

    /// Carry every position stored for `buffer` through `change`: the
    /// selections of each pane that has shown the buffer, whether or not it
    /// shows it now, every jump list, the open Insert session's typed run
    /// and autoindent records, every position a script is tracking,
    /// every open prompt's snapshot, the open completion session, the
    /// buffer's diagnostics and decorations, and its last insertion. The
    /// change is also queued for the buffer's language servers.
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
        for (pane, entry) in self.panes.iter_mut() {
            if Some(pane) != acting
                && let Some(state) = entry.buffer_mut(buffer)
            {
                state.carry_selections(change);
            }
            // Every pane's list, not only those showing `buffer`: a list
            // holds entries for buffers its pane is not showing right now,
            // which is what makes cross-buffer Ctrl-o work.
            entry.jumps.translate_in_place(buffer, change);
        }
        if let Some(insert) = self.insert.as_deref_mut().filter(|s| s.buffer() == buffer) {
            insert.carry_marks(change);
            if let Some(session) = self.layers.buffer_completion_mut() {
                session.carry(change);
            }
        }
        self.tracked.carry(buffer, change);
        for snapshot in self.layers.snapshots_mut() {
            snapshot.carry(buffer, change);
        }
        self.buffers.carry(buffer, change);
        self.decorations.remap_through(buffer, change);
    }

    /// Reset every position stored for `buffer`, whose text was replaced
    /// with no change to carry them through: each pane that holds state for
    /// it starts over from `fresh`, and every other position stored for it
    /// is dropped (an open completion session on it is marked dead), its
    /// decorations with it. Its sign-source registrations hold no position
    /// and stay.
    pub(in crate::editor) fn reset(
        &mut self,
        buffer: BufferId,
        fresh: impl Fn() -> PaneBufferState,
    ) {
        for entry in self.panes.values_mut() {
            if let Some(state) = entry.buffer_mut(buffer) {
                *state = fresh();
            }
        }
        self.drop_shared(buffer);
        self.decorations.clear_entries(buffer);
    }

    /// Forget every position stored for `buffer`, which was closed:
    /// each pane's state for it is removed, and what `reset` drops beyond the
    /// panes goes with it, its sign-source registrations included.
    pub(in crate::editor) fn forget_buffer(&mut self, buffer: BufferId) {
        for entry in self.panes.values_mut() {
            entry.forget_buffer(buffer);
        }
        self.drop_shared(buffer);
        self.decorations.remove_buffer(buffer);
    }

    /// The stores that are not a pane's own state for `buffer`, decorations
    /// aside: `reset` and `forget_buffer` each drop those their own way. A buffer
    /// with language-server attachments reaches here only after
    /// `EditorState::lsp_buffer_closing` detached it, so its servers have
    /// seen `didClose`.
    fn drop_shared(&mut self, buffer: BufferId) {
        debug_assert!(
            !self.buffers.lsp.has_doc(buffer),
            "drop_shared: buffer still attached to a language server"
        );
        for entry in self.panes.values_mut() {
            entry.jumps.prune_buffer(buffer);
        }
        self.tracked.prune_buffer(buffer);
        for snapshot in self.layers.snapshots_mut() {
            snapshot.forget(buffer);
        }
        if self.insert.as_deref().is_some_and(|s| s.buffer() == buffer)
            && let Some(session) = self.layers.buffer_completion_mut()
        {
            session.forget();
        }
        self.forget_diagnostics(buffer);
        self.buffers.last_inserts.remove(buffer);
    }

    /// Drop what describes `buffer`'s text rather than pointing into it:
    /// its diagnostics, what each server was last told in a diagnostics
    /// pull, and its decorations. Its sign-source registrations hold no
    /// position and stay. `true` when it had diagnostics.
    pub(in crate::editor) fn forget_content(&mut self, buffer: BufferId) -> bool {
        let had_diagnostics = self.forget_diagnostics(buffer);
        self.decorations.clear_entries(buffer);
        had_diagnostics
    }

    /// Drop `buffer`'s diagnostics and what each server was last told in a
    /// diagnostics pull. `true` when it had diagnostics.
    fn forget_diagnostics(&mut self, buffer: BufferId) -> bool {
        let had_diagnostics = self.buffers.diagnostics.remove_buffer(buffer);
        self.buffers.lsp.forget_pulls(buffer);
        had_diagnostics
    }
}

impl super::EditorState {
    /// The open session, of any kind.
    pub(in crate::editor) fn open_session(&self) -> Option<OpenSession<'_>> {
        edit_session::open_session(self.input.insert_session(), &self.active_session)
    }

    /// The buffers, and every position stored against their text, borrowed
    /// apart so a `Buffer` method that changes text can carry them.
    pub(in crate::editor) fn edit_parts(&mut self) -> (&mut BufferStore, PositionStores<'_>) {
        self.parts_with_closing(None)
    }

    /// [`Self::edit_parts`] for the teardown of an Insert layer, whose
    /// `session` has left the stack and is still carried and composed into.
    pub(in crate::editor) fn closing_insert_parts<'a>(
        &'a mut self,
        session: &'a mut InsertSession,
    ) -> (&'a mut BufferStore, PositionStores<'a>) {
        self.parts_with_closing(Some(session))
    }

    fn parts_with_closing<'a>(
        &'a mut self,
        closing: Option<&'a mut InsertSession>,
    ) -> (&'a mut BufferStore, PositionStores<'a>) {
        (
            &mut self.buffers,
            PositionStores::new(
                &mut self.panes,
                &mut self.active_session,
                closing,
                &mut self.input,
                &mut self.buffer_positions,
                &mut self.config.decorations,
            ),
        )
    }
}

/// Empty stores for a test that changes a detached `Buffer`, one no pane,
/// jump list or prompt refers to.
#[cfg(test)]
#[derive(Default)]
pub(crate) struct DetachedStores {
    panes: PaneView,
    session: Option<EditSession>,
    input: InputStack,
    buffers: BufferPositions,
    decorations: DecorationStores,
}

#[cfg(test)]
impl DetachedStores {
    pub(crate) fn stores(&mut self) -> PositionStores<'_> {
        PositionStores::new(
            &mut self.panes,
            &mut self.session,
            None,
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
        let (mut view, _) = hume_engine::pipeline::EngineView::new(
            super::theme::fallback_theme(),
            hume_grid::Rect::default(),
        );
        let buffer = view.buffers.insert(());
        let mut stores = Self::default();
        let pane = stores
            .panes
            .open(
                &mut view,
                buffer,
                buf,
                1,
                super::pane_state::PaneChrome::Bare,
            )
            .pane_id();
        stores
            .panes
            .entries
            .seeded_mut(pane, buffer)
            .set_selections(selections, buf.text());
        (stores, pane, buffer)
    }

    /// `pane`'s selections for `buffer`.
    pub(crate) fn selections(
        &self,
        pane: PaneId,
        buffer: BufferId,
    ) -> hume_editing::selection::SelectionSet {
        self.panes.entries.seeded(pane, buffer).selections().clone()
    }
}
