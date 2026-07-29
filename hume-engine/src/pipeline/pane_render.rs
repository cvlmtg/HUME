use unicode_segmentation::UnicodeSegmentation;

use crate::format::{push_arena_text, unicode_display_width};
use crate::providers::VirtualLineAnchor;
use crate::render::{self, ComposeCtx};
use crate::types::{CellContent, DisplayRow, Grapheme, ResolvedStyle, RowKind};

use super::{FrameScratch, PaneRenderCtx, ViewportCursor};

/// Extra columns appended past `content_width` when clipping `WrapMode::None`
/// formatting to the horizontal window (see `render_buffer_line`). Covers a
/// cell that starts just inside the right edge but is wider than one column
/// (a double-width CJK glyph or a wide tab stop) — without slack, such a cell
/// would be scanned but never pushed, clipping it one column too early.
const H_WINDOW_SLACK: u16 = 4;

// ---------------------------------------------------------------------------
// Fused pipeline
// ---------------------------------------------------------------------------

/// Orchestrate the four pipeline stages for one pane using a fused per-line loop.
///
/// Instead of materialising all rows for the entire visible range before styling
/// or rendering, this function processes one buffer line at a time:
///
/// ```text
/// pre: sort selections, pre-collect virtual lines, compute col_widths
/// for each buffer line:
///   drain_virtual_lines(Before)
///   render_buffer_line   (format → style → compose, per display row)
///   drain_virtual_lines(After)
/// tilde filler rows if needed
/// ```
///
/// Peak scratch memory is O(max_graphemes_per_line) rather than
/// O(total_visible_graphemes), a ~16× reduction on a 200×50 terminal.
pub(crate) fn render_pane(
    pane_ctx: &PaneRenderCtx,
    scratch: &mut FrameScratch,
    buf: &mut ratatui::buffer::Buffer,
) {
    use crate::layout;

    // ── Stage 1: Layout ───────────────────────────────────────────────────
    let visible = layout::compute_viewport(
        pane_ctx.rope,
        &pane_ctx.pane.viewport,
        &pane_ctx.settings.wrap_mode,
        pane_ctx.pane.providers.gutter_columns(),
        pane_ctx.settings.tab_width,
    );

    // ── Pre-render: per-frame constant setup ──────────────────────────────

    // Selections arrive pre-sorted from the editor; copy once, reuse every row.
    scratch
        .style
        .populate_sorted_sels(&pane_ctx.pane.selections, pane_ctx.pane.primary_idx);

    // Pre-collect virtual lines from all providers; sort by anchor.
    scratch.format.virtual_lines.clear();
    for (id, provider) in &pane_ctx.pane.providers.virtual_lines {
        let before = scratch.format.virtual_lines.len();
        provider.virtual_lines(
            visible.line_range.clone(),
            visible.content_width,
            &mut scratch.format.virtual_lines,
        );
        // Stamp the real provider id on every entry just added — never trust
        // a provider's self-reported `provider_id` (it could misreport
        // another provider's id).
        for vl in &mut scratch.format.virtual_lines[before..] {
            vl.provider_id = *id;
        }
    }
    scratch
        .format
        .virtual_lines
        .sort_by_key(|vl| vl.anchor.sort_key());

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

    let mut vc = ViewportCursor {
        screen_row: 0,
        viewport_height: visible.content_height.min(pane_ctx.rect.height),
        top_skip_remaining: visible.top_skip_rows as usize,
        vl_cursor: 0,
    };

    // ── Fused per-line loop ──────────────────────────────────────────────
    for line_idx in visible.line_range.clone() {
        drain_virtual_lines(
            VirtualLineAnchor::Before(line_idx),
            &mut vc,
            scratch,
            &compose_ctx,
            &mut canvas,
        );
        if vc.is_full() {
            break;
        }

        render_buffer_line(
            pane_ctx,
            line_idx,
            &mut vc,
            scratch,
            &compose_ctx,
            &mut canvas,
        );
        if vc.is_full() {
            break;
        }

        drain_virtual_lines(
            VirtualLineAnchor::After(line_idx),
            &mut vc,
            scratch,
            &compose_ctx,
            &mut canvas,
        );
        if vc.is_full() {
            break;
        }
    }

    render::render_tilde_fillers(
        vc.screen_row,
        &scratch.col_widths,
        &compose_ctx,
        &mut canvas,
    );
}

// ---------------------------------------------------------------------------
// Per-line helpers
// ---------------------------------------------------------------------------

