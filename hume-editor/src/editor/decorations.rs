//! Steel-writable decoration stores: inlay hints, gutter signs, virtual
//! lines, end-of-line text, extra highlights, and line backgrounds. Not
//! LSP-specific (any plugin can set them) — LSP is their first client, not
//! their owner. Every
//! kind is keyed the same way — `BufferId` first, then a per-buffer
//! `Vec<(source, entries)>`, so unrelated plugins' entries for the same
//! buffer coexist without a cross-buffer scan to find them — see
//! `SourceStore`, generic over the source-key type so `lsp/diagnostics.rs`'s
//! `DiagnosticsStore` (keyed by `ServerId` instead of a plugin-chosen
//! `String`) shares this exact write/remap machinery rather than
//! reimplementing it. Most render providers read these fresh every frame;
//! `virtual_lines` is the exception — resolving each entry's scope is costly
//! enough that its per-pane sync gates on `generation` instead (see that
//! field's doc).

use std::ops::Range;

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

/// One `(set-signs! …)` entry: a gutter marker on the line `pos` starts.
/// `pos` is that line's line-start char offset, not the Steel-facing line
/// number — the host boundary (`host_impl.rs`'s `line_start_offset`)
/// converts at set time, so this remaps through edits with everything else;
/// the render side derives the current line back via `char_to_line` at
/// rebuild. `Clone`: `decoration_providers.rs`'s
/// `visible_line_anchored` clones a viewport-filtered subset out from under
/// an immutable store borrow before resolving each entry's scope (which
/// needs `&mut self`) — same reason `EolTextEntry`/`LineBgEntry` carry it.
#[derive(Clone)]
pub(crate) struct SignEntry {
    pub(crate) pos: usize,
    pub(crate) text: String,
    pub(crate) scope: String,
    pub(crate) priority: i64,
}

/// One `(set-virtual-lines! …)` entry: a synthetic line of text anchored to
/// the line `pos` starts (rendered after it, or before when `before` is
/// set). `pos` is that line's line-start char offset, not the Steel-facing
/// line number — the host boundary (`host_impl.rs`'s `line_start_offset`)
/// converts at set time, so this remaps through edits like every other kind;
/// the render side derives the current line back via `char_to_line` at
/// rebuild. `scope` styles bytes `segments` doesn't cover
/// (`ui.virtual` fallback when both are absent); `segments` are
/// `(byte_start, byte_end, scope_name)` ranges into `text`, already
/// sorted/non-overlapping/in-bounds — guaranteed by the host boundary
/// (`virtual_line_segments_to_bytes` in `host_impl.rs`), which also converts
/// the Steel-facing char offsets to these byte offsets. Kept as a separate
/// type rather than reusing `hume_scripting::VirtualLineSpec` directly: that
/// type's `segments` are unvalidated char offsets, this one's are validated
/// byte offsets — deliberately different shapes, not merely a field rename.
#[derive(Clone)]
pub(crate) struct VirtualLineEntry {
    pub(crate) pos: usize,
    pub(crate) text: String,
    pub(crate) before: bool,
    pub(crate) scope: Option<String>,
    pub(crate) segments: Vec<(usize, usize, String)>,
}

/// One `(set-eol-text! …)` entry: `text` appended at the end of the line
/// `pos` starts. `pos` is that line's line-start char offset, not the
/// Steel-facing line number — the host boundary (`host_impl.rs`'s
/// `line_start_offset`) converts at set time, so this remaps through edits
/// like every other kind; the render side derives the current line back via
/// `char_to_line` at rebuild. The diagnostics plugin's
/// per-line summary (`"[n] <message>"` or a bare message) is this kind's
/// first client, not its owner, same as every other kind here is to LSP.
/// `Clone`: `decoration_providers.rs`'s `visible_line_anchored` clones a
/// viewport-filtered subset out from under an immutable store borrow before
/// resolving each entry's scope (which needs `&mut self`).
#[derive(Clone)]
pub(crate) struct EolTextEntry {
    pub(crate) pos: usize,
    pub(crate) text: String,
    pub(crate) scope: String,
}

