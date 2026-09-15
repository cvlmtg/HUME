//! The single authority on the document's display-line list.
//!
//! A document's display lines come from two independent sources: a buffer
//! line's own content display lines (one per wrap display line — exactly
//! one when wrapping is off), and virtual display lines contributed by
//! [`DecorationSource`](crate::providers::DecorationSource) providers,
//! anchored `Before` or `After` a line. Rendering, scrolling, cursor
//! placement, mouse mapping and visual movement all need the same
//! flattened view of those two sources, and any two implementations of that
//! view which disagree by a single display line produce a cursor that
//! draws in the wrong place or a viewport that scrolls past content.
//!
//! [`DisplayLineMap`] is that one implementation. It bundles everything the
//! display-line list depends on — rope, resolved wrap mode, tab width,
//! whitespace config, providers, content width — so consumers hold one
//! `&mut DisplayLineMap` instead of threading eight-to-eleven parameters
//! through every walk. What it learns about each line it visits goes in
//! the pane's own [`line_store::PaneLineStore`], which every walk of that
//! pane shares so none repeats another's work.
//!
//! Addresses are [`DisplayLinePos`]: a buffer line plus a slot index into
//! that line's *visual block*, which runs `before`-virtuals, then
//! content/wrap display lines, then `after`-virtuals. `Viewport`'s
//! `top_line`/`top_slot` pair is the persisted form of exactly that
//! address.

use std::ops::Range;

use ropey::Rope;

use crate::format::{FormatBound, LineFormat, format_buffer_line};
use crate::providers::{Decoration, DecorationKinds, InlineInsert, ProviderSet, VirtualLineAnchor};
use crate::types::{DisplayLine, Grapheme, ScopeId};
use hume_rope::column::DisplayLineCol;
use hume_rope::line::ContentLine;

pub mod line_store;

use line_store::{FormatKey, PaneLineStore};

mod locate;
mod pos;
mod render;
pub mod scroll;
mod stepping;

pub use pos::{BlockBreakdown, BlockSlot, DisplayColTarget, DisplayLinePos};
pub use scroll::carry;

// ---------------------------------------------------------------------------
// Display-line map
// ---------------------------------------------------------------------------

/// Everything the render stage needs to style and compose one display line.
pub struct RenderDisplayLine<'m> {
    pub display_line: &'m DisplayLine,
    /// The graphemes `display_line.graphemes` indexes into.
    pub graphemes: &'m [Grapheme],
    /// Buffer-line text that the display line's real graphemes index by byte
    /// range. Empty for virtual display lines, which have no buffer text.
    pub line_text: &'m str,
    /// Arena backing `Whitespace`/`Placeholder`/`Virtual` cell text.
    pub virtual_texts: &'m str,
    /// The display line's own background scope (`VirtualLine::base_scope`)
    /// — `None` for content display lines, which get their background from
    /// `Decoration::LineBg`/cursorline instead (`pane_render.rs`'s
    /// `LineStyle::tint`).
    pub base_scope: Option<ScopeId>,
}

/// The single authority on the document's display-line list. See the module doc.
pub struct DisplayLineMap<'a> {
    rope: &'a Rope,
    /// Everything this map's formats depend on besides the line's own text —
    /// wrap mode (always resolved against `content_width`: `WrapMode::wrap_width`
    /// panics on the `width: 0` sentinel, and [`DisplayLineMap::new`] is the one
    /// funnel every consumer passes through, so it resolves there rather
    /// than trusting callers to), tab width, whitespace config, and the
    /// buffer's identity/generation — and, verbatim, this map's store-scope
    /// key. One field rather than four: see [`FormatKey`]'s own doc for why
    /// the scroll pass and the render pass sharing a store depends on it.
    key: FormatKey,
    providers: &'a ProviderSet,
    content_width: u16,
    h_window: Option<Range<DisplayLineCol>>,
    /// Everything this map knows about the lines it has visited — the
    /// pane's own store, so every other walk of that pane this frame shares
    /// what this one formats. See [`line_store`]'s module doc.
    store: &'a mut PaneLineStore,
    /// Inline inserts for the line currently being formatted. Reused across
    /// the lines one map visits.
    inline_inserts: Vec<InlineInsert>,
    /// Scratch for one `DecorationSource::decorations_for_line` call at a
    /// time — drained into `virtual_lines`/`inline_inserts` immediately
    /// after, so this stays empty between calls. Reused across providers and
    /// lines to avoid a per-call allocation.
    decorations: Vec<Decoration>,
}

