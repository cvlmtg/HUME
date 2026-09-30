//! Cluster position <-> display line conversions. Internally self-contained past
//! the map's own private fields and its
//! `content_line_of`/`ensure_formatted`/`format_at` (all defined in the
//! parent, reachable here as a descendant of `display_lines`).

use hume_rope::cluster::{ClusterBound, ClusterRange, ClusterStart};
use hume_rope::column::{BufferLineCol, ByteCol, DisplayLineCol};
use hume_rope::line::{ContentLine, RopeyLine};

use crate::format::FormatBound;
use crate::types::{CellContent, DisplayLine};

use super::DisplayLineMap;
use super::pos::{DisplayColTarget, DisplayLinePos};

impl<'a> DisplayLineMap<'a> {
    /// Locate `pos`: its display line, and its display column in that
    /// display line.
    pub fn locate(&mut self, pos: ClusterStart) -> (DisplayLinePos, DisplayLineCol) {
        self.debug_assert_in_text(pos);
        let (ropey_line, target_byte) =
            hume_rope::lines::char_to_line_byte(self.rope, pos.offset());
        let line = self.content_line_of(ropey_line);
        let before = self.block(line).before;
        // Only up to the target: everything past it is irrelevant to where
        // this one position sits.
        let idx = self.ensure_formatted(line, FormatBound::ToByte(target_byte));
        let (sub, display_col) = self.locate_in_line(idx, target_byte, pos);
        (DisplayLinePos::new(line, before + sub), display_col)
    }

    /// A position is typed for the text it was read from; one held across
    /// an edit (an LSP completion anchor, a stale selection) must be
    /// revalidated against the current buffer before it reaches here.
    fn debug_assert_in_text(&self, pos: ClusterStart) {
        debug_assert!(
            pos.offset().index() < self.rope.len_chars(),
            "{pos:?} lies past the end of a buffer of {} chars",
            self.rope.len_chars()
        );
    }

