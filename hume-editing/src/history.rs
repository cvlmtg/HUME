use std::collections::HashMap;
use std::time::Instant;

use crate::changeset::{ChangeSet, ChangeSetBuilder};
use crate::selection::SelectionSet;
use crate::transaction::Transaction;

// ── Arena index ───────────────────────────────────────────────────────────────

/// A stable key into the History revision arena.
///
/// IDs are assigned once (monotonically increasing) and never reused, even
/// after a revision is evicted by `undo-levels` trimming. This makes stale
/// IDs held by other structs (e.g. `Buffer::saved_revision`, search caches)
/// safe by construction: an evicted ID simply never matches again, rather
/// than risking silently matching a *different* revision that reused the
/// same slot.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct RevisionId(pub(crate) usize);

impl RevisionId {
    /// Construct a `RevisionId` from a raw arena index.
    ///
    /// Prefer [`History::root_id`] for the root and reserve this for
    /// external code that must name a specific revision (e.g. tests).
    pub fn new(id: usize) -> Self {
        Self(id)
    }
}

// ── Revision ──────────────────────────────────────────────────────────────────

/// A single node in the undo tree.
///
/// Each revision stores both a forward Transaction (parent → this state, for
/// redo) and an inverse Transaction (this state → parent, for undo). No buffer
/// snapshot is stored — undo reconstructs the previous state by applying the
/// inverse Transaction.
///
/// The `children` vec records all revisions that branch from this one. The
/// **last** child (highest index) is the most recently created branch and is
/// the default redo target: after undoing and making a new edit, redo goes
/// to the most recent edit.
struct Revision {
    /// Apply this to move from the current state back to the parent state (undo).
    /// Its `selection` is the pre-edit selection — where cursors were before
    /// this revision was created.
    inverse: Transaction,
    /// Apply this to move from the parent state forward to this state (redo).
    /// Its `selection` is the post-edit selection.
    forward: Transaction,
    /// The parent revision. `None` only for the root.
    parent: Option<RevisionId>,
    /// Child revisions — branches created from this state.
    /// The last entry is the most recently created child (default redo target).
    children: Vec<RevisionId>,
    /// When this revision was created. Reserved for `:earlier`/`:later` time travel.
    #[allow(dead_code)]
    timestamp: Instant,
}

// ── History ───────────────────────────────────────────────────────────────────

/// Tree-structured undo/redo history.
///
/// ## Structure
///
/// Revisions are stored in an arena (`HashMap<RevisionId, Revision>`) keyed
/// by stable, monotonically-assigned IDs that are never reused. The root
/// revision (id 0) represents the initial document state and has identity
/// changesets. `current` tracks the active revision — the state that
/// matches the document's current buffer and selections.
///
/// All ordering (which child is newest, ancestor chains) comes from the
/// `children` vecs and `parent` links, never from map iteration order.
///
/// ## Branching
///
/// Undoing to state A then making a new edit C preserves the old redo path
/// (B) as a sibling of C — no edit is discarded by undoing/redoing.
///
/// ```text
///  root
///   └─ A        (first edit)
///       ├─ B    (second edit, later undone)
///       └─ C    (new edit after undoing to A — C is now the redo target)
/// ```
///
/// ## Undo/Redo
///
/// - **Undo**: apply `current.inverse`, set `current = current.parent`.
/// - **Redo**: pick the last child of `current`, apply its `forward`, set
///   `current` to that child.
///
/// ## What History does NOT own
///
/// Buffers. The caller holds the current buffer. History stores only
/// Transactions (changeset + selections), keeping it a pure data structure
/// with no buffer dependency.
pub struct History {
    /// Arena of all revisions, keyed by stable `RevisionId`.
    revisions: HashMap<RevisionId, Revision>,
    /// The currently active revision.
    current: RevisionId,
    /// Next ID to assign in `record`. Monotonic — never reused, even for
    /// evicted revisions.
    next_id: usize,
}

