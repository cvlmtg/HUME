use unicode_segmentation::UnicodeSegmentation;

use crate::format::unicode_display_width;
use crate::layout::VisibleRange;
use crate::pane::ViewportState;
use crate::providers::GutterColumn;
use crate::theme::Theme;
use crate::types::{CellContent, DisplayRow, EditorMode, Grapheme, ResolvedStyle, RowKind};

// ---------------------------------------------------------------------------
// Stage 4: compose
// ---------------------------------------------------------------------------

/// Per-frame constants needed by `compose_row`. Bundle these once per pane
/// and pass them through without repeating at each call site.
pub(crate) struct ComposeCtx<'a> {
    pub gutter_columns: &'a [Box<dyn GutterColumn>],
    pub visible: &'a VisibleRange,
    pub viewport: &'a ViewportState,
    pub mode: EditorMode,
    pub primary_head_line: usize,
    pub tab_width: u8,
    /// Pre-resolved from `theme.ui.virtual_text` — avoids repeated field access in the hot loop.
    pub tilde_style: ratatui::style::Style,
    /// Pre-resolved from `theme.ui.indent_guide`.
    pub indent_guide_style: ratatui::style::Style,
    pub pane_rect: ratatui::layout::Rect,
    pub theme: &'a Theme,
    /// Background colour from `ui.background`, threaded to every row so trailing
    /// cells and gutter cells use the theme bg rather than the terminal default.
    pub pane_bg: Option<ratatui::style::Color>,
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

/// Render a single display row at `screen_row` into the ratatui buffer.
///
/// `line_str` is the pre-materialised text of the buffer line that owns this
/// row (used to resolve `CellContent::Grapheme` byte ranges). Pass `""` for
/// virtual/filler rows that have no backing buffer line.
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

    // ── Gutter ────────────────────────────────────────────────────────
    let mut gutter_x = compose_ctx.pane_rect.x;
    for (col_provider, &col_width) in compose_ctx.gutter_columns.iter().zip(col_widths.iter()) {
        let cell =
            col_provider.render_row(row.kind, compose_ctx.mode, compose_ctx.primary_head_line);
        let text = cell.as_str();
        // GutterCell.scope is a &'static str, not an interned ScopeId — use
        // the slow path. Gutter rendering is ~100 calls/frame, not per-grapheme.
        let scope_style: ratatui::style::Style =
            compose_ctx.theme.resolve_by_name(cell.scope).into();
        // Cursorline/pane bg is the base; the gutter scope style layers on top.
        // If the scope defines its own bg, it wins; otherwise the row bg shows through.
        let style = match row_bg.or(compose_ctx.pane_bg) {
            Some(bg) => ratatui::style::Style::default().bg(bg).patch(scope_style),
            None => scope_style,
        };

        // Right-align within usable width, then write a trailing separator space.
        // `usable` bounds how much of `text` may be written: a builtin column
        // (only `LineNumberColumn` today) always fits, but a future
        // plugin-supplied column isn't guaranteed to — `set_string` only clips
        // to the terminal buffer, not to this column's width or the pane
        // rect, so an overlong cell would otherwise bleed into the content
        // area or the neighbouring pane. Truncate on grapheme-cluster
        // boundaries (never raw chars/bytes — the project's text-boundary
        // invariant) by accumulating display width until `usable` is
        // exhausted.
        let usable = col_width.saturating_sub(1); // 1 col reserved as right-padding separator
        let mut truncated_len = text.len();
        let mut text_width = 0u16;
        for (byte_idx, g) in text.grapheme_indices(true) {
            let w = unicode_display_width(g) as u16;
            if text_width + w > usable {
                truncated_len = byte_idx;
                break;
            }
            text_width += w;
        }
        let text = &text[..truncated_len];
        let pad = usable.saturating_sub(text_width);
        for px in 0..pad {
            canvas.set_cell(gutter_x + px, y, " ", style);
        }
        canvas.set_string(gutter_x + pad, y, text, style);
        let sep_x = (gutter_x + pad + text_width).min(right_edge.saturating_sub(1));
        canvas.set_cell(sep_x, y, " ", style);

        gutter_x += col_width;
    }

    // ── Content ───────────────────────────────────────────────────────
    let content_x_origin = compose_ctx.pane_rect.x + compose_ctx.visible.gutter_width;
    let h_offset = compose_ctx.viewport.horizontal_offset;

    match row.kind {
        RowKind::Filler => {
            canvas.set_cell(content_x_origin, y, "~", compose_ctx.tilde_style);
            match compose_ctx.pane_bg {
                Some(bg) => canvas.fill_row_bg(content_x_origin + 1, right_edge, y, bg),
                None => canvas.clear_row_span(content_x_origin + 1, right_edge, y),
            }
        }
        _ => {
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
                if g.col + g.width as u16 <= h_offset {
                    continue;
                }
                // Clip cells that start before the viewport edge.
                let visible_col = g.col.saturating_sub(h_offset);
                let screen_x = content_x_origin + visible_col;
                if screen_x >= right_edge {
                    break; // past right edge — done with this row
                }

                let ratatui_style: ratatui::style::Style = (*style).into();

                match &g.content {
                    CellContent::Grapheme => {
                        if g.byte_range.start <= g.byte_range.end
                            && g.byte_range.end <= line_str.len()
                        {
                            let text = &line_str[g.byte_range.clone()];
                            canvas.set_cell(screen_x, y, text, ratatui_style);
                            // For double-width chars, blank the continuation cell.
                            if g.width >= 2 && screen_x + 1 < right_edge {
                                canvas.set_cell(screen_x + 1, y, " ", ratatui_style);
                            }
                        }
                    }
                    CellContent::Indicator(s) => {
                        canvas.set_cell(screen_x, y, s, ratatui_style);
                        // Fill remaining tab/wide cells with spaces.
                        for extra in 1..g.width as u16 {
                            let ex = screen_x + extra;
                            if ex < right_edge {
                                canvas.set_cell(ex, y, " ", ratatui_style);
                            }
                        }
                    }
                    CellContent::Virtual(s) => {
                        canvas.set_cell(screen_x, y, s, ratatui_style);
                    }
                    CellContent::Empty => {
                        canvas.set_cell(screen_x, y, " ", ratatui_style);
                    }
                    CellContent::WidthContinuation => unreachable!(),
                }
            }
        }
    }

    // ── Indent guides ─────────────────────────────────────────────────
    // Draw guides only on line-start rows (not wrap/virtual/filler) so
    // that continuation rows don't clobber content at guide positions.
    // Drawn after content so they appear on top of leading-whitespace cells.
    if matches!(row.kind, RowKind::LineStart { .. }) {
        let depth = graphemes[row.graphemes.clone()]
            .first()
            .map(|g| g.indent_depth)
            .unwrap_or(0);
        let tw = compose_ctx.tab_width.max(1) as u16;
        // Draw a guide at each inner tab-stop: col = k*tw for k in 1..depth.
        // These positions are guaranteed to lie within the leading whitespace.
        for k in 1..depth {
            let guide_col = k as u16 * tw;
            // Account for horizontal scroll.
            if guide_col + tw > h_offset {
                let visible_col = guide_col.saturating_sub(h_offset);
                let screen_x = content_x_origin + visible_col;
                if screen_x < right_edge {
                    canvas.set_cell(screen_x, y, "│", compose_ctx.indent_guide_style);
                }
            }
        }
    }
}

