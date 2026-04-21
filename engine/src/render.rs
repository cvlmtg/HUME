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
    buf: &mut ratatui::buffer::Buffer,
    // Background colour to fill the entire row (gutter + content) before
    // writing graphemes. Used for cursorline highlighting so the tint
    // extends to the right edge even past the last character.
    // `None` → clear to terminal default (normal rows).
    row_bg: Option<ratatui::style::Color>,
) {
    let y = compose_ctx.pane_rect.y + screen_row;

    // ── Gutter ────────────────────────────────────────────────────────
    let mut gutter_x = compose_ctx.pane_rect.x;
    for (col_provider, &col_width) in compose_ctx.gutter_columns.iter().zip(col_widths.iter()) {
        let cell = col_provider.render_row(row.kind, compose_ctx.mode, compose_ctx.primary_head_line);
        let text = cell.as_str();
        // GutterCell.scope is a &'static str, not an interned ScopeId — use
        // the slow path. Gutter rendering is ~100 calls/frame, not per-grapheme.
        let scope_style: ratatui::style::Style = compose_ctx.theme.resolve_by_name(cell.scope).into();
        // Cursorline bg is the base; the gutter scope style layers on top.
        // If the scope defines its own bg, it wins; otherwise the row bg shows through.
        let style = if let Some(bg) = row_bg {
            ratatui::style::Style::default().bg(bg).patch(scope_style)
        } else {
            scope_style
        };

        // Right-align within usable width, then write a trailing separator space.
        let usable = col_width.saturating_sub(1); // 1 col reserved as right-padding separator
        let text_len = unicode_display_width(text) as u16;
        let pad = usable.saturating_sub(text_len);
        for px in 0..pad {
            set_cell(buf, gutter_x + px, y, " ", style);
        }
        // set_string writes each character into its own cell, handles multi-byte UTF-8.
        buf.set_string(gutter_x + pad, y, text, style);
        set_cell(buf, gutter_x + pad + text_len, y, " ", style);

        gutter_x += col_width;
    }

    // ── Content ───────────────────────────────────────────────────────
    let content_x_origin = compose_ctx.pane_rect.x + compose_ctx.visible.gutter_width;
    let right_edge = compose_ctx.pane_rect.x + compose_ctx.pane_rect.width;
    let h_offset = compose_ctx.viewport.horizontal_offset;

    match row.kind {
        RowKind::Filler => {
            set_cell(buf, content_x_origin, y, "~", compose_ctx.tilde_style);
            clear_row_span(buf, content_x_origin + 1, right_edge, y);
        }
        _ => {
            // Fill the content row with the row background (cursorline or default).
            // This ensures the tint extends to the right edge past the last grapheme.
            match row_bg {
                Some(bg) => fill_row_bg(buf, content_x_origin, right_edge, y, bg),
                None     => clear_row_span(buf, content_x_origin, right_edge, y),
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
                            set_cell(buf, screen_x, y, text, ratatui_style);
                            // For double-width chars, blank the continuation cell.
                            if g.width >= 2 && screen_x + 1 < right_edge {
                                set_cell(buf, screen_x + 1, y, " ", ratatui_style);
                            }
                        }
                    }
                    CellContent::Indicator(s) => {
                        set_cell(buf, screen_x, y, s, ratatui_style);
                        // Fill remaining tab/wide cells with spaces.
                        for extra in 1..g.width as u16 {
                            let ex = screen_x + extra;
                            if ex < right_edge {
                                set_cell(buf, ex, y, " ", ratatui_style);
                            }
                        }
                    }
                    CellContent::Virtual(s) => {
                        set_cell(buf, screen_x, y, s, ratatui_style);
                    }
                    CellContent::Empty => {
                        set_cell(buf, screen_x, y, " ", ratatui_style);
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
                    set_cell(buf, screen_x, y, "│", compose_ctx.indent_guide_style);
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
    buf: &mut ratatui::buffer::Buffer,
) {
    // Skip the first `top_skip_rows` rows from the formatted output so the
    // viewport starts partway through a wrapped line when scrolled.
    let visible = compose_ctx.visible;
    let skip = visible.top_skip_rows as usize;
    let render_rows = rows.iter().skip(skip).take(visible.content_height as usize);

    // Pre-compute per-column widths into the caller-supplied scratch (no alloc after warmup).
    col_widths.clear();
    col_widths.extend(compose_ctx.gutter_columns.iter().map(|c| c.width(visible.last_line_idx) as u16));

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
                    let end = line_text_offsets.get(rel + 1).copied().unwrap_or(line_texts.len());
                    &line_texts[start..end]
                } else {
                    ""
                };
            }
            _ => {}
        }

        compose_row(row, graphemes, styles, current_line_str, screen_row, col_widths, compose_ctx, buf, None);
        screen_row += 1;
    }

    render_tilde_fillers(screen_row, compose_ctx, buf);
}

