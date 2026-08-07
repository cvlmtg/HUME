use unicode_segmentation::UnicodeSegmentation;

use crate::format::unicode_display_width;
use crate::layout::PaneGeometry;
use crate::pane::ViewportState;
use crate::providers::{GutterColumn, GutterRowCtx, ProviderId};
use crate::theme::Theme;
use crate::types::{CellContent, DisplayRow, EditorMode, Grapheme, ResolvedStyle, RowKind, ScopeId};

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
    pub tilde_style: ratatui::style::Style,
    /// Pre-resolved from `theme.ui.indent_guide`.
    pub indent_guide_style: ratatui::style::Style,
    /// From the `indent-guides` setting — gates the draw loop below.
    pub show_indent_guides: bool,
    pub pane_rect: ratatui::layout::Rect,
    pub theme: &'a Theme,
    /// Background colour from `ui.background`, threaded to every row so trailing
    /// cells and gutter cells use the theme bg rather than the terminal default.
    pub pane_bg: Option<ratatui::style::Color>,
    /// Buffer rope, passed to `GutterColumn::render_row` via `GutterRowCtx`
    /// so gutter providers (git-signs, diagnostics) can query buffer content
    /// without pre-owning it.
    pub rope: &'a ropey::Rope,
    /// `DEFAULT_GUTTER_SCOPE` ("ui.linenr"), interned once at
    /// `EngineView::new` — the fallback scope `compose_gutter` and
    /// `render_tilde_fillers` resolve under when a cell/column has nothing
    /// more specific to say. Threaded in rather than re-interned here: the
    /// per-cell hot path only ever does an O(1) `ScopeId` index.
    pub default_gutter_scope: ScopeId,
}

/// The pane's drawing surface — every cell write for a pane goes through here.
///
/// Wraps the ratatui `Buffer` and, when set, a dim target: fg/bg is blended
/// toward it on every write. This is the single chokepoint for the non-focused
/// pane dim effect — `compose_row` / `render_tilde_fillers` never touch `buf`
/// directly, so a future write site cannot forget to dim. Replaces the old
/// `dim_rect` post-pass (a second full-rect walk after `render_pane`) without
/// reopening that gap: the blend still happens exactly once per cell, just
/// inline in the single write instead of a separate sweep.
pub(crate) struct PaneCanvas<'a> {
    buf: &'a mut ratatui::buffer::Buffer,
    dim: Option<DimTarget>,
}

/// Flattened, per-cell-ready form of a `(Color, f32)` dim target.
///
/// Resolving `Color::Rgb(..)` out of the enum happens once here, in
/// `PaneCanvas::new`, rather than on every `blend_color`/`blend_style` call —
/// `dim` is loop-invariant for the whole pane, so re-matching it per cell
/// (once per gutter cell, per grapheme, per indent-guide cell) was pure
/// per-frame overhead.
#[derive(Clone, Copy)]
struct DimTarget {
    r: u8,
    g: u8,
    b: u8,
    factor: f32,
}

impl<'a> PaneCanvas<'a> {
    pub(crate) fn new(
        buf: &'a mut ratatui::buffer::Buffer,
        dim: Option<(ratatui::style::Color, f32)>,
    ) -> Self {
        // Non-RGB target is a no-op — mirrors the prior `dim_rect` semantics.
        let dim = dim.and_then(|(color, factor)| match color {
            ratatui::style::Color::Rgb(r, g, b) => Some(DimTarget { r, g, b, factor }),
            _ => None,
        });
        Self { buf, dim }
    }

    fn set_cell(&mut self, x: u16, y: u16, text: &str, style: ratatui::style::Style) {
        set_cell(self.buf, x, y, text, blend_style(style, self.dim));
    }

    fn set_string(&mut self, x: u16, y: u16, text: &str, style: ratatui::style::Style) {
        self.buf
            .set_string(x, y, text, blend_style(style, self.dim));
    }

    fn fill_row_bg(&mut self, x_start: u16, x_end: u16, y: u16, bg: ratatui::style::Color) {
        fill_row_bg(self.buf, x_start, x_end, y, blend_color(bg, self.dim));
    }

