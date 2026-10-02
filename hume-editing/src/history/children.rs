use super::RevisionId;

/// The revisions that branch from one revision, in creation order, plus the
/// one redo continues along.
///
/// `redo` is `Some` exactly when `ids` is non-empty, and is always a member
/// of `ids`. The fields are private to this file, so every write goes through
/// a method that keeps that true.
#[derive(Default)]
pub(super) struct Children {
    ids: Vec<RevisionId>,
    redo: Option<RevisionId>,
}

impl Children {
    /// Append a newly created child and make it the redo target.
    pub(super) fn push(&mut self, id: RevisionId) {
        self.ids.push(id);
        self.redo = Some(id);
    }

    /// Drop `id`, keeping the creation order of the rest. When it was the
    /// redo target, the newest remaining child takes over.
    pub(super) fn remove(&mut self, id: RevisionId) {
        self.ids.retain(|&c| c != id);
        if self.redo == Some(id) {
            self.redo = self.ids.last().copied();
        }
    }

    /// Make `child` the redo target.
    ///
    /// # Panics
    /// Panics when `child` is not one of these children: the caller walked an
    /// edge the tree does not have, which means the history is corrupt.
    pub(super) fn set_redo(&mut self, child: RevisionId) {
        assert!(
            self.ids.contains(&child),
            "redo target must be a child of the revision"
        );
        self.redo = Some(child);
    }

    /// The child redo continues along, `None` for a leaf.
    pub(super) fn redo(&self) -> Option<RevisionId> {
        self.redo
    }

    /// Children in creation order, oldest first.
    pub(super) fn iter(&self) -> impl Iterator<Item = RevisionId> + '_ {
        self.ids.iter().copied()
    }

    pub(super) fn len(&self) -> usize {
        self.ids.len()
    }
}