/// Write styled display rows into the ratatui buffer.
///
/// Order of operations:
/// 1. For each display row (clipped to `top_skip_rows` and `content_height`):
///    a. Write gutter cells for all columns.
///    b. Write content cells, accounting for horizontal scroll.
///    c. Overlay indent guides on leading-whitespace cells.
/// 2. Tilde filler rows for empty space past EOF.
///
/// Overlays are composited by the caller (EngineView::render) after this
/// function returns, by calling each `OverlayProvider::render`.
///
/// `compose_ctx` must be pre-constructed by the caller (same pattern as the
/// fused pipeline). `col_widths` is a caller-supplied scratch buffer.
// Kept for testing the non-fused path; the live pipeline uses `compose_fused`.
#[allow(clippy::too_many_arguments, dead_code)]
pub(crate) fn compose(
    rows: &[DisplayRow],
    graphemes: &[Grapheme],
    styles: &[ResolvedStyle],
    line_texts: &str,
    line_text_offsets: &[usize],
    compose_ctx: &ComposeCtx,
    col_widths: &mut Vec<u16>,
    canvas: &mut PaneCanvas,
) {
    // Skip the first `top_skip_rows` rows from the formatted output so the
    // viewport starts partway through a wrapped line when scrolled.
    let visible = compose_ctx.visible;
    let skip = visible.top_skip_rows as usize;
    let render_rows = rows.iter().skip(skip).take(visible.content_height as usize);

    // Pre-compute per-column widths into the caller-supplied scratch (no alloc after warmup).
    col_widths.clear();
    col_widths.extend(
        compose_ctx
            .gutter_columns
            .iter()
            .map(|c| c.width(visible.last_line_idx) as u16),
    );

    let mut screen_row: u16 = 0;
    // Current line text slice, looked up once per new line_idx.
    let mut current_line_str: &str = "";
    let mut current_line: Option<usize> = None;

    for row in render_rows {
        if screen_row >= compose_ctx.pane_rect.height {
            break;
        }

        // ── Look up the pre-materialised line text for this buffer line ────
        // The Format stage already copied the rope into `line_texts`; we just
        // slice it here. No rope access, no allocation.
        match row.kind.line_idx() {
            Some(line_idx) if current_line != Some(line_idx) => {
                current_line = Some(line_idx);
                let rel = line_idx.saturating_sub(visible.line_range.start);
                current_line_str = if rel < line_text_offsets.len() {
                    let start = line_text_offsets[rel];
                    let end = line_text_offsets
                        .get(rel + 1)
                        .copied()
                        .unwrap_or(line_texts.len());
                    &line_texts[start..end]
                } else {
                    ""
                };
            }
            _ => {}
        }

        compose_row(
            row,
            graphemes,
            styles,
            current_line_str,
            screen_row,
            col_widths,
            compose_ctx,
            canvas,
            None,
        );
        screen_row += 1;
    }

    render_tilde_fillers(screen_row, compose_ctx, canvas);
}

