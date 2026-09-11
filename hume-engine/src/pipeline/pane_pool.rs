use std::ops::{Index, IndexMut};

use slotmap::SlotMap;

use crate::pane::Pane;

use super::{DetachedPane, PaneId};

/// Proof that a pane was just inserted into the pool and isn't yet reachable
/// from any `LayoutTree` — minted only by [`PanePool::insert`], in this
/// module, and consumed by [`super::LayoutTree::split_leaf`]/
/// [`super::LayoutTree::leaf`] to splice it in. Not `Clone`/`Copy`: a
/// duplicable token would defeat the "exactly one tree ends up naming this
/// pane" guarantee. Mirrors [`DetachedPane`]'s enforcement on the opposite
/// transition (a pane about to leave the pool) at the same strength: this is
/// the compiler-checked half of "a pane in the pool with no tree leaf naming
/// it is a leak" — nothing stops a determined caller from pulling the id out
/// via [`Self::pane_id`] and never grafting it in, same as `DetachedPane`
/// allows for the destroy side, but the natural, ergonomic path requires a
/// real graft.
#[must_use = "an unattached pane must be spliced into a layout tree"]
#[derive(Debug)]
pub struct UnattachedPane(PaneId);

impl UnattachedPane {
    pub fn pane_id(&self) -> PaneId {
        self.0
    }
}

#[cfg(test)]
impl UnattachedPane {
    /// Mint a token for a `PaneId` never actually inserted into a pool —
    /// `LayoutTree`'s own unit tests (`pipeline::tests`) exercise tree
    /// shape in isolation from `PanePool`, off ids minted by a throwaway
    /// `SlotMap` (see that module's own `pane_ids` helper), and need a
    /// token to hand `split_leaf` without building a real `EngineView`.
    pub(crate) fn for_test(id: PaneId) -> Self {
        Self(id)
    }
}

/// Every pane that exists, across every tab — active or stashed — regardless
/// of which one is on screen. Wraps the raw `SlotMap` so the type itself
/// distinguishes the two things a caller can legitimately want from it: a
/// specific pane by id (`Index`/`get`/`get_mut`, the common case — nothing
/// below changes for the ~200 sites that already spell `view.panes[pid]`),
/// or every pane regardless of tab visibility
/// ([`Self::every_pane_across_all_tabs`], named so a call site states the
/// same thing a `// pane-pool-safe: <reason>` comment used to state in
/// prose). There is deliberately no `iter`/`values`/`values_mut`/`keys`/
/// `drain`/`IntoIterator` — the frame's own working set is
/// `EngineView::active_pane_ids()` (the active tab's leaves only); a bare
/// walk of the whole pool is right for buffer-lifecycle cleanup and wrong
/// for anything scoped to what's currently on screen, and this type has no
/// spelling for the wrong one to hide behind.
pub struct PanePool(SlotMap<PaneId, Pane>);

impl PanePool {
    pub(super) fn with_key() -> Self {
        Self(SlotMap::with_key())
    }

    pub fn get(&self, id: PaneId) -> Option<&Pane> {
        self.0.get(id)
    }

    pub fn get_mut(&mut self, id: PaneId) -> Option<&mut Pane> {
        self.0.get_mut(id)
    }

    pub fn contains_key(&self, id: PaneId) -> bool {
        self.0.contains_key(id)
    }

    pub fn len(&self) -> usize {
        self.0.len()
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// Insert a pane and mint the [`UnattachedPane`] token proving it isn't
    /// yet reachable from any layout tree. `pub(super)`: `EngineView::
    /// insert_pane` (`pipeline/mod.rs`) is the one sanctioned public
    /// wrapper — see `UnattachedPane`'s own doc.
    pub(super) fn insert(&mut self, pane: Pane) -> UnattachedPane {
        UnattachedPane(self.0.insert(pane))
    }

    /// Remove a pane, taking the [`DetachedPane`] token proving it has
    /// already been pruned from every layout tree that could reach it.
    /// `pub(super)`: `EngineView::remove_pane` is the one sanctioned public
    /// wrapper.
    pub(super) fn remove(&mut self, detached: DetachedPane) -> Option<Pane> {
        self.0.remove(detached.pane_id())
    }

    pub fn get_disjoint_mut<const N: usize>(&mut self, ids: [PaneId; N]) -> Option<[&mut Pane; N]> {
        self.0.get_disjoint_mut(ids)
    }

    /// Every pane in the pool, active tab or not. Use only for work that
    /// must reach a background tab's pane too — buffer-lifecycle cleanup (a
    /// closed/reloaded buffer must be forgotten everywhere, or a stale
    /// reference in a background tab resurfaces the moment it's refocused),
    /// or the Steel `(panes)` builtin, which enumerates every open pane by
    /// design. Per-frame work (sizing, decoration, scroll, mirroring) wants
    /// `EngineView::active_pane_ids()` instead.
    pub fn every_pane_across_all_tabs(&self) -> impl Iterator<Item = (PaneId, &Pane)> {
        self.0.iter()
    }

    /// Mutable counterpart of [`Self::every_pane_across_all_tabs`].
    pub fn every_pane_across_all_tabs_mut(&mut self) -> impl Iterator<Item = (PaneId, &mut Pane)> {
        self.0.iter_mut()
    }
}

impl Index<PaneId> for PanePool {
    type Output = Pane;

    fn index(&self, id: PaneId) -> &Pane {
        &self.0[id]
    }
}

impl IndexMut<PaneId> for PanePool {
    fn index_mut(&mut self, id: PaneId) -> &mut Pane {
        &mut self.0[id]
    }
}
