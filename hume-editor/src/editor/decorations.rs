//! Steel-writable decoration stores: inlay hints, gutter signs, virtual
//! lines, end-of-line text, and extra highlights. Not LSP-specific (any
//! plugin can set them) — LSP is their first client, not their owner. Every
//! kind is keyed the same way — `BufferId` first, then a per-buffer
//! `Vec<(source, entries)>`, so unrelated plugins' entries for the same
//! buffer coexist without a cross-buffer scan to find them (same shape as
//! `DiagnosticsStore::by_buffer`) — see `SourceStore`. Most render providers
//! read these fresh every frame; `virtual_lines` is the exception — resolving
//! each entry's scope is costly enough that its per-pane sync gates on
//! `generation` instead (see that field's doc).

use rustc_hash::FxHashMap;

use hume_editing::changeset::{Assoc, ChangeSet};
use hume_engine::pipeline::BufferId;

/// One `(set-inlay-hints! …)` entry: `text` rendered `before` or after the
/// char at `pos`.
pub(crate) struct InlayHintEntry {
    pub(crate) pos: usize,
    pub(crate) text: String,
    pub(crate) before: bool,
}

/// One `(set-signs! …)` entry: a gutter marker on `line` (0-indexed).
pub(crate) struct SignEntry {
    pub(crate) line: usize,
    pub(crate) text: String,
    pub(crate) scope: String,
    pub(crate) priority: i64,
}

/// One `(set-virtual-lines! …)` entry: a synthetic line of text anchored to
/// buffer `line` (0-indexed) — rendered after it, or before when `before` is
/// set. `scope` styles bytes `segments` doesn't cover (`ui.virtual` fallback
/// when both are absent); `segments` are `(byte_start, byte_end, scope_name)`
/// ranges into `text`, already sorted/non-overlapping/in-bounds — guaranteed
/// by the host boundary (`virtual_line_segments_to_bytes` in `host_impl.rs`),
/// which also converts the Steel-facing char offsets to these byte offsets.
/// Kept as a separate type rather than reusing `hume_scripting::VirtualLineSpec`
/// directly: that type's `segments` are unvalidated char offsets, this one's
/// are validated byte offsets — deliberately different shapes, not merely a
/// field rename.
#[derive(Clone)]
pub(crate) struct VirtualLineEntry {
    pub(crate) line: usize,
    pub(crate) text: String,
    pub(crate) before: bool,
    pub(crate) scope: Option<String>,
    pub(crate) segments: Vec<(usize, usize, String)>,
}

/// One `(set-eol-text! …)` entry: `text` appended at the end of buffer
/// `line` (0-indexed) — the diagnostics plugin's per-line summary (`"[n]
/// <message>"` or a bare message) is its first client, not its owner, same
/// as every other kind here is to LSP. Was `InlineDiagnosticEntry` /
/// `set-inline-diagnostics!` — renamed because it was always "text appended
/// at end of line", never diagnostics-specific.
pub(crate) struct EolTextEntry {
    pub(crate) line: usize,
    pub(crate) text: String,
    pub(crate) scope: String,
}

/// One `(set-extra-highlights! …)` entry: a char range styled with `scope`.
pub(crate) struct ExtraHighlightEntry {
    pub(crate) start: usize,
    pub(crate) end: usize,
    pub(crate) scope: String,
}

/// Sort key every entry kind provides — [`SourceStore::set`] sorts by this
/// so the remap chokepoint's batch position/range mapping
/// (`ChangeSet::map_positions`/`map_ranges`) can rely on ascending input,
/// its documented precondition.
pub(crate) trait Positioned {
    fn pos(&self) -> usize;
}

impl Positioned for InlayHintEntry {
    fn pos(&self) -> usize {
        self.pos
    }
}

impl Positioned for SignEntry {
    fn pos(&self) -> usize {
        self.line
    }
}

impl Positioned for VirtualLineEntry {
    fn pos(&self) -> usize {
        self.line
    }
}

