//! Per-editor buffer store: mirrors engine `SlotMap<BufferId, ()>`.
//!
//! `BufferStore` holds the authoritative `Buffer` structs keyed by `BufferId`.
//! IDs are allocated by the engine's `SlotMap<BufferId, ()>`; this
//! store mirrors that slotmap. **Never insert/remove through only one side**:
//! always go through the `Editor::open_buffer` / `Editor::close_buffer` choke-points.

use std::path::Path;

use slotmap::SecondaryMap;

use hume_engine::pipeline::BufferId;
use hume_platform::path::strip_unc_prefix_cow;

use crate::editor::buffer::Buffer;

/// Mirrors the engine's `SlotMap<BufferId, ()>` with the full
/// `Buffer` structs. Owns all per-buffer content, history, and file metadata.
pub(crate) struct BufferStore {
    /// The buffer content keyed by `BufferId`.
    buffers: SecondaryMap<BufferId, Buffer>,
    /// Open-order list. Used for `:bnext` / `:bprev` cycling.
    order: Vec<BufferId>,
    /// Most-recently-*focused* list, tail = most recently focused. Seeded at
    /// open (see `open`'s own doc for why) and otherwise promoted by the two
    /// focus chokepoints, [`crate::editor::focus::focus_pane`] and
    /// [`crate::editor::buffer::lifecycle::switch_pane_to_buffer`] (focused
    /// switch only), plus `cmd_goto_alternate_buffer`'s own explicit touches
    /// for a remote-pane dispatch; see that function's doc for why it can't
    /// rely on the two chokepoints alone.
    /// Always holds the same entries as `order`, just in a different order.
    mru: Vec<BufferId>,
    /// Monotonic counter bumped once per user edit/undo/redo, in any open
    /// buffer. The `doc_ops` five-function chokepoint is the sole writer.
    /// Unlike a buffer text's version (per-buffer, and moved by system refreshes too:
    /// `set_view_content`, `reload_from_text`), this is global
    /// and edit-only: `PasteStamp` stamps it so a paste can tell "did
    /// anything change, anywhere" without caring which buffer, and a
    /// `:messages` refresh or `:e!` between a kill and a paste must not look
    /// like an edit. See `PasteStamp`'s doc for the read side.
    edit_seq: u64,
}

impl BufferStore {
    pub(in crate::editor) fn new() -> Self {
        Self {
            buffers: SecondaryMap::new(),
            order: Vec::new(),
            mru: Vec::new(),
            edit_seq: 0,
        }
    }

    /// Current edit sequence; see the field doc.
    pub(in crate::editor) fn edit_seq(&self) -> u64 {
        self.edit_seq
    }

    /// Advance the edit sequence by one. Called only from the `doc_ops`
    /// chokepoint, once per actual mutation (never on a no-op undo/redo at a
    /// history boundary, never on a read-only-refused edit).
    pub(in crate::editor) fn bump_edit_seq(&mut self) {
        self.edit_seq += 1;
    }

    /// Register a new buffer slot.
    ///
    /// Seeds `mru` too, ahead of any focus it may never receive: a buffer
    /// opened in a background pane and never focused still needs a valid
    /// `close_buffer` replacement target (`mru_excluding`), and an absent
    /// entry there would wrongly fall into the "last buffer" scratch-replace
    /// branch instead. Seeded at the *head*, not via `touch_mru` (which
    /// would put it at the tail): the tail is "most recently viewed," and a
    /// background open (a workspace edit opening files it never shows, a
    /// plugin priming a buffer) has never been viewed at all. `second_most_
    /// recent`/`mru_excluding` (`Ctrl-6`, `:b#`, `close`'s replacement
    /// target) would otherwise treat it as the most-recent "other" buffer,
    /// ahead of whatever the user actually last looked at.
    pub(in crate::editor) fn open(&mut self, id: BufferId, doc: Buffer) {
        self.buffers.insert(id, doc);
        self.order.push(id);
        self.mru.insert(0, id);
    }