/// Emit all virtual lines whose anchor matches `anchor`, advancing `vc`.
///
/// Stops early if the viewport fills up. After returning, `vc.vl_cursor`
/// points past the last consumed virtual line.
///
/// `top_skip_rows` (`vc.top_skip_remaining`) counts display rows of
/// `top_line`'s whole visual block (`before` + wrap rows + `after`) — see
/// `ViewportState::top_row_offset`'s doc. A virtual row here is skipped
/// through the same `try_skip` budget as a buffer wrap row in
/// `render_buffer_line`, so scrolling moves through a virtual block one row
/// at a time, same as real content.
fn drain_virtual_lines(
    anchor: VirtualLineAnchor,
    vc: &mut ViewportCursor,
    scratch: &mut FrameScratch,
    compose_ctx: &ComposeCtx,
    canvas: &mut render::PaneCanvas,
) {
    let line_idx = match anchor {
        VirtualLineAnchor::Before(n) | VirtualLineAnchor::After(n) => n,
    };
    while vc.vl_cursor < scratch.format.virtual_lines.len()
        && scratch.format.virtual_lines[vc.vl_cursor].anchor == anchor
    {
        if vc.try_skip() {
            vc.vl_cursor += 1;
            continue;
        }
        if vc.is_full() {
            return;
        }
        emit_virtual_row(
            vc.vl_cursor,
            line_idx,
            vc.screen_row,
            scratch,
            compose_ctx,
            canvas,
        );
        vc.vl_cursor += 1;
        vc.screen_row += 1;
    }
}

/// Format, style, and render one buffer line (or a Filler row past EOF).
///
/// For a real buffer line this runs all four pipeline stages in order:
/// format → highlight → (for each display row) style → compose.
/// Scratch per-line buffers are cleared at the end so the next line starts
/// fresh.
fn render_buffer_line(
    pane_ctx: &PaneRenderCtx,
    line_idx: usize,
    vc: &mut ViewportCursor,
    scratch: &mut FrameScratch,
    compose_ctx: &ComposeCtx,
    canvas: &mut render::PaneCanvas,
) {
    use crate::{format, style};

    // `render_pane`'s caller loop iterates `visible.line_range`, which
    // `compute_line_range` (layout.rs) caps at `last_line_idx < rope.len_lines()`
    // — every line reaching here is a real buffer line, never the phantom
    // trailing line ropey reports past `last_line_idx`.
    debug_assert!(line_idx < pane_ctx.rope.len_lines());

    scratch.format.line_texts.clear();

    // Collect and sort inline decorations for this line.
    scratch.inline_inserts.clear();
    for (_, provider) in &pane_ctx.pane.providers.inline_decorations {
        provider.decorations_for_line(line_idx, &mut scratch.inline_inserts);
    }
    scratch.inline_inserts.sort_by_key(|i| i.byte_offset);

    // Clip formatting to the visible horizontal window in `WrapMode::None`
    // — a single unwrapped line can be arbitrarily long (e.g. a minified
    // JSON/JS file), so scanning past the right edge would cost
    // O(line_length) per frame and, pre-clip, overflow `current_col`
    // (`u16`) on lines wider than 65535 columns. Wrapping modes are
    // already bounded by `wrap_width`, so they pass `None`. `H_WINDOW_SLACK`
    // covers the tail case where a cell starting just inside the right
    // edge (e.g. a double-width CJK glyph) still needs to be considered.
    let h_window = (!pane_ctx.settings.wrap_mode.is_wrapping()).then(|| {
        let h_offset = compose_ctx.viewport.horizontal_offset;
        let end = h_offset
            .saturating_add(compose_ctx.visible.content_width)
            .saturating_add(H_WINDOW_SLACK);
        h_offset..end
    });

    // Stage 2 (per line): format into scratch.format.display_rows + scratch.format.graphemes.
    // `inline_inserts` is kept outside `scratch.format` to allow simultaneous
    // `&scratch.inline_inserts` and `&mut scratch.format` without a borrow conflict.
    format::format_buffer_line(
        pane_ctx.rope,
        line_idx,
        pane_ctx.settings.tab_width,
        &pane_ctx.settings.whitespace,
        &pane_ctx.settings.wrap_mode,
        h_window,
        &scratch.inline_inserts,
        &mut scratch.format,
    );

    // Stage 3 (per line): build highlight intervals for this buffer line.
    style::rebuild_tier_bufs(
        line_idx,
        pane_ctx.syntax,
        &pane_ctx.pane.providers.highlights,
        pane_ctx.rope,
        &mut scratch.style,
    );

    scratch
        .style
        .styles
        .resize(scratch.format.graphemes.len(), ResolvedStyle::default());

    let line_start_char = pane_ctx.rope.line_to_char(line_idx);
    let line_end_char = pane_ctx.rope.line_to_char(line_idx + 1);
    // Cursorline highlights only the primary cursor's line.
    let is_head_line = scratch
        .style
        .primary_idx_in_sorted
        .and_then(|i| scratch.style.sorted_sels.get(i))
        .is_some_and(|s| s.head >= line_start_char && s.head < line_end_char);
    // line_str/virtual_texts borrow scratch.format's arenas; must not clear them
    // inside the loop.
    let line_str = scratch.format.line_texts.as_str();
    let virtual_texts = scratch.format.virtual_texts.as_str();

    for row_idx in 0..scratch.format.display_rows.len() {
        if vc.try_skip() {
            continue;
        }
        if vc.is_full() {
            break;
        }

        // Stage 3 (per row): resolve styles for this display row.
        style::style_row(
            &scratch.format.display_rows[row_idx],
            &scratch.format.graphemes,
            line_start_char,
            line_end_char,
            is_head_line,
            pane_ctx.settings.mode,
            pane_ctx.theme,
            &mut scratch.style,
        );

        // Stage 4 (per row): write to the ratatui buffer.
        let row_bg = if is_head_line {
            pane_ctx.theme.ui.cursorline.bg
        } else {
            None
        };
        render::compose_row(
            &scratch.format.display_rows[row_idx],
            &scratch.format.graphemes,
            &scratch.style.styles,
            line_str,
            virtual_texts,
            vc.screen_row,
            &scratch.col_widths,
            compose_ctx,
            canvas,
            row_bg,
        );
        vc.screen_row += 1;
    }

    scratch.clear_line();
}

