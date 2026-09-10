use crate::cell::Cell;
use crate::color::Rgb;
use crate::geometry::Rect;
use crate::grid::Grid;
use crate::style::ResolvedStyle;

/// The frame's drawing surface — every cell write, pane or chrome, goes
/// through here.
///
/// Wraps the frame's [`Grid`] and, when set, a dim target: fg/bg is blended
/// toward it on every write. This is the single chokepoint for the non-focused
/// pane dim effect — `compose_display_line` / `render_tilde_fillers` (`hume-engine`)
/// never touch the grid directly, so a future write site cannot forget to
/// dim: the blend happens exactly once per cell, inline in the single write,
/// never a separate sweep over an already-drawn rect. Chrome (menus,
/// pickers, the drawer, the statusline) is never dimmed, so it always passes
/// `dim: None` — the field only ever blends for a pane.
///
/// Also the single place a placeholder cell's style is carried:
/// [`Canvas::new`] takes it pre-resolved (`theme.ui.invisible`, resolved once
/// per frame by the caller) so a [`Canvas::write_text_run`] placeholder never
/// needs a `Theme` hand-threaded down to this crate — `hume-grid` has no
/// dependency on `hume-engine`'s theme type, only on the one `ResolvedStyle`
/// value it resolves to.
pub struct Canvas<'a> {
    grid: &'a mut Grid,
    /// Colour every write is blended toward, and by how much. `None` for
    /// chrome, which is never dimmed.
    dim: Option<(Rgb, f32)>,
    /// Layered onto a [`Canvas::write_text_run`] placeholder cell so it reads
    /// distinctly from ordinary text (buffer text gets this same layering
    /// via `style_display_line`'s Tier 2d½; chrome has no per-cell style tiers of its
    /// own, so the canvas carries the one style every write needs for it).
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

    /// Write one cell, `advance` columns wide, dropped whole rather than
    /// split if it would cross `right_edge` — the same rule
    /// [`Canvas::write_text_run`] follows for a multi-cluster run.
    ///
    /// The frame's lowest-level writer, for a caller drawing exactly one
    /// pre-measured glyph rather than a run: `compose_display_line`/`compose_gutter`'s
    /// (`hume-engine`) per-cell fills and straddle fallbacks. `Grid::set_glyph`
    /// itself has no `right_edge` — only the grid's own physical edge — which
    /// is what made a bare `set_cell` call unsafe to expose before this bound
    /// existed: nothing stopped a write from crossing a pane, lane, or box
    /// boundary that sits inside the grid's bounds. The trailing columns of
    /// a wide glyph are still the grid's business, not a caller's —
    /// `Grid::set_glyph` claims them itself.
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
    /// "the terminal's default". Glyph styles are routinely partial on
    /// purpose — `ui.virtual_text` (the `~` fillers) and the statusline
    /// separator set a foreground only, and are drawn over a background some
    /// earlier fill put down. Making the omission inherit is what lets a
    /// writer draw a glyph without first having to find out what it is
    /// standing on.
    ///
    /// Modifiers and the underline replace outright, and that asymmetry is
    /// deliberate: an opaque overlay — a completion popup over highlighted
    /// code — must not inherit the bold of whatever it covered. Colours
    /// compose; emphasis does not — see [`ResolvedStyle::over`], which does
    /// the composing.
    fn over_painted(&self, x: u16, y: u16, style: ResolvedStyle) -> ResolvedStyle {
        let Some(under) = self.grid.cell(x, y).map(Cell::style) else {
            return style;
        };
        style.over(under)
    }

    /// Write `text` cell by cell from `(x, y)`, stopping before `right_edge`,
    /// and return the column just past the last cell written.
    ///
    /// The frame's single text writer for anything measured beforehand: UI
    /// chrome (statusline, menus, pickers, the drawer), gutter cells, and —
    /// inside `compose_display_line`, for a `CellContent::Whitespace`/`Placeholder`
    /// cell — a pane-content whitespace glyph or unrenderable-cluster
    /// stand-in. That last case still measures against `CHROME_TAB_WIDTH`
    /// (this method's fixed tab width, not the pane's real one) safely: the
    /// resolved string is always a pre-built glyph (`→`, `<200b>`) that
    /// itself never contains a literal `\t` needing the pane's own tab-stop
    /// math to re-measure — a real buffer tab's cell is written directly by
    /// `compose_display_line`'s own tab-arm, never routed through here.
    /// Writes through [`Canvas::write_cell`] rather than `Grid::set_glyph`
    /// directly — `write_cell` is the primitive this method is built on,
    /// one layer too low on its own for anything measured beforehand.
    ///
    /// **It agrees with [`hume_rope::width`], the width model everything
    /// else in the frame is measured with.** A [`Cell`] stores the display
    /// width its writer measured rather than letting anything downstream
    /// re-derive it (see this crate's own doc), so a caller that sized a
    /// field with `str_width` and then drew it by walking clusters a second,
    /// different way could reserve columns nothing was drawn in, or draw
    /// wider than it reserved. Here the advance returned is exactly
    /// `str_width(text, 0, 1)`, because that is the same per-cluster width
    /// this walks by — measurement and drawing cannot drift, since they are
    /// one model. Chrome has no tab
    /// stops of its own, so a tab measures and draws as exactly one cell — a
    /// plain space — rather than advancing to the next multiple of some tab
    /// width. Any other cluster the terminal must not be shown as itself (a
    /// control character, or one measuring zero columns) draws as its
    /// codepoint placeholder instead, the same substitution buffer text gets
    /// from `hume-engine`'s `format::grapheme_display` — `grapheme_width`
    /// already sized the run for that placeholder, so it spans exactly the
    /// columns reserved for it. That placeholder is drawn in this canvas's
    /// `invisible_style` rather than `style`, so it reads distinctly from
    /// ordinary text — buffer text gets the same layering via `style_display_line`'s
    /// Tier 2d½; chrome has no per-cell style tiers, so this is its
    /// equivalent.
    ///
    /// **`right_edge` is required, not implied.** `Grid::set_glyph`/
    /// `fill_span` clip only at the grid's own edge and nothing narrower, so
    /// a caller drawing into a pane, a gutter lane, or a bordered box had to
    /// remember to pre-truncate or bleed past it. Taking the bound as an
    /// argument moves that from something each call site remembers to
    /// something the signature asks for. A cluster that would straddle
    /// `right_edge` is dropped whole, never split — the same rule
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
            // Classified once — tab vs. placeholder vs. plain is decided
            // here, not re-tested per branch below (a tab is also a control
            // character, so testing `needs_placeholder` first would draw a
            // multi-cell `<9>` into the single cell reserved for it;
            // `classify` itself orders that check, matching
            // `format::grapheme_display`'s own tab-before-placeholder order).
            let classified = hume_rope::width::classify(
                cluster,
                (cx - x) as usize,
                hume_rope::width::CHROME_TAB_WIDTH,
            );
            // Cluster::width() reads classify()'s own decision — not a second raw measurement.
            let width = classified.width() as u16;
            if cx.saturating_add(width) > right_edge {
                break;
            }
            match classified {
                hume_rope::width::Cluster::Tab { .. } => {
                    // Chrome's tab is exactly one cell (see this method's
                    // doc), so it draws as one plain space.
                    self.write_cell(cx, y, " ", 1, style, right_edge);
                }
                hume_rope::width::Cluster::Placeholder(p) => {
                    // A cluster the terminal must not be shown as itself is
                    // drawn as its codepoint, the same substitution buffer
                    // text gets (`format::grapheme_display`). `classify`
                    // above already sized the run for that placeholder, so
                    // it spans exactly the columns reserved for it — one
                    // cell per character of `<200b>`. Colours fall back to
                    // the row's own, so a selected menu row or a cursorline
                    // still shows through, but the *emphasis*
                    // (`invisible_style`'s modifiers and underline) replaces
                    // rather than unions with the run's: a placeholder inside
                    // a bold field reads as a placeholder, not as bold text.
                    // That is why this composes with `ResolvedStyle::over`
                    // instead of `layer`, which unions modifiers.
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

    /// Write `glyph` — a single grapheme cluster, one column wide — into
    /// each of `count` cells starting at `(x, y)`, clipped at `right_edge`,
    /// and return the column just past the last cell written.
    ///
    /// The span counterpart of [`Canvas::write_text_run`] for repeating one
    /// glyph many times, most commonly a horizontal box-drawing border line
    /// — without building a `String` of it first just to hand it to a
    /// grapheme walker one call site already knows walks a single repeated
    /// cluster. `glyph` must already be exactly one column and free of
    /// anything `write_text_run` would substitute for (a control character,
    /// a zero-width cluster): callers pass a compile-time constant, never
    /// buffer-derived text, so that is a property of the call site rather
    /// than something this method has to verify.
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
            hume_rope::width::grapheme_width(glyph, 0, hume_rope::width::CHROME_TAB_WIDTH),
            1,
            "fill_glyph_run repeats `glyph` at width 1 per cell — a wider cluster needs write_text_run"
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
    /// `Grid::fill_span` only overwrites glyph and style together — there is
    /// no way to touch style alone and leave a previous glyph showing, so an
    /// opaque overlay (popup, statusline fill) never needs a second pass to
    /// blank what it covers. The chrome-facing counterpart of the pane-only
    /// `Canvas::fill_row_bg`; blending is currently always a no-op there
    /// (chrome passes `dim: None`), but routing both through this one method
    /// keeps every write, pane or chrome, going through one blend point
    /// rather than two conventions.
    ///
    /// One `fill_span` per row rather than `write_cell` per cell — sound only
    /// because a background fill never needs to read what it's painting
    /// over: `Grid::reset` blanks the frame before any pane draws, panes
    /// tile without overlap, and `compose_gutter` (`hume-engine`) only ever
    /// writes left of `content_x_origin`, so nothing in the same frame has
    /// painted this rect before a fill reaches it.
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

    /// Fill a horizontal span with spaces, `bg` as the background colour —
    /// used for cursorline highlighting so the tint extends past the last
    /// grapheme. `None` blanks the span to the terminal's own colours
    /// instead (taken only when `theme.ui.background.bg` is `None`, which is
    /// exactly when `dim` is `None` too — the pipeline gates both on that
    /// same value — so a `None` bg never needs a blend). A bg-only
    /// [`Canvas::fill_rect_bg`] style blends identically to blending the
    /// colour alone, so both cases route through it without a separate
    /// blend step of their own.
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
/// Takes the size rather than `&Grid` so a caller already holding an
/// exclusive `&mut Grid` (or its wrapping `Canvas`) can still call this —
/// `Grid::size()` is `Copy`, unlike the grid itself. `Grid::fill_span`
/// independently clamps `x`/`y` against its own bounds on every call, so
/// this bound is what keeps a caller from iterating rows the fill would
/// have no-opped on anyway, not the only thing standing between a rect and
/// an out-of-bounds write.
///
/// `pub`, not `pub(crate)`: pure rect/size geometry, no write capability —
/// `hume-engine`'s seam-junction pass (`pipeline/mod.rs`) clamps a seam rect
/// with it ahead of its own per-cell loop, independently of [`Canvas`].
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
/// colour is the terminal's own default — there is no numeric value to
/// blend, so it stays as it is; `None` dim (chrome, always) is likewise free.
#[inline]
fn blend_style(mut style: ResolvedStyle, dim: Option<(Rgb, f32)>) -> ResolvedStyle {
    if let Some((target, factor)) = dim {
        style.fg = style.fg.map(|c| c.lerp(target, factor));
        style.bg = style.bg.map(|c| c.lerp(target, factor));
    }
    style
}