    /// Find a buffer by its canonical resolved path.
    ///
    /// Returns the first `BufferId` whose `buffer.path()` matches `path` once
    /// both sides are stripped of a Windows `\\?\` verbatim prefix (a no-op
    /// off Windows). Most callers reach here via `fs::canonicalize`
    /// (`\\?\C:\…` on Windows) and match as-is, but the `:b <name>` fallback
    /// for a deleted backing file uses `std::path::absolute` (no prefix),
    /// which would otherwise dedup-miss against an already-open buffer.
    pub(in crate::editor) fn find_by_path(&self, path: &Path) -> Option<BufferId> {
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
    pub(in crate::editor) fn find_by_label(&self, label: &str) -> Option<BufferId> {
        self.buffers
            .iter()
            .find_map(|(id, buf)| buf.label.as_deref().filter(|l| *l == label).map(|_| id))
    }

    /// Infallible getter. Panics if `id` was never seeded: that is a caller bug.
    pub(crate) fn get(&self, id: BufferId) -> &Buffer {
        self.buffers
            .get(id)
            .expect("BufferStore: unseeded BufferId")
    }

    /// Infallible mutable getter.
    pub(in crate::editor) fn get_mut(&mut self, id: BufferId) -> &mut Buffer {
        self.buffers
            .get_mut(id)
            .expect("BufferStore: unseeded BufferId")
    }

    /// Non-panicking getter: `None` for stale / unknown IDs.
    pub(in crate::editor) fn try_get(&self, id: BufferId) -> Option<&Buffer> {
        self.buffers.get(id)
    }

    /// Non-panicking mutable getter: `None` for stale / unknown IDs.
    pub(in crate::editor) fn try_get_mut(&mut self, id: BufferId) -> Option<&mut Buffer> {
        self.buffers.get_mut(id)
    }

    /// Iterate all open buffers in open-order.  Yields `(BufferId, &Buffer)`.
    pub(in crate::editor) fn iter(&self) -> impl Iterator<Item = (BufferId, &Buffer)> {
        self.order
            .iter()
            .filter_map(|&id| self.buffers.get(id).map(|buf| (id, buf)))
    }

    /// Every open buffer whose text version has moved since the last call:
    /// the observation-point source for `on-text-changed`
    /// (`EditorEvent::OnTextChanged`'s doc has the full contract: what bumps
    /// text version, what coalesces, what never fires). Advances each touched
    /// buffer's `announced_version` to match as it goes, so a buffer
    /// reported once stays quiet until it mutates again, so a burst of edits
    /// between two calls coalesces into one entry. Walks `order` (open-order)
    /// for deterministic event ordering.
    ///
    /// Unlike `edit_seq` (global and edit-only by design: a `:messages`
    /// refresh or `:e!` must not look like an edit to paste-stamping, see its
    /// doc), this is per-buffer and fires for every text replacement
    /// `install` performs.
    pub(in crate::editor) fn take_text_changed(&mut self) -> Vec<BufferId> {
        let Self { order, buffers, .. } = self;
        order
            .iter()
            .filter_map(|&id| {
                let buf = buffers.get_mut(id)?;
                if buf.text().version() == buf.announced_version {
                    return None;
                }
                buf.announced_version = buf.text().version();
                Some(id)
            })
            .collect()
    }

    /// Apply the `undo-levels` cap to every open buffer's history.
    ///
    /// There is no per-buffer scope for this setting, so every buffer
    /// always tracks the same cap.
    pub(in crate::editor) fn set_undo_levels_all(&mut self, levels: usize) {
        for buf in self.buffers.values_mut() {
            buf.set_undo_levels(levels);
        }
    }

    /// Clear every open buffer's setting overrides back to "inherit from
    /// global", called by `:reload-config`'s reset so a `set-buffer-option!`
    /// from the previous `init.scm` (e.g. one fired from an `OnLanguageSet`
    /// hook) doesn't outlive the config that set it.
    pub(in crate::editor) fn clear_overrides_all(&mut self) {
        for buf in self.buffers.values_mut() {
            buf.overrides = crate::editor::settings::BufferOverrides::default();
        }
    }

    /// Clear every open buffer's language identity and syntax attachment,
    /// called by `:reload-config`'s reset immediately before `state.config.languages`
    /// is replaced with a fresh `LanguageRegistry`. `reset_config_state` reads
    /// `language_explicit` on every buffer *before* calling this, so a
    /// `:set buffer language=`/`set-buffer-option!`'s `"language"` assertion can be
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
    /// runs `setup_buffer_syntax` at all, leaving the buffer highlighted
    /// from a grammar registry that no longer exists unless this clears it
    /// directly.
    pub(in crate::editor) fn clear_languages_all(&mut self) {
        for buf in self.buffers.values_mut() {
            buf.language = None;
            buf.language_explicit = false;
            buf.syntax = None;
        }
    }

    /// Remove `id` from the store.
    pub(in crate::editor) fn close(&mut self, id: BufferId) {
        self.buffers.remove(id);
        self.order.retain(|&x| x != id);
        self.mru.retain(|&x| x != id);
    }

    /// Move `id` to the tail of the MRU list: "most recently viewed."
    pub(in crate::editor) fn touch_mru(&mut self, id: BufferId) {
        self.mru.retain(|&x| x != id);
        self.mru.push(id);
    }

    /// The most-recently-used buffer that is not `id`.
    pub(in crate::editor) fn mru_excluding(&self, id: BufferId) -> Option<BufferId> {
        self.mru.iter().rev().find(|&&x| x != id).copied()
    }

    /// The buffer just before the most-recently-viewed one: `Ctrl-6`'s own
    /// "alternate buffer," one single global history shared by every pane
    /// rather than a per-pane notion: `mru`'s tail is always "whatever was
    /// last viewed, however it was viewed" (a keypress on the focused pane,
    /// or a `goto-alternate-buffer` touch from any other), so the entry
    /// right before it is always "the previous one," regardless of which
    /// pane is asking. `None` when fewer than two buffers are open (`mru`
    /// is seeded at open, so this needs no buffer to have actually been
    /// *viewed*; see `open`'s own doc). Distinct from `mru_excluding`: that
    /// skips *by value*, useful
    /// when the caller already knows which specific buffer to exclude; this
    /// is a pure positional read, since here the "current" buffer is
    /// whatever the tail *happens to be*
    /// right now, not a value some caller already has in hand.
    pub(in crate::editor) fn second_most_recent(&self) -> Option<BufferId> {
        self.mru.iter().rev().nth(1).copied()
    }

    /// Next buffer in open-order (wraps around). Returns `id` if only one buffer.
    pub(in crate::editor) fn next(&self, current: BufferId) -> BufferId {
        let pos = self.order.iter().position(|&x| x == current).unwrap_or(0);
        let next = (pos + 1) % self.order.len().max(1);
        self.order.get(next).copied().unwrap_or(current)
    }

    /// Previous buffer in open-order (wraps around). Returns `id` if only one buffer.
    pub(in crate::editor) fn prev(&self, current: BufferId) -> BufferId {
        let pos = self.order.iter().position(|&x| x == current).unwrap_or(0);
        let prev = if pos == 0 {
            self.order.len().saturating_sub(1)
        } else {
            pos - 1
        };
        self.order.get(prev).copied().unwrap_or(current)
    }

    #[cfg(test)]
    pub(in crate::editor) fn len(&self) -> usize {
        self.buffers.len()
    }
}

#[cfg(test)]
mod tests;
