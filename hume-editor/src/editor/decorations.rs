//! Steel-writable decoration stores: inlay hints, gutter signs, virtual
//! lines, inline diagnostics, and extra highlights. Not LSP-specific (any
//! plugin can set them) — LSP is their first client, not their owner. The
//! render providers read these fresh every frame; nothing here needs a
//! dirty-tracking generation counter.
//!
//! `inlay_hints` / `inline_diagnostics` are keyed by `BufferId` alone (one
//! owner per buffer, always replaced wholesale by the next
//! `(set-inlay-hints! …)` / `(set-inline-diagnostics! …)`); `signs` /
//! `virtual_lines` / `extra_highlights` are keyed by `BufferId` first, then a
//! per-buffer `Vec<(source, entries)>` so unrelated plugins' entries for the
//! same buffer coexist without a cross-buffer scan to find them (same shape
//! as `DiagnosticsStore::by_buffer`). Only the char-offset stores
//! (`inlay_hints`, `extra_highlights`) remap through edits — `signs` /
//! `virtual_lines` / `inline_diagnostics` are line-indexed and
//! encoding/edit-independent for v1.

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

/// One `(set-virtual-lines! …)` entry: a synthetic line of text rendered
/// after buffer `line` (0-indexed). `scope` styles the whole line
/// (`ui.virtual` fallback when absent) — inlay hints and this both
/// predate a segmented-styling API, so a whole-line scope is what the
/// landed store can express.
pub(crate) struct VirtualLineEntry {
    pub(crate) line: usize,
    pub(crate) text: String,
    pub(crate) scope: Option<String>,
}

/// One `(set-inline-diagnostics! …)` entry: text appended at the end of
/// buffer `line` (0-indexed) — the diagnostics plugin's per-line summary
/// (`"[n] <message>"` or a bare message). Keyed and rendered exactly like
/// `inlay_hints` (unconditional per-frame rebuild in
/// `update_inline_diagnostics_providers`, cheap enough that it doesn't need
/// the dirty-tracking generation gate `virtual_lines` uses).
pub(crate) struct InlineDiagnosticEntry {
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

#[derive(Default)]
pub(crate) struct DecorationStores {
    inlay_hints: FxHashMap<BufferId, Vec<InlayHintEntry>>,
    signs: FxHashMap<BufferId, Vec<(String, Vec<SignEntry>)>>,
    virtual_lines: FxHashMap<BufferId, Vec<(String, Vec<VirtualLineEntry>)>>,
    extra_highlights: FxHashMap<BufferId, Vec<(String, Vec<ExtraHighlightEntry>)>>,
    /// One owner per buffer (always replaced wholesale by the diagnostics
    /// plugin's next `(set-inline-diagnostics! …)`), same shape as
    /// `inlay_hints` — no per-source multiplexing needed since diagnostics
    /// are the only client.
    inline_diagnostics: FxHashMap<BufferId, Vec<InlineDiagnosticEntry>>,
    /// Bumped by `set_virtual_lines` — the render write side mirrors
    /// `virtual_lines` into a per-pane Arc only when this changed since its
    /// last sync, rather than every frame (unlike inlay hints, this runs in
    /// scroll/cursor math too, not just render, so avoiding needless
    /// per-frame rebuild work matters more here).
    virtual_lines_generation: u64,
}

impl DecorationStores {
    /// A fresh, empty store — used by `ConfigState::new` for both session
    /// start (`prior_generation: 0`, nothing to carry forward) and
    /// `:reload-config`'s reset (the outgoing `ConfigState`'s own
    /// `decorations.virtual_lines_generation()`).
    ///
    /// Bumps `prior_generation` rather than resetting to `0`: on a second
    /// (or later) reload, a plain reset-to-`0` could coincidentally equal a
    /// pane's already-synced counter in `Editor::virtual_lines_synced` (e.g.
    /// a pane that hasn't synced since the *first* reload also left it at
    /// `0`), which would skip the sync that clears the pane's stale `Arc` of
    /// the old virtual lines.
    pub(crate) fn reset(prior_generation: u64) -> Self {
        Self {
            virtual_lines_generation: prior_generation.wrapping_add(1),
            ..Default::default()
        }
    }

