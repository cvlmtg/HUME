use std::time::{Duration, SystemTime};

use rustc_hash::FxHashMap;

use crate::changeset::ChangeSet;
use crate::selection::SelectionSet;
use crate::transaction::Transaction;

mod children;
use children::Children;

// ── Arena index ───────────────────────────────────────────────────────────────

/// A stable key into the History revision arena.
///
/// IDs are assigned once (monotonically increasing) and never reused, even
/// after a revision is evicted by `undo-levels` trimming. This makes stale
/// IDs held by other structs (e.g. `Buffer::saved_revision`, search caches)
/// safe: an evicted ID never matches again, rather than matching a
/// *different* revision that reused the same slot.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct RevisionId(pub(crate) usize);

impl RevisionId {
    /// The id numbered `n` in `history`, `None` when no such revision exists
    /// (never recorded, or evicted by `undo-levels` trimming). The only way
    /// to mint an id from a raw number.
    pub fn checked(history: &History, n: usize) -> Option<Self> {
        let id = Self(n);
        history.revisions.contains_key(&id).then_some(id)
    }

    /// The raw number, for a foreign coordinate system (a Steel integer).
    pub fn index(self) -> usize {
        self.0
    }
}

/// What separates the current text from another revision's.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RevisionPath {
    /// The revision is not in the tree: never recorded, or evicted.
    Unknown,
    /// The revision is the current one.
    Here,
    /// Maps the current text to the revision's text.
    Changes(ChangeSet),
}

/// One revision as seen from outside the crate: where it sits in the tree
/// and how old it is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RevisionNode {
    id: RevisionId,
    parent: Option<RevisionId>,
    age: Duration,
}

impl RevisionNode {
    pub fn id(&self) -> RevisionId {
        self.id
    }

    /// `None` only for the root.
    pub fn parent(&self) -> Option<RevisionId> {
        self.parent
    }

    /// Wall-clock age as of the `now` the enumeration was taken at.
    pub fn age(&self) -> Duration {
        self.age
    }
}

/// A single node in the undo tree.
///
/// Every revision but the root links to its parent with a forward
/// Transaction (parent → this state, for redo) and an inverse Transaction
/// (this state → parent, for undo). No buffer snapshot is stored: undo
/// reconstructs the previous state by applying the inverse Transaction.
///
/// `children` records all revisions that branch from this one and which of
/// them redo continues along: the child most recently created or walked
/// through, so redo stays on the branch the user is on.
struct Revision {
    /// `None` only for the root.
    link: Option<Link>,
    /// Child revisions: branches created from this state.
    children: Children,
    /// When this revision was created. Read by the `:earlier`/`:later`
    /// step-resolution queries below via [`History::age`]: a wall-clock
    /// [`SystemTime`], not a monotonic [`std::time::Instant`], because
    /// `:earlier 30m` means thirty minutes of wall-clock time, including
    /// any span the machine spent suspended; `Instant` is `CLOCK_MONOTONIC`
    /// (Linux) / `CLOCK_UPTIME_RAW` (macOS), both of which stop advancing
    /// across sleep.
    timestamp: SystemTime,
}

/// A revision's edge to its parent.
struct Link {
    parent: RevisionId,
    /// Apply this to move from the revision back to the parent state (undo).
    /// Its `selection` is the pre-edit selection, where cursors were before
    /// this revision was created.
    inverse: Transaction,
    /// Apply this to move from the parent state forward to the revision
    /// (redo). Its `selection` is the post-edit selection.
    forward: Transaction,
}

impl Revision {
    fn parent(&self) -> Option<RevisionId> {
        self.link.as_ref().map(|link| link.parent)
    }

    /// # Panics
    /// Panics on the root, which has no parent to step to.
    fn link(&self) -> &Link {
        self.link
            .as_ref()
            .expect("a revision stepped through is never the root")
    }
}

