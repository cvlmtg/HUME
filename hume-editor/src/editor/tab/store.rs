//! Per-editor tab store: the display order of every open tab page and the
//! stashed window layout of every inactive one.
//!
//! A "tab" here is Vim's sense: a saved window layout (its own
//! [`FocusedLayout`]: a tree and its focused pane), not a per-buffer strip. Panes are
//! never shared across tabs, but all live in the single global pool
//! (`EngineView::panes`) regardless of which tab is active. Only which
//! panes a tab's `LayoutTree` *reaches* differs.
//! **Never mutate `ids`/`stash`/`order`/`current` directly outside this file**:
//! go through `TabStore`'s own methods, mirroring `BufferStore`'s own
//! module-doc convention.

use slotmap::{SecondaryMap, SlotMap, new_key_type};

use hume_engine::pipeline::{FocusedLayout, LayoutTree, PaneId};

new_key_type! {
    /// Opaque handle to a tab. Minted only in `hume-editor`: the engine has
    /// no notion of tabs, only of the one `LayoutTree` currently active.
    /// `pub(crate)`, wider than the rest of this module: `crate::tabline`
    /// (a sibling of `editor`, matching where `crate::statusline` lives;
    /// see that module's own doc) holds and displays `TabId`s built by
    /// `crate::editor::frame::sync_tabline_view`, so the type must be
    /// visible on both sides of that boundary.
    pub(crate) struct TabId;
}

/// Display/cycle order for every open tab, and the layout of each one not
/// on screen. The active tab's layout lives in `EngineView` instead, so
/// every layout exists once.
///
/// A switch moves layouts rather than copying them: the caller installs the
/// incoming layout in `EngineView` (an existing tab's comes out of here
/// through [`Self::take`]), which hands back the outgoing one, then finishes
/// with [`Self::switch`], [`Self::open_after_current`] or
/// [`Self::close_current`]. Between `take` and that call, the taken tab has
/// a layout in neither place.
pub(in crate::editor) struct TabStore {
    order: Vec<TabId>,
    current: TabId,
    ids: SlotMap<TabId, ()>,
    stash: SecondaryMap<TabId, FocusedLayout>,
}

impl TabStore {
    /// A single-tab store whose one tab is the layout already live in
    /// `EngineView`.
    pub(in crate::editor) fn new() -> Self {
        let mut ids = SlotMap::with_key();
        let id = ids.insert(());
        Self {
            order: vec![id],
            current: id,
            ids,
            stash: SecondaryMap::new(),
        }
    }

    pub(in crate::editor) fn current(&self) -> TabId {
        self.current
    }

    /// Display/cycle order: every tab, including the active one.
    #[cfg(test)]
    pub(in crate::editor) fn order(&self) -> &[TabId] {
        &self.order
    }

    pub(in crate::editor) fn len(&self) -> usize {
        self.order.len()
    }

    /// Every tab in display order with its focused pane, reading the active
    /// tab's from `live_focus` (`EngineView::focused_pane`).
    pub(in crate::editor) fn focused_panes(
        &self,
        live_focus: PaneId,
    ) -> impl Iterator<Item = (TabId, PaneId)> {
        self.order.iter().map(move |&id| {
            let pid = if id == self.current {
                live_focus
            } else {
                self.stash[id].focused()
            };
            (id, pid)
        })
    }

    /// Every inactive tab's `LayoutTree`.
    pub(in crate::editor) fn inactive_layouts(&self) -> impl Iterator<Item = &LayoutTree> {
        self.stash.values().map(FocusedLayout::tree)
    }

    /// `current`'s index in `order`. Every method below that needs it reads
    /// this instead of taking a caller-supplied id (unlike, say,
    /// `BufferStore::next`, which takes one and so has a real "unknown id"
    /// case to handle with an `unwrap_or` fallback): `TabStore` reads its
    /// own `current` field, so a missing position would mean `order` and
    /// `current` had already gone out of sync: exactly the corruption
    /// `open_after_current`'s own insert below would otherwise mask instead
    /// of failing loudly.
    ///
    /// `pub(in crate::editor)` rather than private: `Editor::sync_tabline_view`
    /// needs the same index. It reads it from here rather than re-deriving it
    /// from `order()`/`current()` separately, which would let the tabline's
    /// own copy silently drift from this one's panic message the moment
    /// either changes.
    pub(in crate::editor) fn current_pos(&self) -> usize {
        self.order
            .iter()
            .position(|&t| t == self.current)
            .expect("current tab is always present in order")
    }

    /// Move inactive tab `id`'s layout out, for the caller to install in
    /// `EngineView` before finishing the switch (see the struct doc).
    pub(in crate::editor) fn take(&mut self, id: TabId) -> FocusedLayout {
        self.stash
            .remove(id)
            .expect("only an inactive tab is taken, and only once")
    }

    /// Stash `outgoing`, the layout `EngineView` displaced, as the current
    /// tab's, and make `target` current. `target`'s layout was taken first.
    pub(in crate::editor) fn switch(&mut self, outgoing: FocusedLayout, target: TabId) {
        self.stash_current(outgoing);
        self.current = target;
    }

    /// Stash `outgoing` as the current tab's (same as [`Self::switch`]),
    /// allocate a fresh id for the new tab whose layout `EngineView` now
    /// holds, insert it into `order` right after `current` (Vim's `:tabnew`
    /// placement), and make it current.
    pub(in crate::editor) fn open_after_current(&mut self, outgoing: FocusedLayout) -> TabId {
        let pos = self.current_pos() + 1;
        self.stash_current(outgoing);
        let id = self.ids.insert(());
        self.order.insert(pos, id);
        self.current = id;
        id
    }

    fn stash_current(&mut self, outgoing: FocusedLayout) {
        let replaced = self.stash.insert(self.current, outgoing);
        assert!(replaced.is_none(), "the active tab has no stashed layout");
    }

    /// The tab that should gain focus once `current` closes: its right
    /// neighbour, or its left one when `current` is already the rightmost
    /// tab (Vim's own `:tabclose` placement: adjacent, never a wrap to the
    /// far end). Only meaningful with more than one tab open; panics
    /// otherwise, same precondition [`Self::close_current`] asserts.
    pub(in crate::editor) fn adjacent(&self) -> TabId {
        assert!(self.order.len() > 1, "adjacent requires more than one tab");
        let pos = self.current_pos();
        let neighbor = if pos + 1 < self.order.len() {
            pos + 1
        } else {
            pos - 1
        };
        self.order[neighbor]
    }

    /// Remove `current` from the store and make `survivor`, whose layout
    /// was taken first, current. Panics if `current` is the only tab;
    /// callers check `len() > 1` first.
    pub(in crate::editor) fn close_current(&mut self, survivor: TabId) {
        assert!(
            self.order.len() > 1,
            "close_current requires more than one tab"
        );
        let closed = self.current;
        self.order.retain(|&t| t != closed);
        self.ids.remove(closed);
        self.current = survivor;
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