/// Draw tilde filler rows from `start_screen_row` up to (but not including)
/// `visible.content_height`, clamped to `pane_rect.height`.
///
/// Used by both the fused pipeline and the `compose` batch wrapper to fill any
/// remaining vertical space after the last real content row has been rendered.
pub(crate) fn render_tilde_fillers(
    start_screen_row: u16,
    compose_ctx: &ComposeCtx,
    buf: &mut ratatui::buffer::Buffer,
) {
    let mut screen_row = start_screen_row;
    while screen_row < compose_ctx.visible.content_height.min(compose_ctx.pane_rect.height) {
        let y = compose_ctx.pane_rect.y + screen_row;
        let right_edge = compose_ctx.pane_rect.x + compose_ctx.pane_rect.width;
        clear_row_span(buf, compose_ctx.pane_rect.x, right_edge, y);
        let content_x = compose_ctx.pane_rect.x + compose_ctx.visible.gutter_width;
        set_cell(buf, content_x, y, "~", compose_ctx.tilde_style);
        screen_row += 1;
    }
}

// ---------------------------------------------------------------------------
// Cell write helper
// ---------------------------------------------------------------------------

/// Write `text` to the ratatui buffer cell at `(x, y)`, clipping to buffer bounds.
#[inline]
fn set_cell(buf: &mut ratatui::buffer::Buffer, x: u16, y: u16, text: &str, style: ratatui::style::Style) {
    let area = buf.area();
    if x < area.x + area.width
        && y < area.y + area.height
        && let Some(cell) = buf.cell_mut(ratatui::layout::Position { x, y })
    {
        cell.set_symbol(text);
        cell.set_style(style);
    }
}

/// Fill a horizontal span with spaces using an explicit background colour.
///
/// Used for cursorline highlighting so the tint extends past the last grapheme.
#[inline]
fn fill_row_bg(buf: &mut ratatui::buffer::Buffer, x_start: u16, x_end: u16, y: u16, bg: ratatui::style::Color) {
    if x_start >= x_end { return; }
    let area = buf.area();
    let x_start = x_start.max(area.x);
    let x_end = x_end.min(area.x + area.width);
    if x_start >= x_end || y >= area.y + area.height { return; }
    let style = ratatui::style::Style::default().bg(bg);
    for x in x_start..x_end {
        buf[(x, y)].set_char(' ').set_style(style);
    }
}