impl<'a> DisplayLineMap<'a> {
    pub fn new(
        rope: &'a Rope,
        providers: &'a ProviderSet,
        content_width: u16,
        key: FormatKey,
        store: &'a mut PaneLineStore,
    ) -> Self {
        debug_assert!(
            hume_rope::lines::ends_with_newline(rope),
            "DisplayLineMap requires a trailing '\\n' (the buffer invariant) — \
             without it `last_line`'s content-line derivation drops the \
             rope's actual last content line"
        );
        debug_assert!(
            content_width >= 1,
            "DisplayLineMap requires content_width >= 1 — a 0 here leaves \
             WrapMode::resolve's width:0 sentinel unresolved, and \
             wrap_width() then panics far from this call site. Callers pass \
             pane_width.max(1) (see Pane::content_width)."
        );
        let key = key.resolve(content_width);
        store.scope(key);
        Self {
            rope,
            key,
            providers,
            content_width,
            h_window: None,
            store,
            inline_inserts: Vec::new(),
            decorations: Vec::new(),
        }
    }

    /// The format of a named entry.
    ///
    /// Every format read goes through here on an index its caller was handed
    /// by [`DisplayLineMap::ensure_formatted`], so "the caller ensured this line" is
    /// carried by a value rather than by the two calls happening in order.
    fn format_at(&self, idx: usize) -> &LineFormat {
        &self.store.entry(idx).format
    }

    /// Clip `WrapMode::None` formatting to a horizontal column window — the
    /// render path's bound on arbitrarily long unwrapped lines.
    ///
    /// Display-line counts are unaffected (no-wrap is one content display
    /// line however wide the line is), so this changes only which graphemes
    /// the render accessors emit. Editor-side consumers want whole lines and
    /// leave it `None`.
    ///
    /// Does not re-scope the store: `h_window` is not part of [`FormatKey`],
    /// only recorded on the [`LineFormat`] a later `ensure_format_at` produces,
    /// so an entry's block shape and virtual display lines survive this call
    /// and only its format is subject to being recut. That is also what keeps the
    /// frame's two passes from sharing a *format* in `WrapMode::None`, where
    /// only the render pass clips — they still share the block shape.
    pub fn with_h_window(mut self, h_window: Option<Range<DisplayLineCol>>) -> Self {
        debug_assert!(
            h_window.is_none() || !self.key.wrap_mode.is_wrapping(),
            "with_h_window is a WrapMode::None-only clip — a wrapping DisplayLineMap \
             would silently under-count content display lines, since ensure_format_at \
             passes h_window through to the formatter even while wrapping"
        );
        self.h_window = h_window;
        self
    }

    pub fn is_wrapping(&self) -> bool {
        self.key.wrap_mode.is_wrapping()
    }

    /// The wrap column display lines are actually laid out against — `None`
    /// for `WrapMode::None`, otherwise the *resolved* width (the mode's own
    /// explicit width, or `content_width` when the mode used the `0`
    /// sentinel). Distinct from [`DisplayLineMap::content_width`]: an explicit wrap
    /// width doesn't move when the pane resizes, so a resize-driven
    /// staleness check (a `DisplayLine`-relative sticky column surviving a
    /// wrap-width change) must compare this, not the raw content width, or
    /// it invalidates latches a resize never actually affected.
    pub fn resolved_wrap_width(&self) -> Option<u16> {
        self.key.wrap_mode.wrap_width()
    }

    /// Width available for content — the same `content_width` the caller
    /// passed to [`DisplayLineMap::new`] (gutter already subtracted). The one column
    /// bound `locate`'s columns are relative to, so a caller sizing anything
    /// against display columns (horizontal scroll) reads it here rather than
    /// re-deriving it from the pane and risking the two drifting apart.
    pub fn content_width(&self) -> u16 {
        self.content_width
    }

    // ── Block shape ──────────────────────────────────────────────────────

    /// The display-line breakdown of `line`'s visual block.
    pub fn block(&mut self, line: ContentLine) -> BlockBreakdown {
        let idx = self.block_entry(line);
        self.breakdown(idx)
    }

    /// The breakdown of an entry already in hand.
    ///
    /// Split out so [`DisplayLineMap::resolve`] can compute a slot from the same
    /// breakdown it needs to walk, without re-finding the entry `block`
    /// already holds.
    fn breakdown(&mut self, idx: usize) -> BlockBreakdown {
        let content = self.content_display_lines(idx);
        let entry = self.store.entry(idx);
        BlockBreakdown {
            before: entry.before,
            content,
            after: entry.after(),
        }
    }