/// One `(set-extra-highlights! …)` entry: a char range styled with `scope`.
pub(crate) struct ExtraHighlightEntry {
    pub(crate) start: usize,
    pub(crate) end: usize,
    pub(crate) scope: String,
}

/// One `(set-line-backgrounds! …)` entry: a full-row background tint on the
/// line `pos` starts. `pos` is that line's line-start char offset, not the
/// Steel-facing line number — the host boundary (`host_impl.rs`'s
/// `line_start_offset`) converts at set time, so this remaps through edits
/// like every other line-anchored kind; the render side derives the current
/// line back via `char_to_line` at rebuild. No `priority` field
/// — unlike signs, row tints have no single-slot contention, so same-line
/// entries from different sources break ties by source name.
/// `Clone`: see `EolTextEntry`'s doc — same
/// `visible_line_anchored` consumer.
#[derive(Clone)]
pub(crate) struct LineBgEntry {
    pub(crate) pos: usize,
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
        self.pos
    }
}

impl Positioned for VirtualLineEntry {
    fn pos(&self) -> usize {
        self.pos
    }
}

impl Positioned for EolTextEntry {
    fn pos(&self) -> usize {
        self.pos
    }
}

impl Positioned for ExtraHighlightEntry {
    fn pos(&self) -> usize {
        self.start
    }
}

impl Positioned for LineBgEntry {
    fn pos(&self) -> usize {
        self.pos
    }
}

/// The five point-anchored kinds (every kind but `ExtraHighlightEntry`,
/// which remaps as a range instead via `RangeAnchored` — see
/// [`SourceStore::remap_ranges`]) — drives [`SourceStore::remap_points`]' batch
/// `ChangeSet::map_positions` call.
pub(crate) trait PointAnchored: Positioned {
    /// Sticky side for an edit landing exactly at this kind's position —
    /// see `ChangeSet::Assoc`'s doc and each impl below for the reasoning.
    const ASSOC: Assoc;
    fn set_pos(&mut self, pos: usize);
}

/// `Assoc::Before`: an insertion at the hint's char stays glued to the char
/// it annotates, sticking to what was already there rather than swallowing
/// newly typed text into "before the hint".
impl PointAnchored for InlayHintEntry {
    const ASSOC: Assoc = Assoc::Before;
    fn set_pos(&mut self, pos: usize) {
        self.pos = pos;
    }
}

/// `Assoc::After` for every line-anchored kind below: an
/// insertion containing a newline landing exactly at a line-start anchor
/// (`o` — open line above) must keep the decoration on the *original* line
/// content — `Assoc::Before` would strand it on the newly inserted blank
/// line instead.
impl PointAnchored for SignEntry {
    const ASSOC: Assoc = Assoc::After;
    fn set_pos(&mut self, pos: usize) {
        self.pos = pos;
    }
}

impl PointAnchored for VirtualLineEntry {
    const ASSOC: Assoc = Assoc::After;
    fn set_pos(&mut self, pos: usize) {
        self.pos = pos;
    }
}

impl PointAnchored for EolTextEntry {
    const ASSOC: Assoc = Assoc::After;
    fn set_pos(&mut self, pos: usize) {
        self.pos = pos;
    }
}

impl PointAnchored for LineBgEntry {
    const ASSOC: Assoc = Assoc::After;
    fn set_pos(&mut self, pos: usize) {
        self.pos = pos;
    }
}

/// The one kind remapped as a range rather than a point (see
/// `PointAnchored`'s doc) — drives [`SourceStore::remap_ranges`]' batch
/// `ChangeSet::map_ranges` call. `Positioned::pos()` supplies the range's
/// start; this supplies the end.
pub(crate) trait RangeAnchored: Positioned {
    fn end(&self) -> usize;
    fn set_range(&mut self, start: usize, end: usize);
}

impl RangeAnchored for ExtraHighlightEntry {
    fn end(&self) -> usize {
        self.end
    }
    fn set_range(&mut self, start: usize, end: usize) {
        self.start = start;
        self.end = end;
    }
}