impl Positioned for EolTextEntry {
    fn pos(&self) -> usize {
        self.line
    }
}

impl Positioned for ExtraHighlightEntry {
    fn pos(&self) -> usize {
        self.start
    }
}

/// One decoration kind's per-source entries, for every buffer — the shape
/// every one of `DecorationStores`' five fields used to hand-roll separately
/// (`FxHashMap<BufferId, Vec<(source, Vec<T>)>>`, find-or-push
/// replace-wholesale-by-source). Written once, instantiated per kind; the
/// type system carries the per-kind payload differences.
pub(crate) struct SourceStore<T> {
    by_buffer: FxHashMap<BufferId, Vec<(String, Vec<T>)>>,
}

impl<T> Default for SourceStore<T> {
    fn default() -> Self {
        Self {
            by_buffer: FxHashMap::default(),
        }
    }
}

impl<T> SourceStore<T> {
    /// All entries for `bid`, across every source, paired with their source
    /// name — signs need the name for a deterministic priority tie-break;
    /// every other kind's caller discards it.
    fn for_buffer(&self, bid: BufferId) -> impl Iterator<Item = (&str, &T)> {
        self.by_buffer
            .get(&bid)
            .into_iter()
            .flat_map(|entry| entry.iter())
            .flat_map(|(source, entries)| entries.iter().map(move |e| (source.as_str(), e)))
    }

    #[cfg(test)]
    fn entries_for(&self, source: &str, bid: BufferId) -> &[T] {
        self.by_buffer
            .get(&bid)
            .and_then(|entry| entry.iter().find(|(s, _)| s == source))
            .map(|(_, v)| v.as_slice())
            .unwrap_or(&[])
    }

    /// Every source's entries for `bid`, mutably — the remap chokepoint's
    /// entry point: it rewrites positions in place per source, one batch
    /// call per source rather than per entry.
    fn sources_mut(&mut self, bid: BufferId) -> impl Iterator<Item = &mut Vec<T>> {
        self.by_buffer
            .get_mut(&bid)
            .into_iter()
            .flat_map(|entry| entry.iter_mut().map(|(_, v)| v))
    }

    fn remove_buffer(&mut self, bid: BufferId) {
        self.by_buffer.remove(&bid);
    }

    fn is_empty_for(&self, bid: BufferId) -> bool {
        !self
            .by_buffer
            .get(&bid)
            .is_some_and(|entry| entry.iter().any(|(_, v)| !v.is_empty()))
    }
}

impl<T: Positioned> SourceStore<T> {
    /// Replaces `source`'s entries for `bid` wholesale, sorted by `pos` (see
    /// `Positioned`'s doc).
    fn set(&mut self, source: String, bid: BufferId, mut entries: Vec<T>) {
        entries.sort_by_key(Positioned::pos);
        let slot = self.by_buffer.entry(bid).or_default();
        match slot.iter_mut().find(|(s, _)| *s == source) {
            Some(existing) => existing.1 = entries,
            None => slot.push((source, entries)),
        }
    }
}

#[derive(Default)]
pub(crate) struct DecorationStores {
    inlay_hints: SourceStore<InlayHintEntry>,
    signs: SourceStore<SignEntry>,
    virtual_lines: SourceStore<VirtualLineEntry>,
    extra_highlights: SourceStore<ExtraHighlightEntry>,
    eol_text: SourceStore<EolTextEntry>,
    /// Bumped by every `set_*` and by `remove_buffer`, across every kind —
    /// the virtual-lines render write side mirrors `virtual_lines` into a
    /// per-pane Arc only when this changed since its last sync, rather than
    /// every frame (unlike inlay hints/EOL text, that sync runs in
    /// scroll/cursor math too, not just render, so avoiding needless
    /// per-frame rebuild work matters more there). One store-wide counter
    /// rather than one per kind: writes are event-driven and rare relative
    /// to frames, so a spurious cross-kind resync costs ~nothing next to a
    /// second counter to keep in sync.
    generation: u64,
}