/// Draw tilde filler rows from `start_screen_row` up to (but not including)
/// `visible.content_height`, clamped to `pane_rect.height`.
///
/// Used by both the fused pipeline and the `compose` batch wrapper to fill any
/// remaining vertical space after the last real content row has been rendered.
pub(crate) fn render_tilde_fillers(
    start_screen_row: u16,
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
        match compose_ctx.pane_bg {
            Some(bg) => canvas.fill_row_bg(compose_ctx.pane_rect.x, right_edge, y, bg),
            None => canvas.clear_row_span(compose_ctx.pane_rect.x, right_edge, y),
        }
        let content_x = compose_ctx.pane_rect.x + compose_ctx.visible.gutter_width;
        canvas.set_cell(content_x, y, "~", compose_ctx.tilde_style);
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
mod tests {
    use super::*;
    use crate::format::strip_line_ending;
    use crate::pane::ViewportState;
    use crate::theme::Theme;
    use crate::types::{CellContent, DisplayRow, Grapheme, ResolvedStyle, RowKind};
    use ropey::Rope;

    fn make_test_buf(w: u16, h: u16) -> ratatui::buffer::Buffer {
        ratatui::buffer::Buffer::empty(ratatui::layout::Rect {
            x: 0,
            y: 0,
            width: w,
            height: h,
        })
    }

    fn simple_row(graphemes: std::ops::Range<usize>) -> DisplayRow {
        DisplayRow {
            kind: RowKind::LineStart { line_idx: 0 },
            graphemes,
        }
    }

    fn simple_grapheme(col: u16, byte_start: usize, ch_len: usize) -> Grapheme {
        Grapheme {
            byte_range: byte_start..byte_start + ch_len,
            // char_offset is not needed for render tests (selections handled in style stage).
            char_offset: byte_start,
            col,
            width: 1,
            content: CellContent::Grapheme,
            indent_depth: 0,
            scope: None,
        }
    }

    /// Materialise all lines in `line_range` from `rope` into the flat
    /// `line_texts` / `line_text_offsets` format that `compose` expects.
    fn make_line_texts(rope: &Rope, line_range: std::ops::Range<usize>) -> (String, Vec<usize>) {
        let mut texts = String::new();
        let mut offsets: Vec<usize> = Vec::new();
        for line_idx in line_range {
            offsets.push(texts.len());
            if line_idx < rope.len_lines() {
                for chunk in rope.line(line_idx).chunks() {
                    texts.push_str(chunk);
                }
                strip_line_ending(&mut texts);
            }
        }
        (texts, offsets)
    }

    #[test]
    fn renders_simple_text() {
        let rope = Rope::from_str("hi\n");
        let graphemes = vec![simple_grapheme(0, 0, 1), simple_grapheme(1, 1, 1)];
        let rows = vec![simple_row(0..2)];
        let styles = vec![ResolvedStyle::default(); 2];
        let visible = VisibleRange {
            line_range: 0..1,
            top_skip_rows: 0,
            content_height: 5,
            content_width: 20,
            gutter_width: 0,
            last_line_idx: 0,
        };
        let viewport = ViewportState::new(20, 5);
        let pane_rect = ratatui::layout::Rect {
            x: 0,
            y: 0,
            width: 20,
            height: 5,
        };
        let mut buf = make_test_buf(20, 5);
        let theme = Theme::default();
        let (line_texts, line_text_offsets) = make_line_texts(&rope, visible.line_range.clone());
        let mut col_widths = Vec::new();
        let ctx = ComposeCtx {
            gutter_columns: &[],
            visible: &visible,
            viewport: &viewport,
            mode: EditorMode::Normal,
            primary_head_line: 0,
            tab_width: 4,
            tilde_style: ratatui::style::Style::default(),
            indent_guide_style: ratatui::style::Style::default(),
            pane_rect,
            theme: &theme,
            pane_bg: None,
        };
        let mut canvas = PaneCanvas::new(&mut buf, None);
        compose(
            &rows,
            &graphemes,
            &styles,
            &line_texts,
            &line_text_offsets,
            &ctx,
            &mut col_widths,
            &mut canvas,
        );

        assert_eq!(
            buf.cell(ratatui::layout::Position { x: 0, y: 0 })
                .unwrap()
                .symbol(),
            "h"
        );
        assert_eq!(
            buf.cell(ratatui::layout::Position { x: 1, y: 0 })
                .unwrap()
                .symbol(),
            "i"
        );
    }

    #[test]
    fn filler_rows_have_tilde() {
        let rope = Rope::from_str("x\n");
        let graphemes = vec![simple_grapheme(0, 0, 1)];
        let rows = [simple_row(0..1)];
        let styles = vec![ResolvedStyle::default()];
        let visible = VisibleRange {
            line_range: 0..1,
            top_skip_rows: 0,
            content_height: 5, // 5 rows requested but only 1 line
            content_width: 20,
            gutter_width: 0,
            last_line_idx: 0,
        };
        let viewport = ViewportState::new(20, 5);
        let pane_rect = ratatui::layout::Rect {
            x: 0,
            y: 0,
            width: 20,
            height: 5,
        };
        let mut buf = make_test_buf(20, 5);
        let theme = Theme::default();
        let (line_texts, line_text_offsets) = make_line_texts(&rope, visible.line_range.clone());
        let mut col_widths = Vec::new();
        let ctx = ComposeCtx {
            gutter_columns: &[],
            visible: &visible,
            viewport: &viewport,
            mode: EditorMode::Normal,
            primary_head_line: 0,
            tab_width: 4,
            tilde_style: ratatui::style::Style::default(),
            indent_guide_style: ratatui::style::Style::default(),
            pane_rect,
            theme: &theme,
            pane_bg: None,
        };
        let mut canvas = PaneCanvas::new(&mut buf, None);
        compose(
            &rows,
            &graphemes,
            &styles,
            &line_texts,
            &line_text_offsets,
            &ctx,
            &mut col_widths,
            &mut canvas,
        );

        // Row 0 has 'x', rows 1–4 should have '~'
        for r in 1..5u16 {
            assert_eq!(
                buf.cell(ratatui::layout::Position { x: 0, y: r })
                    .unwrap()
                    .symbol(),
                "~",
                "row {} should be tilde",
                r
            );
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn do_compose(
        rope: &Rope,
        rows: &[DisplayRow],
        graphemes: &[Grapheme],
        styles: &[ResolvedStyle],
        visible: VisibleRange,
        viewport: ViewportState,
        tab_width: u8,
        w: u16,
        h: u16,
    ) -> ratatui::buffer::Buffer {
        let pane_rect = ratatui::layout::Rect {
            x: 0,
            y: 0,
            width: w,
            height: h,
        };
        let mut buf = make_test_buf(w, h);
        let theme = Theme::default();
        let (line_texts, line_text_offsets) = make_line_texts(rope, visible.line_range.clone());
        let mut col_widths = Vec::new();
        let ctx = ComposeCtx {
            gutter_columns: &[],
            visible: &visible,
            viewport: &viewport,
            mode: EditorMode::Normal,
            primary_head_line: 0,
            tab_width,
            tilde_style: ratatui::style::Style::default(),
            indent_guide_style: ratatui::style::Style::default(),
            pane_rect,
            theme: &theme,
            pane_bg: None,
        };
        let mut canvas = PaneCanvas::new(&mut buf, None);
        compose(
            rows,
            graphemes,
            styles,
            &line_texts,
            &line_text_offsets,
            &ctx,
            &mut col_widths,
            &mut canvas,
        );
        buf
    }

    #[test]
    fn top_skip_rows_skips_first_row() {
        let rope = Rope::from_str("ab\ncd\n");
        // Two rows, skip the first → only "cd" appears at screen row 0.
        let g = vec![
            simple_grapheme(0, 0, 1), // 'a' on line 0
            simple_grapheme(1, 1, 1), // 'b' on line 0
            Grapheme {
                byte_range: 0..1,
                char_offset: 3,
                col: 0,
                width: 1,
                content: CellContent::Grapheme,
                indent_depth: 0,
                scope: None,
            }, // 'c' on line 1
            Grapheme {
                byte_range: 1..2,
                char_offset: 4,
                col: 1,
                width: 1,
                content: CellContent::Grapheme,
                indent_depth: 0,
                scope: None,
            }, // 'd' on line 1
        ];
        let rows = vec![
            DisplayRow {
                kind: RowKind::LineStart { line_idx: 0 },
                graphemes: 0..2,
            },
            DisplayRow {
                kind: RowKind::LineStart { line_idx: 1 },
                graphemes: 2..4,
            },
        ];
        let styles = vec![ResolvedStyle::default(); 4];
        let visible = VisibleRange {
            line_range: 0..2,
            top_skip_rows: 1, // skip row 0
            content_height: 5,
            content_width: 20,
            gutter_width: 0,
            last_line_idx: 1,
        };
        let viewport = ViewportState::new(20, 5);
        let buf = do_compose(&rope, &rows, &g, &styles, visible, viewport, 4, 20, 5);
        // With skip=1, the first rendered row should show line 1 ("cd").
        assert_eq!(
            buf.cell(ratatui::layout::Position { x: 0, y: 0 })
                .unwrap()
                .symbol(),
            "c"
        );
        assert_eq!(
            buf.cell(ratatui::layout::Position { x: 1, y: 0 })
                .unwrap()
                .symbol(),
            "d"
        );
    }

    #[test]
    fn horizontal_scroll_clips_left_columns() {
        let rope = Rope::from_str("abcde");
        let graphemes: Vec<Grapheme> = (0..5u16)
            .map(|i| Grapheme {
                byte_range: (i as usize)..(i as usize + 1),
                char_offset: i as usize,
                col: i,
                width: 1,
                content: CellContent::Grapheme,
                indent_depth: 0,
                scope: None,
            })
            .collect();
        let rows = vec![simple_row(0..5)];
        let styles = vec![ResolvedStyle::default(); 5];
        let visible = VisibleRange {
            line_range: 0..1,
            top_skip_rows: 0,
            content_height: 5,
            content_width: 20,
            gutter_width: 0,
            last_line_idx: 0,
        };
        let mut viewport = ViewportState::new(20, 5);
        viewport.horizontal_offset = 2; // skip columns 0 and 1
        let buf = do_compose(
            &rope, &rows, &graphemes, &styles, visible, viewport, 4, 20, 5,
        );
        // With h_offset=2, screen col 0 shows 'c' (buf col 2).
        assert_eq!(
            buf.cell(ratatui::layout::Position { x: 0, y: 0 })
                .unwrap()
                .symbol(),
            "c"
        );
        assert_eq!(
            buf.cell(ratatui::layout::Position { x: 1, y: 0 })
                .unwrap()
                .symbol(),
            "d"
        );
    }

    #[test]
    fn indent_guide_drawn_at_inner_tab_stops() {
        // A line with indent_depth=2 and tab_width=4 should show a guide at col 4.
        // (guides at k*tab_width for k in 1..depth, so k=1 => col 4)
        let rope = Rope::from_str("        foo"); // 8 spaces + "foo"
        let graphemes: Vec<Grapheme> = (0..11u16)
            .map(|i| Grapheme {
                byte_range: (i as usize)..(i as usize + 1),
                char_offset: i as usize,
                col: i,
                width: 1,
                content: CellContent::Grapheme,
                indent_depth: 2, // 8 spaces / 4 tab_width = depth 2
                scope: None,
            })
            .collect();
        let rows = vec![DisplayRow {
            kind: RowKind::LineStart { line_idx: 0 },
            graphemes: 0..11,
        }];
        let styles = vec![ResolvedStyle::default(); 11];
        let visible = VisibleRange {
            line_range: 0..1,
            top_skip_rows: 0,
            content_height: 5,
            content_width: 20,
            gutter_width: 0,
            last_line_idx: 0,
        };
        let viewport = ViewportState::new(20, 5);
        let buf = do_compose(
            &rope, &rows, &graphemes, &styles, visible, viewport, 4, 20, 5,
        );
        // A guide should appear at screen col 4 (k=1, tw=4).
        assert_eq!(
            buf.cell(ratatui::layout::Position { x: 4, y: 0 })
                .unwrap()
                .symbol(),
            "│"
        );
        // Col 0 has the space content (no guide at depth boundary).
        assert_ne!(
            buf.cell(ratatui::layout::Position { x: 0, y: 0 })
                .unwrap()
                .symbol(),
            "│"
        );
        // Col 8 is where content starts — no guide there.
        assert_ne!(
            buf.cell(ratatui::layout::Position { x: 8, y: 0 })
                .unwrap()
                .symbol(),
            "│"
        );
    }

    #[test]
    fn indent_guide_not_drawn_on_wrap_rows() {
        let rope = Rope::from_str("    text");
        let graphemes: Vec<Grapheme> = (0..8u16)
            .map(|i| Grapheme {
                byte_range: (i as usize)..(i as usize + 1),
                char_offset: i as usize,
                col: i,
                width: 1,
                content: CellContent::Grapheme,
                indent_depth: 1,
                scope: None,
            })
            .collect();
        let rows = vec![
            DisplayRow {
                kind: RowKind::LineStart { line_idx: 0 },
                graphemes: 0..4,
            },
            DisplayRow {
                kind: RowKind::Wrap {
                    line_idx: 0,
                    wrap_row: 1,
                },
                graphemes: 4..8,
            },
        ];
        let styles = vec![ResolvedStyle::default(); 8];
        let visible = VisibleRange {
            line_range: 0..1,
            top_skip_rows: 0,
            content_height: 5,
            content_width: 20,
            gutter_width: 0,
            last_line_idx: 0,
        };
        let viewport = ViewportState::new(20, 5);
        let buf = do_compose(
            &rope, &rows, &graphemes, &styles, visible, viewport, 4, 20, 5,
        );
        // depth=1 means no inner guides (guides at k in 1..1 — empty range).
        // Also verify wrap row (screen row 1) has no guide either.
        assert_ne!(
            buf.cell(ratatui::layout::Position { x: 0, y: 1 })
                .unwrap()
                .symbol(),
            "│"
        );
    }

    #[test]
    fn indicator_content_fills_tab_width() {
        // A tab indicator with width=4 should write the indicator char at col 0
        // and spaces at cols 1-3.
        let rope = Rope::from_str("\t");
        let graphemes = vec![Grapheme {
            byte_range: 0..1,
            char_offset: 0,
            col: 0,
            width: 4,
            content: CellContent::Indicator("→"),
            indent_depth: 0,
            scope: None,
        }];
        let rows = [simple_row(0..1)];
        let styles = vec![ResolvedStyle::default()];
        let visible = VisibleRange {
            line_range: 0..1,
            top_skip_rows: 0,
            content_height: 5,
            content_width: 20,
            gutter_width: 0,
            last_line_idx: 0,
        };
        let viewport = ViewportState::new(20, 5);
        let buf = do_compose(
            &rope, &rows, &graphemes, &styles, visible, viewport, 4, 20, 5,
        );
        assert_eq!(
            buf.cell(ratatui::layout::Position { x: 0, y: 0 })
                .unwrap()
                .symbol(),
            "→"
        );
        assert_eq!(
            buf.cell(ratatui::layout::Position { x: 1, y: 0 })
                .unwrap()
                .symbol(),
            " "
        );
        assert_eq!(
            buf.cell(ratatui::layout::Position { x: 2, y: 0 })
                .unwrap()
                .symbol(),
            " "
        );
        assert_eq!(
            buf.cell(ratatui::layout::Position { x: 3, y: 0 })
                .unwrap()
                .symbol(),
            " "
        );
    }

    // ── Gutter text overflow (B8) ────────────────────────────────────────

    struct OverlongGutter;
    impl GutterColumn for OverlongGutter {
        fn width(&self, _: usize) -> u8 {
            4
        }
        fn render_row(&self, _: RowKind, _: EditorMode, _: usize) -> crate::providers::GutterCell {
            crate::providers::GutterCell {
                content: crate::providers::GutterCellContent::Static("TOOLONG"),
                scope: crate::types::Scope("ui.linenr"),
            }
        }
        fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
            self
        }
    }

    #[test]
    fn gutter_text_wider_than_column_is_truncated_not_bled_into_content() {
        // Gutter column width() = 4 → usable = 3. Cell text "TOOLONG" (7
        // cols) must truncate to "TOO", not spill "LONG" into the content
        // area (which starts right after the gutter, at x=4).
        let graphemes = vec![simple_grapheme(0, 0, 1)];
        let rows = [simple_row(0..1)];
        let styles = vec![ResolvedStyle::default()];
        let gutter_columns: Vec<Box<dyn GutterColumn>> = vec![Box::new(OverlongGutter)];
        let visible = VisibleRange {
            line_range: 0..1,
            top_skip_rows: 0,
            content_height: 1,
            content_width: 6,
            gutter_width: 4,
            last_line_idx: 0,
        };
        let viewport = ViewportState::new(10, 1);
        let pane_rect = ratatui::layout::Rect {
            x: 0,
            y: 0,
            width: 10,
            height: 1,
        };
        let mut buf = make_test_buf(10, 1);
        let theme = Theme::default();
        let col_widths = vec![4u16];
        let ctx = ComposeCtx {
            gutter_columns: &gutter_columns,
            visible: &visible,
            viewport: &viewport,
            mode: EditorMode::Normal,
            primary_head_line: 0,
            tab_width: 4,
            tilde_style: ratatui::style::Style::default(),
            indent_guide_style: ratatui::style::Style::default(),
            pane_rect,
            theme: &theme,
            pane_bg: None,
        };
        let mut canvas = PaneCanvas::new(&mut buf, None);
        compose_row(
            &rows[0],
            &graphemes,
            &styles,
            "X",
            0,
            &col_widths,
            &ctx,
            &mut canvas,
            None,
        );

        let sym = |x: u16| {
            buf.cell(ratatui::layout::Position { x, y: 0 })
                .unwrap()
                .symbol()
                .to_string()
        };
        assert_eq!(sym(0), "T");
        assert_eq!(sym(1), "O");
        assert_eq!(sym(2), "O");
        assert_eq!(sym(3), " ", "separator cell, not 'L'");
        // Content area starts at x=4 (gutter_width). Must show the real
        // grapheme 'X', never a straggler from "LONG".
        assert_eq!(sym(4), "X");
        assert_ne!(sym(4), "L");
        assert_ne!(sym(5), "O");
        assert_ne!(sym(6), "N");
    }

    #[test]
    fn gutter_overflow_does_not_bleed_into_neighbouring_pane() {
        // Same overlong-gutter setup as above, but with a narrow pane_rect
        // (width 5 = gutter(4) + 1 content col) simulating a second pane
        // starting immediately at x=5 in the same shared buffer. Pre-seed
        // the whole buffer with a marker glyph so any write past this pane's
        // own right edge is directly observable.
        let graphemes = vec![simple_grapheme(0, 0, 1)];
        let rows = [simple_row(0..1)];
        let styles = vec![ResolvedStyle::default()];
        let gutter_columns: Vec<Box<dyn GutterColumn>> = vec![Box::new(OverlongGutter)];
        let visible = VisibleRange {
            line_range: 0..1,
            top_skip_rows: 0,
            content_height: 1,
            content_width: 1,
            gutter_width: 4,
            last_line_idx: 0,
        };
        let viewport = ViewportState::new(5, 1);
        let pane_rect = ratatui::layout::Rect {
            x: 0,
            y: 0,
            width: 5,
            height: 1,
        };
        let mut buf = make_test_buf(11, 1);
        for x in 0..11u16 {
            set_cell(&mut buf, x, 0, "Z", ratatui::style::Style::default());
        }
        let theme = Theme::default();
        let col_widths = vec![4u16];
        let ctx = ComposeCtx {
            gutter_columns: &gutter_columns,
            visible: &visible,
            viewport: &viewport,
            mode: EditorMode::Normal,
            primary_head_line: 0,
            tab_width: 4,
            tilde_style: ratatui::style::Style::default(),
            indent_guide_style: ratatui::style::Style::default(),
            pane_rect,
            theme: &theme,
            pane_bg: None,
        };
        let mut canvas = PaneCanvas::new(&mut buf, None);
        compose_row(
            &rows[0],
            &graphemes,
            &styles,
            "X",
            0,
            &col_widths,
            &ctx,
            &mut canvas,
            None,
        );

        // x=5..10 belongs to the "next pane" — must remain untouched ('Z').
        for x in 5..11u16 {
            assert_eq!(
                buf.cell(ratatui::layout::Position { x, y: 0 })
                    .unwrap()
                    .symbol(),
                "Z",
                "neighbouring pane's column {x} must be untouched"
            );
        }
    }

    #[test]
    fn set_cell_out_of_bounds_no_panic() {
        let mut buf = make_test_buf(10, 5);
        // Call with coordinates well beyond the buffer area — must not panic.
        set_cell(&mut buf, 100, 100, "x", ratatui::style::Style::default());
        set_cell(&mut buf, 10, 0, "x", ratatui::style::Style::default()); // exactly at boundary
    }

    #[test]
    fn clear_row_span_fills_with_blank() {
        let mut buf = make_test_buf(10, 3);
        // Write something so we can confirm clearing works.
        for x in 0..10 {
            set_cell(&mut buf, x, 1, "X", ratatui::style::Style::default());
        }
        // Clear the middle 4 columns of row 1.
        clear_row_span(&mut buf, 3, 7, 1);
        for x in 0..10 {
            let sym = buf
                .cell(ratatui::layout::Position { x, y: 1 })
                .unwrap()
                .symbol();
            if (3..7).contains(&x) {
                assert_eq!(sym, " ", "col {x} should be blank");
            } else {
                assert_eq!(sym, "X", "col {x} should be untouched");
            }
        }
    }

    #[test]
    fn clear_row_span_clips_right_edge() {
        let mut buf = make_test_buf(10, 3);
        for x in 0..10 {
            set_cell(&mut buf, x, 0, "X", ratatui::style::Style::default());
        }
        // x_end extends past the buffer's right edge — should clip, not panic.
        clear_row_span(&mut buf, 8, 20, 0);
        for x in 0..10 {
            let sym = buf
                .cell(ratatui::layout::Position { x, y: 0 })
                .unwrap()
                .symbol();
            if x >= 8 {
                assert_eq!(sym, " ");
            } else {
                assert_eq!(sym, "X");
            }
        }
    }

    #[test]
    fn clear_row_span_empty_range_no_panic() {
        let mut buf = make_test_buf(10, 3);
        // x_start == x_end and x_start > x_end should both be no-ops.
        clear_row_span(&mut buf, 5, 5, 0);
        clear_row_span(&mut buf, 7, 3, 0);
    }

    // ── fused dim (compose path) ───────────────────────────────────────

    /// `dim` on `PaneCanvas` must blend each written cell's fg/bg toward the
    /// target inline — replacing the old `dim_rect` post-pass. Verifies the
    /// same lerp oracle (255→0 at 0.5 ⇒ 128) holds through `compose_row`.
    #[test]
    fn compose_row_dims_cells_inline() {
        use ratatui::style::Color;
        let rope = Rope::from_str("x\n");
        let graphemes = vec![simple_grapheme(0, 0, 1)];
        let rows = [simple_row(0..1)];
        let styles = vec![ResolvedStyle {
            fg: Some(Color::Rgb(255, 255, 255)),
            bg: Some(Color::Rgb(0, 0, 0)),
            ..Default::default()
        }];
        let visible = VisibleRange {
            line_range: 0..1,
            top_skip_rows: 0,
            content_height: 1,
            content_width: 2,
            gutter_width: 0,
            last_line_idx: 0,
        };
        let viewport = ViewportState::new(2, 1);
        let pane_rect = ratatui::layout::Rect {
            x: 0,
            y: 0,
            width: 2,
            height: 1,
        };
        let mut buf = make_test_buf(2, 1);
        let theme = Theme::default();
        let (line_texts, line_text_offsets) = make_line_texts(&rope, visible.line_range.clone());
        let mut col_widths = Vec::new();
        let ctx = ComposeCtx {
            gutter_columns: &[],
            visible: &visible,
            viewport: &viewport,
            mode: EditorMode::Normal,
            primary_head_line: 0,
            tab_width: 4,
            tilde_style: ratatui::style::Style::default(),
            indent_guide_style: ratatui::style::Style::default(),
            pane_rect,
            theme: &theme,
            pane_bg: Some(Color::Rgb(0, 0, 0)),
        };
        let mut canvas = PaneCanvas::new(&mut buf, Some((Color::Rgb(0, 0, 0), 0.5)));
        compose(
            &rows,
            &graphemes,
            &styles,
            &line_texts,
            &line_text_offsets,
            &ctx,
            &mut col_widths,
            &mut canvas,
        );
        let cell = buf.cell(ratatui::layout::Position { x: 0, y: 0 }).unwrap();
        // Independent oracle: 255 lerp 0 at 0.5 ⇒ 127.5, rounds to 128.
        assert_eq!(cell.fg, Color::Rgb(128, 128, 128));
        // bg already at target ⇒ blend is a no-op.
        assert_eq!(cell.bg, Color::Rgb(0, 0, 0));
    }

    /// A non-RGB dim target must be a no-op (cell keeps its original colours),
    /// matching the prior `dim_rect` semantics.
    #[test]
    fn compose_row_non_rgb_dim_target_is_noop() {
        use ratatui::style::Color;
        let rope = Rope::from_str("x\n");
        let graphemes = vec![simple_grapheme(0, 0, 1)];
        let rows = [simple_row(0..1)];
        let styles = vec![ResolvedStyle {
            fg: Some(Color::Rgb(255, 255, 255)),
            ..Default::default()
        }];
        let visible = VisibleRange {
            line_range: 0..1,
            top_skip_rows: 0,
            content_height: 1,
            content_width: 2,
            gutter_width: 0,
            last_line_idx: 0,
        };
        let viewport = ViewportState::new(2, 1);
        let pane_rect = ratatui::layout::Rect {
            x: 0,
            y: 0,
            width: 2,
            height: 1,
        };
        let mut buf = make_test_buf(2, 1);
        let theme = Theme::default();
        let (line_texts, line_text_offsets) = make_line_texts(&rope, visible.line_range.clone());
        let mut col_widths = Vec::new();
        let ctx = ComposeCtx {
            gutter_columns: &[],
            visible: &visible,
            viewport: &viewport,
            mode: EditorMode::Normal,
            primary_head_line: 0,
            tab_width: 4,
            tilde_style: ratatui::style::Style::default(),
            indent_guide_style: ratatui::style::Style::default(),
            pane_rect,
            theme: &theme,
            pane_bg: Some(Color::Rgb(0, 0, 0)),
        };
        let mut canvas = PaneCanvas::new(&mut buf, Some((Color::Reset, 0.5)));
        compose(
            &rows,
            &graphemes,
            &styles,
            &line_texts,
            &line_text_offsets,
            &ctx,
            &mut col_widths,
            &mut canvas,
        );
        assert_eq!(
            buf.cell(ratatui::layout::Position { x: 0, y: 0 })
                .unwrap()
                .fg,
            Color::Rgb(255, 255, 255)
        );
    }
}