/// Tree-structured undo/redo history.
///
/// Revisions live in an arena keyed by monotonically assigned IDs that are
/// never reused. The root (id 0) is the initial document state and has no
/// transactions; `current` is the revision matching the buffer and selections.
/// All ordering comes from the `parent`/`children` links, never from map
/// iteration order.
///
/// Editing after an undo adds a sibling branch, so no redo path is discarded;
/// redo continues along the child last walked through. History stores only
/// transactions, never buffers: the caller owns the current buffer.
pub struct History {
    /// Arena of all revisions, keyed by stable `RevisionId`.
    revisions: FxHashMap<RevisionId, Revision>,
    /// The currently active revision.
    current: RevisionId,
    /// Next ID to assign in `record`. Monotonic, never reused, even for
    /// evicted revisions.
    next_id: usize,
    /// Maximum non-root revisions to retain. `0` means unlimited (no
    /// trimming). Enforced lazily in `record`, not the moment it is set.
    undo_levels: usize,
    /// Bumped whenever `current` moves or the tree gains or loses a
    /// revision, so an observer can tell the tree changed without comparing
    /// it.
    change_seq: u64,
}

impl Default for History {
    fn default() -> Self {
        Self::new()
    }
}

impl History {
    /// Create a new history rooted at the initial document state, the state
    /// before any edit.
    pub fn new() -> Self {
        let root = Revision {
            link: None,
            children: Children::default(),
            timestamp: SystemTime::now(),
        };

        let mut revisions = FxHashMap::default();
        revisions.insert(Self::ROOT, root);

        Self {
            revisions,
            current: Self::ROOT,
            next_id: 1,
            undo_levels: 0,
            change_seq: 0,
        }
    }

    /// Drop every revision and return to a fresh root, keeping `undo_levels`.
    /// `change_seq` never moves back, and moves forward only when a revision
    /// was dropped: resetting a root-only tree changes nothing an observer
    /// could see.
    pub fn reset(&mut self) {
        let undo_levels = self.undo_levels;
        let change_seq = self.change_seq + u64::from(self.len() > 1);
        *self = Self::new();
        self.undo_levels = undo_levels;
        self.change_seq = change_seq;
    }

    /// Set the maximum number of non-root revisions to retain. `0` means
    /// unlimited.
    ///
    /// Takes effect on the *next* `record` call, not immediately, matching
    /// Vim's `undolevels` semantics, where lowering the cap does not
    /// retroactively trim existing history.
    pub fn set_undo_levels(&mut self, levels: usize) {
        self.undo_levels = levels;
    }

    /// The current `undo-levels` cap. `0` means unlimited.
    pub fn undo_levels(&self) -> usize {
        self.undo_levels
    }

    /// Record a new edit and advance the current position to it.
    ///
    /// Creates a new revision as a child of the current revision and makes
    /// it the new `current`. The caller provides both the forward and inverse
    /// changesets. The inverse must have been computed against the pre-edit
    /// buffer before that buffer was replaced.
    ///
    /// # Arguments
    ///
    /// - `forward_cs`: the ChangeSet that was applied to produce the new state.
    /// - `inverse_cs`: `forward_cs.invert(&pre_edit_text)`, which reverses the edit.
    /// - `pre_edit_sels`: cursor positions before the edit (stored in `inverse`
    ///   so undo restores them).
    /// - `post_edit_sels`: cursor positions after the edit (stored in `forward`
    ///   so redo restores them).
    ///
    /// If `undo-levels` trimming promotes a child of the root to become the
    /// new root (see `Self::enforce_undo_levels`), returns the id of that
    /// promoted revision. Callers holding an external `RevisionId` (e.g. a
    /// "clean" save point) must remap it to [`Self::ROOT`] if it matches, so
    /// that state stays reachable. Promotion also overwrites whatever state
    /// `ROOT` previously represented, so a caller-held id equal to `ROOT`
    /// itself no longer names the same state after a promotion and must be
    /// invalidated, not left pointing at ROOT. Returns `None` when no
    /// promotion occurred.
    pub fn record(
        &mut self,
        forward_cs: ChangeSet,
        inverse_cs: ChangeSet,
        pre_edit_sels: SelectionSet,
        post_edit_sels: SelectionSet,
    ) -> Option<RevisionId> {
        let new_id = RevisionId(self.next_id);
        self.next_id += 1;
        let parent_id = self.current;

        let revision = Revision {
            link: Some(Link {
                parent: parent_id,
                // inverse carries pre-edit sels: after undoing, cursors return there.
                inverse: Transaction::new(inverse_cs, pre_edit_sels),
                // forward carries post-edit sels: after redoing, cursors land there.
                forward: Transaction::new(forward_cs, post_edit_sels),
            }),
            children: Children::default(),
            timestamp: SystemTime::now(),
        };

        self.revisions.insert(new_id, revision);
        self.revisions
            .get_mut(&parent_id)
            .expect("parent exists")
            .children
            .push(new_id);
        self.set_current(new_id);

        self.enforce_undo_levels()
    }

