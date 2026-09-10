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
//! content/wrap display lines, then `after`-virtuals. `ViewportState`'s
//! `top_line`/`top_slot` pair is the persisted form of exactly that
//! address.

use std::ops::Range;

use ropey::Rope;

use crate::format::{FormatBound, LineFormat, format_buffer_line};
use crate::providers::{Decoration, DecorationKinds, InlineInsert, ProviderSet, VirtualLineAnchor};
use crate::types::{CellContent, DisplayLine, Grapheme, ScopeId};
use hume_rope::column::{BufferLineCol, DisplayLineCol};
use hume_rope::line::ContentLine;
use hume_rope::offset::{CharOffset, ExclusiveRange};

pub mod line_store;

use line_store::{FormatKey, PaneLineStore};

// ---------------------------------------------------------------------------
// Addresses
// ---------------------------------------------------------------------------

/// The address of one display line: `slot` indexes into `line`'s visual
/// block (`before`-virtuals, content/wrap display lines, `after`-virtuals —
/// in that order).
///
/// `Ord` is lexicographic on `(line, slot)`, which is document order, so
/// comparing two addresses answers "which comes first on screen".
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq, PartialOrd, Ord)]
pub struct DisplayLinePos {
    pub line: ContentLine,
    pub slot: usize,
}

impl DisplayLinePos {
    pub fn new(line: ContentLine, slot: usize) -> Self {
        Self { line, slot }
    }
}

/// Which slot of a line's visual block a display line falls in — virtual
/// display lines anchored before it, its own wrap/content display lines, or
/// virtual display lines anchored after. The payload is the display line's
/// index within its own group, so `Content(2)` is a line's third content
/// display line and `Before(0)` is the first virtual display line above it.
///
/// Named `BlockSlot` rather than `DisplayLineKind` to stay distinct from
/// [`crate::types::DisplayLineKind`] (`LineStart`/`Wrap`/`Virtual`/`Filler`)
/// — a different question about the same display line: that one classifies
/// how a display line was produced, this one where it sits within its
/// line's block.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum BlockSlot {
    Before(usize),
    Content(usize),
    After(usize),
}

/// Display-line breakdown of one buffer line's visual block: virtual
/// display lines anchored `Before` it, its own wrap/content display lines,
/// and virtual display lines anchored `After` it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct BlockBreakdown {
    pub before: usize,
    pub content: usize,
    pub after: usize,
}

impl BlockBreakdown {
    /// Total display lines this line's whole visual block occupies.
    pub fn total(&self) -> usize {
        self.before + self.content + self.after
    }
}

