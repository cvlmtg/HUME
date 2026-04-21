//! Per-editor buffer store: mirrors engine `SlotMap<BufferId, SharedBuffer>`.
//!
//! `BufferStore` holds the authoritative `Buffer` structs keyed by `BufferId`.
//! IDs are allocated by the engine's `SlotMap<BufferId, SharedBuffer>`; this
//! store mirrors that slotmap. **Never insert/remove through only one side** —
//! always go through the `Editor::open_buffer` / `Editor::close_buffer` choke-points.

use std::path::Path;

use slotmap::SecondaryMap;

use engine::pipeline::BufferId;

use crate::editor::buffer::Buffer;

// ── BufferStore ───────────────────────────────────────────────────────────────

/// Mirrors the engine's `SlotMap<BufferId, SharedBuffer>` with the full
/// `Buffer` structs. Owns all per-buffer content, history, and file metadata.
pub(crate) struct BufferStore {
    /// The buffer content keyed by `BufferId`.
    buffers: SecondaryMap<BufferId, Buffer>,
    /// Open-order list. Used for `:bnext` / `:bprev` cycling.
    order: Vec<BufferId>,
    /// Most-recently-used list, tail = most recent.
    /// Length is always ≤ `order.len()`; entries are unique.
    mru: Vec<BufferId>,
}

impl BufferStore {
    pub(crate) fn new() -> Self {
        Self {
            buffers: SecondaryMap::new(),
            order: Vec::new(),
            mru: Vec::new(),
        }
    }

    /// Register a new buffer slot. Called from `Editor::open_buffer` after the
    /// engine slot is allocated.
    pub(crate) fn open(&mut self, id: BufferId, doc: Buffer) {
        self.buffers.insert(id, doc);
        self.order.push(id);
        self.touch_mru(id);
    }

    /// Find a buffer by its canonical resolved path.
    ///
    /// Returns the first `BufferId` whose `buffer.path == Some(canonical_path)`.
    /// Used by `:e` to deduplicate already-open files.
    pub(crate) fn find_by_path(&self, path: &Path) -> Option<BufferId> {
        self.buffers.iter().find_map(|(id, buf)| {
            buf.path.as_deref().filter(|p| *p == path).map(|_| id)
        })
    }

    /// Infallible getter. Panics if `id` was never seeded — that is a caller bug.
    pub(crate) fn get(&self, id: BufferId) -> &Buffer {
        self.buffers.get(id).expect("BufferStore: unseeded BufferId")
    }

    /// Infallible mutable getter.
    pub(crate) fn get_mut(&mut self, id: BufferId) -> &mut Buffer {
        self.buffers.get_mut(id).expect("BufferStore: unseeded BufferId")
    }

    /// Non-panicking getter — `None` for stale / unknown IDs.
    pub(crate) fn try_get(&self, id: BufferId) -> Option<&Buffer> {
        self.buffers.get(id)
    }

    /// Iterate all open buffers in open-order.  Yields `(BufferId, &Buffer)`.
    pub(crate) fn iter(&self) -> impl Iterator<Item = (BufferId, &Buffer)> {
        self.order.iter().filter_map(|&id| self.buffers.get(id).map(|buf| (id, buf)))
    }

    /// Remove `id` from the store.
    ///
    /// Returns the most-recently-used buffer excluding `id` (the recommended
    /// replacement target), or `None` if `id` was the only buffer.
    pub(crate) fn close(&mut self, id: BufferId) -> Option<BufferId> {
        let replacement = self.mru_excluding(id);
        self.buffers.remove(id);
        self.order.retain(|&x| x != id);
        self.mru.retain(|&x| x != id);
        replacement
    }

    /// Move `id` to the tail of the MRU list (call on every buffer switch).
    pub(crate) fn touch_mru(&mut self, id: BufferId) {
        self.mru.retain(|&x| x != id);
        self.mru.push(id);
    }