impl DecorationStores {
    /// A fresh, empty store — used by `ConfigState::new` for both session
    /// start (`prior_generation: 0`, nothing to carry forward) and
    /// `:reload-config`'s reset (the outgoing `ConfigState`'s own
    /// `decorations.generation()`).
    ///
    /// Bumps `prior_generation` rather than resetting to `0`: on a second
    /// (or later) reload, a plain reset-to-`0` could coincidentally equal a
    /// pane's already-synced counter in `Editor::virtual_lines_synced` (e.g.
    /// a pane that hasn't synced since the *first* reload also left it at
    /// `0`), which would skip the sync that clears the pane's stale `Arc` of
    /// the old virtual lines.
    pub(crate) fn reset(prior_generation: u64) -> Self {
        Self {
            generation: prior_generation.wrapping_add(1),
            ..Default::default()
        }
    }

    /// Replaces `source`'s inlay hints for `bid` wholesale.
    pub(crate) fn set_inlay_hints(
        &mut self,
        source: String,
        bid: BufferId,
        hints: Vec<InlayHintEntry>,
    ) {
        self.inlay_hints.set(source, bid, hints);
        self.generation += 1;
    }

    /// All inlay hints for `bid`, across every source.
    pub(crate) fn inlay_hints_for_buffer(
        &self,
        bid: BufferId,
    ) -> impl Iterator<Item = &InlayHintEntry> {
        self.inlay_hints.for_buffer(bid).map(|(_, e)| e)
    }

    /// Replaces `source`'s EOL text for `bid` wholesale.
    pub(crate) fn set_eol_text(
        &mut self,
        source: String,
        bid: BufferId,
        entries: Vec<EolTextEntry>,
    ) {
        self.eol_text.set(source, bid, entries);
        self.generation += 1;
    }

    /// All EOL text entries for `bid`, across every source.
    pub(crate) fn eol_text_for_buffer(&self, bid: BufferId) -> impl Iterator<Item = &EolTextEntry> {
        self.eol_text.for_buffer(bid).map(|(_, e)| e)
    }

    /// Replaces `source`'s signs for `bid` wholesale.
    pub(crate) fn set_signs(&mut self, source: String, bid: BufferId, signs: Vec<SignEntry>) {
        self.signs.set(source, bid, signs);
        self.generation += 1;
    }

    #[cfg(test)]
    pub(crate) fn signs_for(&self, source: &str, bid: BufferId) -> &[SignEntry] {
        self.signs.entries_for(source, bid)
    }

    /// All signs for `bid`, across every source, paired with their source
    /// name — the render write side merges them into one per-line winner.
    /// The source name is exposed so ties (two sources, same line, same
    /// priority) can be broken deterministically rather than by `FxHashMap`
    /// iteration order.
    pub(crate) fn signs_for_buffer(
        &self,
        bid: BufferId,
    ) -> impl Iterator<Item = (&str, &SignEntry)> {
        self.signs.for_buffer(bid)
    }

    /// Replaces `source`'s virtual lines for `bid` wholesale.
    pub(crate) fn set_virtual_lines(
        &mut self,
        source: String,
        bid: BufferId,
        lines: Vec<VirtualLineEntry>,
    ) {
        self.virtual_lines.set(source, bid, lines);
        self.generation += 1;
    }

    /// Current generation — bumped by every `set_*` call and `remove_buffer`,
    /// across every kind.
    pub(crate) fn generation(&self) -> u64 {
        self.generation
    }

    /// All virtual-line entries for `bid`, across every source — the render
    /// write side merges them all into one per-line bucket.
    pub(crate) fn virtual_lines_for_buffer(
        &self,
        bid: BufferId,
    ) -> impl Iterator<Item = &VirtualLineEntry> {
        self.virtual_lines.for_buffer(bid).map(|(_, e)| e)
    }

