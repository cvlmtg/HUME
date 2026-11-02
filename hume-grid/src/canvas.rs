use crate::cell::Cell;
use crate::color::Rgb;
use crate::geometry::Rect;
use crate::grid::Grid;
use crate::style::ResolvedStyle;

/// The frame's drawing surface: every cell write, pane or chrome, goes
/// through here.
///
/// When `dim` is set, fg/bg is blended toward it once per cell at the write
/// itself, so no write site can skip the non-focused pane dim. Chrome
/// (menus, pickers, the drawer, the statusline) passes `dim: None`.
///
/// The placeholder style arrives pre-resolved (`theme.ui.invisible`), so
/// this crate needs no dependency on `hume-engine`'s theme type.
pub struct Canvas<'a> {
    grid: &'a mut Grid,
    /// Colour every write is blended toward, and by how much. `None` for
    /// chrome.
    dim: Option<(Rgb, f32)>,
    /// Style of a [`Canvas::write_text_run`] placeholder cell.
    invisible_style: ResolvedStyle,
}

impl<'a> Canvas<'a> {
    pub fn new(
        grid: &'a mut Grid,
        invisible_style: ResolvedStyle,
        dim: Option<(Rgb, f32)>,
    ) -> Self {
        Self {
            grid,
            dim,
            invisible_style,
        }
    }

    /// Write one pre-measured cell, `advance` columns wide, dropped whole
    /// rather than split if it would cross `right_edge`.
    ///
    /// `Grid::set_glyph` bounds writes only at the grid's own edge, so this
    /// writer takes `right_edge` to keep a cell from crossing a pane, lane,
    /// or box border inside the grid. `Grid::set_glyph` claims a wide
    /// glyph's trailing columns itself.
    pub fn write_cell(
        &mut self,
        x: u16,
        y: u16,
        text: &str,
        advance: u8,
        style: ResolvedStyle,
        right_edge: u16,
    ) {
        if x.saturating_add(advance as u16) > right_edge {
            return;
        }
        let style = self.over_painted(x, y, blend_style(style, self.dim));
        self.grid.set_glyph(x, y, text, advance, style);
    }

    /// Resolve `style` against whatever is already painted at `(x, y)`.
    ///
    /// A colour the caller left unset means "whatever is already there", not
    /// "the terminal's default": `ui.virtual_text` (the `~` fillers) and the
    /// statusline separator set a foreground only and draw over an earlier
    /// fill's background.
    ///
    /// Modifiers and the underline replace outright, so an opaque overlay (a
    /// completion popup over highlighted code) does not inherit the bold of
    /// what it covers. Colours compose; emphasis does not.
    fn over_painted(&self, x: u16, y: u16, style: ResolvedStyle) -> ResolvedStyle {
        let Some(under) = self.grid.cell(x, y).map(Cell::style) else {
            return style;
        };
        style.over(under)
    }

    /// Write `text` cell by cell from `(x, y)`, stopping before `right_edge`,
    /// and return the column just past the last cell written.
    ///
    /// The frame's text writer for anything measured beforehand: UI chrome,
    /// gutter cells, and a pane's pre-built whitespace or placeholder glyph
    /// (`→`, `<200b>`). Such a glyph never holds a literal `\t`, so measuring
    /// it at `CHROME_TAB_WIDTH` is safe; a buffer tab is drawn by
    /// `compose_display_line` itself.
    ///
    /// Each cluster advances by the same [`hume_rope::width`] model a caller
    /// sized the field with (`chrome_width`), so measurement and drawing
    /// cannot drift. A tab is one cell, drawn as a space. Any other cluster
    /// the terminal must not show as itself (a control character, a
    /// zero-width cluster) draws as its codepoint placeholder in
    /// `invisible_style`, the substitution buffer text gets from
    /// `format::grapheme_display`. The placeholder is the codepoint rather
    /// than a blank because a bidi override that renders like a space is the
    /// Trojan Source attack: the cluster must stay visible.
    ///
    /// `Grid` clips only at its own edge, so `right_edge` bounds the run at
    /// a pane, lane, or box border. A cluster that would straddle it is
    /// dropped whole, never split, the same rule
    /// [`hume_rope::width::truncate_to_width`] follows.
    pub fn write_text_run(
        &mut self,
        x: u16,
        y: u16,
        text: &str,
        style: ResolvedStyle,
        right_edge: u16,
    ) -> u16 {
        // Neither style is blended here: every cell below is written through
        // `write_cell`, which applies the dim once, at the single write point.
        let invisible_style = self.invisible_style;
        let mut cx = x;
        for cluster in unicode_segmentation::UnicodeSegmentation::graphemes(text, true) {
            // A tab is also a control character; `classify` tests for it
            // first so it is never drawn as a multi-cell `<9>`.
            let classified = hume_rope::width::classify(
                cluster,
                (cx - x) as usize,
                hume_rope::width::CHROME_TAB_WIDTH,
            );
            let width = classified.width() as u16;
            if cx.saturating_add(width) > right_edge {
                break;
            }
            match classified {
                hume_rope::width::Cluster::Tab { .. } => {
                    self.write_cell(cx, y, " ", 1, style, right_edge);
                }
                hume_rope::width::Cluster::Placeholder(p) => {
                    // One cell per character of `<200b>`. `over` rather than
                    // `layer`: colours fall back to the row's (a selected
                    // menu row shows through), but `invisible_style`'s
                    // emphasis replaces the run's instead of unioning with it.
                    let placeholder_style = invisible_style.over(style);
                    for (i, ch) in p.as_str().chars().enumerate() {
                        let mut glyph = [0u8; 4];
                        self.write_cell(
                            cx + i as u16,
                            y,
                            ch.encode_utf8(&mut glyph),
                            1,
                            placeholder_style,
                            right_edge,
                        );
                    }
                }
                hume_rope::width::Cluster::Plain { .. } => {
                    self.write_cell(cx, y, cluster, width as u8, style, right_edge);
                }
            }
            cx += width;
        }
        cx
    }