/// Emit one virtual line: segment its text into graphemes, push them into
/// scratch, compose, then clear.
///
/// The provider hands over plain `text` + scoped byte-range `segments`
/// (`VirtualLine`) rather than pre-built `Grapheme`s — this function does the
/// same grapheme/width/col bookkeeping `format_buffer_line` does for real
/// buffer lines, so providers can't get that arithmetic wrong.
pub(super) fn emit_virtual_row(
    vl_idx: usize,
    line_idx: usize,
    screen_row: u16,
    scratch: &mut FrameScratch,
    compose_ctx: &ComposeCtx,
    canvas: &mut render::PaneCanvas,
) {
    // Field-split: read `virtual_lines`, write `graphemes`/`virtual_texts` —
    // different sub-struct fields of `scratch.format`.
    let g_start = scratch.format.graphemes.len();
    let vl = &scratch.format.virtual_lines[vl_idx];
    let provider_id = vl.provider_id;

    // Copy the whole line's text into the shared arena once; each grapheme's
    // `CellContent::Virtual` cell then references a sub-range of this copy.
    let (arena_base, _) = push_arena_text(&mut scratch.format.virtual_texts, &vl.text);

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

        scratch.format.graphemes.push(Grapheme {
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
            // Both cells of a double-wide char stay on the same (virtual) row.
            scratch.format.graphemes.push(Grapheme {
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

    scratch.format.display_rows.push(DisplayRow {
        kind: RowKind::Virtual {
            provider_id,
            anchor_line: line_idx,
        },
        graphemes: g_start..scratch.format.graphemes.len(),
    });
    // Virtual rows are skipped by the style stage (no highlight tiers, no
    // cursor/selection), but each grapheme can still carry its own `scope`
    // (set by the `VirtualLineSource` that produced it). Resolve that scope
    // now — `theme.default` layered with the resolved scope for `Some`, or
    // the themed `virtual_text` fallback for graphemes with no scope of
    // their own (matches the tilde-filler / no-decoration look).
    scratch.style.styles.clear();
    scratch
        .style
        .styles
        .extend(scratch.format.graphemes.iter().map(|g| {
            match g.scope {
                Some(id) => compose_ctx
                    .theme
                    .default
                    .layer(compose_ctx.theme.resolve(id)),
                None => compose_ctx.theme.ui.virtual_text,
            }
        }));

    let row_idx = scratch.format.display_rows.len() - 1;
    render::compose_row(
        &scratch.format.display_rows[row_idx],
        &scratch.format.graphemes,
        &scratch.style.styles,
        "",
        &scratch.format.virtual_texts,
        screen_row,
        &scratch.col_widths,
        compose_ctx,
        canvas,
        None,
    );

    scratch.clear_line();
}
