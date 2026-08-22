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
//! threading eight-to-eleven parameters through every walk, and it caches the
//! line it last looked at so stepping within one line's block is free.
//!
//! Addresses are [`RowPos`]: a buffer line plus a row index into that line's
//! *visual block*, which runs `before`-virtuals, then content/wrap rows, then
//! `after`-virtuals. `ViewportState`'s `top_line`/`top_row_offset` pair is the
//! persisted form of exactly that address.

use std::ops::Range;

use ropey::Rope;

use crate::format::{FormatBound, FormatScratch, format_buffer_line};
use crate::pane::{WhitespaceConfig, WrapMode};
use crate::providers::{
    Decoration, DecorationKinds, InlineInsert, ProviderSet, VirtualLine, VirtualLineAnchor,
};
use crate::types::{CellContent, DisplayRow, Grapheme, ScopeId};

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

/// What a display row is. The payload is the row's index within its own group,
/// so `Content(2)` is a line's third content row and `Before(0)` is the first
/// virtual row above it.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum RowKind {
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

/// The line [`RowMap`] last resolved, with whatever it learned about it.
struct CachedLine {
    line: usize,
    breakdown: RowsBreakdown,
    /// This line's virtual rows, `Before` ones first — the order
    /// [`VirtualLineAnchor::sort_key`] imposes, so the `i`th `After` row is at
    /// index `before + i`.
    virtual_lines: Vec<VirtualLine>,
    /// How much of this line the scratch's `display_rows`/`graphemes`/
    /// `line_texts`/`virtual_texts` currently hold. `None` until it is
    /// formatted at all; otherwise the bound the scan was run to, which is a
    /// *lower* bound on what is really there (a bounded scan that never hit
    /// its stop walked the whole line).
    extent: Option<FormatBound>,
}

/// Everything the render stage needs to style and compose one display row.
pub struct RenderRow<'m> {
    pub row: &'m DisplayRow,
    /// The graphemes `row.graphemes` indexes into.
    pub graphemes: &'m [Grapheme],
    /// Buffer-line text that the row's real graphemes index by byte range.
    /// Empty for virtual rows, which have no buffer text.
    pub line_text: &'m str,
    /// Arena backing `Indicator`/`Virtual` cell text.
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
    /// Always resolved — [`WrapMode::wrap_width`] panics on the `width: 0`
    /// sentinel, and [`RowMap::new`] is the one funnel every consumer passes
    /// through, so it resolves there rather than trusting callers to.
    wrap_mode: WrapMode,
    tab_width: u8,
    whitespace: WhitespaceConfig,
    providers: &'a ProviderSet,
    content_width: u16,
    h_window: Option<Range<u32>>,
    scratch: &'a mut FormatScratch,
    /// Inline inserts for the line currently being formatted. Reused across
    /// the lines one map visits.
    inline_inserts: Vec<InlineInsert>,
    /// Scratch for one `DecorationSource::decorations_for_line` call at a
    /// time — drained into `virtual_lines`/`inline_inserts` immediately
    /// after, so this stays empty between calls. Reused across providers and
    /// lines to avoid a per-call allocation.
    decorations: Vec<Decoration>,
    cached: Option<CachedLine>,
}

impl<'a> RowMap<'a> {
    pub fn new(
        rope: &'a Rope,
        wrap_mode: WrapMode,
        tab_width: u8,
        whitespace: WhitespaceConfig,
        providers: &'a ProviderSet,
        content_width: u16,
        scratch: &'a mut FormatScratch,
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
        Self {
            rope,
            wrap_mode: wrap_mode.resolve(content_width),
            tab_width,
            whitespace,
            providers,
            content_width,
            h_window: None,
            scratch,
            inline_inserts: Vec::new(),
            decorations: Vec::new(),
            cached: None,
        }
    }

    /// Clip `WrapMode::None` formatting to a horizontal column window — the
    /// render path's bound on arbitrarily long unwrapped lines.
    ///
    /// Row counts are unaffected (no-wrap is one content row however wide the
    /// line is), so this changes only which graphemes the render accessors
    /// emit. Editor-side consumers want whole lines and leave it `None`.
    pub fn with_h_window(mut self, h_window: Option<Range<u32>>) -> Self {
        debug_assert!(
            h_window.is_none() || !self.wrap_mode.is_wrapping(),
            "with_h_window is a WrapMode::None-only clip — a wrapping RowMap \
             would silently under-count content rows, since format_line \
             passes h_window through to the formatter even while wrapping"
        );
        self.h_window = h_window;
        self
    }

