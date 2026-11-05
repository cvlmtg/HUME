//! A `Buffer` with the `SelectionSet` it is edited through, so a test calls
//! one method per edit, undo or redo instead of threading selections and a
//! detached position store through every call.

use hume_core::{BufferId, PaneHandle, PaneId};
use hume_editing::edit::Edited;
use hume_editing::history::RevisionId;
use hume_editing::selection::SelectionSet;
use hume_editing::state::EditState;
use hume_editing::text::BufferText;

use super::store::BufferStore;
use super::{Buffer, NewBuffer};
use crate::editor::edit_session::EditGroup;
use crate::editor::position_stores::DetachedStores;

pub(crate) struct DocHelper {
    store: BufferStore,
    id: BufferId,
    sels: SelectionSet,
    edit_group: Option<EditGroup>,
}

impl DocHelper {
    pub(crate) fn new(text: BufferText, sels: SelectionSet) -> Self {
        let (store, id) = BufferStore::holding(NewBuffer::Scratch(text));
        Self {
            store,
            id,
            sels,
            edit_group: None,
        }
    }

    pub(super) fn buf(&self) -> &Buffer {
        self.store.get(self.id)
    }

    pub(super) fn buf_mut(&mut self) -> &mut Buffer {
        self.store.get_mut(self.id)
    }

    pub(crate) fn apply_edit(&mut self, cmd: impl FnOnce(EditState) -> Edited) {
        let sels = self.sels.clone();
        self.sels = self
            .store
            .get_mut(self.id)
            .apply_edit(
                PaneHandle::with_pane(self.id, PaneId::default()),
                &mut DetachedStores::default().stores(),
                sels,
                cmd,
            )
            .into_selections();
    }

    pub(crate) fn apply_edit_grouped(&mut self, cmd: impl FnOnce(EditState) -> Edited) {
        let sels = self.sels.clone();
        let group = self
            .edit_group
            .as_mut()
            .expect("apply_edit_grouped called without an open group");
        let (new_sels, cs) = self.store.get_mut(self.id).apply_edit_grouped(
            self.id,
            &mut DetachedStores::default().stores(),
            PaneId::default(),
            sels,
            cmd,
        );
        group.compose(&cs);
        self.sels = new_sels;
    }

    pub(crate) fn begin_edit_group(&mut self) {
        let pre_sels = self.sels.clone();
        self.edit_group = Some(self.buf_mut().begin_edit_group(pre_sels.clone(), pre_sels));
    }

    pub(crate) fn commit_edit_group(&mut self) {
        let post_sels = self.sels.clone();
        let group = self
            .edit_group
            .take()
            .expect("commit_edit_group called without an open group");
        self.buf_mut()
            .commit_edit_group(group, post_sels, PaneId::default());
    }

    pub(crate) fn undo(&mut self) {
        self.undo_n(1);
    }

    pub(crate) fn redo(&mut self) {
        self.redo_n(1);
    }

    /// Returns the number of steps actually taken (short of `count` at the
    /// root).
    pub(crate) fn undo_n(&mut self, count: usize) -> usize {
        let Some((new_sels, _cs, steps)) = self.store.get_mut(self.id).undo_n(
            self.id,
            &mut DetachedStores::default().stores(),
            PaneId::default(),
            count,
        ) else {
            return 0;
        };
        self.sels = new_sels;
        steps
    }

    /// Returns the number of steps actually taken (short of `count` at the
    /// tip).
    pub(crate) fn redo_n(&mut self, count: usize) -> usize {
        let Some((new_sels, _cs, steps)) = self.store.get_mut(self.id).redo_n(
            self.id,
            &mut DetachedStores::default().stores(),
            PaneId::default(),
            count,
        ) else {
            return 0;
        };
        self.sels = new_sels;
        steps
    }

    pub(crate) fn goto_revision(&mut self, target: RevisionId) {
        if let Some((new_sels, _cs, _steps)) = self.store.get_mut(self.id).goto_revision(
            self.id,
            &mut DetachedStores::default().stores(),
            PaneId::default(),
            target,
        ) {
            self.sels = new_sels;
        }
    }

    /// Reload the buffer text in place, preserving history. The current
    /// selections are carried through the reload like any pane's, and the
    /// history revision records them before and after.
    pub(crate) fn reload_from(&mut self, new_text: BufferText) {
        let (mut stores, pane, id) =
            DetachedStores::with_pane(self.store.get_mut(self.id), self.sels.clone());
        self.buf_mut()
            .replace_text_recorded(id, &mut stores.stores(), new_text, pane);
        self.buf_mut().mark_saved();
        self.sels = stores.selections(pane, id);
    }

    pub(crate) fn text(&self) -> &BufferText {
        self.buf().text()
    }
    pub(crate) fn sels(&self) -> &SelectionSet {
        &self.sels
    }
    pub(crate) fn revision_dirty(&self) -> bool {
        self.buf().revision_dirty()
    }
    pub(crate) fn mark_saved(&mut self) {
        self.buf_mut().mark_saved();
    }
    pub(crate) fn can_undo(&self) -> bool {
        self.buf().can_undo()
    }
    pub(crate) fn set_undo_levels(&mut self, levels: usize) {
        self.buf_mut().set_undo_levels(levels);
    }
}