    /// The store entry for `line`, building its block shape if this is the
    /// first time this store has seen it.
    ///
    /// Only the *shape* — the format arrives separately, from whoever first
    /// needs the line's display lines. Under `WrapMode::None` that may be
    /// much later, or never.
    fn block_entry(&mut self, line: ContentLine) -> usize {
        if let Some(idx) = self.store.find(line) {
            return idx;
        }

        let idx = self.store.insert(line);
        // `insert`'s `rebind` already cleared this entry's `virtual_lines`,
        // keeping its allocation — taken out as scratch rather than building
        // a separate `Vec` and overwriting it on return, which would throw
        // that allocation away. Taken rather than borrowed because the
        // provider intake below needs `&mut self` for `self.decorations`,
        // which rules out holding a borrow of the store across it; put back
        // once the intake is done.
        let mut virtual_lines = std::mem::take(&mut self.store.entry_mut(idx).virtual_lines);
        self.decorations.clear();
        for (id, provider) in self
            .providers
            .decoration_sources(DecorationKinds::VIRTUAL_LINE)
        {
            let start = virtual_lines.len();
            provider.decorations_for_line(line, &mut self.decorations);
            // A provider that declared VIRTUAL_LINE but emitted something
            // else is a provider bug — ignored, not a panic.
            for d in self.decorations.drain(..) {
                if let Decoration::VirtualLine(vl) = d {
                    virtual_lines.push(vl);
                }
            }
            // Never trust a provider's self-reported id: it could name another
            // provider's display lines, which the gutter would then attribute wrongly.
            for vl in &mut virtual_lines[start..] {
                vl.provider_id = id;
            }
        }
        // A display line anchored outside the queried line is a provider
        // bug. Drop it rather than count it against a line it does not
        // belong to.
        virtual_lines.retain(|vl| match vl.anchor {
            VirtualLineAnchor::Before(n) | VirtualLineAnchor::After(n) => n == line,
        });
        // `Before` display lines ahead of `After` display lines; stable, so
        // provider registration order survives within each group.
        virtual_lines.sort_by_key(|vl| vl.anchor.sort_key());
        // Providers are plugin code and the trait makes no ordering promise
        // enforceable at the boundary — sort here so `segment_virtual_line`'s
        // cursor scan (which requires sorted, non-overlapping input) never
        // has to trust it, same posture as `rebuild_line_decorations` takes
        // for highlight spans.
        for vl in &mut virtual_lines {
            vl.segments.sort_by_key(|(start, _, _)| *start);
        }

        let before = virtual_lines
            .iter()
            .filter(|vl| matches!(vl.anchor, VirtualLineAnchor::Before(_)))
            .count();

        let entry = self.store.entry_mut(idx);
        entry.virtual_lines = virtual_lines;
        entry.before = before;
        idx
    }

    /// How many content display lines `line`'s block occupies.
    ///
    /// `WrapMode::None` is always exactly one, and formatting cannot return
    /// another answer there — so counting never runs the formatter. That is
    /// the difference between O(1) and O(line length) per query on a minified
    /// line megabytes wide. Under a wrapping mode the count *is* the
    /// formatter's output, so the line gets formatted here if it wasn't
    /// already.
    fn content_display_lines(&mut self, idx: usize) -> usize {
        if !self.key.wrap_mode.is_wrapping() {
            return 1;
        }
        // `Full`: the display-line count is the output, so nothing may be clipped.
        self.ensure_format_at(idx, FormatBound::Full);
        let entry = self.store.entry(idx);
        let content = entry.format.display_lines.len();
        debug_assert!(
            content >= 1,
            "line {} counted zero content display lines; every line occupies at least one",
            entry.line.index()
        );
        content
    }

    /// Index of the last buffer line a cursor can occupy.
    pub fn last_line(&self) -> ContentLine {
        hume_rope::lines::last_content_line(self.rope)
    }

    /// The content line a char offset resolves to, clamping the buffer's own
    /// trailing phantom line down to [`DisplayLineMap::last_line`] — reachable when
    /// `char_offset == len_chars()` (the debug_assert in every caller below
    /// admits it), and there is no display line to address on a line that
    /// doesn't exist. Same posture as `hume_rope::lines::place_char_column`
    /// on a phantom `line` argument: land on the last real line instead of
    /// an address no render pass can lay out.
    fn content_line_of(&self, ropey_line: hume_rope::line::RopeyLine) -> ContentLine {
        ropey_line
            .to_content(self.rope)
            .unwrap_or_else(|| self.last_line())
    }