/// One decoration kind's per-source entries, for every buffer
/// (`FxHashMap<BufferId, Vec<(source, Vec<T>)>>`, kept sorted ascending by
/// `source` — see `set`). Written once, instantiated per kind; the type
/// system carries the per-kind payload differences. Generic over the source
/// key `K` (not just `String`) so `lsp/diagnostics.rs`'s `DiagnosticsStore`
/// — keyed by `ServerId`, otherwise the exact same shape (per-buffer,
/// per-source, wholesale-replace, remap-through-a-`ChangeSet`) — can wrap
/// this instead of hand-rolling the same write/remap logic a second time.
pub(crate) struct SourceStore<K, T> {
    by_buffer: FxHashMap<BufferId, Vec<(K, Vec<T>)>>,
}

impl<K, T> Default for SourceStore<K, T> {
    fn default() -> Self {
        Self {
            by_buffer: FxHashMap::default(),
        }
    }
}

impl<K, T> SourceStore<K, T> {
    /// Every source's entries for `bid`, grouped (not flattened) — the
    /// primitive `for_buffer` and a per-source-structure caller (e.g.
    /// `DiagnosticsStore::for_range`'s per-source `partition_point` prune)
    /// both build on.
    pub(crate) fn groups_for_buffer(&self, bid: BufferId) -> impl Iterator<Item = (&K, &[T])> {
        self.by_buffer
            .get(&bid)
            .into_iter()
            .flat_map(|entry| entry.iter().map(|(k, v)| (k, v.as_slice())))
    }

    /// All entries for `bid`, across every source in ascending source-name
    /// order (see `set`), paired with their source. Signs need the source
    /// for a deterministic priority tie-break; virtual lines and extra
    /// highlights (the two kinds with no per-line collapse) rely on this
    /// ascending order directly, so two sources anchored to the same line
    /// render in a name-deterministic order rather than whichever call
    /// `set-*!` happened to land first this session; every other kind's
    /// caller discards it.
    pub(crate) fn for_buffer(&self, bid: BufferId) -> impl Iterator<Item = (&K, &T)> {
        self.groups_for_buffer(bid)
            .flat_map(|(k, entries)| entries.iter().map(move |e| (k, e)))
    }

    #[cfg(test)]
    fn entries_for<Q>(&self, source: &Q, bid: BufferId) -> &[T]
    where
        K: std::borrow::Borrow<Q>,
        Q: PartialEq + ?Sized,
    {
        self.by_buffer
            .get(&bid)
            .and_then(|entry| entry.iter().find(|(s, _)| s.borrow() == source))
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

    /// Drops every entry for `bid`. Returns whether `bid` had an entry to
    /// drop — `DiagnosticsStore::remove_buffer` uses this to only bump its
    /// generation when the removal actually changed anything.
    pub(crate) fn remove_buffer(&mut self, bid: BufferId) -> bool {
        self.by_buffer.remove(&bid).is_some()
    }

    fn is_empty_for(&self, bid: BufferId) -> bool {
        !self
            .by_buffer
            .get(&bid)
            .is_some_and(|entry| entry.iter().any(|(_, v)| !v.is_empty()))
    }

    /// Every buffer with at least one source registered, of any kind —
    /// `DiagnosticsStore::buffers_with_diagnostics`'s sole caller.
    pub(crate) fn buffers(&self) -> impl Iterator<Item = BufferId> + '_ {
        self.by_buffer.keys().copied()
    }

    /// Drops every source `keep` rejects, across every buffer; a buffer left
    /// with zero sources is dropped from `by_buffer` entirely rather than
    /// kept as an empty `Vec`. Returns the buffers actually touched.
    /// `DiagnosticsStore::remove_server`'s sole caller — decoration kinds
    /// have no per-source removal (a source only ever replaces its own
    /// entries wholesale via `set`, never disappears on its own).
    pub(crate) fn retain_sources(&mut self, mut keep: impl FnMut(&K) -> bool) -> Vec<BufferId> {
        let mut touched = Vec::new();
        self.by_buffer.retain(|&bid, entry| {
            let before = entry.len();
            entry.retain(|(k, _)| keep(k));
            if entry.len() != before {
                touched.push(bid);
            }
            !entry.is_empty()
        });
        touched
    }
}