    /// Trim the tree down to at most `undo_levels` non-root revisions,
    /// Vim-`undolevels`-style: evict from the root end, oldest first.
    ///
    /// The revision on the path to `current` is always protected: `current`
    /// is a freshly recorded leaf, and `undo_levels` (when enforced) is at
    /// least 1, so it is never a candidate for eviction.
    ///
    /// Each iteration looks at the root's children (creation order, oldest
    /// first):
    /// - More than one child: the root has old alternate branches. The
    ///   oldest branch *not* on the path to `current` is discarded whole
    ///   (mirrors Vim freeing an entire unreachable redo branch).
    /// - Exactly one child `C` (so `C` is necessarily on the path to
    ///   `current`, and `C != current`, see above): there is nothing to
    ///   discard without cutting into the live path, so `C` is *promoted*:
    ///   its children become the root's children, and `C` itself is
    ///   removed. This may still overshoot below the cap when a whole
    ///   branch is discarded in one step, as Vim does.
    ///
    /// Returns the id of the last revision promoted into the root, if any.
    fn enforce_undo_levels(&mut self) -> Option<RevisionId> {
        if self.undo_levels == 0 {
            return None;
        }

        let mut last_promoted = None;
        while self.revisions.len() - 1 > self.undo_levels {
            let root_children = &self.revisions[&Self::ROOT].children;

            if root_children.len() > 1 {
                let protected = self.root_child_on_current_path();
                let victim = root_children
                    .iter()
                    .find(|&c| c != protected)
                    .expect("more than one child, at most one is protected");
                self.remove_subtree(victim);
            } else {
                let c_id = root_children
                    .iter()
                    .next()
                    .expect("the cap is exceeded, so the root has a child");
                let c = self.revisions.remove(&c_id).expect("child exists");
                for child in c.children.iter() {
                    let link = self
                        .revisions
                        .get_mut(&child)
                        .expect("child exists")
                        .link
                        .as_mut();
                    link.expect("a child has a parent").parent = Self::ROOT;
                }
                let root = self.revisions.get_mut(&Self::ROOT).expect("root exists");
                root.children = c.children;
                root.timestamp = c.timestamp;
                last_promoted = Some(c_id);
            }
        }
        last_promoted
    }

    /// Walk parent links from `current` up to find which child of the root
    /// lies on the path to `current`.
    fn root_child_on_current_path(&self) -> RevisionId {
        let mut id = self.current;
        while let Some(parent) = self.revisions[&id].parent() {
            if parent == Self::ROOT {
                return id;
            }
            id = parent;
        }
        unreachable!("current must have an ancestor that is a child of root")
    }

    /// Remove `id` and every revision in its subtree from the arena, and
    /// detach `id` from its parent's `children` list.
    fn remove_subtree(&mut self, id: RevisionId) {
        if let Some(parent) = self.revisions[&id].parent() {
            self.revisions
                .get_mut(&parent)
                .expect("parent exists")
                .children
                .remove(id);
        }

        let mut stack = vec![id];
        while let Some(next) = stack.pop() {
            if let Some(revision) = self.revisions.remove(&next) {
                stack.extend(revision.children.iter());
            }
        }
    }

