use hume_grid::{Cell, Grid, Rect, Rgb};

use crate::layout::PaneGeometry;
use crate::pane::ViewportState;
use crate::providers::{GutterColumn, GutterRowCtx, ProviderId};
use crate::theme::Theme;
use crate::types::{
    CellContent, DisplayRow, EditorMode, Grapheme, ResolvedStyle, RowKind, ScopeId,
};

// ---------------------------------------------------------------------------
// Stage 4: compose
// ---------------------------------------------------------------------------

/// Glyph drawn at each inner indent-guide tab stop. Single source of truth —
/// referenced by `compose_row` and by tests, so the glyph only ever needs to
/// change in one place.
pub(crate) const INDENT_GUIDE_GLYPH: &str = "╎";

/// Per-frame constants needed by `compose_row`. Bundle these once per pane
/// and pass them through without repeating at each call site.
pub(crate) struct ComposeCtx<'a> {
    pub gutter_columns: &'a [(ProviderId, Box<dyn GutterColumn>)],
    pub visible: &'a PaneGeometry,
    pub viewport: &'a ViewportState,
    pub mode: EditorMode,
    pub primary_head_line: usize,
    pub tab_width: u8,
    /// Pre-resolved from `theme.ui.virtual_text` — avoids repeated field access in the hot loop.
    pub tilde_style: ResolvedStyle,
    /// Pre-resolved from `theme.ui.indent_guide`.
    pub indent_guide_style: ResolvedStyle,
    /// From the `indent-guides` setting — gates the draw loop below.
    pub show_indent_guides: bool,
    pub pane_rect: Rect,
    /// `theme.ui.background.bg` is read directly wherever a row/gutter cell
    /// falls back to the pane's own background (trailing cells past the
    /// last grapheme, blank gutter cells) rather than through a cached copy
    /// on this struct — `theme` is already here, so a copy would only be
    /// another place that value could drift from it.
    pub theme: &'a Theme,
    /// Buffer rope, passed to `GutterColumn::render_row` via `GutterRowCtx`
    /// so gutter providers (git-signs, diagnostics) can query buffer content
    /// without pre-owning it.
    pub rope: &'a ropey::Rope,
    /// `DEFAULT_GUTTER_SCOPE` ("ui.linenr"), interned once at
    /// `EngineView::new` — the fallback scope `compose_gutter` resolves
    /// under when a cell/column has nothing more specific to say. Threaded
    /// in rather than re-interned here: the per-cell hot path only ever
    /// does an O(1) `ScopeId` index.
    pub default_gutter_scope: ScopeId,
}

/// The frame's drawing surface — every cell write, pane or chrome, goes
/// through here.
///
/// Wraps the frame's [`Grid`] and, when set, a dim target: fg/bg is blended
/// toward it on every write. This is the single chokepoint for the non-focused
/// pane dim effect — `compose_row` / `render_tilde_fillers` never touch the
/// grid directly, so a future write site cannot forget to dim: the blend happens
/// exactly once per cell, inline in the single write, never a separate sweep
/// over an already-drawn rect. Chrome (menus,
/// pickers, the drawer, the statusline) is never dimmed, so it always passes
/// `dim: None` — the field only ever blends for a pane.
///
/// Also the single place `theme.ui.invisible` is resolved: [`Canvas::new`]
/// converts it once, so a placeholder cell written through
/// [`Canvas::write_text_run`] never needs that style hand-threaded down from
/// the caller's own `&Theme`.
pub struct Canvas<'a> {
    grid: &'a mut Grid,
    /// Colour every write is blended toward, and by how much. `None` for
    /// chrome, which is never dimmed.
    dim: Option<(Rgb, f32)>,
    /// Resolved from `theme.ui.invisible` once per frame — layered onto a
    /// [`Canvas::write_text_run`] placeholder cell so it reads distinctly from
    /// ordinary text (buffer text gets this same layering via `style_row`'s
    /// Tier 2d½; chrome has no per-cell style tiers of its own, so the canvas
    /// carries the one style every write needs for it).
    invisible_style: ResolvedStyle,
}

impl<'a> Canvas<'a> {
    pub fn new(grid: &'a mut Grid, theme: &Theme, dim: Option<(Rgb, f32)>) -> Self {
        Self {
            grid,
            dim,
            invisible_style: theme.ui.invisible,
        }
    }