impl<K, T: Positioned> SourceStore<K, T> {
    /// Every source's entries for `bid` whose `pos` falls in `range`.
    ///
    /// Each source's slice is `pos`-sorted (`set`), so both bounds come from
    /// a binary search — a caller filtering a viewport out of a buffer-wide
    /// store pays for the entries it keeps, not for every entry the servers
    /// published.
    pub(crate) fn in_range(&self, bid: BufferId, range: Range<usize>) -> impl Iterator<Item = &T> {
        self.groups_for_buffer(bid).flat_map(move |(_source, es)| {
            let lo = es.partition_point(|e| e.pos() < range.start);
            let hi = es.partition_point(|e| e.pos() < range.end);
            es[lo..hi].iter()
        })
    }
}

impl<K: Ord, T: Positioned> SourceStore<K, T> {
    /// Replaces `source`'s entries for `bid` wholesale, sorted by `pos` (see
    /// `Positioned`'s doc). `slot` itself stays sorted ascending by `source`
    /// — a binary-search insert rather than find-or-push — so
    /// `for_buffer`'s iteration order is deterministic by construction
    /// instead of "whichever source called `set` first this session".
    pub(crate) fn set(&mut self, source: K, bid: BufferId, mut entries: Vec<T>) {
        entries.sort_by_key(Positioned::pos);
        let slot = self.by_buffer.entry(bid).or_default();
        match slot.binary_search_by(|(s, _)| s.cmp(&source)) {
            Ok(idx) => slot[idx].1 = entries,
            Err(idx) => slot.insert(idx, (source, entries)),
        }
    }
}

impl<K, T: PointAnchored> SourceStore<K, T> {
    /// Remaps every point-anchored entry for `bid` through `cs`, one batch
    /// `ChangeSet::map_positions` call per source, using `T::ASSOC`. Returns
    /// whether `bid` had any entry to remap — callers use this to skip a
    /// dirty-tracking stamp bump when `bid` had nothing for this kind.
    fn remap_points(&mut self, bid: BufferId, cs: &ChangeSet) -> bool {
        let mut touched = false;
        for entries in self.sources_mut(bid) {
            if entries.is_empty() {
                continue;
            }
            touched = true;
            let mut positions: Vec<usize> = entries.iter().map(Positioned::pos).collect();
            cs.map_positions(&mut positions, T::ASSOC);
            for (entry, pos) in entries.iter_mut().zip(positions) {
                entry.set_pos(pos);
            }
        }
        touched
    }
}

impl<K, T: RangeAnchored> SourceStore<K, T> {
    /// Remaps every range-anchored entry for `bid` through `cs`, dropping any
    /// range a covering deletion collapsed to zero width. Returns whether
    /// `bid` had any entry to remap, same as `remap_points`. The one other
    /// implementor of this policy before it moved here
    /// (`DiagnosticsStore::remap_through`) was a near-verbatim copy of this
    /// method against `StoredDiag` instead of `ExtraHighlightEntry` — this
    /// generic version now backs both.
    pub(crate) fn remap_ranges(&mut self, bid: BufferId, cs: &ChangeSet) -> bool {
        let mut touched = false;
        for spans in self.sources_mut(bid) {
            if spans.is_empty() {
                continue;
            }
            touched = true;
            let mut ranges: Vec<(usize, usize)> =
                spans.iter().map(|s| (s.pos(), s.end())).collect();
            cs.map_ranges(&mut ranges);
            debug_assert!(
                ranges.windows(2).all(|w| w[0].0 <= w[1].0),
                "map_ranges must preserve sort order"
            );
            let mut idx = 0;
            spans.retain_mut(|s| {
                let (start, end) = ranges[idx];
                idx += 1;
                if end <= start {
                    false // collapsed by a covering deletion — drop
                } else {
                    s.set_range(start, end);
                    true
                }
            });
        }
        touched
    }
}