    /// Undo one step: return the inverse Transaction for the current revision
    /// and move to the parent. Returns `None` if already at the root (nothing
    /// to undo).
    ///
    /// Returns an owned `Transaction` (cloned from the arena) rather than a
    /// reference, to avoid lifetime conflicts when the caller also holds a
    /// reference to other fields of the owning struct (e.g. `Buffer::text`).
    /// `Transaction` is cheap to clone: its ChangeSet is a `Vec<Operation>`.
    pub fn undo(&mut self) -> Option<Transaction> {
        let old_current = self.current;
        // One lookup, not two: `parent` and `inverse` both come off the same
        // arena entry, read before `self.current` moves past it.
        let rev = &self.revisions[&old_current];
        let link = rev.link.as_ref()?;
        let (parent, inverse) = (link.parent, link.inverse.clone());
        self.set_current(parent);
        Some(inverse)
    }

    /// Redo one step: return the forward Transaction of the redo child and
    /// move to it. Returns `None` if the current revision has no children.
    ///
    /// The redo child is the one most recently created or walked through, as
    /// in Vim: after undoing and making a new edit, redo goes to that edit,
    /// and after jumping into an older branch, redo stays on it.
    ///
    /// Returns an owned `Transaction` for the same reason as [`Self::undo`].
    pub fn redo(&mut self) -> Option<Transaction> {
        // Copy out child_id before mutating current.
        let child_id = self.revisions[&self.current].children.redo()?;
        self.set_current(child_id);
        Some(self.revisions[&child_id].link().forward.clone())
    }

    /// Walk up to `count` revisions toward the root, returning the inverse
    /// Transactions to apply, in order. Loops [`Self::undo`], the single
    /// definition of one step. Short of `count` when the walk reaches the
    /// root; empty when `count == 0` or already at the root.
    ///
    /// Fold with `ChangeSet::compose_all` to apply once.
    pub fn undo_n(&mut self, count: usize) -> Vec<Transaction> {
        let mut txns = Vec::new();
        for _ in 0..count {
            match self.undo() {
                Some(txn) => txns.push(txn),
                None => break,
            }
        }
        txns
    }

    /// Redo up to `count` steps forward along the redo chain (each revision's
    /// redo child); loops [`Self::redo`]. See [`Self::undo_n`] for the caller-side
    /// composition contract.
    pub fn redo_n(&mut self, count: usize) -> Vec<Transaction> {
        let mut txns = Vec::new();
        for _ in 0..count {
            match self.redo() {
                Some(txn) => txns.push(txn),
                None => break,
            }
        }
        txns
    }

    /// True if there is at least one revision above the current position.
    pub fn can_undo(&self) -> bool {
        self.revisions[&self.current].link.is_some()
    }

    /// True if the current revision has at least one child.
    pub fn can_redo(&self) -> bool {
        self.revisions[&self.current].children.redo().is_some()
    }

    /// Wall-clock age of revision `id` as of `now`. Every step-resolution
    /// query below measures through this rather than reading `timestamp`
    /// directly, so they can't drift on how the clock read is taken. `now` is
    /// a parameter rather than a fresh `SystemTime::now()` per call: a walk
    /// spanning thousands of revisions takes one clock read for the whole
    /// query instead of one per node, and every node is compared against the
    /// same instant. `unwrap_or_default` (age `0`) is deliberate for a
    /// `SystemTime` that moved backwards since the revision was stamped (a
    /// manual clock set, an NTP step): reading that revision as "just now" is
    /// the safe direction to round a clock glitch, since it makes the
    /// revision look *younger*, never older, so `undo_steps_older_than`/
    /// `redo_steps_newer_than` can only under-, never over-, travel as a
    /// result.
    fn age(&self, id: RevisionId, now: SystemTime) -> Duration {
        now.duration_since(self.revisions[&id].timestamp)
            .unwrap_or_default()
    }