    /// Replaces `bid`'s inlay hints wholesale, sorted by `pos` (required for
    /// `remap_through`'s batch position mapping).
    pub(crate) fn set_inlay_hints(&mut self, bid: BufferId, mut hints: Vec<InlayHintEntry>) {
        hints.sort_by_key(|h| h.pos);
        self.inlay_hints.insert(bid, hints);
    }

    /// `bid`'s inlay hints, sorted by `pos` (see `set_inlay_hints`).
    pub(crate) fn inlay_hints_for(&self, bid: BufferId) -> &[InlayHintEntry] {
        self.inlay_hints.get(&bid).map(Vec::as_slice).unwrap_or(&[])
    }

    /// Replaces `bid`'s inline-diagnostic entries wholesale.
    pub(crate) fn set_inline_diagnostics(
        &mut self,
        bid: BufferId,
        entries: Vec<InlineDiagnosticEntry>,
    ) {
        self.inline_diagnostics.insert(bid, entries);
    }

    /// `bid`'s inline-diagnostic entries (see `set_inline_diagnostics`).
    pub(crate) fn inline_diagnostics_for(&self, bid: BufferId) -> &[InlineDiagnosticEntry] {
        self.inline_diagnostics
            .get(&bid)
            .map(Vec::as_slice)
            .unwrap_or(&[])
    }

    /// Replaces `source`'s signs for `bid` wholesale.
    pub(crate) fn set_signs(&mut self, source: String, bid: BufferId, signs: Vec<SignEntry>) {
        let entry = self.signs.entry(bid).or_default();
        match entry.iter_mut().find(|(s, _)| *s == source) {
            Some(slot) => slot.1 = signs,
            None => entry.push((source, signs)),
        }
    }

    #[cfg(test)]
    pub(crate) fn signs_for(&self, source: &str, bid: BufferId) -> &[SignEntry] {
        self.signs
            .get(&bid)
            .and_then(|entry| entry.iter().find(|(s, _)| s == source))
            .map(|(_, v)| v.as_slice())
            .unwrap_or(&[])
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
        self.signs
            .get(&bid)
            .into_iter()
            .flat_map(|entry| entry.iter())
            .flat_map(|(source, entries)| entries.iter().map(move |e| (source.as_str(), e)))
    }

    /// Replaces `source`'s virtual lines for `bid` wholesale.
    pub(crate) fn set_virtual_lines(
        &mut self,
        source: String,
        bid: BufferId,
        lines: Vec<VirtualLineEntry>,
    ) {
        let entry = self.virtual_lines.entry(bid).or_default();
        match entry.iter_mut().find(|(s, _)| *s == source) {
            Some(slot) => slot.1 = lines,
            None => entry.push((source, lines)),
        }
        self.virtual_lines_generation += 1;
    }

    /// Current generation — bumped by every `set_virtual_lines` call, across
    /// every source/buffer. One counter for simplicity: comparing it costs
    /// an unaffected pane one wasted equality check per frame, which is
    /// cheaper than tracking per-buffer generations for a store this small.
    pub(crate) fn virtual_lines_generation(&self) -> u64 {
        self.virtual_lines_generation
    }

    /// All virtual-line entries for `bid`, across every source — the render
    /// write side merges them all into one per-line bucket.
    pub(crate) fn virtual_lines_for_buffer(
        &self,
        bid: BufferId,
    ) -> impl Iterator<Item = &VirtualLineEntry> {
        self.virtual_lines
            .get(&bid)
            .into_iter()
            .flat_map(|entry| entry.iter())
            .flat_map(|(_source, entries)| entries.iter())
    }