    /// Pull `pos` into the document: `line` into `0..=last_line()`, then
    /// `slot` into that line's block.
    pub fn clamp(&mut self, pos: DisplayLinePos) -> DisplayLinePos {
        let line = pos.line.min(self.last_line());
        let total = self.block(line).total();
        DisplayLinePos::new(line, pos.slot.min(total.saturating_sub(1)))
    }

    /// Which block slot `pos` addresses.
    pub fn slot(&mut self, pos: DisplayLinePos) -> BlockSlot {
        self.resolve(pos).1
    }

    /// The entry index and block slot `pos` addresses, in one walk — for a
    /// caller (`render_display_line`) that needs both without resolving the line's
    /// block twice.
    fn resolve(&mut self, pos: DisplayLinePos) -> (usize, BlockSlot) {
        let idx = self.block_entry(pos.line);
        let b = self.breakdown(idx);
        debug_assert!(
            pos.slot < b.total(),
            "slot {} is past line {}'s block of {}",
            pos.slot,
            pos.line.index(),
            b.total()
        );
        let slot = if pos.slot < b.before {
            BlockSlot::Before(pos.slot)
        } else if pos.slot < b.before + b.content {
            BlockSlot::Content(pos.slot - b.before)
        } else {
            BlockSlot::After(pos.slot - b.before - b.content)
        };
        (idx, slot)
    }

    // ── Formatting ───────────────────────────────────────────────────────

    /// Guarantee `line`'s entry holds its content display lines, formatted
    /// at least as far as `bound` reaches. Returns the entry it resolved,
    /// so a read accessor can be handed the line by value.
    fn ensure_formatted(&mut self, line: ContentLine, bound: FormatBound) -> usize {
        let idx = self.block_entry(line);
        self.ensure_format_at(idx, bound);
        idx
    }

    /// [`DisplayLineMap::ensure_formatted`] for an entry already in hand.
    ///
    /// Split out so [`DisplayLineMap::content_display_lines`] can format while counting
    /// without re-finding the entry it is already holding.
    fn ensure_format_at(&mut self, idx: usize, bound: FormatBound) {
        debug_assert!(
            self.h_window.is_none() || matches!(bound, FormatBound::Full),
            "a bounded query on an h_window map would clip twice — the render \
             path bounds its own formats by window and never asks for one"
        );
        let line = self.store.entry(idx).line;
        // Any wrapping mode needs the whole line: a clipped scan would emit
        // fewer display lines than the count `block` already committed to.
        // Applied before the check *and* the record below, so a wrapping
        // query never stores a bound narrower than what it actually ran.
        let bound = if self.key.wrap_mode.is_wrapping() {
            FormatBound::Full
        } else {
            bound
        };
        // Already formatted far enough, cut to the same window — by an
        // earlier query on this map, or by the frame's other pass over this
        // pane.
        if self
            .store
            .entry(idx)
            .format
            .covers(bound, self.h_window.as_ref())
        {
            return;
        }

        // Inline inserts are queried here, not just at render time: they
        // participate in wrapping, so counting display lines without them
        // makes the display-line list disagree with what the renderer
        // emits the moment an inlay hint pushes a line past the wrap column.
        self.inline_inserts.clear();
        self.decorations.clear();
        for (_, provider) in self.providers.decoration_sources(DecorationKinds::INLINE) {
            provider.decorations_for_line(line, &mut self.decorations);
        }
        // A provider that declared INLINE but emitted something else is a
        // provider bug — ignored, not a panic.
        self.inline_inserts
            .extend(self.decorations.drain(..).filter_map(|d| match d {
                Decoration::Inline(ins) => Some(ins),
                _ => None,
            }));
        self.inline_inserts.sort_by_key(|i| i.byte_offset);

        let format = &mut self.store.entry_mut(idx).format;
        format.reset();
        format_buffer_line(
            self.rope,
            line.into(),
            self.key.tab_width,
            &self.key.whitespace,
            &self.key.wrap_mode,
            self.h_window.clone(),
            bound,
            &self.inline_inserts,
            format,
        );
        format.extent = Some(bound);
        format.h_window = self.h_window.clone();
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests;