    /// Undo steps needed to reach the state as of `age` ago, for `:earlier`.
    /// `Ok(n)` feeds straight into [`Self::undo_n`], so the count (not a
    /// revision id) is what crosses the crate boundary and the tree stays
    /// unenumerable. `Err(n)` means the request is unsatisfiable (the walk
    /// reached the root while it was still younger than `age`); `n` is the
    /// number of real ancestors above the root, i.e. as far as `undo_n` can
    /// actually travel. The caller decides how to report that exhaustion;
    /// this function only distinguishes the two cases.
    ///
    /// Walks up toward the root while the revision underfoot is still
    /// younger than `age`. Strict `<` keeps an exact hit un-counted as
    /// unsatisfiable: landing on a revision exactly `age` old is a hit, not
    /// an over-travel.
    pub fn undo_steps_older_than(&self, age: Duration) -> Result<usize, usize> {
        let now = SystemTime::now();
        let mut steps = 0;
        let mut id = self.current;
        while self.age(id, now) < age {
            let Some(parent) = self.revisions[&id].parent() else {
                return Err(steps);
            };
            steps += 1;
            id = parent;
        }
        Ok(steps)
    }

    /// Redo steps needed to reach the state as of `age` ago, for `:later`.
    /// `Ok(n)` feeds straight into [`Self::redo_n`]. `Err(n)` mirrors
    /// [`Self::undo_steps_older_than`]'s: the walk reached a leaf that is
    /// still older than `age`, so the request is unsatisfiable, and `n` is
    /// every step `redo_n` can actually take along this chain.
    ///
    /// Walks down the redo chain (the same path [`Self::redo_n`] takes) while
    /// the next child is still at least `age` old, stopping
    /// before the first child young enough to postdate it. Strict `>` keeps
    /// an exact hit un-counted as unsatisfiable, same boundary as
    /// `undo_steps_older_than`'s `<`.
    pub fn redo_steps_newer_than(&self, age: Duration) -> Result<usize, usize> {
        let now = SystemTime::now();
        let mut steps = 0;
        let mut id = self.current;
        while let Some(child) = self.revisions[&id].children.redo() {
            // Single `age()` call per child: doubles as the step decision
            // and, when this turns out to be the last child, the check for
            // whether the walk ended satisfied or not.
            let child_age = self.age(child, now);
            if child_age < age {
                break;
            }
            id = child;
            steps += 1;
            if self.revisions[&id].children.redo().is_none() && child_age > age {
                return Err(steps);
            }
        }
        Ok(steps)
    }

    /// Total number of revisions in the tree (including the root).
    ///
    /// A `History` always contains at least the root revision, so it is never
    /// empty. `is_empty()` would be a constant `false` and is intentionally
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

    /// A counter that changes whenever a revision is recorded or evicted, or
    /// `current` moves, and at no other time.
    pub fn change_seq(&self) -> u64 {
        self.change_seq
    }

    /// Every revision, in id order, aged as of `now`. One `now` for the whole
    /// enumeration, so every node is measured against the same instant.
    pub fn nodes(&self, now: SystemTime) -> Vec<RevisionNode> {
        let mut nodes: Vec<_> = self
            .revisions
            .iter()
            .map(|(&id, revision)| RevisionNode {
                id,
                parent: revision.parent(),
                age: now.duration_since(revision.timestamp).unwrap_or_default(),
            })
            .collect();
        nodes.sort_by_key(|node| node.id.0);
        nodes
    }

    /// Parent of a revision. `None` for the root or for an id that is out of
    /// bounds or has been evicted by `undo-levels` trimming.
    ///
    /// Using `.get` instead of direct indexing lets callers safely query a
    /// stale (evicted) id without panicking.
    pub fn parent(&self, id: RevisionId) -> Option<RevisionId> {
        self.revisions.get(&id)?.parent()
    }