    pub fn is_wrapping(&self) -> bool {
        self.wrap_mode.is_wrapping()
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
        if let Some(c) = &self.cached
            && c.line == line
        {
            return c.breakdown;
        }

        // Recycle the previous line's buffer instead of allocating per line.
        let mut virtual_lines = match self.cached.take() {
            Some(c) => {
                let mut v = c.virtual_lines;
                v.clear();
                v
            }
            None => Vec::new(),
        };

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
        let after = virtual_lines.len() - before;

        // `WrapMode::None` is always exactly one content row, and formatting
        // cannot return another answer there — so counting never runs the
        // formatter. That is the difference between O(1) and O(line length)
        // per query on a minified line megabytes wide.
        let content = if self.wrap_mode.is_wrapping() {
            // `Full`: the row count *is* the output, so nothing may be clipped.
            self.format_line(line, FormatBound::Full);
            self.scratch.display_rows.len()
        } else {
            1
        };
        debug_assert!(
            content >= 1,
            "line {line} counted zero content rows; every line occupies at least one"
        );

        let breakdown = RowsBreakdown {
            before,
            content,
            after,
        };
        self.cached = Some(CachedLine {
            line,
            breakdown,
            virtual_lines,
            extent: self.wrap_mode.is_wrapping().then_some(FormatBound::Full),
        });
        breakdown
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

    /// What kind of row `pos` addresses.
    pub fn kind(&mut self, pos: RowPos) -> RowKind {
        let b = self.block(pos.line);
        debug_assert!(
            pos.row < b.total(),
            "row {} is past line {}'s block of {}",
            pos.row,
            pos.line,
            b.total()
        );
        if pos.row < b.before {
            RowKind::Before(pos.row)
        } else if pos.row < b.before + b.content {
            RowKind::Content(pos.row - b.before)
        } else {
            RowKind::After(pos.row - b.before - b.content)
        }
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
        self.ensure_formatted(line, FormatBound::ToByte(target_byte));
        let (sub, display_col) = self.locate_in_line(line, target_byte, char_offset);
        (RowPos::new(line, before + sub), display_col)
    }

    /// Which content sub-row of `line` holds `target_byte` (line-relative,
    /// resolved by the caller), and at what column. Requires `line` to be
    /// formatted into the scratch at least as far as `ToByte(target_byte)`.
    fn locate_in_line(&self, line: usize, target_byte: usize, char_offset: usize) -> (usize, u32) {
        let rows = &self.scratch.display_rows;
        let graphemes = &self.scratch.graphemes;

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
            "locate_in_line: line {line}, char_offset {char_offset} matched \
             no row — every content row should claim some byte range of the \
             line"
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
        if self.wrap_mode.is_wrapping() {
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
        self.ensure_formatted(pos.line, FormatBound::ToDisplayCol(target_display_col));

        let line_start = self.rope.line_to_char(pos.line);
        let Some(row) = self.scratch.display_rows.get(sub) else {
            return line_start;
        };
        let graphemes = &self.scratch.graphemes[row.graphemes.clone()];
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
                // Eligibility by content type: `Grapheme`/`WidthContinuation`
                // are real content, always eligible. `Empty` (EOL sentinel)
                // has a buffer position but isn't content, so it only
                // answers when nothing else can (an empty line) — gated on
                // `admit_eol`. `Virtual` (inline-insert) carries the real
                // grapheme's `char_offset` it precedes, so minimising
                // distance against it elsewhere on the row would land on a
                // character that cell isn't at — excluded outright, not just
                // deprioritised. `Indicator` covers tab/space glyphs, which
                // *are* real content, except the newline indicator, which
                // shares the EOL sentinel's column and must be excluded the
                // same way — singled out by `byte_range` being empty, just
                // like the sentinel it's drawn on top of (`format.rs`'s
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
                            CellContent::Grapheme | CellContent::WidthContinuation => true,
                            CellContent::Empty => admit_eol,
                            CellContent::Virtual { .. } => false,
                            CellContent::Indicator { .. } => !g.byte_range.is_empty(),
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
        self.ensure_formatted(pos.line, FormatBound::Full);

        let rows = &self.scratch.display_rows;
        let graphemes = &self.scratch.graphemes;
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
        match self.kind(pos) {
            RowKind::Content(sub) => {
                // `Full`: the render stage emits whole rows, and its own
                // clipping is the map's `h_window`, applied inside the format.
                self.ensure_formatted(pos.line, FormatBound::Full);
                RenderRow {
                    row: &self.scratch.display_rows[sub],
                    graphemes: &self.scratch.graphemes,
                    line_text: &self.scratch.line_texts,
                    virtual_texts: &self.scratch.virtual_texts,
                    base_scope: None,
                }
            }
            RowKind::Before(i) => self.segment_virtual_row(pos.line, i),
            RowKind::After(i) => {
                let before = self.block(pos.line).before;
                self.segment_virtual_row(pos.line, before + i)
            }
        }
    }

    /// Lay one virtual row out into its own scratch and borrow it back.
    ///
    /// Uses `FormatScratch::virtual_row`, not the content-line buffers: a
    /// `Before` row renders ahead of its line's content rows, which may
    /// already be formatted and cached (`block` runs the formatter in
    /// wrapping mode to count wrap rows) — clobbering the shared buffers
    /// here would destroy that cached format and force a redundant reformat
    /// of the content rows that follow.
    fn segment_virtual_row(&mut self, line: usize, vl_idx: usize) -> RenderRow<'_> {
        let cached = self
            .cached
            .as_ref()
            .expect("kind() resolved this line's block");
        let vl = &cached.virtual_lines[vl_idx];
        let provider_id = vl.provider_id;
        let base_scope = vl.base_scope;
        let tab_width = self.tab_width;
        let vrow = &mut self.scratch.virtual_row;
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
                byte_range: 0..0, // zero-length: virtual, no buffer position
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
                anchor_line: line,
            },
            graphemes: 0..vrow.graphemes.len(),
        });

        RenderRow {
            row,
            graphemes: &vrow.graphemes,
            // A virtual row has no buffer text — every cell resolves out of
            // `virtual_texts` instead, whether it is `Virtual` or the
            // `Indicator` a tab or control character becomes.
            line_text: "",
            virtual_texts: &vrow.texts,
            base_scope,
        }
    }

    // ── Formatting ───────────────────────────────────────────────────────

    /// Guarantee the scratch holds `line`'s content rows, formatted at least
    /// as far as `bound` reaches.
    fn ensure_formatted(&mut self, line: usize, bound: FormatBound) {
        debug_assert!(
            self.h_window.is_none() || matches!(bound, FormatBound::Full),
            "a bounded query on an h_window map would clip twice — the render \
             path bounds its own formats by window and never asks for one"
        );
        // Any wrapping mode needs the whole line: a clipped scan would emit
        // fewer rows than the count `block` already committed to. Applied
        // before the check *and* the record below, so a wrapping query never
        // stores a bound narrower than what it actually ran.
        let bound = if self.wrap_mode.is_wrapping() {
            FormatBound::Full
        } else {
            bound
        };
        // After `block`, which either confirms the cache or replaces it.
        let breakdown = self.block(line);
        if self
            .cached
            .as_ref()
            .is_some_and(|c| c.extent.is_some_and(|e| e.covers(bound)))
        {
            return;
        }
        self.format_line(line, bound);
        debug_assert_eq!(
            self.scratch.display_rows.len(),
            breakdown.content,
            "line {line} formatted to a different row count than it was counted at"
        );
        self.cached
            .as_mut()
            .expect("block() above populated the cache")
            .extent = Some(bound);
    }

    /// Format `line`'s content rows into the scratch.
    ///
    /// Inline inserts are queried and passed here, not just at render time:
    /// they participate in wrapping, so counting rows without them makes the
    /// row list disagree with what the renderer emits the moment an inlay hint
    /// pushes a line past the wrap column.
    fn format_line(&mut self, line: usize, bound: FormatBound) {
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

        self.scratch.clear_line_bufs();
        self.scratch.line_texts.clear();
        format_buffer_line(
            self.rope,
            line,
            self.tab_width,
            &self.whitespace,
            &self.wrap_mode,
            self.h_window.clone(),
            bound,
            &self.inline_inserts,
            &mut *self.scratch,
        );
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests;
