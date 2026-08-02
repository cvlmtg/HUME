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
    /// Monotonic counter bumped once per user edit/undo/redo, in any open
    /// buffer — the `doc_ops` five-function chokepoint is the sole writer.
    /// Unlike `Buffer::text_gen` (per-buffer, bumped by system refreshes too —
    /// `set_view_content`, `reload_from_text`), this is deliberately global
    /// and edit-only: `PasteStamp` stamps it so a paste can tell "did
    /// anything change, anywhere" without caring which buffer, and a
    /// `:messages` refresh or `:e!` between a kill and a paste must not look
    /// like an edit. See `PasteStamp`'s doc for the read side.
    edit_seq: u64,
}

impl BufferStore {
    pub(crate) fn new() -> Self {
        Self {
            buffers: SecondaryMap::new(),
            order: Vec::new(),
            mru: Vec::new(),
            edit_seq: 0,
        }
    }

    /// Current edit sequence — see the field doc.
    pub(crate) fn edit_seq(&self) -> u64 {
        self.edit_seq
    }

    /// Advance the edit sequence by one. Called only from the `doc_ops`
    /// chokepoint, once per actual mutation (never on a no-op undo/redo at a
    /// history boundary, never on a read-only-refused edit).
    pub(crate) fn bump_edit_seq(&mut self) {
        self.edit_seq += 1;
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

    /// Clear every open buffer's setting overrides back to "inherit from
    /// global" — called by `:reload-config`'s reset so a `set-buffer-option!`
    /// from the previous `init.scm` (e.g. one fired from an `OnLanguageSet`
    /// hook) doesn't outlive the config that set it.
    pub(crate) fn clear_overrides_all(&mut self) {
        for buf in self.buffers.values_mut() {
            buf.overrides = crate::settings::BufferOverrides::default();
        }
    }

    /// Clear every open buffer's language identity and syntax attachment —
    /// called by `:reload-config`'s reset immediately before `state.config.languages`
    /// is replaced with a fresh `LanguageRegistry`. `reset_config_state` reads
    /// `language_explicit` on every buffer *before* calling this, so a
    /// `:set buffer language=`/`set-buffer-language!` assertion can be
    /// restored after the post-reload re-detect sweep rather than being
    /// silently overwritten by whatever plain detection finds.
    ///
    /// `Buffer.language` is a `LanguageId`, an index into that registry; left
    /// alone across the swap it would dangle (surviving only by the
    /// coincidence that `languages.scm` re-interns identical names in
    /// identical order). Clearing it also restores `set_buffer_language`'s
    /// `None -> Some` transition on the post-reload re-detect sweep, so
    /// `OnLanguageSet` and its downstream syntax/LSP wiring re-fire instead
    /// of hitting that function's unchanged-value early return.
    ///
    /// `Buffer.syntax` holds an `Arc<GrammarBundle>` from that same outgoing
    /// registry (via `Syntax::bundle`) and must go with it: normally only
    /// `setup_buffer_syntax` (reached through `set_buffer_language`) tears it
    /// down, but when a buffer's language doesn't re-detect after the reload
    /// (`None -> None`), `set_buffer_language`'s unchanged-value guard never
    /// runs `setup_buffer_syntax` at all — leaving the buffer highlighted
    /// from a grammar registry that no longer exists unless this clears it
    /// directly.
    pub(crate) fn clear_languages_all(&mut self) {
        for buf in self.buffers.values_mut() {
            buf.language = None;
            buf.language_explicit = false;
            buf.syntax = None;
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