    /// Write one cell, `advance` columns wide.
    ///
    /// The trailing columns of a wide glyph are the grid's business, not a
    /// caller's: [`Grid::set_glyph`] claims them itself, so no write site
    /// has to remember to blank them.
    fn set_cell(&mut self, x: u16, y: u16, text: &str, advance: u8, style: ResolvedStyle) {
        let style = self.over_painted(x, y, blend_style(style, self.dim));
        // static-glyph-safe: `Canvas`'s own cell primitive.
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
    /// inside `compose_row`, for a `CellContent::Indicator`/`Placeholder`
    /// cell — a pane-content whitespace glyph or unrenderable-cluster
    /// stand-in. That last case still measures against `CHROME_TAB_WIDTH`
    /// (this method's fixed tab width, not the pane's real one) safely: the
    /// resolved string is always a pre-built glyph (`→`, `<200b>`) that
    /// itself never contains a literal `\t` needing the pane's own tab-stop
    /// math to re-measure — a real buffer tab's cell is written directly by
    /// `compose_row`'s own tab-arm, never routed through here.
    /// `Grid::set_glyph`/`fill_span` are not called directly here, for two
    /// reasons — they are the primitives this method is built on, not
    /// wrong, just one layer too low for anything measured beforehand.
    ///
    /// **It agrees with [`hume_rope::width`], the width model everything
    /// else in the frame is measured with.** A [`Cell`] stores the display
    /// width its writer measured rather than letting anything downstream
    /// re-derive it (see the [`hume_grid`] crate doc), so a caller that
    /// sized a field with `str_width` and then drew it by walking clusters a
    /// second, different way could reserve columns nothing was drawn in, or
    /// draw wider than it reserved. Here the advance returned is exactly
    /// `str_width(text, 0, 1)`, because that is the same per-cluster width
    /// this walks by — measurement and drawing cannot drift, since they are
    /// one model. Chrome has no tab
    /// stops of its own, so a tab measures and draws as exactly one cell — a
    /// plain space — rather than advancing to the next multiple of some tab
    /// width. Any other cluster the terminal must not be shown as itself (a
    /// control character, or one measuring zero columns) draws as its
    /// codepoint placeholder instead, the same substitution buffer text gets
    /// from `format::grapheme_display` — `grapheme_width` already sized the
    /// run for that placeholder, so it spans exactly the columns reserved
    /// for it. That placeholder is drawn in this canvas's resolved
    /// `theme.ui.invisible` rather than `style`, so it reads distinctly from
    /// ordinary text — buffer text gets the same layering via `style_row`'s
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
        // `set_cell`, which applies the dim once, at the single write point.
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
            // display-width-safe: Cluster::width() reads classify()'s own decision — not a second raw measurement.
            let width = classified.width() as u16;
            if cx.saturating_add(width) > right_edge {
                break;
            }
            match classified {
                hume_rope::width::Cluster::Tab { .. } => {
                    // Chrome's tab is exactly one cell (see this method's
                    // doc), so it draws as one plain space.
                    self.set_cell(cx, y, " ", 1, style);
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
                        self.set_cell(
                            cx + i as u16,
                            y,
                            ch.encode_utf8(&mut glyph),
                            1,
                            placeholder_style,
                        );
                    }
                }
                hume_rope::width::Cluster::Plain { .. } => {
                    self.set_cell(cx, y, cluster, width as u8, style);
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
            self.set_cell(cx, y, glyph, 1, style);
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
    /// One `fill_span` per row rather than `set_cell` per cell — sound only
    /// because a background fill never needs to read what it's painting
    /// over: `Grid::reset` blanks the frame before any pane draws, panes
    /// tile without overlap, and `compose_gutter` only ever writes left of
    /// `content_x_origin`, so nothing in the same frame has painted this
    /// rect before a fill reaches it.
    pub fn fill_rect_bg(&mut self, rect: Rect, style: ResolvedStyle) {
        let (x0, y0, x1, y1) = clamp_rect_to_grid(self.grid.size(), rect);
        if x0 >= x1 {
            return;
        }
        let style = blend_style(style, self.dim);
        for y in y0..y1 {
            // static-glyph-safe: `Canvas`'s own cell primitive; blanks a span, writes no text.
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
    fn fill_row_bg(&mut self, x_start: u16, x_end: u16, y: u16, bg: Option<Rgb>) {
        self.fill_rect_bg(
            Rect::new(x_start, y, x_end.saturating_sub(x_start), 1),
            ResolvedStyle {
                bg,
                ..Default::default()
            },
        );
    }
}

/// Resolve a gutter cell's scope to a style, layered over the row's
/// background (cursorline tint or pane bg, whichever `row_bg` already
/// resolved to). Shared by `compose_gutter`'s per-cell loop and its
/// leftover-width blank fill so the two resolution paths can't drift.
///
/// `scope` is always already-interned — every gutter provider (`SignSource`,
/// `LineNumberColumn`) interns at construction, so this is an O(1) `Theme::resolve`
/// index, never a by-name lookup.
///
/// Cursorline/pane bg is the base; the gutter scope style layers on top.
/// If the scope defines its own bg, it wins; otherwise the row bg shows
/// through.
fn gutter_cell_style(
    scope: ScopeId,
    theme: &crate::theme::Theme,
    row_bg: Option<Rgb>,
) -> ResolvedStyle {
    let scope_style = theme.resolve(scope);
    match row_bg {
        Some(bg) => ResolvedStyle {
            bg: Some(bg),
            ..Default::default()
        }
        .layer(scope_style),
        None => scope_style,
    }
}

/// Write one row's gutter cells (all columns) at screen row `y`.
///
/// Shared by `compose_row` (real buffer/wrap/virtual rows) and
/// `render_tilde_fillers` (`RowKind::Filler` rows) so a filler row's gutter
/// is never silently blank — a custom column must be consulted for filler
/// rows too, not just `LineNumberColumn`'s blank-for-Filler default.
///
/// `lane_widths` must already be populated by the caller (one entry per
/// gutter column) — see `compose_row`'s doc comment for why it isn't folded
/// into `ComposeCtx`.
fn compose_gutter(
    row_kind: RowKind,
    lane_widths: &[u16],
    compose_ctx: &ComposeCtx,
    row_bg: Option<Rgb>,
    y: u16,
    canvas: &mut Canvas,
) {
    let mut gutter_x = compose_ctx.pane_rect.x;
    // A column's configured width (in particular `signcolumn`'s up-to-127
    // slots) is never checked against the pane's actual width — `layout.rs`
    // only clamps *content* width down to make room for the gutter, not the
    // other way around. Without this bound, a gutter wider than the pane
    // would write straight through the pane's right edge into whatever is
    // drawn next to it in the shared terminal buffer (a neighbouring pane,
    // most commonly).
    let pane_right_edge = compose_ctx.pane_rect.right();
    let gutter_ctx = GutterRowCtx {
        mode: compose_ctx.mode,
        primary_head_line: compose_ctx.primary_head_line,
        rope: compose_ctx.rope,
    };
    // Constant for the whole row; resolved once so the per-cell loop and the
    // leftover-width fill below can't drift apart on it.
    let cell_bg = row_bg.or(compose_ctx.theme.ui.background.bg);
    for ((_, lane_provider), &lane_width) in
        compose_ctx.gutter_columns.iter().zip(lane_widths.iter())
    {
        if lane_width == 0 {
            continue;
        }
        let lane_x = gutter_x;
        if lane_x >= pane_right_edge {
            continue;
        }
        let lane_width = lane_width.min(pane_right_edge - lane_x);
        let cells = lane_provider.render_row_cells(row_kind, &gutter_ctx);
        // Distribute `lane_width` across `cells.len()` sub-cells. Only the
        // column's right padding (1 cell) is reserved — no separators between
        // sub-cells. `usable_per_cell` is how much of each sub-cell's text
        // may be written before truncation.
        let n_cells = cells.len().max(1);
        let usable_per_cell = lane_width.saturating_sub(1) / n_cells as u16;
        let mut last_scope: ScopeId = compose_ctx.default_gutter_scope;
        for (cell_idx, cell) in cells.iter().enumerate() {
            let is_last = cell_idx == cells.len() - 1;
            let text = cell.as_str();
            let style = gutter_cell_style(cell.scope, compose_ctx.theme, cell_bg);

            // Right-align within usable width. `usable_per_cell` bounds how
            // much of `text` may be written: a builtin column (only
            // `LineNumberColumn` today) always fits, but a future
            // plugin-supplied column isn't guaranteed to, and an overlong
            // cell must not bleed into the content area or the neighbouring
            // pane. `write_text_run` measures by the same rule this
            // truncation does, so `pad` and the separator below land where
            // the text actually ends.
            //
            // A gutter cell is a glyph in a fixed-width lane with no tab
            // stops of its own — the same convention `hume-editor`'s chrome
            // measurements use, hence the shared constant rather than a bare
            // `1`. For every non-tab cluster the parameter is inert.
            let (text, text_width) = hume_rope::width::truncate_to_width(
                text,
                usable_per_cell as usize,
                hume_rope::width::CHROME_TAB_WIDTH,
            );
            let text_width = text_width as u16;
            let pad = usable_per_cell.saturating_sub(text_width);
            for px in 0..pad {
                canvas.set_cell(gutter_x + px, y, " ", 1, style);
            }
            // `after` is where the write actually stopped — used below
            // instead of a second `gutter_x + pad + text_width` measurement,
            // so the separator's position can't drift from the draw.
            let after =
                canvas.write_text_run(gutter_x + pad, y, text, style, gutter_x + usable_per_cell);
            // Only write a separator after the last cell — it's the column's
            // right padding, not a separator between sub-cells.
            if is_last {
                canvas.set_cell(after, y, " ", 1, style);
                gutter_x += usable_per_cell + 1;
            } else {
                gutter_x += usable_per_cell;
            }
            last_scope = cell.scope;
        }
        // Any leftover width (e.g. sub-cell widths that don't evenly divide
        // lane_width - 1) fills as blanks under the last cell's scope —
        // preserves the single-cell builtin behaviour where the whole column
        // shared one scope. Bounded by `lane_x`, not `pane_rect.x`: for
        // every column after the first, `pane_rect.x` is the pane's left
        // edge, not this column's — using it here left leftover cells
        // unpainted and `gutter_x` short of the column boundary for any
        // non-first column with uneven leftover.
        if gutter_x < lane_x + lane_width {
            let style = gutter_cell_style(last_scope, compose_ctx.theme, cell_bg);
            while gutter_x < lane_x + lane_width {
                canvas.set_cell(gutter_x, y, " ", 1, style);
                gutter_x += 1;
            }
        }
        // Providers are a public extension point; their cell-width math
        // isn't guaranteed to sum to lane_width exactly. Land on the column
        // boundary regardless, so the next column never inherits any drift.
        gutter_x = lane_x + lane_width;
    }
}

/// Render a single display row at `screen_row` into the frame grid.
///
/// `line_str` is the pre-materialised text of the buffer line that owns this
/// row (used to resolve `CellContent::Grapheme` byte ranges). Pass `""` for
/// virtual/filler rows that have no backing buffer line.
///
/// `virtual_texts` is the per-frame arena backing this row's
/// `CellContent::Indicator`/`Virtual` ranges (`FormatScratch::virtual_texts`
/// for a content row, `virtual_row.texts` for a provider's virtual row) —
/// same lifetime/borrow rationale as `line_str`.
///
/// `lane_widths` must already be populated by the caller (one entry per gutter
/// column). Passed separately from `compose_ctx` because in the fused pipeline it lives
/// in `FrameScratch`, which cannot be bundled into `ComposeCtx` without
/// creating a conflicting borrow.
#[allow(clippy::too_many_arguments)]
pub(crate) fn compose_row(
    row: &DisplayRow,
    graphemes: &[Grapheme],
    styles: &[ResolvedStyle],
    line_str: &str,
    virtual_texts: &str,
    screen_row: u16,
    lane_widths: &[u16],
    compose_ctx: &ComposeCtx,
    canvas: &mut Canvas,
    // Background colour to fill the entire row (gutter + content) before
    // writing graphemes. Used for cursorline highlighting so the tint
    // extends to the right edge even past the last character.
    // `None` → clear to terminal default (normal rows).
    row_bg: Option<Rgb>,
) {
    let y = compose_ctx.pane_rect.y + screen_row;
    let right_edge = compose_ctx.pane_rect.right();

    // Filler rows are rendered exclusively by `render_tilde_fillers`, never
    // routed through here — it has its own gutter + tilde + background
    // handling since a filler row has no backing graphemes to iterate.
    debug_assert!(!matches!(row.kind, RowKind::Filler));

    compose_gutter(row.kind, lane_widths, compose_ctx, row_bg, y, canvas);

    // ── Content ───────────────────────────────────────────────────────
    let content_x_origin = compose_ctx.pane_rect.x + compose_ctx.visible.gutter_width;
    let h_offset = compose_ctx.viewport.horizontal_offset;

    // Fill trailing cells with row bg (cursorline) or pane bg, so the theme
    // background shows past the last grapheme rather than the terminal default.
    canvas.fill_row_bg(
        content_x_origin,
        right_edge,
        y,
        row_bg.or(compose_ctx.theme.ui.background.bg),
    );

    let row_graphemes = &graphemes[row.graphemes.start..row.graphemes.end];
    let row_styles = &styles[row.graphemes.start..row.graphemes.end];

    for (g, style) in row_graphemes.iter().zip(row_styles.iter()) {
        // Skip WidthContinuation — already handled by the primary cell.
        if matches!(g.content, CellContent::WidthContinuation) {
            continue;
        }

        // Horizontal scroll: skip cells left of the viewport.
        if g.display_col + g.width as u32 <= h_offset {
            continue;
        }
        // Clip cells that start before the viewport edge. `g.display_col` is
        // a row column (`u32`), which with wrapping off spans the whole
        // unwrapped line, but this render path always runs behind
        // `with_h_window` (`pane_render.rs`), so a cell surviving the skip
        // above sits within one viewport width of `h_offset` — safely
        // representable in the terminal-cell (`u16`) domain the rest of
        // compose works in.
        let content_x = g.display_col.saturating_sub(h_offset);
        debug_assert!(
            u16::try_from(content_x).is_ok(),
            "on-screen column {content_x} exceeds a u16 — h_window should have clipped this cell"
        );
        let screen_x = content_x_origin + content_x as u16;
        if screen_x >= right_edge {
            break; // past right edge — done with this row
        }

        let cell_style = *style;

        // A multi-column cell (double-width CJK grapheme, tab
        // Indicator) whose left edge sits before `h_offset` still
        // passes the skip check above once its right edge crosses
        // it — but `content_x` above already clamped to 0, so
        // rendering the glyph there would draw its *full* width at
        // the viewport's left edge instead of the fraction that's
        // actually scrolled into view, shifting the row. Render
        // spaces for the visible remainder instead (matches Helix).
        // Impossible for width-1 cells: straddling needs
        // `g.display_col < h_offset < g.display_col + g.width`, which has no
        // integer solution when `g.width == 1`.
        if g.display_col < h_offset {
            let visible_cells = g.width as u32 - (h_offset - g.display_col);
            for i in 0..visible_cells as u16 {
                let sx = screen_x + i;
                if sx < right_edge {
                    canvas.set_cell(sx, y, " ", 1, cell_style);
                }
            }
            continue;
        }

        match &g.content {
            CellContent::Grapheme => {
                if g.byte_range.start <= g.byte_range.end && g.byte_range.end <= line_str.len() {
                    if screen_x + g.width as u16 > right_edge {
                        // A wide grapheme whose right half would cross
                        // `right_edge` cannot be drawn — there is no such
                        // thing as half a glyph, and the cell past the edge
                        // belongs to whatever the terminal renders next (a
                        // neighbouring pane, the divider seam). Render spaces
                        // for the columns that are ours, mirroring the
                        // h-scroll straddle policy below.
                        for sx in screen_x..right_edge {
                            canvas.set_cell(sx, y, " ", 1, cell_style);
                        }
                    } else {
                        let text = &line_str[g.byte_range.clone()];
                        canvas.set_cell(screen_x, y, text, g.width, cell_style);
                    }
                }
            }
            CellContent::Indicator { start, len } | CellContent::Placeholder { start, len } => {
                let s = resolve_arena_text(virtual_texts, *start, *len);
                // The indicator's text may be wider than one cell — an
                // unrenderable cluster's `<200b>` placeholder spans as many
                // cells as it has characters — so it is written across the
                // span rather than into the first cell. A one-glyph
                // indicator (a whitespace marker, a tab's `→`) writes one
                // cell and leaves the rest to the fill below, exactly as
                // before.
                let cell_end = (screen_x + g.width as u16).min(right_edge);
                let after = canvas.write_text_run(screen_x, y, s, cell_style, cell_end);
                // Fill the reserved cells the text didn't cover: a tab's
                // expanse beyond its marker, or a wide cell's second column.
                for ex in after..cell_end {
                    canvas.set_cell(ex, y, " ", 1, cell_style);
                }
            }
            CellContent::Virtual { start, len } => {
                let s = resolve_arena_text(virtual_texts, *start, *len);
                if screen_x + g.width as u16 > right_edge {
                    // Same straddle policy as `Grapheme` above: a wide
                    // decoration glyph (an inlay hint containing CJK text)
                    // cannot be drawn half-on-screen.
                    for sx in screen_x..right_edge {
                        canvas.set_cell(sx, y, " ", 1, cell_style);
                    }
                } else {
                    canvas.set_cell(screen_x, y, s, g.width, cell_style);
                }
            }
            CellContent::Empty => {
                canvas.set_cell(screen_x, y, " ", 1, cell_style);
            }
            CellContent::WidthContinuation => unreachable!(),
        }
    }

    // ── Indent guides ─────────────────────────────────────────────────
    // Draw guides only on line-start rows (not wrap/virtual/filler) so
    // that continuation rows don't clobber content at guide positions.
    // Drawn after content so they appear on top of leading-whitespace cells.
    if compose_ctx.show_indent_guides && matches!(row.kind, RowKind::LineStart { .. }) {
        let depth = row_graphemes.first().map(|g| g.indent_depth).unwrap_or(0);
        let tw = hume_rope::width::indent_stop(1, compose_ctx.tab_width);
        // Draw a guide at each inner tab-stop. These positions are
        // guaranteed to lie within the leading whitespace.
        for k in 1..depth {
            let guide_display_col = hume_rope::width::indent_stop(k as u32, compose_ctx.tab_width);
            // Account for horizontal scroll.
            if guide_display_col + tw > h_offset {
                let content_x = guide_display_col.saturating_sub(h_offset);
                debug_assert!(
                    u16::try_from(content_x).is_ok(),
                    "on-screen indent guide column {content_x} exceeds a u16"
                );
                let screen_x = content_x_origin + content_x as u16;
                if screen_x < right_edge {
                    canvas.set_cell(
                        screen_x,
                        y,
                        INDENT_GUIDE_GLYPH,
                        1,
                        compose_ctx.indent_guide_style,
                    );
                }
            }
        }
    }
}

/// Resolve a `(start, len)` arena range into the underlying text. Never
/// panics — `start`/`len` are always produced by the same `push_arena_text`
/// call that sized the arena, so an out-of-range slice should not happen, but
/// degrading to an empty string is cheaper than a debug_assert on a hot path.
#[inline]
fn resolve_arena_text(arena: &str, start: u32, len: u16) -> &str {
    let start = start as usize;
    let end = start + len as usize;
    arena.get(start..end).unwrap_or("")
}

/// Draw tilde filler rows from `start_screen_row` up to (but not including)
/// `visible.content_height`, clamped to `pane_rect.height`.
///
/// Called by the fused pipeline (`render_pane`) to fill any remaining
/// vertical space after the last real content row has been rendered.
pub(crate) fn render_tilde_fillers(
    start_screen_row: u16,
    lane_widths: &[u16],
    compose_ctx: &ComposeCtx,
    canvas: &mut Canvas,
) {
    let mut screen_row = start_screen_row;
    while screen_row
        < compose_ctx
            .visible
            .content_height
            .min(compose_ctx.pane_rect.height)
    {
        let y = compose_ctx.pane_rect.y + screen_row;
        let right_edge = compose_ctx.pane_rect.right();
        // Gutter first — it already paints a real background (row_bg, or
        // theme.ui.background.bg, patched with the column's scope, see
        // `compose_gutter`) across the
        // whole gutter width, including column 0. The tilde below patches its
        // fg on top of that, matching editor convention: `~` sits at the
        // pane's left edge, ignoring/overriding the line-number gutter, never
        // shifted into the content area.
        compose_gutter(RowKind::Filler, lane_widths, compose_ctx, None, y, canvas);
        let content_x_origin = compose_ctx.pane_rect.x + compose_ctx.visible.gutter_width;
        canvas.fill_row_bg(
            content_x_origin,
            right_edge,
            y,
            compose_ctx.theme.ui.background.bg,
        );
        canvas.set_cell(compose_ctx.pane_rect.x, y, "~", 1, compose_ctx.tilde_style);
        screen_row += 1;
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
#[inline]
pub(crate) fn clamp_rect_to_grid((width, height): (u16, u16), rect: Rect) -> (u16, u16, u16, u16) {
    (
        rect.x,
        rect.y,
        rect.right().min(width),
        rect.bottom().min(height),
    )
}

/// Blend both fg and bg of `style` toward `dim`'s target, if any.
///
/// A `None` colour is the terminal's own default — there is no numeric value
/// to blend, so it stays as it is.
#[inline]
fn blend_style(mut style: ResolvedStyle, dim: Option<(Rgb, f32)>) -> ResolvedStyle {
    if let Some((target, factor)) = dim {
        style.fg = style.fg.map(|c| c.lerp(target, factor));
        style.bg = style.bg.map(|c| c.lerp(target, factor));
    }
    style
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests;