    /// Which content display line of `idx`'s line holds `target_byte`
    /// (line-relative, resolved by the caller), and at what column. `idx` must
    /// come from an [`DisplayLineMap::ensure_formatted`] bounded at least as far as
    /// `ToByte(target_byte)`.
    fn locate_in_line(
        &self,
        idx: usize,
        target_byte: ByteCol,
        pos: ClusterStart,
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
                // its `pos`: `style::resolve_grapheme_display_col` skips
                // forward past any `Virtual` cells to reach it, the same rule
                // selection styling applies.
                let display_col =
                    crate::style::resolve_grapheme_display_col(pos, graphemes, &dline.graphemes)
                        .map_or_else(
                            // Past every grapheme on the display line (end of line).
                            || last.display_col.advance_saturating(last.width as u32),
                            |(display_col, _)| display_col,
                        );
                return (i, display_col);
            }
        }

        // No display line claimed the offset: answer with the end of the
        // last one. Every content display line has at least one grapheme
        // (an empty line still gets its EOL sentinel), and the last one's
        // `is_last` branch above matches any `target_byte` at or past its
        // own start, so reaching here means either `lines` is empty or
        // every display line was skipped for having no graphemes, both of
        // which indicate a formatting bug rather than a normal input.
        debug_assert!(
            !lines.is_empty(),
            "locate_in_line: line {}, {pos:?} matched \
             no display line; every content display line should claim \
             some byte range of the line",
            entry.line.index()
        );
        let last_dline = lines.len().saturating_sub(1);
        let display_col = lines
            .get(last_dline)
            .filter(|r| !r.graphemes.is_empty())
            .map_or(DisplayLineCol::new(0), |r| {
                let lg = &graphemes[r.graphemes.end - 1];
                lg.display_col.advance_saturating(lg.width as u32)
            });
        (last_dline, display_col)
    }

    /// The display line `pos` sits on, without resolving its column.
    ///
    /// In `WrapMode::None` a line is exactly one content display line (see
    /// [`DisplayLineMap::block`]), so the sub-index is always 0 and the
    /// answer falls out of the block breakdown with no formatting at all:
    /// the difference between O(1) and O(offset into the line) for the
    /// callers that only want the display line.
    pub fn locate_display_line(&mut self, pos: ClusterStart) -> DisplayLinePos {
        self.debug_assert_in_text(pos);
        if self.key.wrap_mode.is_wrapping() {
            // Wrapping needs the sub-index, which only formatting can
            // answer, and `block` has already formatted the line to count
            // its display lines.
            return self.locate(pos).0;
        }
        let line = self.content_line_of(hume_rope::lines::char_to_ropey_line(
            self.rope,
            pos.offset(),
        ));
        DisplayLinePos::new(line, self.block(line).before)
    }

    /// The cluster `target_display_col` resolves to on `pos`'s display line,
    /// under `target`'s policy.
    ///
    /// A virtual display line is not buffer content, so `pos` landing on
    /// one clamps to the nearest content sub-line of the same line: the
    /// first for a `Before` display line, the last for an `After` one.
    pub fn char_at(
        &mut self,
        pos: DisplayLinePos,
        target_display_col: DisplayLineCol,
        target: DisplayColTarget,
    ) -> ClusterStart {
        let b = self.block(pos.line);
        let sub = pos
            .slot
            .saturating_sub(b.before)
            .min(b.content.saturating_sub(1));
        // Only up to the target column: no cell further right can be the one
        // this column resolves to, under either policy.
        let idx = self.ensure_formatted(pos.line, FormatBound::ToDisplayCol(target_display_col));
        self.resolve_in_display_line(idx, sub, target_display_col, target)
    }

    /// Which cluster on content display line `sub` of `idx`'s line resolves
    /// to `target_display_col`, under `target`'s policy. `idx` must come
    /// from an [`DisplayLineMap::ensure_formatted`] bounded at least up to
    /// `target_display_col`.
    fn resolve_in_display_line(
        &self,
        idx: usize,
        sub: usize,
        target_display_col: DisplayLineCol,
        target: DisplayColTarget,
    ) -> ClusterStart {
        let entry = self.store.entry(idx);
        let line_start = hume_rope::lines::line_start(self.rope, entry.line);
        let format = &entry.format;
        let Some(dline) = format.display_lines.get(sub) else {
            return line_start;
        };
        let graphemes = &format.graphemes[dline.graphemes.clone()];
        if graphemes.is_empty() {
            return line_start;
        }

        match target {
            DisplayColTarget::Cell => graphemes
                .iter()
                .find(|g| target_display_col < g.display_col.advance_saturating(g.width as u32))
                .unwrap_or_else(|| graphemes.last().expect("non-empty checked above"))
                .pos
                .unwrap_or(line_start),
            DisplayColTarget::NearestContent => {
                // Eligibility by content type: `Grapheme` is real content,
                // always eligible. `WidthContinuation` is excluded even
                // though it shares its primary's `pos` (so admitting
                // it can never answer anything the primary itself wouldn't):
                // its `display_col` sits one column *past* the wide glyph,
                // which is exactly where the next real cell starts, so a
                // target landing on that boundary ties between the two, and
                // `min_by_key` keeps the first tied element, which is the
                // continuation (pushed immediately after its primary, ahead
                // of whatever comes next). Left in, that tie silently wins
                // over the following cell's own, distinct `pos`.
                // `Empty` (EOL sentinel) has a buffer position but isn't
                // content, so it only answers when nothing else can (an empty
                // line), gated on `admit_eol`. `Virtual` (inline-insert)
                // carries the cluster of the real grapheme it precedes, so
                // minimising distance against it elsewhere on the display
                // line would land on a character that cell isn't at. It's excluded outright,
                // not just deprioritised. `Whitespace`/`TabFill` cover
                // tab/space glyphs and blank tab fill, which *are* real
                // content, except the newline indicator, which shares the
                // EOL sentinel's column and must be excluded the same way,
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
                        .filter(|g| g.pos.is_some())
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
                        .and_then(|g| g.pos)
                };
                nearest(false)
                    .or_else(|| nearest(true))
                    .unwrap_or(line_start)
            }
        }
    }

    /// `(indent, span)` for content display line `sub` of `idx`'s line.
    /// `indent` is the display column the display line's first cell starts
    /// at: 0 on a line's own first display line, `indent_display_cols` on
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
            .advance_saturating(last.width as u32)
            .cells_since(indent);
        (indent, span)
    }

    /// The display column `pos` sits at, measured from its own
    /// buffer line's start rather than from its display line's. That's the
    /// column a numeric-prefixed vertical move (`9j`/`9k`) latches, since it
    /// targets the same buffer-line column on its landing line regardless
    /// of which display line of that (possibly wrapped) line it lands on.
    ///
    /// Continuation-display-line indent is excluded (see
    /// `DisplayLineMap::display_line_shape`) and inline virtual cells
    /// (inlay hints, ghost text) are included, same as
    /// [`DisplayLineMap::locate`]. The two differ only in what they're
    /// measured from, and coincide under `WrapMode::None`, where a line is
    /// exactly one display line with no indent.
    pub fn buffer_line_col(&mut self, pos: ClusterStart) -> BufferLineCol {
        self.debug_assert_in_text(pos);
        let (ropey_line, target_byte) =
            hume_rope::lines::char_to_line_byte(self.rope, pos.offset());
        let line = self.content_line_of(ropey_line);
        let idx = self.ensure_formatted(line, FormatBound::ToByte(target_byte));
        let (sub, dline_display_col) = self.locate_in_line(idx, target_byte, pos);
        let (dline_indent, _) = self.display_line_shape(idx, sub);
        let preceding: u32 = (0..sub).map(|j| self.display_line_shape(idx, j).1).sum();
        BufferLineCol::new(preceding + dline_display_col.cells_since(dline_indent))
    }

    /// Inverse of [`DisplayLineMap::buffer_line_col`]: the cluster
    /// `target_line_display_col` resolves to on `line`, under `target`'s
    /// policy.
    ///
    /// A line-relative column past the line's total width clamps to its
    /// last display line, where `target`'s own clamp rule (see
    /// [`DisplayColTarget`]) applies. That's the same "stick to the last real
    /// character, land on `\n` only when the line is empty" rule bare
    /// `j`/`k` already gets from [`DisplayLineMap::char_at`].
    pub fn char_at_buffer_line_col(
        &mut self,
        line: ContentLine,
        target_line_display_col: BufferLineCol,
        target: DisplayColTarget,
    ) -> ClusterStart {
        let content_display_lines = self.block(line).content;
        // Only up to the target column: while wrapping, `ensure_formatted`
        // promotes this to `Full` regardless (a display-line-relative bound
        // can't usefully clip a line-relative target), and without wrapping
        // `content_display_lines == 1` so the two columns coincide. It's sound
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
        self.resolve_in_display_line(idx, sub, dline_indent.advance_saturating(remaining), target)
    }

    /// The clusters one content display line covers. `None` when `pos` is
    /// not a content display line.
    ///
    /// Lets a caller scope a line-oriented search (nearest word) to the
    /// head's own display line instead of the whole buffer line.
    pub fn content_display_line_clusters(&mut self, pos: DisplayLinePos) -> Option<ClusterRange> {
        let b = self.block(pos.line);
        let sub = pos.slot.checked_sub(b.before)?;
        if sub >= b.content {
            return None;
        }
        // `Full`: this reads the *next* display line's first cluster to bound
        // the current one, so it needs every display line the line produces.
        let idx = self.ensure_formatted(pos.line, FormatBound::Full);

        let format = self.format_at(idx);
        let lines = &format.display_lines;
        let graphemes = &format.graphemes;
        let first_of = |dline: &DisplayLine| {
            graphemes[dline.graphemes.clone()]
                .iter()
                .filter_map(|g| g.pos)
                .min()
        };

        let start = first_of(lines.get(sub)?)?;
        let end = lines
            .get(sub + 1)
            .and_then(first_of)
            .map(ClusterBound::from)
            .unwrap_or_else(|| {
                hume_rope::lines::ropey_line_start(self.rope, RopeyLine::from(pos.line).advance(1))
            });
        ClusterRange::between(self.rope.slice(..), start, end)
    }
}