    #[cfg(test)]
    pub(crate) fn virtual_lines_for(&self, source: &str, bid: BufferId) -> &[VirtualLineEntry] {
        self.virtual_lines
            .get(&bid)
            .and_then(|entry| entry.iter().find(|(s, _)| s == source))
            .map(|(_, v)| v.as_slice())
            .unwrap_or(&[])
    }

    /// Replaces `source`'s extra highlights for `bid` wholesale, sorted by
    /// `start` (required for `remap_through`'s batch range mapping).
    pub(crate) fn set_extra_highlights(
        &mut self,
        source: String,
        bid: BufferId,
        mut spans: Vec<ExtraHighlightEntry>,
    ) {
        spans.sort_by_key(|s| s.start);
        let entry = self.extra_highlights.entry(bid).or_default();
        match entry.iter_mut().find(|(s, _)| *s == source) {
            Some(slot) => slot.1 = spans,
            None => entry.push((source, spans)),
        }
    }

    #[cfg(test)]
    pub(crate) fn extra_highlights_for(
        &self,
        source: &str,
        bid: BufferId,
    ) -> &[ExtraHighlightEntry] {
        self.extra_highlights
            .get(&bid)
            .and_then(|entry| entry.iter().find(|(s, _)| s == source))
            .map(|(_, v)| v.as_slice())
            .unwrap_or(&[])
    }

    /// All extra-highlight entries for `bid`, across every source — the
    /// render write side merges them all into one highlight-tier bucket.
    pub(crate) fn extra_highlights_for_buffer(
        &self,
        bid: BufferId,
    ) -> impl Iterator<Item = &ExtraHighlightEntry> {
        self.extra_highlights
            .get(&bid)
            .into_iter()
            .flat_map(|entry| entry.iter())
            .flat_map(|(_source, spans)| spans.iter())
    }

    /// Whether `bid` has any char-offset decorations (inlay hints or extra
    /// highlights — the two stores `remap_through` actually touches) that
    /// need to stay in sync with edits. `record_lsp_edits` (`doc_ops.rs`)
    /// uses this to queue a buffer's edits for the remap chokepoint even
    /// with no attached LSP server — decorations are not LSP-owned, LSP is
    /// just their first client.
    pub(crate) fn has_any(&self, bid: BufferId) -> bool {
        self.inlay_hints.get(&bid).is_some_and(|v| !v.is_empty())
            || self
                .extra_highlights
                .get(&bid)
                .is_some_and(|entries| entries.iter().any(|(_, spans)| !spans.is_empty()))
    }

    /// Drops every entry for `bid`, across every source and every store —
    /// called when the buffer is closed. `BufferId` is a versioned slotmap
    /// key, so a future slot reuse can never alias with the closed buffer's
    /// stale entries; this is a memory-leak fix, not a correctness one.
    pub(crate) fn remove_buffer(&mut self, bid: BufferId) {
        self.inlay_hints.remove(&bid);
        self.signs.remove(&bid);
        self.virtual_lines.remove(&bid);
        self.extra_highlights.remove(&bid);
        self.inline_diagnostics.remove(&bid);
    }

    /// Remaps `bid`'s inlay hints and extra highlights through `cs` — the
    /// same chokepoint as the diagnostics remap
    /// (`flush_lsp_pending_changes`), so decoration positions never drift
    /// out of sync with the diagnostics they're often paired with.
    pub(crate) fn remap_through(&mut self, bid: BufferId, cs: &ChangeSet) {
        if let Some(hints) = self.inlay_hints.get_mut(&bid)
            && !hints.is_empty()
        {
            let mut positions: Vec<usize> = hints.iter().map(|h| h.pos).collect();
            cs.map_positions(&mut positions, Assoc::Before);
            for (hint, pos) in hints.iter_mut().zip(positions) {
                hint.pos = pos;
            }
        }

        let Some(entry) = self.extra_highlights.get_mut(&bid) else {
            return;
        };
        for (_source, spans) in entry.iter_mut() {
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
