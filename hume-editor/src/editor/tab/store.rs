//! Per-editor tab store: the display order and stashed window layout of
//! every open tab page.
//!
//! A "tab" here is Vim's sense — a saved window layout (its own
//! [`LayoutTree`] + its own focused pane), not a per-buffer strip. Panes are
//! never shared across tabs, but all live in the single global pool
//! (`EngineView::panes`) regardless of which tab is active — only which
//! panes a tab's `LayoutTree` *reaches* differs.
//! **Never mutate `stash`/`order`/`current` directly outside this file** —
//! go through `TabStore`'s own methods, mirroring `BufferStore`'s own
//! module-doc convention.

use slotmap::{SlotMap, new_key_type};

use hume_engine::pipeline::{LayoutTree, PaneId};

new_key_type! {
    /// Opaque handle to a tab. Minted only in `hume-editor` — the engine has
    /// no notion of tabs, only of the one `LayoutTree` currently active.
    /// `pub(crate)`, wider than the rest of this module: `crate::tabline`
    /// (a sibling of `editor`, matching where `crate::statusline` lives —
    /// see that module's own doc) holds and displays `TabId`s built by
    /// `crate::editor::frame::sync_tabline_view`, so the type must be
    /// visible on both sides of that boundary.
    pub(crate) struct TabId;
}

/// One inactive tab's saved window layout. The *active* tab's equivalent
/// data lives directly in `EngineView::layout` /
/// `EditorState::focus` — never duplicated here while a tab is
/// active (see [`TabStore`]'s doc for why: reading `stash[current]` would
/// return a stale snapshot, not the live tree).
struct TabState {
    layout: LayoutTree,
    focused_pane_id: PaneId,
}

/// Display/cycle order and stashed layout for every open tab.
///
/// `stash` holds one entry per id in `order`, including `current` — but the
/// entry for `current` is stale (superseded by the live `EngineView::layout`)
/// until the next switch overwrites it. Every method below either writes
/// that entry immediately before changing `current` (so it's never stale
/// for longer than one call), or documents that it reads the stale entry
/// deliberately.
pub(in crate::editor) struct TabStore {
    order: Vec<TabId>,
    current: TabId,
    stash: SlotMap<TabId, TabState>,
}

impl TabStore {
    /// Seed a single-tab store wrapping `initial_pane`'s own
    /// `LayoutTree::Leaf` — mirrors how `EngineView::new` seeds its own
    /// `layout` field with the same leaf. That leaf lives in
    /// `EngineView::layout` itself once the caller makes it live; this only
    /// allocates the first `TabId` to name it.
    pub(in crate::editor) fn new(initial_pane: PaneId) -> (Self, TabId) {
        let mut stash = SlotMap::with_key();
        let id = stash.insert(TabState {
            layout: LayoutTree::Leaf(initial_pane),
            focused_pane_id: initial_pane,
        });
        (
            Self {
                order: vec![id],
                current: id,
                stash,
            },
            id,
        )
    }

    pub(in crate::editor) fn current(&self) -> TabId {
        self.current
    }

    /// Display/cycle order — every tab, including the active one.
    pub(in crate::editor) fn order(&self) -> &[TabId] {
        &self.order
    }

    pub(in crate::editor) fn len(&self) -> usize {
        self.order.len()
    }

    /// The `PaneId` a tab last focused. For `id == current()` this is the
    /// stale entry (see the struct doc) — callers wanting the *live*
    /// focused pane of the active tab must read `EditorState::focus`
    /// instead. Panics on an unknown id (a `tab` module bug, not a
    /// user-reachable state).
    pub(in crate::editor) fn stashed_focus(&self, id: TabId) -> PaneId {
        self.stash[id].focused_pane_id
    }

    /// `current`'s index in `order`. Every method below that needs it reads
    /// this instead of taking a caller-supplied id (unlike, say,
    /// `BufferStore::next`, which takes one and so has a real "unknown id"
    /// case to handle with an `unwrap_or` fallback): `TabStore` reads its
    /// own `current` field, so a missing position would mean `order` and
    /// `current` had already gone out of sync — exactly the corruption
    /// `open_after_current`'s own insert below would otherwise mask instead
    /// of failing loudly.
    ///
    /// `pub(in crate::editor)` rather than private: `Editor::sync_tabline_view`
    /// (`frame.rs`) needs the same index and is the one caller outside this
    /// module — it reads it from here rather than re-deriving it from
    /// `order()`/`current()` separately, which would let the tabline's own
    /// copy silently drift from this one's panic message the moment either
    /// changes.
    pub(in crate::editor) fn current_pos(&self) -> usize {
        self.order
            .iter()
            .position(|&t| t == self.current)
            .expect("current tab is always present in order")
    }

