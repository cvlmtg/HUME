//! B5's Steel-writable decoration stores: inlay hints, gutter signs, virtual
//! lines, and extra highlights. Not LSP-specific (any plugin can set them) —
//! LSP is their first client, not their owner, matching the hub's UI
//! decisions for Step 3's render providers (which read these fresh every
//! frame; nothing here needs a dirty-tracking generation counter).
//!
//! `inlay_hints` is keyed by `BufferId` alone (one owner per buffer, always
//! replaced wholesale by the next `(set-inlay-hints! …)`); `signs` /
//! `virtual_lines` / `extra_highlights` are keyed by `(source, BufferId)` so
//! unrelated plugins' entries for the same buffer coexist. Only the
//! char-offset stores (`inlay_hints`, `extra_highlights`) remap through
//! edits — `signs` / `virtual_lines` are line-indexed and, per the card,
//! encoding/edit-independent for v1.

use std::collections::HashMap;

use hume_editing::changeset::{Assoc, ChangeSet};
use hume_engine::pipeline::BufferId;

/// One `(set-inlay-hints! …)` entry: `text` rendered `before` or after the
/// char at `pos`.
pub(crate) struct InlayHintEntry {
    pub(crate) pos: usize,
    pub(crate) text: String,
    pub(crate) before: bool,
}

/// One `(set-signs! …)` entry: a gutter marker on `line` (0-indexed). No
/// reader until Step 3's sign-column provider (U2).
#[allow(dead_code)]
pub(crate) struct SignEntry {
    pub(crate) line: usize,
    pub(crate) text: String,
    pub(crate) scope: String,
    pub(crate) priority: i64,
}

/// One `(set-virtual-lines! …)` entry: a synthetic line of text rendered
/// after buffer `line` (0-indexed). No reader until Step 3's virtual-line
/// provider (U8).
#[allow(dead_code)]
pub(crate) struct VirtualLineEntry {
    pub(crate) line: usize,
    pub(crate) text: String,
}

/// One `(set-extra-highlights! …)` entry: a char range styled with `scope`.
pub(crate) struct ExtraHighlightEntry {
    pub(crate) start: usize,
    pub(crate) end: usize,
    pub(crate) scope: String,
}

#[derive(Default)]
pub(crate) struct DecorationStores {
    inlay_hints: HashMap<BufferId, Vec<InlayHintEntry>>,
    signs: HashMap<(String, BufferId), Vec<SignEntry>>,
    virtual_lines: HashMap<(String, BufferId), Vec<VirtualLineEntry>>,
    extra_highlights: HashMap<(String, BufferId), Vec<ExtraHighlightEntry>>,
}

impl DecorationStores {
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

    /// Replaces `source`'s signs for `bid` wholesale.
    pub(crate) fn set_signs(&mut self, source: String, bid: BufferId, signs: Vec<SignEntry>) {
        self.signs.insert((source, bid), signs);
    }

    #[cfg(test)]
    pub(crate) fn signs_for(&self, source: &str, bid: BufferId) -> &[SignEntry] {
        self.signs
            .get(&(source.to_string(), bid))
            .map(Vec::as_slice)
            .unwrap_or(&[])
    }

    /// All signs for `bid`, across every source, paired with their source
    /// name — the render write side merges them into one per-line winner.
    /// The source name is exposed so ties (two sources, same line, same
    /// priority) can be broken deterministically rather than by `HashMap`
    /// iteration order.
    pub(crate) fn signs_for_buffer(
        &self,
        bid: BufferId,
    ) -> impl Iterator<Item = (&str, &SignEntry)> {
        self.signs
            .iter()
            .filter(move |((_, entry_bid), _)| *entry_bid == bid)
            .flat_map(|((source, _), entries)| entries.iter().map(move |e| (source.as_str(), e)))
    }

    /// Replaces `source`'s virtual lines for `bid` wholesale.
    pub(crate) fn set_virtual_lines(
        &mut self,
        source: String,
        bid: BufferId,
        lines: Vec<VirtualLineEntry>,
    ) {
        self.virtual_lines.insert((source, bid), lines);
    }

    #[cfg(test)]
    pub(crate) fn virtual_lines_for(&self, source: &str, bid: BufferId) -> &[VirtualLineEntry] {
        self.virtual_lines
            .get(&(source.to_string(), bid))
            .map(Vec::as_slice)
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
        self.extra_highlights.insert((source, bid), spans);
    }

    #[cfg(test)]
    pub(crate) fn extra_highlights_for(
        &self,
        source: &str,
        bid: BufferId,
    ) -> &[ExtraHighlightEntry] {
        self.extra_highlights
            .get(&(source.to_string(), bid))
            .map(Vec::as_slice)
            .unwrap_or(&[])
    }

    /// All extra-highlight entries for `bid`, across every source — the
    /// render write side merges them all into one highlight-tier bucket.
    pub(crate) fn extra_highlights_for_buffer(
        &self,
        bid: BufferId,
    ) -> impl Iterator<Item = &ExtraHighlightEntry> {
        self.extra_highlights
            .iter()
            .filter(move |((_, entry_bid), _)| *entry_bid == bid)
            .flat_map(|(_, spans)| spans.iter())
    }

    /// Remaps `bid`'s inlay hints and extra highlights through `cs` — the
    /// same chokepoint as C9's diagnostics remap
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

        for ((_source, entry_bid), spans) in self.extra_highlights.iter_mut() {
            if *entry_bid != bid || spans.is_empty() {
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
