use hume_rope::column::DisplayLineCol;

use hume_grid::{Rect, Rgb};
// `Canvas` and its geometry helper live in `hume-grid` now — see its own doc
// for why a crate with "no other HUME crate" as its rule takes this one
// exception. Re-exported here so every existing `hume_engine::render::Canvas`
// import (and `crate::render::clamp_rect_to_grid` call, in `pipeline/mod.rs`)
// keeps working unchanged.
pub use hume_grid::{Canvas, clamp_rect_to_grid};

use crate::layout::PaneGeometry;
use crate::pane::ViewportState;
use crate::providers::{GutterColumn, GutterCtx, ProviderId};
use crate::theme::Theme;
use crate::types::{
    CellContent, DisplayLine, DisplayLineKind, EditorMode, Grapheme, ResolvedStyle, ScopeId,
};

// ---------------------------------------------------------------------------
// Stage 4: compose
// ---------------------------------------------------------------------------

/// Glyph drawn at each inner indent-guide tab stop. Single source of truth —
/// referenced by `compose_display_line` and by tests, so the glyph only ever needs to
/// change in one place.
pub(crate) const INDENT_GUIDE_GLYPH: &str = "╎";

/// Per-frame constants needed by `compose_display_line`. Bundle these once per pane
/// and pass them through without repeating at each call site.
pub(crate) struct ComposeCtx<'a> {
    pub gutter_columns: &'a [(ProviderId, Box<dyn GutterColumn>)],
    pub visible: &'a PaneGeometry,
    pub viewport: &'a ViewportState,
    pub mode: EditorMode,
    pub primary_head_line: hume_rope::line::ContentLine,
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
    /// Buffer rope, passed to `GutterColumn::render_cells` via `GutterCtx`
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

/// Write one display line's gutter cells (all columns) at screen row `y`.
///
/// Shared by `compose_display_line` (real buffer/wrap/virtual display
/// lines) and `render_tilde_fillers` (`DisplayLineKind::Filler` display
/// lines) so a filler display line's gutter is never silently blank — a
/// custom column must be consulted for filler display lines too, not just
/// `LineNumberColumn`'s blank-for-Filler default.
///
/// `lane_widths` must already be populated by the caller (one entry per
/// gutter column) — see `compose_display_line`'s doc comment for why it isn't folded
/// into `ComposeCtx`.
fn compose_gutter(
    line_kind: DisplayLineKind,
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
    let gutter_ctx = GutterCtx {
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
        let cells = lane_provider.render_cells(line_kind, &gutter_ctx);
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
            // `fill_glyph_run`, not a `write_cell` loop: the space glyph is a
            // compile-time constant repeated `pad` times, exactly its
            // intended use, and both bound the write at `right_edge` equally
            // — `fill_glyph_run` just does it once for the whole span
            // instead of once per cell.
            canvas.fill_glyph_run(gutter_x, y, " ", pad, style, gutter_x + usable_per_cell);
            // `after` is where the write actually stopped — used below
            // instead of a second `gutter_x + pad + text_width` measurement,
            // so the separator's position can't drift from the draw.
            let after =
                canvas.write_text_run(gutter_x + pad, y, text, style, gutter_x + usable_per_cell);
            // Only write a separator after the last cell — it's the column's
            // right padding, not a separator between sub-cells.
            if is_last {
                canvas.fill_glyph_run(after, y, " ", 1, style, lane_x + lane_width);
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
                canvas.write_cell(gutter_x, y, " ", 1, style, lane_x + lane_width);
                gutter_x += 1;
            }
        }
        // Providers are a public extension point; their cell-width math
        // isn't guaranteed to sum to lane_width exactly. Land on the column
        // boundary regardless, so the next column never inherits any drift.
        gutter_x = lane_x + lane_width;
    }
}