    #[cfg(test)]
    pub(crate) fn virtual_lines_for(&self, source: &str, bid: BufferId) -> &[VirtualLineEntry] {
        self.virtual_lines.entries_for(source, bid)
    }

    /// Replaces `source`'s extra highlights for `bid` wholesale.
    pub(crate) fn set_extra_highlights(
        &mut self,
        source: String,
        bid: BufferId,
        spans: Vec<ExtraHighlightEntry>,
    ) {
        self.extra_highlights.set(source, bid, spans);
        self.generation += 1;
    }

    #[cfg(test)]
    pub(crate) fn extra_highlights_for(
        &self,
        source: &str,
        bid: BufferId,
    ) -> &[ExtraHighlightEntry] {
        self.extra_highlights.entries_for(source, bid)
    }

    /// All extra-highlight entries for `bid`, across every source — the
    /// render write side merges them all into one highlight-tier bucket.
    pub(crate) fn extra_highlights_for_buffer(
        &self,
        bid: BufferId,
    ) -> impl Iterator<Item = &ExtraHighlightEntry> {
        self.extra_highlights.for_buffer(bid).map(|(_, e)| e)
    }

    /// Whether `bid` has any char-offset decorations (inlay hints or extra
    /// highlights — the two kinds `remap_through` actually touches) that
    /// need to stay in sync with edits. `record_lsp_edits` (`doc_ops.rs`)
    /// uses this to queue a buffer's edits for the remap chokepoint even
    /// with no attached LSP server — decorations are not LSP-owned, LSP is
    /// just their first client.
    pub(crate) fn has_any(&self, bid: BufferId) -> bool {
        !self.inlay_hints.is_empty_for(bid) || !self.extra_highlights.is_empty_for(bid)
    }

    /// Drops every entry for `bid`, across every source and every kind —
    /// called when the buffer is closed, or reloaded from disk while keeping
    /// the same `BufferId`. `BufferId` is a versioned slotmap key, so a
    /// future slot reuse can never alias with the closed buffer's stale
    /// entries — but a *reload* keeps the same key, so clearing
    /// `virtual_lines` without bumping `generation` would leave a pane's
    /// `virtual_lines_synced` entry looking still-current: it would keep
    /// mirroring the pre-reload virtual lines at now-meaningless line
    /// anchors. The bump forces every pane on `bid` to resync.
    pub(crate) fn remove_buffer(&mut self, bid: BufferId) {
        self.inlay_hints.remove_buffer(bid);
        self.signs.remove_buffer(bid);
        self.virtual_lines.remove_buffer(bid);
        self.extra_highlights.remove_buffer(bid);
        self.eol_text.remove_buffer(bid);
        self.generation += 1;
    }

    /// Remaps `bid`'s inlay hints and extra highlights through `cs` — the
    /// same chokepoint as the diagnostics remap
    /// (`flush_lsp_pending_changes`), so decoration positions never drift
    /// out of sync with the diagnostics they're often paired with.
    pub(crate) fn remap_through(&mut self, bid: BufferId, cs: &ChangeSet) {
        for hints in self.inlay_hints.sources_mut(bid) {
            if hints.is_empty() {
                continue;
            }
            let mut positions: Vec<usize> = hints.iter().map(|h| h.pos).collect();
            cs.map_positions(&mut positions, Assoc::Before);
            for (hint, pos) in hints.iter_mut().zip(positions) {
                hint.pos = pos;
            }
        }

        for spans in self.extra_highlights.sources_mut(bid) {
            if spans.is_empty() {
                continue;
            }
            let mut ranges: Vec<(usize, usize)> = spans.iter().map(|s| (s.start, s.end)).collect();
            cs.map_ranges(&mut ranges);
            let mut idx = 0;
            spans.retain_mut(|s| {
                let (start, end) = ranges[idx];
                idx += 1;
                if end <= start {
                    false // collapsed by a covering deletion — drop
                } else {
                    s.start = start;
                    s.end = end;
                    true
                }
            });
        }
    }
}

#[cfg(test)]
mod tests;