impl History {
    /// Create a new history rooted at the initial document state.
    ///
    /// The root revision has identity changesets (all Retain) and carries
    /// `initial_sels` as its selection — this is the state before any edit.
    /// `buf_len` is the character length of the initial buffer (needed to
    /// build the identity ChangeSet).
    pub fn new(initial_sels: SelectionSet, buf_len: usize) -> Self {
        // Build an identity ChangeSet: retain every character unchanged.
        let mut b = ChangeSetBuilder::new(buf_len);
        b.retain_rest();
        let identity_cs = b.finish();

        // The root's forward and inverse are both identity transactions.
        // The selection is the initial cursor state.
        let root = Revision {
            inverse: Transaction::new(identity_cs.clone(), initial_sels.clone()),
            forward: Transaction::new(identity_cs, initial_sels),
            parent: None,
            children: Vec::new(),
            timestamp: Instant::now(),
        };

        let mut revisions = HashMap::new();
        revisions.insert(Self::ROOT, root);

        Self {
            revisions,
            current: Self::ROOT,
            next_id: 1,
        }
    }

    /// Record a new edit and advance the current position to it.
    ///
    /// Creates a new revision as a child of the current revision and makes
    /// it the new `current`. The caller provides both the forward and inverse
    /// changesets — the inverse must have been computed against the pre-edit
    /// buffer before that buffer was replaced.
    ///
    /// # Arguments
    ///
    /// - `forward_cs`: the ChangeSet that was applied to produce the new state.
    /// - `inverse_cs`: `forward_cs.invert(&pre_edit_buf)` — reverses the edit.
    /// - `pre_edit_sels`: cursor positions before the edit (stored in `inverse`
    ///   so undo restores them).
    /// - `post_edit_sels`: cursor positions after the edit (stored in `forward`
    ///   so redo restores them).
    pub fn record(
        &mut self,
        forward_cs: ChangeSet,
        inverse_cs: ChangeSet,
        pre_edit_sels: SelectionSet,
        post_edit_sels: SelectionSet,
    ) {
        let new_id = RevisionId(self.next_id);
        self.next_id += 1;
        let parent_id = self.current;

        let revision = Revision {
            // inverse carries pre-edit sels: after undoing, cursors return there.
            inverse: Transaction::new(inverse_cs, pre_edit_sels),
            // forward carries post-edit sels: after redoing, cursors land there.
            forward: Transaction::new(forward_cs, post_edit_sels),
            parent: Some(parent_id),
            children: Vec::new(),
            timestamp: Instant::now(),
        };

        self.revisions.insert(new_id, revision);
        self.revisions
            .get_mut(&parent_id)
            .expect("parent exists")
            .children
            .push(new_id);
        self.current = new_id;
    }

    /// Undo: return the inverse Transaction for the current revision and move
    /// to the parent. Returns `None` if already at the root (nothing to undo).
    ///
    /// The returned Transaction carries the pre-edit buffer transform and the
    /// pre-edit selections. The caller applies it to the current buffer to
    /// restore the previous state and selections.
    ///
    /// Returns an owned `Transaction` (cloned from the arena) rather than a
    /// reference, to avoid lifetime conflicts when the caller also holds a
    /// reference to other fields of the owning struct (e.g. `Buffer::text`).
    /// `Transaction` is cheap to clone: its ChangeSet is a `Vec<Operation>`.
    pub fn undo(&mut self) -> Option<Transaction> {
        let old_current = self.current;
        // Copy out the parent id before mutating current.
        let parent = self.revisions[&old_current].parent?;
        self.current = parent;
        // Clone the inverse from the revision we just stepped out of.
        Some(self.revisions[&old_current].inverse.clone())
    }

    /// Redo: return the forward Transaction of the most recent child and move
    /// to it. Returns `None` if the current revision has no children.
    ///
    /// The most recent child (last in `children`) is chosen to match
    /// Vim/Helix behaviour: after undoing and making a new edit, redo goes
    /// to the most recent edit, not the historically first one.
    ///
    /// Returns an owned `Transaction` for the same reason as [`Self::undo`].
    pub fn redo(&mut self) -> Option<Transaction> {
        // Copy out child_id before mutating current.
        let child_id = *self.revisions[&self.current].children.last()?;
        self.current = child_id;
        Some(self.revisions[&child_id].forward.clone())
    }

    /// True if there is at least one revision above the current position.
    pub fn can_undo(&self) -> bool {
        self.revisions[&self.current].parent.is_some()
    }

    /// True if the current revision has at least one child.
    pub fn can_redo(&self) -> bool {
        !self.revisions[&self.current].children.is_empty()
    }