/// Render a single display line at `screen_row` into the frame grid.
///
/// `line_str` is the pre-materialised text of the buffer line that owns
/// this display line (used to resolve `CellContent::Grapheme` byte
/// ranges). Pass `""` for virtual/filler display lines that have no
/// backing buffer line.
///
/// `virtual_texts` is the per-frame arena backing this display line's
/// `CellContent::Whitespace`/`Placeholder`/`Virtual` ranges
/// (`LineFormat::virtual_texts` for a content display line, `virtual_line.texts`
/// for a provider's virtual display line) — same lifetime/borrow rationale as
/// `line_str`.
///
/// `lane_widths` must already be populated by the caller (one entry per gutter
/// column). Passed separately from `compose_ctx` because in the fused pipeline it lives
/// in `FrameScratch`, which cannot be bundled into `ComposeCtx` without
/// creating a conflicting borrow.
#[allow(clippy::too_many_arguments)]
pub(crate) fn compose_display_line(
    display_line: &DisplayLine,
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

    // Filler display lines are rendered exclusively by
    // `render_tilde_fillers`, never routed through here — it has its own
    // gutter + tilde + background handling since a filler display line has
    // no backing graphemes to iterate.
    debug_assert!(!matches!(display_line.kind, DisplayLineKind::Filler));

    compose_gutter(
        display_line.kind,
        lane_widths,
        compose_ctx,
        row_bg,
        y,
        canvas,
    );

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

    let line_graphemes = &graphemes[display_line.graphemes.start..display_line.graphemes.end];
    let line_styles = &styles[display_line.graphemes.start..display_line.graphemes.end];

    for (g, style) in line_graphemes.iter().zip(line_styles.iter()) {
        // Skip WidthContinuation — already handled by the primary cell.
        if matches!(g.content, CellContent::WidthContinuation) {
            continue;
        }

        // Horizontal scroll: skip cells left of the viewport.
        if g.display_col.advance(g.width as u32) <= h_offset {
            continue;
        }
        // Clip cells that start before the viewport edge. `g.display_col` is
        // a display-line column (`DisplayLineCol`), which with wrapping off spans the
        // whole unwrapped line, but this render path always runs behind
        // `with_h_window` (`pane_render.rs`), so a cell surviving the skip
        // above sits within one viewport width of `h_offset` — safely
        // representable in the terminal-cell (`u16`) domain the rest of
        // compose works in. `.get()` here (not `cells_since`, which
        // debug-panics on inversion): a cell straddling `h_offset` — handled
        // below — legitimately has `g.display_col < h_offset`, and this
        // clamps that case to 0 rather than treating it as a bug.
        let content_x = g.display_col.get().saturating_sub(h_offset.get());
        debug_assert!(
            u16::try_from(content_x).is_ok(),
            "on-screen column {content_x} exceeds a u16 — h_window should have clipped this cell"
        );
        let screen_x = content_x_origin + content_x as u16;
        if screen_x >= right_edge {
            break; // past right edge — done with this display line
        }

        let cell_style = *style;

        // A multi-column cell (double-width CJK grapheme, a tab's
        // Whitespace glyph or TabFill) whose left edge sits before `h_offset` still
        // passes the skip check above once its right edge crosses
        // it — but `content_x` above already clamped to 0, so
        // rendering the glyph there would draw its *full* width at
        // the viewport's left edge instead of the fraction that's
        // actually scrolled into view, shifting the display line. Render
        // spaces for the visible remainder instead (matches Helix).
        // Impossible for width-1 cells: straddling needs
        // `g.display_col < h_offset < g.display_col + g.width`, which has no
        // integer solution when `g.width == 1`.
        if g.display_col < h_offset {
            let visible_cells = g.width as u32 - h_offset.cells_since(g.display_col);
            for i in 0..visible_cells as u16 {
                let sx = screen_x + i;
                canvas.write_cell(sx, y, " ", 1, cell_style, right_edge);
            }
            continue;
        }

        match &g.content {
            CellContent::Grapheme => {
                // `format.rs` cuts every `byte_range` out of this same line with
                // `grapheme_indices`, so a range that won't slice means the formatted
                // line and `line_str` have desynced — a `PaneLineStore` entry walked
                // against another line's text. `.get()` rather than `&line_str[..]`
                // because the bounds pair alone says nothing about char boundaries: a
                // desynced range can still land mid-cluster and panic.
                let Some(text) = line_str.get(g.byte_range.as_byte_range()) else {
                    debug_assert!(
                        false,
                        "grapheme byte range {}..{} does not slice the {}-byte line — \
                         formatted line and line text have desynced",
                        g.byte_range.start.index(),
                        g.byte_range.end.index(),
                        line_str.len()
                    );
                    // Release: leave the cell as `fill_row_bg` painted it. One blank
                    // cell beats taking the editor down over a frame that is already
                    // wrong.
                    continue;
                };
                if screen_x + g.width as u16 > right_edge {
                    // A wide grapheme whose right half would cross
                    // `right_edge` cannot be drawn — there is no such
                    // thing as half a glyph, and the cell past the edge
                    // belongs to whatever the terminal renders next (a
                    // neighbouring pane, the divider seam). Render spaces
                    // for the columns that are ours, mirroring the
                    // h-scroll straddle policy below.
                    for sx in screen_x..right_edge {
                        canvas.write_cell(sx, y, " ", 1, cell_style, right_edge);
                    }
                } else {
                    canvas.write_cell(screen_x, y, text, g.width, cell_style, right_edge);
                }
            }
            CellContent::Whitespace { start, len } | CellContent::Placeholder { start, len } => {
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
                    canvas.write_cell(ex, y, " ", 1, cell_style, cell_end);
                }
            }
            CellContent::TabFill => {
                // A tab with its indicator off: blank across its whole
                // expanse, same `cell_style` a `Whitespace` glyph's fill
                // would use — this is just that fill with no glyph in front.
                let cell_end = (screen_x + g.width as u16).min(right_edge);
                for ex in screen_x..cell_end {
                    canvas.write_cell(ex, y, " ", 1, cell_style, cell_end);
                }
            }
            CellContent::Virtual { start, len } => {
                let s = resolve_arena_text(virtual_texts, *start, *len);
                if screen_x + g.width as u16 > right_edge {
                    // Same straddle policy as `Grapheme` above: a wide
                    // decoration glyph (an inlay hint containing CJK text)
                    // cannot be drawn half-on-screen.
                    for sx in screen_x..right_edge {
                        canvas.write_cell(sx, y, " ", 1, cell_style, right_edge);
                    }
                } else {
                    canvas.write_cell(screen_x, y, s, g.width, cell_style, right_edge);
                }
            }
            CellContent::Empty => {
                canvas.write_cell(screen_x, y, " ", 1, cell_style, right_edge);
            }
            // Filtered by the `WidthContinuation` skip above before reaching this match.
            CellContent::WidthContinuation => unreachable!(),
        }
    }

    // ── Indent guides ─────────────────────────────────────────────────
    // Draw guides only on line-start display lines (not wrap/virtual/filler)
    // so that continuation display lines don't clobber content at guide
    // positions. Drawn after content so they appear on top of
    // leading-whitespace cells.
    if compose_ctx.show_indent_guides
        && matches!(display_line.kind, DisplayLineKind::LineStart { .. })
    {
        let depth = line_graphemes.first().map(|g| g.indent_depth).unwrap_or(0);
        let tw = hume_rope::width::indent_stop(1, compose_ctx.tab_width);
        // `indent_stop` counts buffer columns from the *line's* column 0 —
        // not the display line's, when a leading inline insert (an inlay
        // hint at byte 0) precedes the real text. A virtual cell carries an
        // empty `byte_range`, so the first non-empty one marks where the
        // buffer line's own columns actually begin on screen.
        let indent_origin = line_graphemes
            .iter()
            .find(|g| !g.byte_range.is_empty())
            .map_or(DisplayLineCol::new(0), |g| g.display_col);
        // Draw a guide at each inner tab-stop. These positions are
        // guaranteed to lie within the leading whitespace.
        for k in 1..depth {
            let guide_display_col = indent_origin.advance(hume_rope::width::indent_stop(
                k as u32,
                compose_ctx.tab_width,
            ));
            // Account for horizontal scroll.
            if guide_display_col.advance(tw) > h_offset {
                // See the content loop's own `.get()` comment above — a guide
                // left of `h_offset` clamps to 0 rather than panicking.
                let content_x = guide_display_col.get().saturating_sub(h_offset.get());
                debug_assert!(
                    u16::try_from(content_x).is_ok(),
                    "on-screen indent guide column {content_x} exceeds a u16"
                );
                let screen_x = content_x_origin + content_x as u16;
                canvas.write_cell(
                    screen_x,
                    y,
                    INDENT_GUIDE_GLYPH,
                    1,
                    compose_ctx.indent_guide_style,
                    right_edge,
                );
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
        compose_gutter(
            DisplayLineKind::Filler,
            lane_widths,
            compose_ctx,
            None,
            y,
            canvas,
        );
        let content_x_origin = compose_ctx.pane_rect.x + compose_ctx.visible.gutter_width;
        canvas.fill_row_bg(
            content_x_origin,
            right_edge,
            y,
            compose_ctx.theme.ui.background.bg,
        );
        canvas.write_cell(
            compose_ctx.pane_rect.x,
            y,
            "~",
            1,
            compose_ctx.tilde_style,
            right_edge,
        );
        screen_row += 1;
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests;