/// Fill a horizontal span of cells on row `y` with blank `Cell::default()`.
///
/// Uses a single slice fill instead of per-cell `set_cell` calls.
/// Cells within a row are contiguous in ratatui's row-major backing Vec, so
/// one `index_of` + `fill` replaces N bounds-checked function calls.
/// Clips silently if `x_end` extends past the buffer boundary.
#[inline]
fn clear_row_span(buf: &mut ratatui::buffer::Buffer, x_start: u16, x_end: u16, y: u16) {
    if x_start >= x_end { return; }
    let area = buf.area();
    let x_start = x_start.max(area.x);
    let x_end = x_end.min(area.x + area.width);
    if x_start >= x_end || y >= area.y + area.height { return; }
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
        ratatui::buffer::Buffer::empty(ratatui::layout::Rect { x: 0, y: 0, width: w, height: h })
    }

    fn simple_row(graphemes: std::ops::Range<usize>) -> DisplayRow {
        DisplayRow { kind: RowKind::LineStart { line_idx: 0 }, graphemes }
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
        let graphemes = vec![
            simple_grapheme(0, 0, 1),
            simple_grapheme(1, 1, 1),
        ];
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
        let pane_rect = ratatui::layout::Rect { x: 0, y: 0, width: 20, height: 5 };
        let mut buf = make_test_buf(20, 5);
        let theme = Theme::default();
        let (line_texts, line_text_offsets) = make_line_texts(&rope, visible.line_range.clone());
        let mut col_widths = Vec::new();
        let ctx = ComposeCtx { gutter_columns: &[], visible: &visible, viewport: &viewport, mode: EditorMode::Normal, primary_head_line: 0, tab_width: 4, tilde_style: ratatui::style::Style::default(), indent_guide_style: ratatui::style::Style::default(), pane_rect, theme: &theme };
        compose(&rows, &graphemes, &styles, &line_texts, &line_text_offsets, &ctx, &mut col_widths, &mut buf);

        assert_eq!(buf.cell(ratatui::layout::Position { x: 0, y: 0 }).unwrap().symbol(), "h");
        assert_eq!(buf.cell(ratatui::layout::Position { x: 1, y: 0 }).unwrap().symbol(), "i");
    }

    #[test]
    fn filler_rows_have_tilde() {
        let rope = Rope::from_str("x\n");
        let graphemes = vec![simple_grapheme(0, 0, 1)];
        let rows = vec![simple_row(0..1)];
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
        let pane_rect = ratatui::layout::Rect { x: 0, y: 0, width: 20, height: 5 };
        let mut buf = make_test_buf(20, 5);
        let theme = Theme::default();
        let (line_texts, line_text_offsets) = make_line_texts(&rope, visible.line_range.clone());
        let mut col_widths = Vec::new();
        let ctx = ComposeCtx { gutter_columns: &[], visible: &visible, viewport: &viewport, mode: EditorMode::Normal, primary_head_line: 0, tab_width: 4, tilde_style: ratatui::style::Style::default(), indent_guide_style: ratatui::style::Style::default(), pane_rect, theme: &theme };
        compose(&rows, &graphemes, &styles, &line_texts, &line_text_offsets, &ctx, &mut col_widths, &mut buf);

        // Row 0 has 'x', rows 1–4 should have '~'
        for r in 1..5u16 {
            assert_eq!(
                buf.cell(ratatui::layout::Position { x: 0, y: r }).unwrap().symbol(),
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
        let pane_rect = ratatui::layout::Rect { x: 0, y: 0, width: w, height: h };
        let mut buf = make_test_buf(w, h);
        let theme = Theme::default();
        let (line_texts, line_text_offsets) = make_line_texts(rope, visible.line_range.clone());
        let mut col_widths = Vec::new();
        let ctx = ComposeCtx { gutter_columns: &[], visible: &visible, viewport: &viewport, mode: EditorMode::Normal, primary_head_line: 0, tab_width, tilde_style: ratatui::style::Style::default(), indent_guide_style: ratatui::style::Style::default(), pane_rect, theme: &theme };
        compose(rows, graphemes, styles, &line_texts, &line_text_offsets, &ctx, &mut col_widths, &mut buf);
        buf
    }

    #[test]
    fn top_skip_rows_skips_first_row() {
        let rope = Rope::from_str("ab\ncd\n");
        // Two rows, skip the first → only "cd" appears at screen row 0.
        let g = vec![
            simple_grapheme(0, 0, 1), // 'a' on line 0
            simple_grapheme(1, 1, 1), // 'b' on line 0
            Grapheme { byte_range: 0..1, char_offset: 3, col: 0, width: 1, content: CellContent::Grapheme, indent_depth: 0 }, // 'c' on line 1
            Grapheme { byte_range: 1..2, char_offset: 4, col: 1, width: 1, content: CellContent::Grapheme, indent_depth: 0 }, // 'd' on line 1
        ];
        let rows = vec![
            DisplayRow { kind: RowKind::LineStart { line_idx: 0 }, graphemes: 0..2 },
            DisplayRow { kind: RowKind::LineStart { line_idx: 1 }, graphemes: 2..4 },
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
        assert_eq!(buf.cell(ratatui::layout::Position { x: 0, y: 0 }).unwrap().symbol(), "c");
        assert_eq!(buf.cell(ratatui::layout::Position { x: 1, y: 0 }).unwrap().symbol(), "d");
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
        let buf = do_compose(&rope, &rows, &graphemes, &styles, visible, viewport, 4, 20, 5);
        // With h_offset=2, screen col 0 shows 'c' (buf col 2).
        assert_eq!(buf.cell(ratatui::layout::Position { x: 0, y: 0 }).unwrap().symbol(), "c");
        assert_eq!(buf.cell(ratatui::layout::Position { x: 1, y: 0 }).unwrap().symbol(), "d");
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
            })
            .collect();
        let rows = vec![DisplayRow { kind: RowKind::LineStart { line_idx: 0 }, graphemes: 0..11 }];
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
        let buf = do_compose(&rope, &rows, &graphemes, &styles, visible, viewport, 4, 20, 5);
        // A guide should appear at screen col 4 (k=1, tw=4).
        assert_eq!(buf.cell(ratatui::layout::Position { x: 4, y: 0 }).unwrap().symbol(), "│");
        // Col 0 has the space content (no guide at depth boundary).
        assert_ne!(buf.cell(ratatui::layout::Position { x: 0, y: 0 }).unwrap().symbol(), "│");
        // Col 8 is where content starts — no guide there.
        assert_ne!(buf.cell(ratatui::layout::Position { x: 8, y: 0 }).unwrap().symbol(), "│");
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
            })
            .collect();
        let rows = vec![
            DisplayRow { kind: RowKind::LineStart { line_idx: 0 }, graphemes: 0..4 },
            DisplayRow { kind: RowKind::Wrap { line_idx: 0, wrap_row: 1 }, graphemes: 4..8 },
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
        let buf = do_compose(&rope, &rows, &graphemes, &styles, visible, viewport, 4, 20, 5);
        // depth=1 means no inner guides (guides at k in 1..1 — empty range).
        // Also verify wrap row (screen row 1) has no guide either.
        assert_ne!(buf.cell(ratatui::layout::Position { x: 0, y: 1 }).unwrap().symbol(), "│");
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
        }];
        let rows = vec![simple_row(0..1)];
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
        let buf = do_compose(&rope, &rows, &graphemes, &styles, visible, viewport, 4, 20, 5);
        assert_eq!(buf.cell(ratatui::layout::Position { x: 0, y: 0 }).unwrap().symbol(), "→");
        assert_eq!(buf.cell(ratatui::layout::Position { x: 1, y: 0 }).unwrap().symbol(), " ");
        assert_eq!(buf.cell(ratatui::layout::Position { x: 2, y: 0 }).unwrap().symbol(), " ");
        assert_eq!(buf.cell(ratatui::layout::Position { x: 3, y: 0 }).unwrap().symbol(), " ");
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
            let sym = buf.cell(ratatui::layout::Position { x, y: 1 }).unwrap().symbol();
            if (3..7).contains(&x) { assert_eq!(sym, " ", "col {x} should be blank"); }
            else { assert_eq!(sym, "X", "col {x} should be untouched"); }
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
            let sym = buf.cell(ratatui::layout::Position { x, y: 0 }).unwrap().symbol();
            if x >= 8 { assert_eq!(sym, " "); } else { assert_eq!(sym, "X"); }
        }
    }

    #[test]
    fn clear_row_span_empty_range_no_panic() {
        let mut buf = make_test_buf(10, 3);
        // x_start == x_end and x_start > x_end should both be no-ops.
        clear_row_span(&mut buf, 5, 5, 0);
        clear_row_span(&mut buf, 7, 3, 0);
    }
}