    /// Move `current` to `id`; the one place `change_seq` follows it.
    fn set_current(&mut self, id: RevisionId) {
        self.current = id;
        self.change_seq += 1;
    }

    /// The two legs of the walk from `current` to `target` through their
    /// lowest common ancestor: the revisions to step out of, newest first,
    /// then the revisions to step into, oldest first. `None` if `target` is
    /// not in the tree; both legs are empty when it is `current`.
    fn path(&self, target: RevisionId) -> Option<(Vec<RevisionId>, Vec<RevisionId>)> {
        if !self.revisions.contains_key(&target) {
            return None;
        }

        // A parent's id is always smaller than its child's (ids ascend and
        // eviction re-points orphans to the root), so stepping the larger
        // id up meets at the lowest common ancestor.
        let (mut up_path, mut down_path) = (Vec::new(), Vec::new());
        let (mut from, mut to) = (self.current, target);
        while from != to {
            if from.0 > to.0 {
                up_path.push(from);
                from = self.revisions[&from].link().parent;
            } else {
                down_path.push(to);
                to = self.revisions[&to].link().parent;
            }
        }
        down_path.reverse();
        Some((up_path, down_path))
    }

    /// The one changeset that maps the current text to `target`'s text,
    /// without moving `current`, the redo targets or [`Self::change_seq`].
    /// It is the net of the walk [`Self::goto_revision`] performs.
    pub fn changes_to(&self, target: RevisionId) -> RevisionPath {
        let Some((up_path, down_path)) = self.path(target) else {
            return RevisionPath::Unknown;
        };
        let steps = up_path
            .iter()
            .map(|id| self.revisions[id].link().inverse.clone())
            .chain(
                down_path
                    .iter()
                    .map(|id| self.revisions[id].link().forward.clone()),
            )
            .map(Transaction::into_changes);
        match ChangeSet::compose_all(steps) {
            Some(cs) => RevisionPath::Changes(cs),
            None => RevisionPath::Here,
        }
    }

    /// Jump to an arbitrary revision in the undo tree, the general case
    /// [`Self::undo_n`]/[`Self::redo_n`] don't need, since each of those
    /// already walks a straight line of `parent`/`children` links with no
    /// LCA to find. This is for a target that isn't known to be a plain
    /// ancestor or descendant of `current` (e.g. jumping to an arbitrary
    /// branch).
    ///
    /// Returns the sequence of [`Transaction`]s that transform the current
    /// buffer into the target state, **in order**: txn₁ maps state A→B, txn₂
    /// maps B→C, and so on, exactly [`ChangeSet::compose`]'s contract
    /// (`self.len_after == other.len_before`). A caller applying them one at
    /// a time is free to; a caller walking many revisions in one logical step instead
    /// folds the list with `ChangeSet::compose_all` into one net transform
    /// and applies that once: same end state, one text mutation instead
    /// of N.
    ///
    /// Returns `None` if `target` is not in the tree (never recorded, or
    /// evicted). A `target` equal to the current revision is an empty walk.
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
        let (up_path, down_path) = self.path(target)?;
        if target == self.current {
            return Some(Vec::new());
        }

        // Build the transaction list.
        let mut txns = Vec::with_capacity(up_path.len() + down_path.len());
        for id in &up_path {
            txns.push(self.revisions[id].link().inverse.clone());
        }
        for id in &down_path {
            txns.push(self.revisions[id].link().forward.clone());
        }

        // The up leg needs no marking: every ancestor of `current` already
        // names the next revision toward it as its redo child.
        for &id in &down_path {
            let parent = self.revisions[&id].link().parent;
            self.revisions
                .get_mut(&parent)
                .expect("parent exists")
                .children
                .set_redo(id);
        }

        self.set_current(target);
        Some(txns)
    }
}

#[cfg(test)]
mod tests;
