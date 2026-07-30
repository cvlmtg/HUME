//! The single authority on the document's display-row list.
//!
//! A document's rows come from two independent sources: a buffer line's own
//! content rows (one per wrap row — exactly one when wrapping is off), and
//! virtual rows contributed by [`VirtualLineSource`](crate::providers::VirtualLineSource)
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
use unicode_segmentation::UnicodeSegmentation;

use crate::format::{FormatScratch, format_buffer_line, push_arena_text, unicode_display_width};
use crate::pane::{WhitespaceConfig, WrapMode};
use crate::providers::{InlineInsert, ProviderSet, VirtualLine, VirtualLineAnchor};
use crate::types::{CellContent, DisplayRow, Grapheme};

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
pub enum ColTarget {
    /// The cell that *contains* this column — what a mouse click asks. A
    /// column inside a wide cell (a tab's expanse, a double-width glyph)
    /// resolves to that cell, and the end-of-line sentinel is a valid landing
    /// spot, so clicking past a line's text puts the cursor on its `\n`
    /// (a real cursor position in HUME's inclusive selection model).
    Cell,
    /// The real grapheme whose start column is *nearest* this one — what a
    /// sticky-column `j`/`k` asks, since it minimises visual column drift.
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
    /// Whether the scratch's `display_rows`/`graphemes`/`line_texts` currently
    /// hold this line's formatted content rows.
    formatted: bool,
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
    h_window: Option<Range<u16>>,
    scratch: &'a mut FormatScratch,
    /// Inline inserts for the line currently being formatted. Reused across
    /// the lines one map visits.
    inline_inserts: Vec<InlineInsert>,
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
            rope.len_chars() == 0 || rope.char(rope.len_chars() - 1) == '\n',
            "RowMap requires a trailing '\\n' (the buffer invariant) — \
             without it `last_line`'s `len_lines() - 2` drops the rope's \
             actual last content line"
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
            cached: None,
        }
    }

    /// Clip `WrapMode::None` formatting to a horizontal column window — the
    /// render path's bound on arbitrarily long unwrapped lines.
    ///
    /// Row counts are unaffected (no-wrap is one content row however wide the
    /// line is), so this changes only which graphemes the render accessors
    /// emit. Editor-side consumers want whole lines and leave it `None`.
    pub fn with_h_window(mut self, h_window: Option<Range<u16>>) -> Self {
        self.h_window = h_window;
        self
    }

    pub fn wrap_mode(&self) -> WrapMode {
        self.wrap_mode
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

        for (id, provider) in &self.providers.virtual_lines {
            let start = virtual_lines.len();
            provider.virtual_lines(line..line + 1, self.content_width, &mut virtual_lines);
            // Never trust a provider's self-reported id: it could name another
            // provider's rows, which the gutter would then attribute wrongly.
            for vl in &mut virtual_lines[start..] {
                vl.provider_id = *id;
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
            self.format_line(line);
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
            formatted: self.wrap_mode.is_wrapping(),
        });
        breakdown
    }

    /// Index of the last buffer line a cursor can occupy.
    ///
    /// Every HUME buffer ends with a structural `\n`, so ropey reports one
    /// extra empty line past the content; the last real line is
    /// `len_lines() - 2`. This is the one place that rule lives.
    pub fn last_line(&self) -> usize {
        self.rope.len_lines().saturating_sub(2)
    }

    /// The document's last display row — the overscroll clamp target.
    ///
    /// Vim-style: the final block row may scroll all the way up to the top of
    /// the viewport, so this is itself a valid viewport address rather than a
    /// bound one row past the end.
    pub fn last_pos(&mut self) -> RowPos {
        let line = self.last_line();
        let total = self.block(line).total();
        RowPos::new(line, total.saturating_sub(1))
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
        let mut cur = self.clamp(pos);
        let steps = delta.unsigned_abs();
        for _ in 0..steps {
            let stepped = if delta >= 0 {
                self.next(cur)
            } else {
                self.prev(cur)
            };
            match stepped {
                Some(next) => cur = next,
                None => break,
            }
        }
        cur
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
    pub fn locate(&mut self, char_offset: usize) -> (RowPos, u16) {
        let line = self.rope.char_to_line(char_offset);
        let before = self.block(line).before;
        self.ensure_formatted(line);
        let (sub, col) = self.locate_in_line(line, char_offset);
        (RowPos::new(line, before + sub), col)
    }

    /// Which content sub-row of `line` holds `char_offset`, and at what column.
    /// Requires `line` to be formatted into the scratch.
    fn locate_in_line(&self, line: usize, char_offset: usize) -> (usize, u16) {
        let line_start_byte = self.rope.char_to_byte(self.rope.line_to_char(line));
        let target_byte = self
            .rope
            .char_to_byte(char_offset)
            .saturating_sub(line_start_byte);

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
                // its `char_offset` — `style::resolve_grapheme_col` skips
                // forward past any `Virtual` cells to reach it, the same rule
                // `style::char_offset_to_col` applies for selection styling.
                let col =
                    crate::style::resolve_grapheme_col(char_offset, graphemes, &row.graphemes)
                        .map_or_else(
                            // Past every grapheme on the row (end of line).
                            || last.col.saturating_add(last.width as u16),
                            |(col, _)| col,
                        );
                return (i, col);
            }
        }

        // No row claimed the offset: answer with the end of the last row.
        let last_row = rows.len().saturating_sub(1);
        let col = rows
            .get(last_row)
            .filter(|r| !r.graphemes.is_empty())
            .map_or(0, |r| {
                let lg = &graphemes[r.graphemes.end - 1];
                lg.col.saturating_add(lg.width as u16)
            });
        (last_row, col)
    }

    /// The char offset `target_col` resolves to on `pos`'s row, under
    /// `target`'s policy.
    ///
    /// A virtual row is not buffer content, so `pos` landing on one clamps to
    /// the nearest content sub-row of the same line — the first for a `Before`
    /// row, the last for an `After` row.
    pub fn char_at(&mut self, pos: RowPos, target_col: u16, target: ColTarget) -> usize {
        let b = self.block(pos.line);
        let sub = pos
            .row
            .saturating_sub(b.before)
            .min(b.content.saturating_sub(1));
        self.ensure_formatted(pos.line);

        let line_start = self.rope.line_to_char(pos.line);
        let Some(row) = self.scratch.display_rows.get(sub) else {
            return line_start;
        };
        let graphemes = &self.scratch.graphemes[row.graphemes.clone()];
        if graphemes.is_empty() {
            return line_start;
        }

        match target {
            ColTarget::Cell => {
                graphemes
                    .iter()
                    .find(|g| target_col < g.col.saturating_add(g.width as u16))
                    .unwrap_or_else(|| graphemes.last().expect("non-empty checked above"))
                    .char_offset
            }
            ColTarget::NearestContent => {
                // Virtual fill cells have no buffer position at all; the
                // end-of-line sentinel has one but is not content, so it only
                // answers when nothing else can (an empty line). An
                // inline-insert (`Virtual`) cell carries the `char_offset` of
                // the *real* grapheme it precedes — a column elsewhere on the
                // row minimising distance against it would land on a
                // character this cell isn't at, so it's excluded outright
                // rather than merely deprioritised.
                let nearest = |admit_eol: bool| {
                    graphemes
                        .iter()
                        .filter(|g| g.char_offset != usize::MAX)
                        .filter(|g| admit_eol || !matches!(g.content, CellContent::Empty))
                        .filter(|g| !matches!(g.content, CellContent::Virtual { .. }))
                        .min_by_key(|g| target_col.abs_diff(g.col))
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
        self.ensure_formatted(pos.line);

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
        // Every HUME buffer ends with `\n`, so `line + 1` is always a line.
        let end = rows
            .get(sub + 1)
            .and_then(first_char_of)
            .unwrap_or_else(|| self.rope.line_to_char(pos.line + 1));
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
                self.ensure_formatted(pos.line);
                RenderRow {
                    row: &self.scratch.display_rows[sub],
                    graphemes: &self.scratch.graphemes,
                    line_text: &self.scratch.line_texts,
                    virtual_texts: &self.scratch.virtual_texts,
                }
            }
            RowKind::Before(i) => self.segment_virtual_row(pos.line, i),
            RowKind::After(i) => {
                let before = self.block(pos.line).before;
                self.segment_virtual_row(pos.line, before + i)
            }
        }
    }

    /// Lay one virtual row out into the scratch and borrow it back.
    fn segment_virtual_row(&mut self, line: usize, vl_idx: usize) -> RenderRow<'_> {
        // Laying out a virtual row reuses the same row/grapheme buffers a
        // content line formats into, so whatever was formatted is gone.
        self.scratch.clear_line_bufs();
        if let Some(c) = &mut self.cached {
            c.formatted = false;
        }

        let cached = self
            .cached
            .as_ref()
            .expect("kind() resolved this line's block");
        let vl = &cached.virtual_lines[vl_idx];
        let provider_id = vl.provider_id;
        let scratch = &mut *self.scratch;

        // One copy of the row's text into the arena; each cell then names a
        // sub-range of it.
        let (arena_base, _) = push_arena_text(&mut scratch.virtual_texts, &vl.text);

        let mut col: u16 = 0;
        for (byte_offset, grapheme_str) in vl.text.grapheme_indices(true) {
            let width = unicode_display_width(grapheme_str).clamp(1, 2) as u8;
            let scope = vl
                .segments
                .iter()
                .find(|(range, _)| range.contains(&byte_offset))
                .map(|(_, scope)| *scope);
            let start = arena_base.saturating_add(u32::try_from(byte_offset).unwrap_or(u32::MAX));
            let len = u16::try_from(grapheme_str.len()).unwrap_or(u16::MAX);

            scratch.graphemes.push(Grapheme {
                byte_range: 0..0, // zero-length: virtual, no buffer position
                char_offset: usize::MAX,
                col,
                width,
                content: CellContent::Virtual { start, len },
                indent_depth: 0,
                scope,
            });
            col = col.saturating_add(width as u16);
            if width == 2 {
                // Both cells of a double-wide glyph stay on this row.
                scratch.graphemes.push(Grapheme {
                    byte_range: 0..0,
                    char_offset: usize::MAX,
                    col,
                    width: 0,
                    content: CellContent::WidthContinuation,
                    indent_depth: 0,
                    scope: None,
                });
            }
        }

        scratch.display_rows.push(DisplayRow {
            kind: crate::types::RowKind::Virtual {
                provider_id,
                anchor_line: line,
            },
            graphemes: 0..scratch.graphemes.len(),
        });

        RenderRow {
            row: scratch.display_rows.last().expect("pushed above"),
            graphemes: &scratch.graphemes,
            // A virtual row has no buffer text; every cell of it is `Virtual`.
            line_text: "",
            virtual_texts: &scratch.virtual_texts,
        }
    }

    // ── Formatting ───────────────────────────────────────────────────────

    /// Guarantee the scratch holds `line`'s formatted content rows.
    fn ensure_formatted(&mut self, line: usize) {
        let breakdown = self.block(line);
        if self.cached.as_ref().is_some_and(|c| c.formatted) {
            return;
        }
        self.format_line(line);
        debug_assert_eq!(
            self.scratch.display_rows.len(),
            breakdown.content,
            "line {line} formatted to a different row count than it was counted at"
        );
        if let Some(c) = &mut self.cached {
            c.formatted = true;
        }
    }

    /// Format `line`'s content rows into the scratch.
    ///
    /// Inline inserts are queried and passed here, not just at render time:
    /// they participate in wrapping, so counting rows without them makes the
    /// row list disagree with what the renderer emits the moment an inlay hint
    /// pushes a line past the wrap column.
    fn format_line(&mut self, line: usize) {
        self.inline_inserts.clear();
        for (_, provider) in &self.providers.inline_decorations {
            provider.decorations_for_line(line, &mut self.inline_inserts);
        }
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