/// Which grapheme a display column resolves to in [`DisplayLineMap::char_at`].
///
/// The two variants are different questions, not different implementations —
/// a click and a sticky-column vertical move optimise different things, and
/// collapsing them regresses one or the other.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum DisplayColTarget {
    /// The cell that *contains* this column — what a mouse click asks. A
    /// column inside a wide cell (a tab's expanse, a double-width glyph)
    /// resolves to that cell, and the end-of-line sentinel is a valid landing
    /// spot, so clicking past a line's text puts the cursor on its `\n`
    /// (a real cursor position in HUME's inclusive selection model).
    Cell,
    /// The real grapheme whose start column is *nearest* this one — what a
    /// sticky-column `j`/`k` asks, since it minimises display column drift.
    /// The end-of-line sentinel is skipped unless it is the display line's
    /// only grapheme (an empty line), so vertical movement stays on content.
    NearestContent,
}

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

    // ── Stepping ─────────────────────────────────────────────────────────

    /// The next display line, crossing into the next line's block as needed.
    /// `None` only at the document's very last display line.
    pub fn next(&mut self, pos: DisplayLinePos) -> Option<DisplayLinePos> {
        let total = self.block(pos.line).total();
        if pos.slot + 1 < total {
            return Some(DisplayLinePos::new(pos.line, pos.slot + 1));
        }
        (pos.line < self.last_line()).then(|| DisplayLinePos::new(pos.line.down(1), 0))
    }

    /// The previous display line. `None` only at the document's first display line.
    pub fn prev(&mut self, pos: DisplayLinePos) -> Option<DisplayLinePos> {
        if pos.slot > 0 {
            return Some(DisplayLinePos::new(pos.line, pos.slot - 1));
        }
        if pos.line.index() == 0 {
            return None;
        }
        let prev_line = pos.line.up(1);
        let total = self.block(prev_line).total();
        Some(DisplayLinePos::new(prev_line, total.saturating_sub(1)))
    }

    /// Step `delta` display lines from `pos`, saturating at either end of the document.
    /// The starting address is clamped first, so a stale viewport self-heals.
    pub fn advance(&mut self, pos: DisplayLinePos, delta: isize) -> DisplayLinePos {
        self.advance_counted(pos, delta).0
    }

    /// [`DisplayLineMap::advance`], plus how many display lines it actually
    /// stepped — fewer than `delta.unsigned_abs()` only when the document's
    /// edge stopped the walk.
    ///
    /// Since [`DisplayLineMap::next`] and [`DisplayLineMap::prev`] are exact
    /// inverses, the count is also the distance back: after stepping `n`
    /// display lines *backward* from `pos`, `distance(result, pos) == n`.
    /// That lets a caller that scrolled backward from the cursor learn the
    /// cursor's resulting screen row without walking the same display lines
    /// forward again.
    pub fn advance_counted(
        &mut self,
        pos: DisplayLinePos,
        delta: isize,
    ) -> (DisplayLinePos, usize) {
        let mut cur = self.clamp(pos);
        let mut taken = 0;
        for _ in 0..delta.unsigned_abs() {
            let stepped = if delta >= 0 {
                self.next(cur)
            } else {
                self.prev(cur)
            };
            match stepped {
                Some(next) => cur = next,
                None => break,
            }
            taken += 1;
        }
        (cur, taken)
    }

    /// Display lines from `from` forward to `to`, or `None` if `to` is
    /// behind `from` or more than `cap` display lines ahead.
    ///
    /// Callers pass the viewport height as `cap`, which keeps this O(height)
    /// however large the document is. Both "behind" and "too far" collapse to
    /// `None` because every caller asks the same question — is `to` visible
    /// from `from` — and neither case is.
    pub fn distance(
        &mut self,
        from: DisplayLinePos,
        to: DisplayLinePos,
        cap: usize,
    ) -> Option<usize> {
        if to < from {
            return None;
        }
        // Every line's block occupies at least one display line (`block`'s
        // own `content >= 1` assert), so crossing `to.line - from.line`
        // lines costs at least that many steps — a line delta beyond `cap`
        // already proves the walk below would return `None`, without
        // formatting a single line under wrap to find out.
        if to.line.index().saturating_sub(from.line.index()) > cap {
            return None;
        }
        let mut cur = from;
        for steps in 0..=cap {
            if cur == to {
                return Some(steps);
            }
            cur = self.next(cur)?;
        }
        None
    }

    /// Whether the whole document fits in `height` display lines.
    ///
    /// Walks at most `height + 1` display lines, so this stays cheap on a
    /// huge buffer where the answer is obviously "no".
    pub fn fits_in(&mut self, height: u16) -> bool {
        // Every document has at least one display line
        // (DisplayLinePos::default()), which cannot fit in a zero-height
        // viewport — short-circuit before the loop below, which never
        // compares its `lines = 1` starting count against `height` if the
        // walk ends on the very first `next()`.
        if height == 0 {
            return false;
        }
        let mut cur = DisplayLinePos::default();
        let mut lines = 1usize;
        while let Some(next) = self.next(cur) {
            cur = next;
            lines += 1;
            if lines > height as usize {
                return false;
            }
        }
        true
    }

    // ── Char offsets ↔ display lines ──────────────────────────────────────

    /// Locate `char_offset`: its display line, and its display column in
    /// that display line.
    pub fn locate(&mut self, char_offset: CharOffset) -> (DisplayLinePos, DisplayLineCol) {
        debug_assert!(
            char_offset.index() <= self.rope.len_chars(),
            "locate: char_offset {char_offset:?} is out of range for a buffer \
             of {} chars — ropey's own `char_to_line` panics past this point, \
             so a caller holding a position from an earlier frame (an LSP \
             completion anchor, a stale selection) must revalidate it \
             against the current buffer before reaching here",
            self.rope.len_chars()
        );
        let (ropey_line, target_byte) = hume_rope::lines::char_to_line_byte(self.rope, char_offset);
        // Unwrapped here, not threaded further: `locate_in_line`/`FormatBound::ToByte`
        // compare directly against `Grapheme.byte_range` (`Range<usize>`,
        // deliberately not `ByteCol` — see CLAUDE.md's per-cell hot-loop note),
        // so this is the correct crossing into that still-untyped subsystem.
        let target_byte = target_byte.index();
        let line = self.content_line_of(ropey_line);
        let before = self.block(line).before;
        // Only up to the target: everything past it is irrelevant to where
        // this one offset sits.
        let idx = self.ensure_formatted(line, FormatBound::ToByte(target_byte));
        let (sub, display_col) = self.locate_in_line(idx, target_byte, char_offset.index());
        (DisplayLinePos::new(line, before + sub), display_col)
    }

    /// Which content display line of `idx`'s line holds `target_byte`
    /// (line-relative, resolved by the caller), and at what column. `idx` must
    /// come from an [`DisplayLineMap::ensure_formatted`] bounded at least as far as
    /// `ToByte(target_byte)`.
    fn locate_in_line(
        &self,
        idx: usize,
        target_byte: usize,
        char_offset: usize,
    ) -> (usize, DisplayLineCol) {
        let entry = self.store.entry(idx);
        let format = &entry.format;
        let lines = &format.display_lines;
        let graphemes = &format.graphemes;

        for (i, dline) in lines.iter().enumerate() {
            if dline.graphemes.is_empty() {
                continue;
            }
            let first = &graphemes[dline.graphemes.start];
            let last = &graphemes[dline.graphemes.end - 1];
            let is_last = i + 1 == lines.len();
            if target_byte >= first.byte_range.start
                && (target_byte < last.byte_range.end || is_last)
            {
                // The real grapheme, not an inline-insert decoration sharing
                // its `char_offset` — `style::resolve_grapheme_display_col`
                // skips forward past any `Virtual` cells to reach it, the
                // same rule `style::char_offset_to_display_col` applies for
                // selection styling.
                let display_col = crate::style::resolve_grapheme_display_col(
                    char_offset,
                    graphemes,
                    &dline.graphemes,
                )
                .map_or_else(
                    // Past every grapheme on the display line (end of line).
                    || last.display_col.advance(last.width as u32),
                    |(display_col, _)| display_col,
                );
                return (i, display_col);
            }
        }

        // No display line claimed the offset: answer with the end of the
        // last one. Every content display line has at least one grapheme
        // (an empty line still gets its EOL sentinel), and the last one's
        // `is_last` branch above matches any `target_byte` at or past its
        // own start — so reaching here means either `lines` is empty or
        // every display line was skipped for having no graphemes, both of
        // which indicate a formatting bug rather than a normal input.
        debug_assert!(
            !lines.is_empty(),
            "locate_in_line: line {}, char_offset {char_offset} matched \
             no display line — every content display line should claim \
             some byte range of the line",
            entry.line.index()
        );
        let last_dline = lines.len().saturating_sub(1);
        let display_col = lines
            .get(last_dline)
            .filter(|r| !r.graphemes.is_empty())
            .map_or(DisplayLineCol::new(0), |r| {
                let lg = &graphemes[r.graphemes.end - 1];
                lg.display_col.advance(lg.width as u32)
            });
        (last_dline, display_col)
    }

    /// The display line `char_offset` sits on, without resolving its column.
    ///
    /// In `WrapMode::None` a line is exactly one content display line (see
    /// [`DisplayLineMap::block`]), so the sub-index is always 0 and the
    /// answer falls out of the block breakdown with no formatting at all —
    /// the difference between O(1) and O(offset into the line) for the
    /// callers that only want the display line.
    pub fn locate_display_line(&mut self, char_offset: CharOffset) -> DisplayLinePos {
        debug_assert!(
            char_offset.index() <= self.rope.len_chars(),
            "locate_display_line: char_offset {char_offset:?} is out of range for a \
             buffer of {} chars — see the debug_assert in DisplayLineMap::locate",
            self.rope.len_chars()
        );
        if self.key.wrap_mode.is_wrapping() {
            // Wrapping needs the sub-index, which only formatting can
            // answer — and `block` has already formatted the line to count
            // its display lines.
            return self.locate(char_offset).0;
        }
        let line =
            self.content_line_of(hume_rope::lines::char_to_ropey_line(self.rope, char_offset));
        DisplayLinePos::new(line, self.block(line).before)
    }

    /// The char offset `target_display_col` resolves to on `pos`'s display
    /// line, under `target`'s policy.
    ///
    /// A virtual display line is not buffer content, so `pos` landing on
    /// one clamps to the nearest content sub-line of the same line — the
    /// first for a `Before` display line, the last for an `After` one.
    pub fn char_at(
        &mut self,
        pos: DisplayLinePos,
        target_display_col: DisplayLineCol,
        target: DisplayColTarget,
    ) -> CharOffset {
        let b = self.block(pos.line);
        let sub = pos
            .slot
            .saturating_sub(b.before)
            .min(b.content.saturating_sub(1));
        // Only up to the target column: no cell further right can be the one
        // this column resolves to, under either policy.
        let idx = self.ensure_formatted(pos.line, FormatBound::ToDisplayCol(target_display_col));
        CharOffset::new(self.resolve_in_display_line(idx, sub, target_display_col, target))
    }

    /// Shared core of [`DisplayLineMap::char_at`] and [`DisplayLineMap::char_at_buffer_line_col`]:
    /// which char offset on content display line `sub` of `idx`'s line
    /// resolves to `target_display_col`, under `target`'s policy. `idx`
    /// must come from an [`DisplayLineMap::ensure_formatted`] bounded at
    /// least up to `target_display_col`.
    fn resolve_in_display_line(
        &self,
        idx: usize,
        sub: usize,
        target_display_col: DisplayLineCol,
        target: DisplayColTarget,
    ) -> usize {
        let entry = self.store.entry(idx);
        let line_start = hume_rope::lines::line_start_char(self.rope, entry.line.into()).index();
        let format = &entry.format;
        let Some(dline) = format.display_lines.get(sub) else {
            return line_start;
        };
        let graphemes = &format.graphemes[dline.graphemes.clone()];
        if graphemes.is_empty() {
            return line_start;
        }

        match target {
            DisplayColTarget::Cell => {
                graphemes
                    .iter()
                    .find(|g| target_display_col < g.display_col.advance(g.width as u32))
                    .unwrap_or_else(|| graphemes.last().expect("non-empty checked above"))
                    .char_offset
            }
            DisplayColTarget::NearestContent => {
                // Eligibility by content type: `Grapheme` is real content,
                // always eligible. `WidthContinuation` is excluded even
                // though it shares its primary's `char_offset` (so admitting
                // it can never answer anything the primary itself wouldn't):
                // its `display_col` sits one column *past* the wide glyph,
                // which is exactly where the next real cell starts, so a
                // target landing on that boundary ties between the two — and
                // `min_by_key` keeps the first tied element, which is the
                // continuation (pushed immediately after its primary, ahead
                // of whatever comes next). Left in, that tie silently wins
                // over the following cell's own, distinct `char_offset`.
                // `Empty` (EOL sentinel) has a buffer position but isn't
                // content, so it only answers when nothing else can (an empty
                // line) — gated on `admit_eol`. `Virtual` (inline-insert)
                // carries the real grapheme's `char_offset` it precedes, so
                // minimising distance against it elsewhere on the display
                // line would land on a character that cell isn't at — excluded outright,
                // not just deprioritised. `Whitespace`/`TabFill` cover
                // tab/space glyphs and blank tab fill, which *are* real
                // content, except the newline indicator, which shares the
                // EOL sentinel's column and must be excluded the same way —
                // singled out by `byte_range` being empty, just like the
                // sentinel it's drawn on top of (`format.rs`'s
                // newline-indicator push).
                //
                // An exhaustive match (not a chain of exclusion filters) so a
                // future `CellContent` variant forces a decision here instead
                // of silently defaulting to eligible.
                let nearest = |admit_eol: bool| {
                    graphemes
                        .iter()
                        // Virtual display line cells (segmented separately
                        // by `segment_virtual_line`) have no buffer position
                        // at all; unreachable from `char_at`, which only
                        // ever formats content display lines, but guarded
                        // defensively.
                        .filter(|g| g.char_offset != usize::MAX)
                        .filter(|g| match g.content {
                            CellContent::Grapheme => true,
                            CellContent::WidthContinuation => false,
                            CellContent::Empty => admit_eol,
                            CellContent::Virtual { .. } => false,
                            // A substitution standing in for real buffer text
                            // is a position the cursor can land on; one
                            // standing in for decoration text (`push_virtual_cells`)
                            // has an empty byte range and is not.
                            CellContent::Whitespace { .. }
                            | CellContent::TabFill
                            | CellContent::Placeholder { .. } => !g.byte_range.is_empty(),
                        })
                        .min_by_key(|g| target_display_col.abs_diff(g.display_col))
                        .map(|g| g.char_offset)
                };
                nearest(false)
                    .or_else(|| nearest(true))
                    .unwrap_or(line_start)
            }
        }
    }

    /// `(indent, span)` for content display line `sub` of `idx`'s line.
    /// `indent` is the display column the display line's first cell starts
    /// at — 0 on a line's own first display line, `indent_display_cols` on
    /// a wrap continuation display line (see
    /// [`crate::types::Grapheme::display_col`]). `span` is the display
    /// line's own content width with that indent excluded, so summing
    /// `span` across every display line before `sub`, plus the
    /// indent-excluded offset within `sub`, converts a display-line-relative
    /// column into one relative to the whole buffer line.
    fn display_line_shape(&self, idx: usize, sub: usize) -> (DisplayLineCol, u32) {
        let format = self.format_at(idx);
        let Some(dline) = format.display_lines.get(sub) else {
            return (DisplayLineCol::new(0), 0);
        };
        let graphemes = &format.graphemes[dline.graphemes.clone()];
        let Some(first) = graphemes.first() else {
            return (DisplayLineCol::new(0), 0);
        };
        let last = graphemes.last().expect("non-empty checked above");
        let indent = first.display_col;
        let span = last
            .display_col
            .advance(last.width as u32)
            .cells_since(indent);
        (indent, span)
    }

    /// The display column `char_offset` sits at, measured from its own
    /// buffer line's start rather than from its display line's — the
    /// column a numeric-prefixed vertical move (`9j`/`9k`) latches, since it
    /// targets the same buffer-line column on its landing line regardless
    /// of which display line of that (possibly wrapped) line it lands on.
    ///
    /// Continuation-display-line indent is excluded (see
    /// `DisplayLineMap::display_line_shape`) and inline virtual cells
    /// (inlay hints, ghost text) are included, same as
    /// [`DisplayLineMap::locate`] — the two differ only in what they're
    /// measured from, and coincide under `WrapMode::None`, where a line is
    /// exactly one display line with no indent.
    pub fn buffer_line_col(&mut self, char_offset: CharOffset) -> BufferLineCol {
        debug_assert!(
            char_offset.index() <= self.rope.len_chars(),
            "buffer_line_col: char_offset {char_offset:?} is out of range for \
             a buffer of {} chars — see the debug_assert in DisplayLineMap::locate",
            self.rope.len_chars()
        );
        let (ropey_line, target_byte) = hume_rope::lines::char_to_line_byte(self.rope, char_offset);
        // Unwrapped here, not threaded further: `locate_in_line`/`FormatBound::ToByte`
        // compare directly against `Grapheme.byte_range` (`Range<usize>`,
        // deliberately not `ByteCol` — see CLAUDE.md's per-cell hot-loop note),
        // so this is the correct crossing into that still-untyped subsystem.
        let target_byte = target_byte.index();
        let line = self.content_line_of(ropey_line);
        let idx = self.ensure_formatted(line, FormatBound::ToByte(target_byte));
        let (sub, dline_display_col) = self.locate_in_line(idx, target_byte, char_offset.index());
        let (dline_indent, _) = self.display_line_shape(idx, sub);
        let preceding: u32 = (0..sub).map(|j| self.display_line_shape(idx, j).1).sum();
        BufferLineCol::new(preceding + dline_display_col.cells_since(dline_indent))
    }

    /// Inverse of [`DisplayLineMap::buffer_line_col`]: the char offset
    /// `target_line_display_col` resolves to on `line`, under `target`'s
    /// policy.
    ///
    /// A line-relative column past the line's total width clamps to its
    /// last display line, where `target`'s own clamp rule (see
    /// [`DisplayColTarget`]) applies — the same "stick to the last real
    /// character, land on `\n` only when the line is empty" rule bare
    /// `j`/`k` already gets from [`DisplayLineMap::char_at`].
    pub fn char_at_buffer_line_col(
        &mut self,
        line: ContentLine,
        target_line_display_col: BufferLineCol,
        target: DisplayColTarget,
    ) -> CharOffset {
        let content_display_lines = self.block(line).content;
        // Only up to the target column: while wrapping, `ensure_formatted`
        // promotes this to `Full` regardless (a display-line-relative bound
        // can't usefully clip a line-relative target), and without wrapping
        // `content_display_lines == 1` so the two columns coincide — sound
        // to read as display-line-relative here for exactly that reason
        // (see `BufferLineCol::as_display_line_unwrapped`'s own doc).
        let idx = self.ensure_formatted(
            line,
            FormatBound::ToDisplayCol(target_line_display_col.as_display_line_unwrapped()),
        );
        let mut remaining = target_line_display_col.get();
        let mut sub = 0;
        let mut dline_indent = DisplayLineCol::new(0);
        for j in 0..content_display_lines {
            let (indent, span) = self.display_line_shape(idx, j);
            sub = j;
            dline_indent = indent;
            if j + 1 == content_display_lines || remaining < span {
                break;
            }
            remaining -= span;
        }
        CharOffset::new(self.resolve_in_display_line(
            idx,
            sub,
            dline_indent.advance(remaining),
            target,
        ))
    }

    /// The char range one content display line covers. `None` when `pos` is
    /// not a content display line.
    ///
    /// Lets a caller scope a line-oriented search (nearest word) to the
    /// head's own display line instead of the whole buffer line.
    pub fn content_display_line_char_bounds(
        &mut self,
        pos: DisplayLinePos,
    ) -> Option<ExclusiveRange<CharOffset>> {
        let b = self.block(pos.line);
        let sub = pos.slot.checked_sub(b.before)?;
        if sub >= b.content {
            return None;
        }
        // `Full`: this reads the *next* display line's first char to bound
        // the current one, so it needs every display line the line produces.
        let idx = self.ensure_formatted(pos.line, FormatBound::Full);

        let format = self.format_at(idx);
        let lines = &format.display_lines;
        let graphemes = &format.graphemes;
        // Filtering out `usize::MAX` (`Grapheme.char_offset`'s own sentinel
        // for "no buffer position") before this closure's result is used
        // makes the surviving `usize` a genuine position again, safe to mint.
        let first_char_of = |dline: &DisplayLine| {
            graphemes[dline.graphemes.clone()]
                .iter()
                .filter(|g| g.char_offset != usize::MAX)
                .map(|g| g.char_offset)
                .min()
        };

        let start = CharOffset::new(first_char_of(lines.get(sub)?)?);
        let end = lines
            .get(sub + 1)
            .and_then(first_char_of)
            .map(CharOffset::new)
            .unwrap_or_else(|| hume_rope::lines::next_line_start(self.rope, pos.line.into()));
        Some(ExclusiveRange::new(start, end))
    }

    // ── Render access ────────────────────────────────────────────────────

    /// Borrow what the render stage needs to style and compose `pos`.
    ///
    /// Content display lines come from the cached format, so a line is
    /// formatted once however many of its display lines get rendered.
    /// Virtual display lines are segmented here — the same
    /// grapheme/width/column bookkeeping `format_buffer_line` does for real
    /// lines, so a provider handing over plain text and scoped byte ranges
    /// cannot get that arithmetic wrong.
    pub fn render_display_line(&mut self, pos: DisplayLinePos) -> RenderDisplayLine<'_> {
        let (idx, slot) = self.resolve(pos);
        match slot {
            BlockSlot::Content(sub) => {
                // `Full`: the render stage emits whole display lines, and
                // its own clipping is the map's `h_window`, applied inside
                // the format.
                self.ensure_format_at(idx, FormatBound::Full);
                let format = self.format_at(idx);
                RenderDisplayLine {
                    display_line: &format.display_lines[sub],
                    graphemes: &format.graphemes,
                    line_text: &format.line_texts,
                    virtual_texts: &format.virtual_texts,
                    base_scope: None,
                }
            }
            BlockSlot::Before(i) => self.segment_virtual_line(idx, i),
            // `resolve` already walked this line's block, so its `before`
            // count is on the entry it handed back — no need to walk it again.
            BlockSlot::After(i) => {
                let before = self.store.entry(idx).before;
                self.segment_virtual_line(idx, before + i)
            }
        }
    }

    /// Lay one virtual display line out into its own scratch and borrow it
    /// back.
    ///
    /// Uses the store's `virtual_line`, not the line's own format: a
    /// `Before` display line renders ahead of its line's content display
    /// lines, which are very likely already formatted (`block` runs the
    /// formatter in wrapping mode to count wrap display lines) — laying the
    /// virtual display line out over them would destroy that and force a
    /// reformat of the content display lines that follow.
    fn segment_virtual_line(&mut self, idx: usize, vl_idx: usize) -> RenderDisplayLine<'_> {
        let tab_width = self.key.tab_width;
        // The entry's virtual display lines and the scratch they lay out
        // into are disjoint parts of the store, borrowed together so the
        // display line's text can be read while its cells are written.
        let (entry, vline) = self.store.entry_and_virtual_line(idx);
        let vl = &entry.virtual_lines[vl_idx];
        let anchor_line = entry.line.into();
        let provider_id = vl.provider_id;
        let base_scope = vl.base_scope;
        vline.clear();

        // `vl.segments` was sorted by `block()` at intake, and
        // `grapheme_indices` yields byte offsets in ascending order, so a
        // single monotonic cursor resolves every grapheme's scope in
        // O(graphemes + segments) instead of a per-grapheme linear scan.
        let mut scope_cursor = crate::style::highlight::IntervalCursor::new(&vl.segments);
        let mut display_col = DisplayLineCol::new(0);
        crate::format::push_virtual_cells(
            &mut vline.texts,
            &mut vline.graphemes,
            &crate::format::VirtualRun {
                text: &vl.text,
                byte_offset: 0, // no buffer position
                char_offset: usize::MAX,
                indent_depth: 0,
            },
            tab_width,
            &mut display_col,
            |byte_offset| scope_cursor.scope_at(byte_offset).or(base_scope),
        );

        let display_line = vline.display_line.insert(DisplayLine {
            kind: crate::types::DisplayLineKind::Virtual {
                provider_id,
                anchor_line,
            },
            graphemes: 0..vline.graphemes.len(),
        });

        RenderDisplayLine {
            display_line,
            graphemes: &vline.graphemes,
            // A virtual display line has no buffer text — every cell resolves out of
            // `virtual_texts` instead: `Virtual` text itself, or the
            // `Placeholder` a control character becomes. A tab needs no
            // arena lookup at all — it's `TabFill`, drawn as blanks directly.
            line_text: "",
            virtual_texts: &vline.texts,
            base_scope,
        }
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
