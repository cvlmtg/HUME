//! The single authority on the document's display-row list.
//!
//! A document's rows come from two independent sources: a buffer line's own
//! content rows (one per wrap row — exactly one when wrapping is off), and
//! virtual rows contributed by [`DecorationSource`](crate::providers::DecorationSource)
//! providers, anchored `Before` or `After` a line. Rendering, scrolling,
//! cursor placement, mouse mapping and visual movement all need the same
//! flattened view of those two sources, and any two implementations of that
//! view which disagree by a single row produce a cursor that draws in the
//! wrong place or a viewport that scrolls past content.
//!
//! [`RowMap`] is that one implementation. It bundles everything the row list
//! depends on — rope, resolved wrap mode, tab width, whitespace config,
//! providers, content width — so consumers hold one `&mut RowMap` instead of
//! threading eight-to-eleven parameters through every walk. What it learns
//! about each line it visits goes in the pane's own
//! [`line_store::PaneLineStore`], which every walk of that pane shares so
//! none repeats another's work.
//!
//! Addresses are [`RowPos`]: a buffer line plus a row index into that line's
//! *visual block*, which runs `before`-virtuals, then content/wrap rows, then
//! `after`-virtuals. `ViewportState`'s `top_line`/`top_row_offset` pair is the
//! persisted form of exactly that address.

use std::ops::Range;

use ropey::Rope;

use crate::format::{FormatBound, LineFormat, format_buffer_line};
use crate::providers::{Decoration, DecorationKinds, InlineInsert, ProviderSet, VirtualLineAnchor};
use crate::types::{CellContent, DisplayRow, Grapheme, ScopeId};

pub mod line_store;

use line_store::{FormatKey, PaneLineStore};

// ---------------------------------------------------------------------------
// Addresses
// ---------------------------------------------------------------------------

/// The address of one display row: `row` indexes into `line`'s visual block
/// (`before`-virtuals, content/wrap rows, `after`-virtuals — in that order).
///
/// `Ord` is lexicographic on `(line, row)`, which is document order, so
/// comparing two addresses answers "which comes first on screen".
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq, PartialOrd, Ord)]
pub struct RowPos {
    pub line: usize,
    pub row: usize,
}

impl RowPos {
    pub fn new(line: usize, row: usize) -> Self {
        Self { line, row }
    }
}

/// Which slot of a line's visual block a display row falls in — virtual rows
/// anchored before it, its own wrap/content rows, or virtual rows anchored
/// after. The payload is the row's index within its own group, so
/// `Content(2)` is a line's third content row and `Before(0)` is the first
/// virtual row above it.
///
/// Named `BlockSlot` rather than `RowKind` to stay distinct from
/// [`crate::types::RowKind`] (`LineStart`/`Wrap`/`Virtual`/`Filler`) — a
/// different question about the same row: that one classifies how a row was
/// produced, this one where it sits within its line's block.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum BlockSlot {
    Before(usize),
    Content(usize),
    After(usize),
}

/// Display-row breakdown of one buffer line's visual block: virtual rows
/// anchored `Before` it, its own wrap/content rows, and virtual rows anchored
/// `After` it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct RowsBreakdown {
    pub before: usize,
    pub content: usize,
    pub after: usize,
}

impl RowsBreakdown {
    /// Total screen rows this line's whole visual block occupies.
    pub fn total(&self) -> usize {
        self.before + self.content + self.after
    }
}

/// Which grapheme a display column resolves to in [`RowMap::char_at`].
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
    /// The end-of-line sentinel is skipped unless it is the row's only
    /// grapheme (an empty line), so vertical movement stays on content.
    NearestContent,
}

// ---------------------------------------------------------------------------
// Row map
// ---------------------------------------------------------------------------