    /// Total number of revisions in the tree (including the root).
    ///
    /// A `History` always contains at least the root revision, so it is never
    /// empty — `is_empty()` would be a constant `false` and is intentionally
    /// absent.
    #[allow(clippy::len_without_is_empty)]
    pub fn len(&self) -> usize {
        self.revisions.len()
    }

    /// The revision id of the root (initial document state, before any edit).
    pub const ROOT: RevisionId = RevisionId(0);

    /// The currently active revision.
    pub fn current_id(&self) -> RevisionId {
        self.current
    }

    /// The initial selections stored in the root revision.
    ///
    /// Returned to the caller so pane state can be seeded when a pane first
    /// views a buffer, or when the buffer is reloaded from disk.
    pub fn initial_sels(&self) -> &SelectionSet {
        self.revisions[&Self::ROOT].forward.selection()
    }

    /// Parent of a revision. `None` for the root or for an id that is out of
    /// bounds or has been evicted by `undo-levels` trimming.
    ///
    /// Using `.get` instead of direct indexing closes the panic vector that
    /// would exist if a caller fabricated a `RevisionId` with an arbitrary
    /// value via [`RevisionId::new`], and also lets callers safely query a
    /// stale (evicted) id without panicking.
    pub fn parent(&self, id: RevisionId) -> Option<RevisionId> {
        self.revisions.get(&id)?.parent
    }

    /// Ancestor chain from `id` up to and including the root.
    ///
    /// Returns `[id, parent, grandparent, ..., root]`.
    fn ancestors(&self, mut id: RevisionId) -> Vec<RevisionId> {
        let mut chain = vec![id];
        while let Some(parent) = self.revisions[&id].parent {
            chain.push(parent);
            id = parent;
        }
        chain
    }

    /// Jump to an arbitrary revision in the undo tree.
    ///
    /// Returns the sequence of [`Transaction`]s that must be applied
    /// **in order** to transform the current buffer into the target state.
    /// The caller is responsible for applying each transaction sequentially —
    /// do **not** try to compose them, since each was computed against the
    /// buffer state at its specific point in history.
    ///
    /// Returns `None` if `target` equals the current revision (no-op) or is
    /// out of bounds.
    ///
    /// ## How it works
    ///
    /// The path from `current` to `target` passes through their Lowest Common
    /// Ancestor (LCA):
    ///
    /// - **Up leg** (`current` → LCA): for each node stepped out of, use its
    ///   `inverse` transaction (same as [`Self::undo`]).
    /// - **Down leg** (LCA → `target`): for each node stepped into, use its
    ///   `forward` transaction (same as [`Self::redo`]).
    pub fn goto_revision(&mut self, target: RevisionId) -> Option<Vec<Transaction>> {
        if target == self.current {
            return None;
        }
        if !self.revisions.contains_key(&target) {
            return None;
        }

        let ancestors_from = self.ancestors(self.current);
        let ancestors_to = self.ancestors(target);

        // Put the "from" ancestor set in a HashSet for O(1) lookup.
        // We need to find the first node in ancestors_to that also appears
        // in ancestors_from — that is the LCA.
        let from_set: std::collections::HashSet<RevisionId> =
            ancestors_from.iter().copied().collect();

        // Find the LCA: walk ancestors_to until we hit a node in from_set.
        let lca = *ancestors_to
            .iter()
            .find(|id| from_set.contains(id))
            .expect("all revisions share at least the root ancestor");

        // Up leg: nodes from `current` up to (not including) LCA.
        // ancestors_from = [current, ..., lca, ...]
        let up_path: Vec<RevisionId> = ancestors_from
            .iter()
            .copied()
            .take_while(|&id| id != lca)
            .collect();

        // Down leg: nodes from LCA's child down to `target`.
        // ancestors_to = [target, ..., lca_child, lca, ...]
        // Take everything before lca, then reverse so it goes lca_child → target.
        let mut down_path: Vec<RevisionId> = ancestors_to
            .iter()
            .copied()
            .take_while(|&id| id != lca)
            .collect();
        down_path.reverse();

        // Build the transaction list.
        let mut txns = Vec::with_capacity(up_path.len() + down_path.len());
        for id in &up_path {
            txns.push(self.revisions[id].inverse.clone());
        }
        for id in &down_path {
            txns.push(self.revisions[id].forward.clone());
        }

        self.current = target;
        Some(txns)
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests;