    /// Snapshot `current`'s live layout/focus into its stash slot — the
    /// shared first half of every op that displaces the live tab.
    fn stash_current(&mut self, layout: LayoutTree, focused_pane_id: PaneId) {
        self.stash[self.current] = TabState {
            layout,
            focused_pane_id,
        };
    }

    /// `id`'s stashed layout/focus, for the caller to write into
    /// `EngineView::layout`/`EditorState::focus`.
    ///
    /// A clone of the stash entry rather than a `mem::replace`-out: the
    /// entry stays valid for the *next* switch away from `id` (see the
    /// struct doc — `stash[current]` is defined-stale, never removed, until
    /// the next switch away overwrites it), and the tree is small enough (a
    /// handful of panes at most) that cloning it is simpler than threading
    /// a placeholder through every caller that would otherwise need one.
    fn load(&self, id: TabId) -> (LayoutTree, PaneId) {
        let entry = &self.stash[id];
        (entry.layout.clone(), entry.focused_pane_id)
    }

    /// Snapshot the *currently active* tab's live layout/focus into its
    /// stash slot, make `target` current, and return `target`'s stashed
    /// layout/focus for the caller to write into `EngineView::layout` /
    /// `EditorState::focus`. `outgoing_layout` is the live tree
    /// being displaced — the caller reads it out of `EngineView::layout`
    /// before calling, since `TabStore` never borrows `EngineView`.
    pub(in crate::editor) fn switch(
        &mut self,
        outgoing_layout: LayoutTree,
        outgoing_focus: PaneId,
        target: TabId,
    ) -> (LayoutTree, PaneId) {
        self.stash_current(outgoing_layout, outgoing_focus);
        self.current = target;
        self.load(target)
    }

    /// Stash the outgoing tab (same as [`Self::switch`]'s first half),
    /// allocate a fresh id wrapping `layout`/`focused_pane_id`, insert it
    /// into `order` right after `current` (Vim's `:tabnew` placement), and
    /// make it current.
    pub(in crate::editor) fn open_after_current(
        &mut self,
        outgoing_layout: LayoutTree,
        outgoing_focus: PaneId,
        layout: LayoutTree,
        focused_pane_id: PaneId,
    ) -> TabId {
        let pos = self.current_pos() + 1;
        self.stash_current(outgoing_layout, outgoing_focus);
        let id = self.stash.insert(TabState {
            layout,
            focused_pane_id,
        });
        self.order.insert(pos, id);
        self.current = id;
        id
    }

    /// The tab that should gain focus once `current` closes — its left
    /// neighbour, or its right one when `current` is already the leftmost
    /// tab (Vim's own `:tabclose` placement: adjacent, never a wrap to the
    /// far end). Only meaningful with more than one tab open; panics
    /// otherwise, same precondition [`Self::close_current`] asserts.
    pub(in crate::editor) fn adjacent(&self) -> TabId {
        assert!(self.order.len() > 1, "adjacent requires more than one tab");
        let pos = self.current_pos();
        let neighbor = if pos == 0 { pos + 1 } else { pos - 1 };
        self.order[neighbor]
    }

    /// Remove `current` from the store (`order` + `stash`), promote
    /// `survivor` as the new current, and return its stashed layout/focus
    /// for the caller to write into `EngineView`/`EditorState`. Panics if
    /// `current` is the only tab — callers check `len() > 1` first.
    pub(in crate::editor) fn close_current(&mut self, survivor: TabId) -> (LayoutTree, PaneId) {
        assert!(
            self.order.len() > 1,
            "close_current requires more than one tab"
        );
        let closed = self.current;
        self.order.retain(|&t| t != closed);
        self.stash.remove(closed);
        self.current = survivor;
        self.load(survivor)
    }

    /// Next tab in display order (wraps around). Returns `current()` if
    /// only one tab is open.
    pub(in crate::editor) fn next(&self) -> TabId {
        let pos = self.current_pos();
        self.order[(pos + 1) % self.order.len()]
    }

    /// Previous tab in display order (wraps around). Returns `current()` if
    /// only one tab is open.
    pub(in crate::editor) fn prev(&self) -> TabId {
        let pos = self.current_pos();
        let prev = if pos == 0 {
            self.order.len() - 1
        } else {
            pos - 1
        };
        self.order[prev]
    }
}

#[cfg(test)]
mod tests;