/// Everything the render stage needs to style and compose one display row.
pub struct RenderRow<'m> {
    pub row: &'m DisplayRow,
    /// The graphemes `row.graphemes` indexes into.
    pub graphemes: &'m [Grapheme],
    /// Buffer-line text that the row's real graphemes index by byte range.
    /// Empty for virtual rows, which have no buffer text.
    pub line_text: &'m str,
    /// Arena backing `Whitespace`/`Placeholder`/`Virtual` cell text.
    pub virtual_texts: &'m str,
    /// The row's own background scope (`VirtualLine::base_scope`) — `None`
    /// for content rows, which get their background from
    /// `Decoration::LineBg`/cursorline instead (`pane_render.rs`'s
    /// `LineStyle::tint`).
    pub base_scope: Option<ScopeId>,
}

/// The single authority on the document's display-row list. See the module doc.
pub struct RowMap<'a> {
    rope: &'a Rope,
    /// Everything this map's formats depend on besides the line's own text —
    /// wrap mode (always resolved against `content_width`: `WrapMode::wrap_width`
    /// panics on the `width: 0` sentinel, and [`RowMap::new`] is the one
    /// funnel every consumer passes through, so it resolves there rather
    /// than trusting callers to), tab width, whitespace config, and the
    /// buffer's identity/generation — and, verbatim, this map's store-scope
    /// key. One field rather than four: see [`FormatKey`]'s own doc for why
    /// the scroll pass and the render pass sharing a store depends on it.
    key: FormatKey,
    providers: &'a ProviderSet,
    content_width: u16,
    h_window: Option<Range<u32>>,
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