    /// Writes `Cell::default()` (terminal-default colours), taken only when
    /// `pane_bg` is `None` — which is exactly when `dim` is `None` too (the
    /// pipeline gates both on the same `theme.ui.background.bg`). No blend needed.
    fn clear_row_span(&mut self, x_start: u16, x_end: u16, y: u16) {
        clear_row_span(self.buf, x_start, x_end, y);
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
    row_bg: Option<ratatui::style::Color>,
) -> ratatui::style::Style {
    let scope_style: ratatui::style::Style = theme.resolve(scope).into();
    match row_bg {
        Some(bg) => ratatui::style::Style::default().bg(bg).patch(scope_style),
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
/// `col_widths` must already be populated by the caller (one entry per
/// gutter column) — see `compose_row`'s doc comment for why it isn't folded
/// into `ComposeCtx`.
fn compose_gutter(
    row_kind: RowKind,
    col_widths: &[u16],
    compose_ctx: &ComposeCtx,
    row_bg: Option<ratatui::style::Color>,
    y: u16,
    canvas: &mut PaneCanvas,
) {
    let mut gutter_x = compose_ctx.pane_rect.x;
    // A column's configured width (in particular `signcolumn`'s up-to-127
    // slots) is never checked against the pane's actual width — `layout.rs`
    // only clamps *content* width down to make room for the gutter, not the
    // other way around. Without this bound, a gutter wider than the pane
    // would write straight through the pane's right edge into whatever is
    // drawn next to it in the shared terminal buffer (a neighbouring pane,
    // most commonly).
    let pane_right_edge = compose_ctx.pane_rect.x + compose_ctx.pane_rect.width;
    let gutter_ctx = GutterRowCtx {
        mode: compose_ctx.mode,
        primary_head_line: compose_ctx.primary_head_line,
        rope: compose_ctx.rope,
    };
    for ((_, col_provider), &col_width) in compose_ctx.gutter_columns.iter().zip(col_widths.iter())
    {
        if col_width == 0 {
            continue;
        }
        let col_start = gutter_x;
        if col_start >= pane_right_edge {
            continue;
        }
        let col_width = col_width.min(pane_right_edge - col_start);
        let cells = col_provider.render_row_cells(row_kind, &gutter_ctx);
        // Distribute `col_width` across `cells.len()` sub-cells. Only the
        // column's right padding (1 cell) is reserved — no separators between
        // sub-cells. `usable_per_cell` is how much of each sub-cell's text
        // may be written before truncation.
        let n_cells = cells.len().max(1);
        let usable_per_cell = col_width.saturating_sub(1) / n_cells as u16;
        let mut last_scope: ScopeId = compose_ctx.default_gutter_scope;
        for (cell_idx, cell) in cells.iter().enumerate() {
            let is_last = cell_idx == cells.len() - 1;
            let text = cell.as_str();
            let style = gutter_cell_style(
                cell.scope,
                compose_ctx.theme,
                row_bg.or(compose_ctx.pane_bg),
            );

            // Right-align within usable width. `usable_per_cell` bounds how
            // much of `text` may be written: a builtin column (only
            // `LineNumberColumn` today) always fits, but a future
            // plugin-supplied column isn't guaranteed to — `set_string` only
            // clips to the terminal buffer, not to this column's width or
            // the pane rect, so an overlong cell would otherwise bleed into
            // the content area or the neighbouring pane. Truncate on
            // grapheme-cluster boundaries (never raw chars/bytes — the
            // project's text-boundary invariant) by accumulating display
            // width until `usable_per_cell` is exhausted.
            let mut truncated_len = text.len();
            let mut text_width = 0u16;
            for (byte_idx, g) in text.grapheme_indices(true) {
                let w = unicode_display_width(g) as u16;
                if text_width + w > usable_per_cell {
                    truncated_len = byte_idx;
                    break;
                }
                text_width += w;
            }
            let text = &text[..truncated_len];
            let pad = usable_per_cell.saturating_sub(text_width);
            for px in 0..pad {
                canvas.set_cell(gutter_x + px, y, " ", style);
            }
            canvas.set_string(gutter_x + pad, y, text, style);
            // Only write a separator after the last cell — it's the column's
            // right padding, not a separator between sub-cells.
            if is_last {
                let sep_x = gutter_x + pad + text_width;
                canvas.set_cell(sep_x, y, " ", style);
                gutter_x += usable_per_cell + 1;
            } else {
                gutter_x += usable_per_cell;
            }
            last_scope = cell.scope;
        }
        // Any leftover width (e.g. sub-cell widths that don't evenly divide
        // col_width - 1) fills as blanks under the last cell's scope —
        // preserves the single-cell builtin behaviour where the whole column
        // shared one scope. Bounded by `col_start`, not `pane_rect.x`: for
        // every column after the first, `pane_rect.x` is the pane's left
        // edge, not this column's — using it here left leftover cells
        // unpainted and `gutter_x` short of the column boundary for any
        // non-first column with uneven leftover.
        if gutter_x < col_start + col_width {
            let style = gutter_cell_style(
                last_scope,
                compose_ctx.theme,
                row_bg.or(compose_ctx.pane_bg),
            );
            while gutter_x < col_start + col_width {
                canvas.set_cell(gutter_x, y, " ", style);
                gutter_x += 1;
            }
        }
        // Providers are a public extension point; their cell-width math
        // isn't guaranteed to sum to col_width exactly. Land on the column
        // boundary regardless, so the next column never inherits any drift.
        gutter_x = col_start + col_width;
    }
}

/// Render a single display row at `screen_row` into the ratatui buffer.
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
/// `col_widths` must already be populated by the caller (one entry per gutter
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
    col_widths: &[u16],
    compose_ctx: &ComposeCtx,
    canvas: &mut PaneCanvas,
    // Background colour to fill the entire row (gutter + content) before
    // writing graphemes. Used for cursorline highlighting so the tint
    // extends to the right edge even past the last character.
    // `None` → clear to terminal default (normal rows).
    row_bg: Option<ratatui::style::Color>,
) {
    let y = compose_ctx.pane_rect.y + screen_row;
    let right_edge = compose_ctx.pane_rect.x + compose_ctx.pane_rect.width;

    // Filler rows are rendered exclusively by `render_tilde_fillers`, never
    // routed through here — it has its own gutter + tilde + background
    // handling since a filler row has no backing graphemes to iterate.
    debug_assert!(!matches!(row.kind, RowKind::Filler));

    compose_gutter(row.kind, col_widths, compose_ctx, row_bg, y, canvas);

    // ── Content ───────────────────────────────────────────────────────
    let content_x_origin = compose_ctx.pane_rect.x + compose_ctx.visible.gutter_width;
    let h_offset = compose_ctx.viewport.horizontal_offset;

    // Fill trailing cells with row bg (cursorline) or pane bg, so the theme
    // background shows past the last grapheme rather than the terminal default.
    match row_bg.or(compose_ctx.pane_bg) {
        Some(bg) => canvas.fill_row_bg(content_x_origin, right_edge, y, bg),
        None => canvas.clear_row_span(content_x_origin, right_edge, y),
    }

    let row_graphemes = &graphemes[row.graphemes.start..row.graphemes.end];
    let row_styles = &styles[row.graphemes.start..row.graphemes.end];

    for (g, style) in row_graphemes.iter().zip(row_styles.iter()) {
        // Skip WidthContinuation — already handled by the primary cell.
        if matches!(g.content, CellContent::WidthContinuation) {
            continue;
        }

        // Horizontal scroll: skip cells left of the viewport.
        if g.col + g.width as u32 <= h_offset {
            continue;
        }
        // Clip cells that start before the viewport edge. `g.col` is a
        // document column (`u32`), but this render path always runs behind
        // `with_h_window` (`pane_render.rs`), so a cell surviving the skip
        // above sits within one viewport width of `h_offset` — safely
        // representable in the terminal-cell (`u16`) domain the rest of
        // compose works in.
        let visible_col = g.col.saturating_sub(h_offset);
        debug_assert!(
            u16::try_from(visible_col).is_ok(),
            "on-screen column {visible_col} exceeds a u16 — h_window should have clipped this cell"
        );
        let screen_x = content_x_origin + visible_col as u16;
        if screen_x >= right_edge {
            break; // past right edge — done with this row
        }

        let ratatui_style: ratatui::style::Style = (*style).into();

        // A multi-column cell (double-width CJK grapheme, tab
        // Indicator) whose left edge sits before `h_offset` still
        // passes the skip check above once its right edge crosses
        // it — but `visible_col` above already clamped to 0, so
        // rendering the glyph there would draw its *full* width at
        // the viewport's left edge instead of the fraction that's
        // actually scrolled into view, shifting the row. Render
        // spaces for the visible remainder instead (matches Helix).
        // Impossible for width-1 cells: straddling needs
        // `g.col < h_offset < g.col + g.width`, which has no integer
        // solution when `g.width == 1`.
        if g.col < h_offset {
            let visible_cells = g.width as u32 - (h_offset - g.col);
            for i in 0..visible_cells as u16 {
                let sx = screen_x + i;
                if sx < right_edge {
                    canvas.set_cell(sx, y, " ", ratatui_style);
                }
            }
            continue;
        }

        match &g.content {
            CellContent::Grapheme => {
                if g.byte_range.start <= g.byte_range.end && g.byte_range.end <= line_str.len() {
                    let text = &line_str[g.byte_range.clone()];
                    canvas.set_cell(screen_x, y, text, ratatui_style);
                    // For double-width chars, blank the continuation cell.
                    if g.width >= 2 && screen_x + 1 < right_edge {
                        canvas.set_cell(screen_x + 1, y, " ", ratatui_style);
                    }
                }
            }
            CellContent::Indicator { start, len } => {
                let s = resolve_arena_text(virtual_texts, *start, *len);
                canvas.set_cell(screen_x, y, s, ratatui_style);
                // Fill remaining tab/wide cells with spaces.
                for extra in 1..g.width as u16 {
                    let ex = screen_x + extra;
                    if ex < right_edge {
                        canvas.set_cell(ex, y, " ", ratatui_style);
                    }
                }
            }
            CellContent::Virtual { start, len } => {
                let s = resolve_arena_text(virtual_texts, *start, *len);
                canvas.set_cell(screen_x, y, s, ratatui_style);
            }
            CellContent::Empty => {
                canvas.set_cell(screen_x, y, " ", ratatui_style);
            }
            CellContent::WidthContinuation => unreachable!(),
        }
    }

    // ── Indent guides ─────────────────────────────────────────────────
    // Draw guides only on line-start rows (not wrap/virtual/filler) so
    // that continuation rows don't clobber content at guide positions.
    // Drawn after content so they appear on top of leading-whitespace cells.
    if compose_ctx.show_indent_guides && matches!(row.kind, RowKind::LineStart { .. }) {
        let depth = graphemes[row.graphemes.clone()]
            .first()
            .map(|g| g.indent_depth)
            .unwrap_or(0);
        let tw = compose_ctx.tab_width.max(1) as u16;
        // Draw a guide at each inner tab-stop: col = k*tw for k in 1..depth.
        // These positions are guaranteed to lie within the leading whitespace.
        for k in 1..depth {
            let guide_col = k as u32 * tw as u32;
            // Account for horizontal scroll.
            if guide_col + tw as u32 > h_offset {
                let visible_col = guide_col.saturating_sub(h_offset);
                debug_assert!(
                    u16::try_from(visible_col).is_ok(),
                    "on-screen indent guide column {visible_col} exceeds a u16"
                );
                let screen_x = content_x_origin + visible_col as u16;
                if screen_x < right_edge {
                    canvas.set_cell(
                        screen_x,
                        y,
                        INDENT_GUIDE_GLYPH,
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
    col_widths: &[u16],
    compose_ctx: &ComposeCtx,
    canvas: &mut PaneCanvas,
) {
    let mut screen_row = start_screen_row;
    while screen_row
        < compose_ctx
            .visible
            .content_height
            .min(compose_ctx.pane_rect.height)
    {
        let y = compose_ctx.pane_rect.y + screen_row;
        let right_edge = compose_ctx.pane_rect.x + compose_ctx.pane_rect.width;
        // Gutter first — it already paints a real background (row_bg/pane_bg
        // patched with the column's scope, see `compose_gutter`) across the
        // whole gutter width, including column 0. The tilde below patches its
        // fg on top of that, matching editor convention: `~` sits at the
        // pane's left edge, ignoring/overriding the line-number gutter, never
        // shifted into the content area.
        compose_gutter(RowKind::Filler, col_widths, compose_ctx, None, y, canvas);
        let content_x = compose_ctx.pane_rect.x + compose_ctx.visible.gutter_width;
        match compose_ctx.pane_bg {
            Some(bg) => canvas.fill_row_bg(content_x, right_edge, y, bg),
            None => canvas.clear_row_span(content_x, right_edge, y),
        }
        canvas.set_cell(compose_ctx.pane_rect.x, y, "~", compose_ctx.tilde_style);
        screen_row += 1;
    }
}

// ---------------------------------------------------------------------------
// Cell write helper
// ---------------------------------------------------------------------------

/// Write `text` to the ratatui buffer cell at `(x, y)`, clipping to buffer bounds.
#[inline]
fn set_cell(
    buf: &mut ratatui::buffer::Buffer,
    x: u16,
    y: u16,
    text: &str,
    style: ratatui::style::Style,
) {
    let area = buf.area();
    if x < area.x + area.width
        && y < area.y + area.height
        && let Some(cell) = buf.cell_mut(ratatui::layout::Position { x, y })
    {
        cell.set_symbol(text);
        cell.set_style(style);
    }
}

/// Clamp `rect` to `buf`'s area, returning exclusive `(x0, y0, x1, y1)`
/// bounds ready for a `for y in y0..y1 { for x in x0..x1 }` cell loop.
/// Shared by every rect-filling primitive below so the clip math lives once.
#[inline]
pub(crate) fn clamp_rect_to_buf(
    buf: &ratatui::buffer::Buffer,
    rect: ratatui::layout::Rect,
) -> (u16, u16, u16, u16) {
    let area = buf.area();
    let x0 = rect.x.max(area.x);
    let y0 = rect.y.max(area.y);
    let x1 = (rect.x + rect.width).min(area.x + area.width);
    let y1 = (rect.y + rect.height).min(area.y + area.height);
    (x0, y0, x1, y1)
}

/// Paint every cell of `rect` with a space glyph and `style`, clipping to buffer bounds.
///
/// `Buffer::set_style` only rewrites `Style`, leaving previous glyphs visible.
/// Opaque overlays (popups, statusline fills) need to overwrite the symbol too.
#[inline]
pub fn fill_rect_bg(
    buf: &mut ratatui::buffer::Buffer,
    rect: ratatui::layout::Rect,
    style: ratatui::style::Style,
) {
    let (x0, y0, x1, y1) = clamp_rect_to_buf(buf, rect);
    for y in y0..y1 {
        for x in x0..x1 {
            buf[(x, y)].set_char(' ').set_style(style);
        }
    }
}

/// Blend `color` toward `target` by `factor` (0.0 = unchanged, 1.0 = fully
/// `target`). Non-RGB colors (indexed/named) are returned unchanged — HUME
/// requires true-color themes (see project CLAUDE.md), so callers only ever
/// need to blend `Color::Rgb`.
#[inline]
fn blend_toward(
    color: ratatui::style::Color,
    target: (u8, u8, u8),
    factor: f32,
) -> ratatui::style::Color {
    use ratatui::style::Color;
    let Color::Rgb(r, g, b) = color else {
        return color;
    };
    let lerp = |c: u8, t: u8| (c as f32 + (t as f32 - c as f32) * factor).round() as u8;
    Color::Rgb(lerp(r, target.0), lerp(g, target.1), lerp(b, target.2))
}

/// Blend a single colour toward `dim`'s target, if any.
#[inline]
fn blend_color(color: ratatui::style::Color, dim: Option<DimTarget>) -> ratatui::style::Color {
    let Some(target) = dim else {
        return color;
    };
    blend_toward(color, (target.r, target.g, target.b), target.factor)
}

/// Blend both fg and bg of `style` toward `dim`'s target, if any. `None`
/// fg/bg are left as-is (no colour to blend).
#[inline]
fn blend_style(mut style: ratatui::style::Style, dim: Option<DimTarget>) -> ratatui::style::Style {
    if let Some(target) = dim {
        let rgb = (target.r, target.g, target.b);
        if let Some(fg) = style.fg {
            style = style.fg(blend_toward(fg, rgb, target.factor));
        }
        if let Some(bg) = style.bg {
            style = style.bg(blend_toward(bg, rgb, target.factor));
        }
    }
    style
}

/// Fill a horizontal span with spaces using an explicit background colour.
///
/// Used for cursorline highlighting so the tint extends past the last grapheme.
#[inline]
fn fill_row_bg(
    buf: &mut ratatui::buffer::Buffer,
    x_start: u16,
    x_end: u16,
    y: u16,
    bg: ratatui::style::Color,
) {
    fill_rect_bg(
        buf,
        ratatui::layout::Rect::new(x_start, y, x_end.saturating_sub(x_start), 1),
        ratatui::style::Style::default().bg(bg),
    );
}

/// Fill a horizontal span of cells on row `y` with blank `Cell::default()`.
///
/// Uses a single slice fill instead of per-cell `set_cell` calls.
/// Cells within a row are contiguous in ratatui's row-major backing Vec, so
/// one `index_of` + `fill` replaces N bounds-checked function calls.
/// Clips silently if `x_end` extends past the buffer boundary.
#[inline]
fn clear_row_span(buf: &mut ratatui::buffer::Buffer, x_start: u16, x_end: u16, y: u16) {
    if x_start >= x_end {
        return;
    }
    let area = buf.area();
    let x_start = x_start.max(area.x);
    let x_end = x_end.min(area.x + area.width);
    if x_start >= x_end || y >= area.y + area.height {
        return;
    }
    let start = buf.index_of(x_start, y);
    let end = buf.index_of(x_end - 1, y) + 1;
    buf.content[start..end].fill(ratatui::buffer::Cell::default());
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests;
