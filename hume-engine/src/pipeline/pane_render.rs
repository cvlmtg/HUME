use crate::render::{self, ComposeCtx};
use crate::rows::{RowKind, RowMap, RowPos};
use crate::types::ResolvedStyle;

use super::{FrameScratch, PaneRenderCtx};

/// Extra columns appended past `content_width` when clipping `WrapMode::None`
/// formatting to the horizontal window (see [`render_pane`]). Covers a cell
/// that starts just inside the right edge but is wider than one column (a
/// double-width CJK glyph or a wide tab stop) — without slack, such a cell
/// would be scanned but never pushed, clipping it one column too early.
const H_WINDOW_SLACK: u16 = 4;

// ---------------------------------------------------------------------------
// Fused pipeline
// ---------------------------------------------------------------------------

/// Render one pane by walking its display rows.
///
/// The walk *is* the layout: starting from the viewport's top row address and
/// stepping one row at a time through `rows::RowMap` visits exactly the rows on
/// screen, in order, whether each comes from a buffer line's wrapping or from a
/// `VirtualLineSource`. There is no separate "which lines are visible" estimate
/// to disagree with what gets emitted, and no skip counter to run down before
/// the first row — the starting address already accounts for a viewport parked
/// partway into a line's block.
///
/// Per-line work (formatting, highlight intervals) happens on the row that
/// first enters a line and is reused by its remaining rows, so a wrapped line
/// is formatted once however many of its rows are on screen.
///
/// Peak scratch memory is O(max_graphemes_per_line) rather than
/// O(total_visible_graphemes), a ~16× reduction on a 200×50 terminal.
pub(crate) fn render_pane(
    pane_ctx: &PaneRenderCtx,
    scratch: &mut FrameScratch,
    buf: &mut ratatui::buffer::Buffer,
) {
    use crate::layout;

    // ── Stage 1: Geometry ─────────────────────────────────────────────────
    let visible = layout::compute_viewport(
        pane_ctx.rope,
        &pane_ctx.pane.viewport,
        pane_ctx.pane.providers.gutter_columns(),
    );

    // ── Pre-render: per-frame constant setup ──────────────────────────────

    // Selections arrive pre-sorted from the editor; copy once, reuse every row.
    scratch
        .style
        .populate_sorted_sels(&pane_ctx.pane.selections, pane_ctx.pane.primary_idx);

    // Gutter column widths: constant for the entire frame.
    scratch.col_widths.clear();
    scratch.col_widths.extend(
        pane_ctx
            .pane
            .providers
            .gutter_columns
            .iter()
            .map(|(_, c)| c.width(visible.last_line_idx) as u16),
    );

    // Bundle per-frame constants so compose_row call sites stay concise.
    let compose_ctx = ComposeCtx {
        gutter_columns: &pane_ctx.pane.providers.gutter_columns,
        visible: &visible,
        viewport: &pane_ctx.pane.viewport,
        mode: pane_ctx.settings.mode,
        primary_head_line: pane_ctx.pane.primary_head_line(pane_ctx.rope),
        tab_width: pane_ctx.settings.tab_width,
        tilde_style: pane_ctx.theme.ui.virtual_text.into(),
        indent_guide_style: pane_ctx.theme.ui.indent_guide.into(),
        show_indent_guides: pane_ctx.settings.show_indent_guides,
        pane_rect: pane_ctx.rect,
        theme: pane_ctx.theme,
        pane_bg: pane_ctx.theme.ui.background.bg,
        rope: pane_ctx.rope,
    };
    let mut canvas = render::PaneCanvas::new(buf, pane_ctx.dim);

    // Clip `WrapMode::None` formatting to the visible horizontal window — a
    // single unwrapped line can be arbitrarily long (a minified JS file is a
    // real case), so scanning past the right edge would cost O(line_length)
    // per frame and, pre-clip, overflow `current_col` (`u16`) on lines wider
    // than 65535 columns. Wrapping modes are already bounded by `wrap_width`.
    let h_window = (!pane_ctx.settings.wrap_mode.is_wrapping()).then(|| {
        let h_offset = pane_ctx.pane.viewport.horizontal_offset;
        let end = h_offset
            .saturating_add(visible.content_width)
            .saturating_add(H_WINDOW_SLACK);
        h_offset..end
    });

    // The row map borrows the format scratch for the whole pass; style and
    // gutter-width scratch are disjoint fields it never touches.
    let FrameScratch {
        format,
        style,
        col_widths,
        ..
    } = scratch;
    let mut rows = RowMap::new(
        pane_ctx.rope,
        pane_ctx.settings.wrap_mode,
        pane_ctx.settings.tab_width,
        pane_ctx.settings.whitespace,
        &pane_ctx.pane.providers,
        visible.content_width,
        format,
    )
    .with_h_window(h_window);
    let last_content_line = rows.last_line();

    // ── Row walk ──────────────────────────────────────────────────────────
    let height = visible.content_height.min(pane_ctx.rect.height);
    let viewport = &pane_ctx.pane.viewport;
    let mut pos = rows.clamp(RowPos::new(
        viewport.top_line,
        viewport.top_row_offset as usize,
    ));
    // Which line's highlight intervals and cursorline state `line` currently
    // holds, so crossing into a new line is the only thing that rebuilds them.
    let mut line: Option<LineStyle> = None;
    let mut screen_row = 0u16;

    while screen_row < height {
        match rows.kind(pos) {
            RowKind::Content(_) => {
                let line = line.get_or_insert_with(|| {
                    LineStyle::enter(pos.line, last_content_line, pane_ctx, style)
                });
                let row = rows.render_row(pos);
                style
                    .styles
                    .resize(row.graphemes.len(), ResolvedStyle::default());
                crate::style::style_row(
                    row.row,
                    row.graphemes,
                    line.start_char,
                    line.end_char,
                    line.is_head_line,
                    pane_ctx.settings.mode,
                    pane_ctx.theme,
                    style,
                );
                let row_bg = line
                    .is_head_line
                    .then_some(pane_ctx.theme.ui.cursorline.bg)
                    .flatten();
                render::compose_row(
                    row.row,
                    row.graphemes,
                    &style.styles,
                    row.line_text,
                    row.virtual_texts,
                    screen_row,
                    col_widths,
                    &compose_ctx,
                    &mut canvas,
                    row_bg,
                );
            }
            RowKind::Before(_) | RowKind::After(_) => {
                let row = rows.render_row(pos);
                // Virtual rows are skipped by the style stage (no highlight
                // tiers, no cursor/selection), but each grapheme can still
                // carry its own `scope` from the provider that produced it:
                // `theme.default` layered with that scope, or the themed
                // `virtual_text` fallback for graphemes with none (matching
                // the tilde-filler / no-decoration look).
                style.styles.clear();
                style.styles.extend(row.graphemes.iter().map(|g| {
                    match g.scope {
                        Some(id) => compose_ctx
                            .theme
                            .default
                            .layer(compose_ctx.theme.resolve(id)),
                        None => compose_ctx.theme.ui.virtual_text,
                    }
                }));
                render::compose_row(
                    row.row,
                    row.graphemes,
                    &style.styles,
                    "",
                    row.virtual_texts,
                    screen_row,
                    col_widths,
                    &compose_ctx,
                    &mut canvas,
                    None,
                );
            }
        }

        screen_row += 1;
        match rows.next(pos) {
            Some(next) => {
                if next.line != pos.line {
                    line = None;
                }
                pos = next;
            }
            None => break,
        }
    }

    render::render_tilde_fillers(screen_row, col_widths, &compose_ctx, &mut canvas);
}