impl<'a> RowMap<'a> {
    pub fn new(
        rope: &'a Rope,
        providers: &'a ProviderSet,
        content_width: u16,
        key: FormatKey,
        store: &'a mut PaneLineStore,
    ) -> Self {
        debug_assert!(
            hume_rope::lines::ends_with_newline(rope),
            "RowMap requires a trailing '\\n' (the buffer invariant) — \
             without it `last_line`'s content-line derivation drops the \
             rope's actual last content line"
        );
        debug_assert!(
            content_width >= 1,
            "RowMap requires content_width >= 1 — a 0 here leaves \
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
    /// by [`RowMap::ensure_formatted`], so "the caller ensured this line" is
    /// carried by a value rather than by the two calls happening in order.
    fn format_at(&self, idx: usize) -> &LineFormat {
        &self.store.entry(idx).format
    }

    /// Clip `WrapMode::None` formatting to a horizontal column window — the
    /// render path's bound on arbitrarily long unwrapped lines.
    ///
    /// Row counts are unaffected (no-wrap is one content row however wide the
    /// line is), so this changes only which graphemes the render accessors
    /// emit. Editor-side consumers want whole lines and leave it `None`.
    ///
    /// Does not re-scope the store: `h_window` is not part of [`FormatKey`],
    /// only recorded on the [`LineFormat`] a later `ensure_format_at` produces,
    /// so an entry's block shape and virtual rows survive this call and only
    /// its format is subject to being recut. That is also what keeps the
    /// frame's two passes from sharing a *format* in `WrapMode::None`, where
    /// only the render pass clips — they still share the block shape.
    pub fn with_h_window(mut self, h_window: Option<Range<u32>>) -> Self {
        debug_assert!(
            h_window.is_none() || !self.key.wrap_mode.is_wrapping(),
            "with_h_window is a WrapMode::None-only clip — a wrapping RowMap \
             would silently under-count content rows, since ensure_format_at \
             passes h_window through to the formatter even while wrapping"
        );
        self.h_window = h_window;
        self
    }

    pub fn is_wrapping(&self) -> bool {
        self.key.wrap_mode.is_wrapping()
    }

    /// The wrap column display rows are actually laid out against — `None`
    /// for `WrapMode::None`, otherwise the *resolved* width (the mode's own
    /// explicit width, or `content_width` when the mode used the `0`
    /// sentinel). Distinct from [`RowMap::content_width`]: an explicit wrap
    /// width doesn't move when the pane resizes, so a resize-driven
    /// staleness check (a `DisplayRow`-relative sticky column surviving a
    /// wrap-width change) must compare this, not the raw content width, or
    /// it invalidates latches a resize never actually affected.
    pub fn resolved_wrap_width(&self) -> Option<u16> {
        self.key.wrap_mode.wrap_width()
    }

    /// Width available for content — the same `content_width` the caller
    /// passed to [`RowMap::new`] (gutter already subtracted). The one column
    /// bound `locate`'s columns are relative to, so a caller sizing anything
    /// against display columns (horizontal scroll) reads it here rather than
    /// re-deriving it from the pane and risking the two drifting apart.
    pub fn content_width(&self) -> u16 {
        self.content_width
    }

    // ── Block shape ──────────────────────────────────────────────────────

    /// The display-row breakdown of `line`'s visual block.
    pub fn block(&mut self, line: usize) -> RowsBreakdown {
        let idx = self.block_entry(line);
        self.breakdown(idx)
    }

    /// The breakdown of an entry already in hand.
    ///
    /// Split out so [`RowMap::resolve`] can compute a slot from the same
    /// breakdown it needs to walk, without re-finding the entry `block`
    /// already holds.
    fn breakdown(&mut self, idx: usize) -> RowsBreakdown {
        let content = self.content_rows(idx);
        let entry = self.store.entry(idx);
        RowsBreakdown {
            before: entry.before,
            content,
            after: entry.after(),
        }
    }

    /// The store entry for `line`, building its block shape if this is the
    /// first time this store has seen it.
    ///
    /// Only the *shape* — the format arrives separately, from whoever first
    /// needs the line's rows. Under `WrapMode::None` that may be much later,
    /// or never.
    fn block_entry(&mut self, line: usize) -> usize {
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
            // provider's rows, which the gutter would then attribute wrongly.
            for vl in &mut virtual_lines[start..] {
                vl.provider_id = id;
            }
        }
        // A row anchored outside the queried line is a provider bug. Drop it
        // rather than count it against a line it does not belong to.
        virtual_lines.retain(|vl| match vl.anchor {
            VirtualLineAnchor::Before(n) | VirtualLineAnchor::After(n) => n == line,
        });
        // `Before` rows ahead of `After` rows; stable, so provider
        // registration order survives within each group.
        virtual_lines.sort_by_key(|vl| vl.anchor.sort_key());
        // Providers are plugin code and the trait makes no ordering promise
        // enforceable at the boundary — sort here so `segment_virtual_row`'s
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

    /// How many content rows `line`'s block occupies.
    ///
    /// `WrapMode::None` is always exactly one, and formatting cannot return
    /// another answer there — so counting never runs the formatter. That is
    /// the difference between O(1) and O(line length) per query on a minified
    /// line megabytes wide. Under a wrapping mode the count *is* the
    /// formatter's output, so the line gets formatted here if it wasn't
    /// already.
    fn content_rows(&mut self, idx: usize) -> usize {
        if !self.key.wrap_mode.is_wrapping() {
            return 1;
        }
        // `Full`: the row count is the output, so nothing may be clipped.
        self.ensure_format_at(idx, FormatBound::Full);
        let entry = self.store.entry(idx);
        let content = entry.format.display_rows.len();
        debug_assert!(
            content >= 1,
            "line {} counted zero content rows; every line occupies at least one",
            entry.line
        );
        content
    }

    /// Index of the last buffer line a cursor can occupy.
    pub fn last_line(&self) -> usize {
        hume_rope::lines::last_content_line(self.rope)
    }

    /// Pull `pos` into the document: `line` into `0..=last_line()`, then `row`
    /// into that line's block.
    pub fn clamp(&mut self, pos: RowPos) -> RowPos {
        let line = pos.line.min(self.last_line());
        let total = self.block(line).total();
        RowPos::new(line, pos.row.min(total.saturating_sub(1)))
    }

    /// Which block slot `pos` addresses.
    pub fn slot(&mut self, pos: RowPos) -> BlockSlot {
        self.resolve(pos).1
    }

    /// The entry index and block slot `pos` addresses, in one walk — for a
    /// caller (`render_row`) that needs both without resolving the line's
    /// block twice.
    fn resolve(&mut self, pos: RowPos) -> (usize, BlockSlot) {
        let idx = self.block_entry(pos.line);
        let b = self.breakdown(idx);
        debug_assert!(
            pos.row < b.total(),
            "row {} is past line {}'s block of {}",
            pos.row,
            pos.line,
            b.total()
        );
        let slot = if pos.row < b.before {
            BlockSlot::Before(pos.row)
        } else if pos.row < b.before + b.content {
            BlockSlot::Content(pos.row - b.before)
        } else {
            BlockSlot::After(pos.row - b.before - b.content)
        };
        (idx, slot)
    }

    // ── Stepping ─────────────────────────────────────────────────────────

    /// The next display row, crossing into the next line's block as needed.
    /// `None` only at the document's very last row.
    pub fn next(&mut self, pos: RowPos) -> Option<RowPos> {
        let total = self.block(pos.line).total();
        if pos.row + 1 < total {
            return Some(RowPos::new(pos.line, pos.row + 1));
        }
        (pos.line < self.last_line()).then(|| RowPos::new(pos.line + 1, 0))
    }

    /// The previous display row. `None` only at the document's first row.
    pub fn prev(&mut self, pos: RowPos) -> Option<RowPos> {
        if pos.row > 0 {
            return Some(RowPos::new(pos.line, pos.row - 1));
        }
        let prev_line = pos.line.checked_sub(1)?;
        let total = self.block(prev_line).total();
        Some(RowPos::new(prev_line, total.saturating_sub(1)))
    }

    /// Step `delta` rows from `pos`, saturating at either end of the document.
    /// The starting address is clamped first, so a stale viewport self-heals.
    pub fn advance(&mut self, pos: RowPos, delta: isize) -> RowPos {
        self.advance_counted(pos, delta).0
    }

    /// [`RowMap::advance`], plus how many rows it actually stepped — fewer than
    /// `delta.unsigned_abs()` only when the document's edge stopped the walk.
    ///
    /// Since [`RowMap::next`] and [`RowMap::prev`] are exact inverses, the count
    /// is also the distance back: after stepping `n` rows *backward* from `pos`,
    /// `distance(result, pos) == n`. That lets a caller that scrolled backward
    /// from the cursor learn the cursor's resulting screen row without walking
    /// the same rows forward again.
    pub fn advance_counted(&mut self, pos: RowPos, delta: isize) -> (RowPos, usize) {
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

    /// Rows from `from` forward to `to`, or `None` if `to` is behind `from` or
    /// more than `cap` rows ahead.
    ///
    /// Callers pass the viewport height as `cap`, which keeps this O(height)
    /// however large the document is. Both "behind" and "too far" collapse to
    /// `None` because every caller asks the same question — is `to` visible
    /// from `from` — and neither case is.
    pub fn distance(&mut self, from: RowPos, to: RowPos, cap: usize) -> Option<usize> {
        if to < from {
            return None;
        }
        // Every line's block occupies at least one row (`block`'s own
        // `content >= 1` assert), so crossing `to.line - from.line` lines
        // costs at least that many steps — a line delta beyond `cap` already
        // proves the walk below would return `None`, without formatting a
        // single line under wrap to find out.
        if to.line.saturating_sub(from.line) > cap {
            return None;
        }
        let mut cur = from;
        for rows in 0..=cap {
            if cur == to {
                return Some(rows);
            }
            cur = self.next(cur)?;
        }
        None
    }

    /// Whether the whole document fits in `height` rows.
    ///
    /// Walks at most `height + 1` rows, so this stays cheap on a huge buffer
    /// where the answer is obviously "no".
    pub fn fits_in(&mut self, height: u16) -> bool {
        // Every document has at least one row (RowPos::default()), which
        // cannot fit in a zero-height viewport — short-circuit before the
        // loop below, which never compares its `rows = 1` starting count
        // against `height` if the walk ends on the very first `next()`.
        if height == 0 {
            return false;
        }
        let mut cur = RowPos::default();
        let mut rows = 1usize;
        while let Some(next) = self.next(cur) {
            cur = next;
            rows += 1;
            if rows > height as usize {
                return false;
            }
        }
        true
    }

    // ── Char offsets ↔ rows ──────────────────────────────────────────────

    /// Locate `char_offset`: its display row, and its display column in that
    /// row.
    pub fn locate(&mut self, char_offset: usize) -> (RowPos, u32) {
        debug_assert!(
            char_offset <= self.rope.len_chars(),
            "locate: char_offset {char_offset} is out of range for a buffer \
             of {} chars — ropey's own `char_to_line` panics past this point, \
             so a caller holding a position from an earlier frame (an LSP \
             completion anchor, a stale selection) must revalidate it \
             against the current buffer before reaching here",
            self.rope.len_chars()
        );
        let (line, target_byte) = hume_rope::lines::char_to_line_byte(self.rope, char_offset);
        let before = self.block(line).before;
        // Only up to the target: everything past it is irrelevant to where
        // this one offset sits.
        let idx = self.ensure_formatted(line, FormatBound::ToByte(target_byte));
        let (sub, display_col) = self.locate_in_line(idx, target_byte, char_offset);
        (RowPos::new(line, before + sub), display_col)
    }

    /// Which content sub-row of `idx`'s line holds `target_byte`
    /// (line-relative, resolved by the caller), and at what column. `idx` must
    /// come from an [`RowMap::ensure_formatted`] bounded at least as far as
    /// `ToByte(target_byte)`.
    fn locate_in_line(&self, idx: usize, target_byte: usize, char_offset: usize) -> (usize, u32) {
        let entry = self.store.entry(idx);
        let format = &entry.format;
        let rows = &format.display_rows;
        let graphemes = &format.graphemes;

        for (i, row) in rows.iter().enumerate() {
            if row.graphemes.is_empty() {
                continue;
            }
            let first = &graphemes[row.graphemes.start];
            let last = &graphemes[row.graphemes.end - 1];
            let is_last = i + 1 == rows.len();
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
                    &row.graphemes,
                )
                .map_or_else(
                    // Past every grapheme on the row (end of line).
                    || last.display_col.saturating_add(last.width as u32),
                    |(display_col, _)| display_col,
                );
                return (i, display_col);
            }
        }

        // No row claimed the offset: answer with the end of the last row.
        // Every content row has at least one grapheme (an empty line still
        // gets its EOL sentinel), and the last row's `is_last` branch above
        // matches any `target_byte` at or past its own start — so reaching
        // here means either `rows` is empty or every row was skipped for
        // having no graphemes, both of which indicate a formatting bug
        // rather than a normal input.
        debug_assert!(
            !rows.is_empty(),
            "locate_in_line: line {}, char_offset {char_offset} matched \
             no row — every content row should claim some byte range of the \
             line",
            entry.line
        );
        let last_row = rows.len().saturating_sub(1);
        let display_col = rows
            .get(last_row)
            .filter(|r| !r.graphemes.is_empty())
            .map_or(0, |r| {
                let lg = &graphemes[r.graphemes.end - 1];
                lg.display_col.saturating_add(lg.width as u32)
            });
        (last_row, display_col)
    }

    /// The display row `char_offset` sits on, without resolving its column.
    ///
    /// In `WrapMode::None` a line is exactly one content row (see
    /// [`RowMap::block`]), so the sub-row is always 0 and the answer falls out
    /// of the block breakdown with no formatting at all — the difference
    /// between O(1) and O(offset into the line) for the callers that only want
    /// the row.
    pub fn locate_row(&mut self, char_offset: usize) -> RowPos {
        debug_assert!(
            char_offset <= self.rope.len_chars(),
            "locate_row: char_offset {char_offset} is out of range for a \
             buffer of {} chars — see the debug_assert in RowMap::locate",
            self.rope.len_chars()
        );
        if self.key.wrap_mode.is_wrapping() {
            // Wrapping needs the sub-row, which only formatting can answer —
            // and `block` has already formatted the line to count its rows.
            return self.locate(char_offset).0;
        }
        let line = self.rope.char_to_line(char_offset);
        RowPos::new(line, self.block(line).before)
    }

    /// The char offset `target_display_col` resolves to on `pos`'s row, under
    /// `target`'s policy.
    ///
    /// A virtual row is not buffer content, so `pos` landing on one clamps to
    /// the nearest content sub-row of the same line — the first for a `Before`
    /// row, the last for an `After` row.
    pub fn char_at(
        &mut self,
        pos: RowPos,
        target_display_col: u32,
        target: DisplayColTarget,
    ) -> usize {
        let b = self.block(pos.line);
        let sub = pos
            .row
            .saturating_sub(b.before)
            .min(b.content.saturating_sub(1));
        // Only up to the target column: no cell further right can be the one
        // this column resolves to, under either policy.
        let idx = self.ensure_formatted(pos.line, FormatBound::ToDisplayCol(target_display_col));
        self.resolve_in_row(idx, sub, target_display_col, target)
    }

    /// Shared core of [`RowMap::char_at`] and [`RowMap::char_at_line_display_col`]:
    /// which char offset on content row `sub` of `idx`'s line resolves to
    /// `target_display_col`, under `target`'s policy. `idx` must come from an
    /// [`RowMap::ensure_formatted`] bounded at least up to
    /// `target_display_col`.
    fn resolve_in_row(
        &self,
        idx: usize,
        sub: usize,
        target_display_col: u32,
        target: DisplayColTarget,
    ) -> usize {
        let entry = self.store.entry(idx);
        let line_start = self.rope.line_to_char(entry.line);
        let format = &entry.format;
        let Some(row) = format.display_rows.get(sub) else {
            return line_start;
        };
        let graphemes = &format.graphemes[row.graphemes.clone()];
        if graphemes.is_empty() {
            return line_start;
        }

        match target {
            DisplayColTarget::Cell => {
                graphemes
                    .iter()
                    .find(|g| target_display_col < g.display_col.saturating_add(g.width as u32))
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
                // minimising distance against it elsewhere on the row would
                // land on a character that cell isn't at — excluded outright,
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
                        // Virtual-row cells (segmented separately by
                        // `segment_virtual_row`) have no buffer position at
                        // all; unreachable from `char_at`, which only ever
                        // formats content rows, but guarded defensively.
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

    /// `(indent, span)` for content row `sub` of `idx`'s line.
    /// `indent` is the display column the row's first cell starts
    /// at — 0 on a line's own first row, `indent_display_cols` on a wrap
    /// continuation row (see [`crate::types::Grapheme::display_col`]).
    /// `span` is the row's own content width with that indent excluded, so
    /// summing `span` across every row before `sub`, plus the indent-excluded
    /// offset within `sub`, converts a row-relative column into one relative
    /// to the whole buffer line.
    fn row_shape(&self, idx: usize, sub: usize) -> (u32, u32) {
        let format = self.format_at(idx);
        let Some(row) = format.display_rows.get(sub) else {
            return (0, 0);
        };
        let graphemes = &format.graphemes[row.graphemes.clone()];
        let Some(first) = graphemes.first() else {
            return (0, 0);
        };
        let last = graphemes.last().expect("non-empty checked above");
        let indent = first.display_col;
        let span = last.display_col.saturating_add(last.width as u32) - indent;
        (indent, span)
    }

    /// The display column `char_offset` sits at, measured from its own
    /// buffer line's start rather than from its display row's — the column a
    /// numeric-prefixed vertical move (`9j`/`9k`) latches, since it targets
    /// the same buffer-line column on its landing line regardless of which
    /// row of that (possibly wrapped) line it lands on.
    ///
    /// Continuation-row indent is excluded (see `RowMap::row_shape`) and
    /// inline virtual cells (inlay hints, ghost text) are included, same as
    /// [`RowMap::locate`] — the two differ only in what they're measured
    /// from, and coincide under `WrapMode::None`, where a line is exactly one
    /// row with no indent.
    pub fn line_display_col(&mut self, char_offset: usize) -> u32 {
        debug_assert!(
            char_offset <= self.rope.len_chars(),
            "line_display_col: char_offset {char_offset} is out of range for \
             a buffer of {} chars — see the debug_assert in RowMap::locate",
            self.rope.len_chars()
        );
        let (line, target_byte) = hume_rope::lines::char_to_line_byte(self.rope, char_offset);
        let idx = self.ensure_formatted(line, FormatBound::ToByte(target_byte));
        let (sub, row_display_col) = self.locate_in_line(idx, target_byte, char_offset);
        let (row_indent, _) = self.row_shape(idx, sub);
        let preceding: u32 = (0..sub).map(|j| self.row_shape(idx, j).1).sum();
        preceding + row_display_col.saturating_sub(row_indent)
    }

    /// Inverse of [`RowMap::line_display_col`]: the char offset
    /// `target_line_display_col` resolves to on `line`, under `target`'s
    /// policy.
    ///
    /// A line-relative column past the line's total width clamps to its last
    /// row, where `target`'s own clamp rule (see [`DisplayColTarget`])
    /// applies — the same "stick to the last real character, land on `\n`
    /// only when the line is empty" rule bare `j`/`k` already gets from
    /// [`RowMap::char_at`].
    pub fn char_at_line_display_col(
        &mut self,
        line: usize,
        target_line_display_col: u32,
        target: DisplayColTarget,
    ) -> usize {
        let content_rows = self.block(line).content;
        // Only up to the target column: while wrapping, `ensure_formatted`
        // promotes this to `Full` regardless (a row-relative bound can't
        // usefully clip a line-relative target), and without wrapping
        // `content_rows == 1` so the two columns coincide.
        let idx = self.ensure_formatted(line, FormatBound::ToDisplayCol(target_line_display_col));
        let mut remaining = target_line_display_col;
        let mut sub = 0;
        let mut row_indent = 0;
        for j in 0..content_rows {
            let (indent, span) = self.row_shape(idx, j);
            sub = j;
            row_indent = indent;
            if j + 1 == content_rows || remaining < span {
                break;
            }
            remaining -= span;
        }
        self.resolve_in_row(idx, sub, remaining + row_indent, target)
    }

    /// The char range one content row covers, as `(start, end_exclusive)`.
    /// `None` when `pos` is not a content row.
    ///
    /// Lets a caller scope a line-oriented search (nearest word) to the head's
    /// own visual row instead of the whole buffer line.
    pub fn content_row_char_bounds(&mut self, pos: RowPos) -> Option<(usize, usize)> {
        let b = self.block(pos.line);
        let sub = pos.row.checked_sub(b.before)?;
        if sub >= b.content {
            return None;
        }
        // `Full`: this reads the *next* row's first char to bound the current
        // one, so it needs every row the line produces.
        let idx = self.ensure_formatted(pos.line, FormatBound::Full);

        let format = self.format_at(idx);
        let rows = &format.display_rows;
        let graphemes = &format.graphemes;
        let first_char_of = |row: &DisplayRow| {
            graphemes[row.graphemes.clone()]
                .iter()
                .filter(|g| g.char_offset != usize::MAX)
                .map(|g| g.char_offset)
                .min()
        };

        let start = first_char_of(rows.get(sub)?)?;
        let end = rows
            .get(sub + 1)
            .and_then(first_char_of)
            .unwrap_or_else(|| hume_rope::lines::line_end_exclusive(self.rope, pos.line));
        Some((start, end))
    }

    // ── Render access ────────────────────────────────────────────────────

    /// Borrow what the render stage needs to style and compose `pos`.
    ///
    /// Content rows come from the cached format, so a line is formatted once
    /// however many of its rows get rendered. Virtual rows are segmented here
    /// — the same grapheme/width/column bookkeeping `format_buffer_line` does
    /// for real lines, so a provider handing over plain text and scoped byte
    /// ranges cannot get that arithmetic wrong.
    pub fn render_row(&mut self, pos: RowPos) -> RenderRow<'_> {
        let (idx, slot) = self.resolve(pos);
        match slot {
            BlockSlot::Content(sub) => {
                // `Full`: the render stage emits whole rows, and its own
                // clipping is the map's `h_window`, applied inside the format.
                self.ensure_format_at(idx, FormatBound::Full);
                let format = self.format_at(idx);
                RenderRow {
                    row: &format.display_rows[sub],
                    graphemes: &format.graphemes,
                    line_text: &format.line_texts,
                    virtual_texts: &format.virtual_texts,
                    base_scope: None,
                }
            }
            BlockSlot::Before(i) => self.segment_virtual_row(idx, i),
            // `resolve` already walked this line's block, so its `before`
            // count is on the entry it handed back — no need to walk it again.
            BlockSlot::After(i) => {
                let before = self.store.entry(idx).before;
                self.segment_virtual_row(idx, before + i)
            }
        }
    }

    /// Lay one virtual row out into its own scratch and borrow it back.
    ///
    /// Uses the store's `virtual_row`, not the line's own format: a `Before`
    /// row renders ahead of its line's content rows, which are very likely
    /// already formatted (`block` runs the formatter in wrapping mode to
    /// count wrap rows) — laying the virtual row out over them would destroy
    /// that and force a reformat of the content rows that follow.
    fn segment_virtual_row(&mut self, idx: usize, vl_idx: usize) -> RenderRow<'_> {
        let tab_width = self.key.tab_width;
        // The entry's virtual rows and the scratch they lay out into are
        // disjoint parts of the store, borrowed together so the row's text
        // can be read while its cells are written.
        let (entry, vrow) = self.store.entry_and_virtual_row(idx);
        let vl = &entry.virtual_lines[vl_idx];
        let anchor_line = entry.line;
        let provider_id = vl.provider_id;
        let base_scope = vl.base_scope;
        vrow.clear();

        // `vl.segments` was sorted by `block()` at intake, and
        // `grapheme_indices` yields byte offsets in ascending order, so a
        // single monotonic cursor resolves every grapheme's scope in
        // O(graphemes + segments) instead of a per-grapheme linear scan.
        let mut scope_cursor = crate::style::highlight::IntervalCursor::new(&vl.segments);
        let mut display_col: u32 = 0;
        crate::format::push_virtual_cells(
            &mut vrow.texts,
            &mut vrow.graphemes,
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

        let row = vrow.row.insert(DisplayRow {
            kind: crate::types::RowKind::Virtual {
                provider_id,
                anchor_line,
            },
            graphemes: 0..vrow.graphemes.len(),
        });

        RenderRow {
            row,
            graphemes: &vrow.graphemes,
            // A virtual row has no buffer text — every cell resolves out of
            // `virtual_texts` instead: `Virtual` text itself, or the
            // `Placeholder` a control character becomes. A tab needs no
            // arena lookup at all — it's `TabFill`, drawn as blanks directly.
            line_text: "",
            virtual_texts: &vrow.texts,
            base_scope,
        }
    }

    // ── Formatting ───────────────────────────────────────────────────────

    /// Guarantee `line`'s entry holds its content rows, formatted at least
    /// as far as `bound` reaches. Returns the entry it resolved, so a read
    /// accessor can be handed the line by value.
    fn ensure_formatted(&mut self, line: usize, bound: FormatBound) -> usize {
        let idx = self.block_entry(line);
        self.ensure_format_at(idx, bound);
        idx
    }

    /// [`RowMap::ensure_formatted`] for an entry already in hand.
    ///
    /// Split out so [`RowMap::content_rows`] can format while counting
    /// without re-finding the entry it is already holding.
    fn ensure_format_at(&mut self, idx: usize, bound: FormatBound) {
        debug_assert!(
            self.h_window.is_none() || matches!(bound, FormatBound::Full),
            "a bounded query on an h_window map would clip twice — the render \
             path bounds its own formats by window and never asks for one"
        );
        let line = self.store.entry(idx).line;
        // Any wrapping mode needs the whole line: a clipped scan would emit
        // fewer rows than the count `block` already committed to. Applied
        // before the check *and* the record below, so a wrapping query never
        // stores a bound narrower than what it actually ran.
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
        // participate in wrapping, so counting rows without them makes the row
        // list disagree with what the renderer emits the moment an inlay hint
        // pushes a line past the wrap column.
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
            line,
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