#[derive(Default)]
pub(crate) struct DecorationStores {
    inlay_hints: SourceStore<String, InlayHintEntry>,
    signs: SourceStore<String, SignEntry>,
    virtual_lines: SourceStore<String, VirtualLineEntry>,
    extra_highlights: SourceStore<String, ExtraHighlightEntry>,
    eol_text: SourceStore<String, EolTextEntry>,
    line_backgrounds: SourceStore<String, LineBgEntry>,
    /// Per-buffer dirty-tracking stamp, touched by every `set_*` and
    /// `remove_buffer` for that buffer, and by `remap_through` when a remap
    /// actually moved one of that buffer's entries — the virtual-lines
    /// render write side mirrors `virtual_lines` into a per-pane Arc only
    /// when *its buffer's* stamp changed since its last sync, rather than
    /// every frame (unlike inlay hints/EOL text, that sync runs in
    /// scroll/cursor math too, not just render, so avoiding needless
    /// per-frame rebuild work matters more there).
    ///
    /// Per-buffer, not one store-wide counter: `remap_through` runs once per
    /// queued edit for *every* LSP-attached buffer, decorated or not
    /// (`record_lsp_edits`'s gate is `lsp_server.is_some() || has_any(bid)`),
    /// so a single global counter would bump on every keystroke in any
    /// LSP-attached buffer — the virtual-lines resync skip would never
    /// actually fire while typing. Per-buffer stamps mean typing in one
    /// buffer doesn't invalidate every pane on every other buffer.
    generation: FxHashMap<BufferId, u64>,
    /// Shared source for every buffer's stamp — see `touch`. Not itself a
    /// generation to compare against; `reset` carries it forward so a fresh
    /// store's stamps are guaranteed to never repeat a value any earlier
    /// store in this session ever handed out (see `reset`'s doc).
    clock: u64,
}

impl DecorationStores {
    /// A fresh, empty store — used by `ConfigState::new` for both session
    /// start (`prior_clock: 0`, nothing to carry forward) and
    /// `:reload-config`'s reset (the outgoing `ConfigState`'s own
    /// `decorations.clock()`).
    ///
    /// Carries `prior_clock` forward rather than starting over at `0`: with
    /// a per-buffer stamp map that resets to empty (every buffer defaulting
    /// back to stamp `0`) on every reload, a buffer that goes untouched
    /// across two-or-more reloads could otherwise see the *same* stamp
    /// sequence repeat (`0, 1, 2, …` again), which could coincidentally
    /// equal a pane's already-synced stamp in `Editor::virtual_lines_synced`
    /// left over from before the *first* reload — skipping the sync that
    /// should clear the pane's stale `Arc` of the old virtual lines. A
    /// single ever-increasing clock, carried forward across every reset,
    /// guarantees no stamp value is ever reused for the life of the
    /// session, so that ABA collision can't happen no matter how many
    /// reloads a pane sits out.
    pub(crate) fn reset(prior_clock: u64) -> Self {
        Self {
            clock: prior_clock.wrapping_add(1),
            ..Default::default()
        }
    }

    /// Stamps `bid` with a fresh value off the shared clock — the one
    /// mutation primitive every `set_*`, `remove_buffer`, and a touched
    /// `remap_through` funnel through, so `generation`/`clock` can never
    /// drift out of sync with each other.
    fn touch(&mut self, bid: BufferId) {
        self.clock = self.clock.wrapping_add(1);
        self.generation.insert(bid, self.clock);
    }

    /// The shared clock backing every buffer's stamp — `:reload-config`
    /// carries this forward into the next store via `reset`. Not a
    /// generation to compare a buffer's stamp against; see `generation`.
    pub(crate) fn clock(&self) -> u64 {
        self.clock
    }

    /// `bid`'s current stamp — `0` if `bid` has never been touched by this
    /// store (a fresh buffer, or one this store was reset since — see
    /// `reset`). Bumped by every `set_*` call and `remove_buffer` for `bid`,
    /// and by `remap_through` when a remap actually moved one of `bid`'s
    /// entries.
    pub(crate) fn generation(&self, bid: BufferId) -> u64 {
        self.generation.get(&bid).copied().unwrap_or(0)
    }

    /// Replaces `source`'s inlay hints for `bid` wholesale.
    pub(crate) fn set_inlay_hints(
        &mut self,
        source: String,
        bid: BufferId,
        hints: Vec<InlayHintEntry>,
    ) {
        self.inlay_hints.set(source, bid, hints);
        self.touch(bid);
    }