// ---------------------------------------------------------------------------
// Per-line style state
// ---------------------------------------------------------------------------

/// The per-line facts every content row of one buffer line shares.
///
/// Built once when the walk crosses into a line; building it also rebuilds the
/// highlight interval buffers, which is the expensive part.
struct LineStyle {
    start_char: usize,
    end_char: usize,
    is_head_line: bool,
}

impl LineStyle {
    fn enter(
        line_idx: usize,
        last_content_line: usize,
        pane_ctx: &PaneRenderCtx,
        style: &mut super::StyleScratch,
    ) -> Self {
        debug_assert!(
            line_idx <= last_content_line,
            "row walk reached line {line_idx}, past the buffer's last \
             content line {last_content_line} — `RowMap::last_line`, not \
             `visible.last_line_idx` (the phantom trailing-\\n line one past it)"
        );
        crate::style::rebuild_tier_bufs(
            line_idx,
            pane_ctx.syntax,
            &pane_ctx.pane.providers.highlights,
            pane_ctx.rope,
            style,
        );
        let start_char = pane_ctx.rope.line_to_char(line_idx);
        let end_char = pane_ctx.rope.line_to_char(line_idx + 1);
        // Cursorline highlights only the primary cursor's line.
        let is_head_line = style
            .primary_idx_in_sorted
            .and_then(|i| style.sorted_sels.get(i))
            .is_some_and(|s| s.head >= start_char && s.head < end_char);
        Self {
            start_char,
            end_char,
            is_head_line,
        }
    }
}
