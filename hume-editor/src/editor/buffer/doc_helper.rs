//! A `Buffer` with the `SelectionSet` it is edited through, so a test calls
//! one method per edit, undo or redo instead of threading selections and a
//! detached position store through every call.

use hume_core::{BufferId, PaneHandle, PaneId};
use hume_editing::edit::Edited;
use hume_editing::history::RevisionId;
use hume_editing::selection::SelectionSet;
use hume_editing::state::EditState;
use hume_editing::text::BufferText;

use super::Buffer;
use crate::editor::edit_session::EditGroup;
use crate::editor::position_stores::DetachedStores;

pub(crate) struct DocHelper {
    pub(super) buf: Buffer,
    sels: SelectionSet,
    edit_group: Option<EditGroup>,
}

impl DocHelper {
    pub(crate) fn new(text: BufferText, sels: SelectionSet) -> Self {
        Self {
            buf: Buffer::at_start(text),
            sels,
            edit_group: None,
        }
    }

    pub(crate) fn apply_edit(&mut self, cmd: impl FnOnce(EditState) -> Edited) {
        let sels = self.sels.clone();
        self.sels = self
            .buf
            .apply_edit(
                PaneHandle::with_pane(BufferId::default(), PaneId::default()),
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
        let (new_sels, cs) = self.buf.apply_edit_grouped(
            BufferId::default(),
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
        self.edit_group = Some(self.buf.begin_edit_group(pre_sels.clone(), pre_sels));
    }

    pub(crate) fn commit_edit_group(&mut self) {
        let post_sels = self.sels.clone();
        let group = self
            .edit_group
            .take()
            .expect("commit_edit_group called without an open group");
        self.buf
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
        let Some((new_sels, _cs, steps)) = self.buf.undo_n(
            BufferId::default(),
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
        let Some((new_sels, _cs, steps)) = self.buf.redo_n(
            BufferId::default(),
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
        if let Some((new_sels, _cs, _steps)) = self.buf.goto_revision(
            BufferId::default(),
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
        let (mut stores, pane, id) = DetachedStores::with_pane(&mut self.buf, self.sels.clone());
        self.buf
            .replace_text_recorded(id, &mut stores.stores(), new_text, pane);
        self.buf.mark_saved();
        self.sels = stores.selections(pane, id);
    }

    pub(crate) fn text(&self) -> &BufferText {
        self.buf.text()
    }
    pub(crate) fn sels(&self) -> &SelectionSet {
        &self.sels
    }
    pub(crate) fn revision_dirty(&self) -> bool {
        self.buf.revision_dirty()
    }
    pub(crate) fn mark_saved(&mut self) {
        self.buf.mark_saved();
    }
    pub(crate) fn can_undo(&self) -> bool {
        self.buf.can_undo()
    }
    pub(crate) fn set_undo_levels(&mut self, levels: usize) {
        self.buf.set_undo_levels(levels);
    }
}