    /// All inlay hints for `bid`, across every source — assertion helper for
    /// tests that check what a server published rather than what a viewport
    /// shows. Production reads go through [`Self::inlay_hints_in_range`].
    #[cfg(test)]
    pub(crate) fn inlay_hints_for_buffer(
        &self,
        bid: BufferId,
    ) -> impl Iterator<Item = &InlayHintEntry> {
        self.inlay_hints.for_buffer(bid).map(|(_, e)| e)
    }

    /// Inlay hints for `bid` anchored inside `range` — the per-frame render
    /// bridge's view, which only ever wants the viewport's worth.
    pub(crate) fn inlay_hints_in_range(
        &self,
        bid: BufferId,
        range: Range<usize>,
    ) -> impl Iterator<Item = &InlayHintEntry> {
        self.inlay_hints.in_range(bid, range)
    }

    /// Replaces `source`'s signs for `bid` wholesale.
    pub(crate) fn set_signs(&mut self, source: String, bid: BufferId, signs: Vec<SignEntry>) {
        self.signs.set(source, bid, signs);
        self.touch(bid);
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
        self.signs.for_buffer(bid).map(|(s, e)| (s.as_str(), e))
    }

    /// Replaces `source`'s virtual lines for `bid` wholesale.
    pub(crate) fn set_virtual_lines(
        &mut self,
        source: String,
        bid: BufferId,
        lines: Vec<VirtualLineEntry>,
    ) {
        self.virtual_lines.set(source, bid, lines);
        self.touch(bid);
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

    /// Replaces `source`'s EOL text for `bid` wholesale.
    pub(crate) fn set_eol_text(
        &mut self,
        source: String,
        bid: BufferId,
        entries: Vec<EolTextEntry>,
    ) {
        self.eol_text.set(source, bid, entries);
        self.touch(bid);
    }

    /// All EOL text entries for `bid`, across every source, paired with
    /// their source name — the render write side needs the name for a
    /// deterministic tie-break when a remap collapses two sources' entries
    /// onto the same line (mirrors `signs_for_buffer`/`line_backgrounds_for_buffer`).
    pub(crate) fn eol_text_for_buffer(
        &self,
        bid: BufferId,
    ) -> impl Iterator<Item = (&str, &EolTextEntry)> {
        self.eol_text.for_buffer(bid).map(|(s, e)| (s.as_str(), e))
    }

    /// Replaces `source`'s line backgrounds for `bid` wholesale.
    pub(crate) fn set_line_backgrounds(
        &mut self,
        source: String,
        bid: BufferId,
        entries: Vec<LineBgEntry>,
    ) {
        self.line_backgrounds.set(source, bid, entries);
        self.touch(bid);
    }

    #[cfg(test)]
    pub(crate) fn line_backgrounds_for(&self, source: &str, bid: BufferId) -> &[LineBgEntry] {
        self.line_backgrounds.entries_for(source, bid)
    }

    /// All line-background entries for `bid`, across every source, paired
    /// with their source name — the render write side needs the name for a
    /// deterministic tie-break when two sources tint the same line.
    pub(crate) fn line_backgrounds_for_buffer(
        &self,
        bid: BufferId,
    ) -> impl Iterator<Item = (&str, &LineBgEntry)> {
        self.line_backgrounds
            .for_buffer(bid)
            .map(|(s, e)| (s.as_str(), e))
    }

    /// Replaces `source`'s extra highlights for `bid` wholesale.
    pub(crate) fn set_extra_highlights(
        &mut self,
        source: String,
        bid: BufferId,
        spans: Vec<ExtraHighlightEntry>,
    ) {
        self.extra_highlights.set(source, bid, spans);
        self.touch(bid);
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

    /// Whether `bid` has any decoration, of any kind, that needs to stay in
    /// sync with edits — every kind remaps through `remap_through` now.
    /// `record_lsp_edits` (`doc_ops.rs`) uses this to queue a buffer's edits
    /// for the remap chokepoint even with no attached LSP server —
    /// decorations are not LSP-owned, LSP is just their first client.
    ///
    /// Exhaustive destructuring, no `..`: a new decoration kind fails to
    /// compile here until it is listed, so a kind can never silently go
    /// unreported to `has_any`'s callers.
    pub(crate) fn has_any(&self, bid: BufferId) -> bool {
        let Self {
            inlay_hints,
            signs,
            virtual_lines,
            eol_text,
            line_backgrounds,
            extra_highlights,
            generation: _,
            clock: _,
        } = self;
        !inlay_hints.is_empty_for(bid)
            || !signs.is_empty_for(bid)
            || !virtual_lines.is_empty_for(bid)
            || !eol_text.is_empty_for(bid)
            || !line_backgrounds.is_empty_for(bid)
            || !extra_highlights.is_empty_for(bid)
    }

    /// Drops every entry for `bid`, across every source and every kind —
    /// called when the buffer is closed, or reloaded from disk while keeping
    /// the same `BufferId`. `BufferId` is a versioned slotmap key, so a
    /// future slot reuse can never alias with the closed buffer's stale
    /// entries — but a *reload* keeps the same key, so clearing
    /// `virtual_lines` without touching `bid`'s stamp would leave a pane's
    /// `virtual_lines_synced` entry looking still-current: it would keep
    /// mirroring the pre-reload virtual lines at now-meaningless line
    /// anchors. Touching unconditionally (not just when a kind had entries)
    /// forces every pane on `bid` to resync.
    ///
    /// Exhaustive destructuring, no `..`: a new decoration kind fails to
    /// compile here until it is listed, so closing or reloading a buffer can
    /// never leave a stale kind behind.
    pub(crate) fn remove_buffer(&mut self, bid: BufferId) {
        let Self {
            inlay_hints,
            signs,
            virtual_lines,
            eol_text,
            line_backgrounds,
            extra_highlights,
            generation: _,
            clock: _,
        } = self;
        inlay_hints.remove_buffer(bid);
        signs.remove_buffer(bid);
        virtual_lines.remove_buffer(bid);
        eol_text.remove_buffer(bid);
        line_backgrounds.remove_buffer(bid);
        extra_highlights.remove_buffer(bid);
        self.touch(bid);
    }

    /// Remaps `bid`'s decorations, of every kind, through `cs` — the same
    /// chokepoint as the diagnostics remap (`flush_lsp_pending_changes`), so
    /// decoration positions never drift out of sync with the diagnostics
    /// they're often paired with. Touches `bid`'s stamp only if some kind
    /// actually had an entry to remap: `record_lsp_edits` (`doc_ops.rs`)
    /// queues *every* edit in an LSP-attached buffer for this chokepoint,
    /// decorated or not, so touching unconditionally would stamp a
    /// zero-decoration buffer on every keystroke — defeating the
    /// virtual-lines pane sync's whole reason to check the stamp in the
    /// first place. A remap that *did* touch something still moved
    /// positions, so cached consumers (the virtual-lines pane sync) must
    /// resync even though nothing called a `set_*` method.
    ///
    /// Exhaustive destructuring, no `..`: a new decoration kind fails to
    /// compile here until it is listed, so a kind can never silently skip
    /// the remap chokepoint and drift out of sync with its buffer's edits.
    pub(crate) fn remap_through(&mut self, bid: BufferId, cs: &ChangeSet) {
        let Self {
            inlay_hints,
            signs,
            virtual_lines,
            eol_text,
            line_backgrounds,
            extra_highlights,
            generation: _,
            clock: _,
        } = self;
        // Every kind always attempts its remap — only whether to bump the
        // stamp is conditional. The array literal's eager evaluation (not
        // the iterator adapters below) is what guarantees none of these six
        // calls get short-circuited away.
        let touched = [
            inlay_hints.remap_points(bid, cs),
            signs.remap_points(bid, cs),
            virtual_lines.remap_points(bid, cs),
            eol_text.remap_points(bid, cs),
            line_backgrounds.remap_points(bid, cs),
            extra_highlights.remap_ranges(bid, cs),
        ]
        .into_iter()
        .any(|kind_touched| kind_touched);
        if touched {
            self.touch(bid);
        }
    }
}

#[cfg(test)]
mod tests;