    /// The most-recently-used buffer that is not `id`.
    pub(crate) fn mru_excluding(&self, id: BufferId) -> Option<BufferId> {
        self.mru.iter().rev().find(|&&x| x != id).copied()
    }

    /// Next buffer in open-order (wraps around). Returns `id` if only one buffer.
    pub(crate) fn next(&self, current: BufferId) -> BufferId {
        let pos = self.order.iter().position(|&x| x == current).unwrap_or(0);
        let next = (pos + 1) % self.order.len().max(1);
        self.order.get(next).copied().unwrap_or(current)
    }

    /// Previous buffer in open-order (wraps around). Returns `id` if only one buffer.
    pub(crate) fn prev(&self, current: BufferId) -> BufferId {
        let pos = self.order.iter().position(|&x| x == current).unwrap_or(0);
        let prev = if pos == 0 { self.order.len().saturating_sub(1) } else { pos - 1 };
        self.order.get(prev).copied().unwrap_or(current)
    }

    #[cfg(test)]
    pub(crate) fn len(&self) -> usize {
        self.buffers.len()
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use crate::core::text::Text;
    use crate::core::selection::SelectionSet;
    use engine::pipeline::{BufferId, EngineView};
    use engine::theme::Theme;

    fn make_id(ev: &mut EngineView) -> BufferId {
        ev.buffers.insert(engine::pipeline::SharedBuffer::new())
    }

    fn make_buf() -> Buffer {
        Buffer::new(Text::from("hello\n"), SelectionSet::default())
    }

    fn store_with_engine() -> (BufferStore, EngineView) {
        let ev = EngineView::new(Theme::default());
        (BufferStore::new(), ev)
    }

    #[test]
    fn open_and_get() {
        let (mut store, mut ev) = store_with_engine();
        let id = make_id(&mut ev);
        store.open(id, make_buf());
        assert_eq!(store.get(id).text().to_string(), "hello\n");
        assert_eq!(store.len(), 1);
    }

    #[test]
    fn close_returns_mru_replacement() {
        let (mut store, mut ev) = store_with_engine();
        let a = make_id(&mut ev);
        let b = make_id(&mut ev);
        store.open(a, make_buf());
        store.open(b, make_buf());
        // b is MRU tail (most recent). closing b should suggest a.
        let replacement = store.close(b);
        assert_eq!(replacement, Some(a));
        assert_eq!(store.len(), 1);
    }

    #[test]
    fn close_last_returns_none() {
        let (mut store, mut ev) = store_with_engine();
        let a = make_id(&mut ev);
        store.open(a, make_buf());
        assert_eq!(store.close(a), None);
    }

    #[test]
    fn next_and_prev_wrap() {
        let (mut store, mut ev) = store_with_engine();
        let a = make_id(&mut ev);
        let b = make_id(&mut ev);
        let c = make_id(&mut ev);
        store.open(a, make_buf());
        store.open(b, make_buf());
        store.open(c, make_buf());
        assert_eq!(store.next(c), a, "next wraps to start");
        assert_eq!(store.prev(a), c, "prev wraps to end");
        assert_eq!(store.next(a), b);
        assert_eq!(store.prev(c), b);
    }

    #[test]
    fn find_by_path_dedup() {
        let (mut store, mut ev) = store_with_engine();
        let id = make_id(&mut ev);
        let mut buf = make_buf();
        buf.path = Some(Arc::new(std::path::PathBuf::from("/tmp/foo.txt")));
        store.open(id, buf);
        assert_eq!(store.find_by_path(Path::new("/tmp/foo.txt")), Some(id));
        assert_eq!(store.find_by_path(Path::new("/tmp/bar.txt")), None);
    }

    #[test]
    fn touch_mru_promotes_to_tail() {
        let (mut store, mut ev) = store_with_engine();
        let a = make_id(&mut ev);
        let b = make_id(&mut ev);
        store.open(a, make_buf());
        store.open(b, make_buf());
        // b is MRU tail. Touch a to make it most recent.
        store.touch_mru(a);
        assert_eq!(store.mru_excluding(a), Some(b));
    }
}