    /// Write `glyph` (a single grapheme cluster, one column wide) into
    /// each of `count` cells starting at `(x, y)`, clipped at `right_edge`,
    /// and return the column just past the last cell written.
    ///
    /// For a repeated glyph such as a box-drawing border, without building a
    /// `String`. `glyph` must be one column wide and need no placeholder (a
    /// control character, a zero-width cluster); callers pass a constant,
    /// never buffer text.
    pub fn fill_glyph_run(
        &mut self,
        x: u16,
        y: u16,
        glyph: &str,
        count: u16,
        style: ResolvedStyle,
        right_edge: u16,
    ) -> u16 {
        debug_assert_eq!(
            hume_rope::width::chrome_cluster_width(glyph),
            1,
            "fill_glyph_run repeats `glyph` at width 1 per cell; a wider cluster needs write_text_run"
        );
        let end = x.saturating_add(count).min(right_edge);
        let mut cx = x;
        while cx < end {
            self.write_cell(cx, y, glyph, 1, style, right_edge);
            cx += 1;
        }
        cx
    }

    /// Paint every cell of `rect` with a space glyph and `style`, clipping to
    /// grid bounds, through this canvas's dim blend.
    ///
    /// Glyph and style are overwritten together, so an opaque overlay (popup,
    /// statusline fill) blanks what it covers in one pass.
    ///
    /// One `fill_span` per row rather than `write_cell` per cell, so the fill
    /// does not compose with what is under it. That is sound because
    /// `Grid::reset` blanks the frame before any pane draws, panes tile
    /// without overlap, and `compose_gutter` (`hume-engine`) writes only left
    /// of `content_x_origin`: nothing has painted this rect before a fill.
    pub fn fill_rect_bg(&mut self, rect: Rect, style: ResolvedStyle) {
        let (x0, y0, x1, y1) = clamp_rect_to_grid(self.grid.size(), rect);
        if x0 >= x1 {
            return;
        }
        let style = blend_style(style, self.dim);
        for y in y0..y1 {
            self.grid.fill_span(y, x0, x1, Cell::blank(style));
        }
    }

    /// Fill a horizontal span with spaces on `bg`, so the cursorline tint
    /// extends past the last grapheme. `None` blanks the span to the
    /// terminal's own colours.
    pub fn fill_row_bg(&mut self, x_start: u16, x_end: u16, y: u16, bg: Option<Rgb>) {
        self.fill_rect_bg(
            Rect::new(x_start, y, x_end.saturating_sub(x_start), 1),
            ResolvedStyle {
                bg,
                ..Default::default()
            },
        );
    }
}

/// Clamp `rect` to a `(width, height)` grid's bounds, returning exclusive
/// `(x0, y0, x1, y1)` ready for a `for y in y0..y1 { … x0..x1 }` loop.
///
/// Takes the size rather than `&Grid` so a caller holding a `&mut Grid` (or
/// its `Canvas`) can still call it.
#[inline]
pub fn clamp_rect_to_grid((width, height): (u16, u16), rect: Rect) -> (u16, u16, u16, u16) {
    (
        rect.x,
        rect.y,
        rect.right().min(width),
        rect.bottom().min(height),
    )
}

/// Blend both fg and bg of `style` toward `dim`'s target, if any. A `None`
/// colour is the terminal's own default and stays as it is.
#[inline]
fn blend_style(mut style: ResolvedStyle, dim: Option<(Rgb, f32)>) -> ResolvedStyle {
    if let Some((target, factor)) = dim {
        style.fg = style.fg.map(|c| c.lerp(target, factor));
        style.bg = style.bg.map(|c| c.lerp(target, factor));
    }
    style
}
