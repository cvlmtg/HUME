//! Per-editor buffer store: mirrors engine `SlotMap<BufferId, ()>`.
//!
//! `BufferStore` holds the authoritative `Buffer` structs keyed by `BufferId`.
//! IDs are allocated by the engine's `SlotMap<BufferId, ()>`; this
//! store mirrors that slotmap. **Never insert/remove through only one side** —
//! always go through the `Editor::open_buffer` / `Editor::close_buffer` choke-points.

use std::path::Path;

use slotmap::SecondaryMap;

use hume_engine::pipeline::BufferId;
use hume_platform::path::strip_unc_prefix_cow;

use crate::editor::buffer::Buffer;

// ── BufferStore ───────────────────────────────────────────────────────────────

/// Mirrors the engine's `SlotMap<BufferId, ()>` with the full
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
    /// Returns the first `BufferId` whose `buffer.path()` matches `path` once
    /// both sides are stripped of a Windows `\\?\` verbatim prefix (a no-op
    /// off Windows) — most callers reach here via `fs::canonicalize`
    /// (`\\?\C:\…` on Windows) and match as-is, but the `:b <name>` fallback
    /// for a deleted backing file uses `std::path::absolute` (no prefix),
    /// which would otherwise dedup-miss against an already-open buffer.
    /// Used by `:e` to deduplicate already-open files.
    pub(crate) fn find_by_path(&self, path: &Path) -> Option<BufferId> {
        let needle = strip_unc_prefix_cow(path);
        self.buffers.iter().find_map(|(id, buf)| {
            buf.path()
                .filter(|p| strip_unc_prefix_cow(p) == needle)
                .map(|_| id)
        })
    }

    /// Find a read-only view buffer by its label (e.g. `"[messages]"`).
    ///
    /// Returns the first `BufferId` whose `buffer.label == Some(label)`.
    /// Used by `open_read_only_view` to reuse existing view buffers instead
    /// of accumulating duplicates.
    pub(crate) fn find_by_label(&self, label: &str) -> Option<BufferId> {
        self.buffers
            .iter()
            .find_map(|(id, buf)| buf.label.as_deref().filter(|l| *l == label).map(|_| id))
    }

    /// Infallible getter. Panics if `id` was never seeded — that is a caller bug.
    pub(crate) fn get(&self, id: BufferId) -> &Buffer {
        self.buffers
            .get(id)
            .expect("BufferStore: unseeded BufferId")
    }

    /// Infallible mutable getter.
    pub(crate) fn get_mut(&mut self, id: BufferId) -> &mut Buffer {
        self.buffers
            .get_mut(id)
            .expect("BufferStore: unseeded BufferId")
    }

    /// Non-panicking getter — `None` for stale / unknown IDs.
    pub(crate) fn try_get(&self, id: BufferId) -> Option<&Buffer> {
        self.buffers.get(id)
    }

    /// Non-panicking mutable getter — `None` for stale / unknown IDs.
    pub(crate) fn try_get_mut(&mut self, id: BufferId) -> Option<&mut Buffer> {
        self.buffers.get_mut(id)
    }

    /// Iterate all open buffers in open-order.  Yields `(BufferId, &Buffer)`.
    pub(crate) fn iter(&self) -> impl Iterator<Item = (BufferId, &Buffer)> {
        self.order
            .iter()
            .filter_map(|&id| self.buffers.get(id).map(|buf| (id, buf)))
    }

    /// Apply the `undo-levels` cap to every open buffer's history.
    ///
    /// Called from the `:set global` side-effect path and from the
    /// post-init.scm settings pickup — there is no per-buffer scope for
    /// this setting, so every buffer always tracks the same cap.
    pub(crate) fn set_undo_levels_all(&mut self, levels: usize) {
        for buf in self.buffers.values_mut() {
            buf.set_undo_levels(levels);
        }
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
        let prev = if pos == 0 {
            self.order.len().saturating_sub(1)
        } else {
            pos - 1
        };
        self.order.get(prev).copied().unwrap_or(current)
    }

    #[cfg(test)]
    pub(crate) fn len(&self) -> usize {
        self.buffers.len()
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests;
