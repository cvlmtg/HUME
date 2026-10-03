use hume_grid::Grid;
use hume_rope::offset::{CharOffset, ExclusiveRange};

use crate::display_lines::DisplayLineMap;
use crate::render::{self, ComposeCtx};
use crate::types::{DisplayLineKind, ResolvedStyle};

use super::{FrameScratch, PaneRenderCtx};

/// Extra columns appended past `content_width` when clipping `WrapMode::None`
/// formatting to the horizontal window (see [`render_pane`]). Covers a cell
/// that starts just inside the right edge but is wider than one column (a
/// double-width CJK glyph or a wide tab stop). Without slack, such a cell
/// would be scanned but never pushed, clipping it one column too early.
const H_WINDOW_SLACK: u16 = 4;

// ---------------------------------------------------------------------------
// Fused pipeline
// ---------------------------------------------------------------------------

/// Render one pane by walking its display lines.
///
/// The walk is the layout: stepping `DisplayLineMap` from the viewport's top
/// address visits exactly the on-screen display lines in order, wrap and
/// virtual lines alike, so no separate visibility estimate can disagree with
/// what is drawn. The top address already covers a viewport parked partway
/// into a line's block.
///
/// Per-line work (formatting, highlights) runs once per buffer line and is
/// reused by its display lines; lines the scroll step already formatted in
/// the shared `store` are not formatted again.
///
/// Peak scratch memory is one [`crate::display_lines::line_store::LineEntry`]
/// per line visited this frame, O(total visible graphemes).
/// [`crate::format::LineFormat::reset_and_shrink`] releases a pathologically
/// wide line's buffers at the frame boundary.
pub(crate) fn render_pane(
    pane_ctx: &mut PaneRenderCtx,
    scratch: &mut FrameScratch,
    store: &mut crate::display_lines::line_store::PaneLineStore,
    grid: &mut Grid,
) {
    use crate::layout;

    // ── Stage 1: Geometry ─────────────────────────────────────────────────
    let visible = layout::compute_viewport(
        pane_ctx.rope,
        pane_ctx.viewport,
        pane_ctx.providers.gutter_columns(),
    );

    // ── Pre-render: per-frame constant setup ──────────────────────────────

    // Style and gutter-width scratch are disjoint fields the display-line
    // map never touches; its own storage is the pane's line store, which
    // already holds whatever the scroll step formatted for this pane this
    // frame.
    let FrameScratch {
        style, lane_widths, ..
    } = scratch;

    // Gutter lane widths: constant for the entire frame.
    lane_widths.clear();
    lane_widths.extend(layout::lane_widths(
        pane_ctx.providers.gutter_columns(),
        visible.last_line_idx,
    ));

    // Clip `WrapMode::None` formatting to the visible horizontal window: a
    // single unwrapped line can be arbitrarily long (a minified JS file is a
    // real case), so scanning past the right edge would cost O(line_length)
    // per frame. Wrapping modes are already bounded by `wrap_width`.
    let h_window = (!pane_ctx.settings.format.wrap_mode.is_wrapping()).then(|| {
        let h_offset = pane_ctx.viewport.horizontal_offset;
        let end = h_offset
            .advance_saturating(visible.content_width as u32)
            .advance_saturating(H_WINDOW_SLACK as u32);
        h_offset..end
    });

    let mut dlm = DisplayLineMap::new(
        pane_ctx.rope,
        pane_ctx.providers,
        visible.content_width,
        pane_ctx.settings.format,
        store,
    )
    .with_h_window(h_window);
    let last_content_line = dlm.last_line();

    // The render pass resolves the top it walks from (see `Viewport::top_at`),
    // so a host with no per-frame healing discipline of its own (a
    // different embedder, or this crate's own `pipeline/tests.rs`) can never
    // desync the walk from a stale address.
    let mut pos = pane_ctx.viewport.top_at(&mut dlm);

    // Bundle per-frame constants so compose_display_line call sites stay concise.
    let compose_ctx = ComposeCtx {
        gutter_columns: &pane_ctx.providers.gutter_columns,
        visible: &visible,
        horizontal_offset: pane_ctx.viewport.horizontal_offset,
        mode: pane_ctx.settings.mode,
        primary_head_line: crate::pane::primary_head_line(pane_ctx.selections, pane_ctx.rope),
        tab_width: pane_ctx.settings.format.tab_width,
        tilde_style: pane_ctx.theme.ui.virtual_text,
        indent_guide_style: pane_ctx.theme.ui.indent_guide,
        indent_guides: pane_ctx.settings.indent_guides,
        pane_rect: pane_ctx.rect,
        theme: pane_ctx.theme,
        rope: pane_ctx.rope,
        default_gutter_scope: pane_ctx.default_gutter_scope,
    };
    let mut canvas = render::Canvas::new(grid, pane_ctx.theme.ui.invisible, pane_ctx.dim);

    // ── Display-line walk ────────────────────────────────────────────────
    let height = visible.content_height.min(pane_ctx.rect.height);
    // Which line's highlight intervals and cursorline state `line` currently
    // holds, so crossing into a new line is the only thing that rebuilds them.
    let mut line: Option<LineStyle> = None;
    let mut screen_row = 0u16;

    while screen_row < height {
        // One resolve per display line, not two: `render_display_line` already walks
        // this line's block to answer, so branching on the display line it
        // hands back (rather than calling `dlm.slot(pos)` first to decide
        // which arm to take) is what keeps this a single walk.
        let rendered = dlm.render_display_line(pos);
        match rendered.display_line.kind {
            DisplayLineKind::Virtual { .. } => {
                // Virtual display lines are skipped by the style stage (no
                // highlight tiers, no cursor/selection), but each grapheme
                // can still carry its own `scope` from the provider that
                // produced it (already folded with `base_scope` in
                // `format_virtual_lines`): `theme.default` layered with that
                // scope, or the themed `virtual_text` fallback for
                // graphemes with none (matching the tilde-filler /
                // no-decoration look).
                style.styles.clear();
                let own = rendered.display_line.graphemes.clone();
                style.styles.extend(rendered.graphemes[own].iter().map(|g| {
                    match g.scope {
                        Some(id) => compose_ctx
                            .theme
                            .default
                            .layer(compose_ctx.theme.resolve(id)),
                        None => compose_ctx.theme.ui.virtual_text,
                    }
                }));
                // Row-wide fill, the virtual display line's counterpart of
                // content display lines' `Decoration::LineBg` tint:
                // extends `base_scope`'s `bg` across the gutter and past
                // the last grapheme to the window border, instead of
                // stopping at end-of-text.
                let row_bg = rendered
                    .base_scope
                    .and_then(|scope| compose_ctx.theme.resolve(scope).bg);
                render::compose_display_line(
                    &rendered,
                    &style.styles,
                    screen_row,
                    lane_widths,
                    &compose_ctx,
                    &mut canvas,
                    row_bg,
                );
            }
            _ => {
                let line = line.get_or_insert_with(|| {
                    LineStyle::enter(
                        pos.line,
                        last_content_line,
                        compose_ctx.primary_head_line,
                        pane_ctx,
                        style,
                    )
                });
                style
                    .styles
                    .resize(rendered.graphemes.len(), ResolvedStyle::default());
                crate::style::style_display_line(
                    rendered.display_line,
                    rendered.graphemes,
                    line.chars,
                    pane_ctx.selections,
                    line.is_head_line,
                    line.tint,
                    pane_ctx.settings.mode,
                    pane_ctx.settings.cursor_is_block,
                    pane_ctx.theme,
                    style,
                );
                // Cursorline wins over the tint: a theme whose cursorline
                // has no `bg` falls through to the tint automatically.
                let row_bg = line
                    .is_head_line
                    .then_some(pane_ctx.theme.ui.cursorline.bg)
                    .flatten()
                    .or_else(|| line.tint.and_then(|scope| pane_ctx.theme.resolve(scope).bg));
                render::compose_display_line(
                    &rendered,
                    &style.styles[rendered.display_line.graphemes.clone()],
                    screen_row,
                    lane_widths,
                    &compose_ctx,
                    &mut canvas,
                    row_bg,
                );
            }
        }

        screen_row += 1;
        match dlm.next(pos) {
            Some(next) => {
                if next.line != pos.line {
                    line = None;
                }
                pos = next;
            }
            None => break,
        }
    }

    render::render_tilde_fillers(screen_row, lane_widths, &compose_ctx, &mut canvas);
}

// ---------------------------------------------------------------------------
// Per-line style state
// ---------------------------------------------------------------------------

/// The per-line facts every content display line of one buffer line shares.
///
/// Built once when the walk crosses into a line; building it also rebuilds the
/// highlight interval buffers, which is the expensive part.
struct LineStyle {
    chars: ExclusiveRange<CharOffset>,
    is_head_line: bool,
    /// A provider-requested full-row background tint for this line, if any
    /// (`Decoration::LineBg`), resolved once here and read at both paint
    /// sites (the row-fill `row_bg` and `style_display_line`'s per-grapheme
    /// layering) so they can't disagree about which line is tinted.
    tint: Option<crate::types::ScopeId>,
}

impl LineStyle {
    fn enter(
        line_idx: hume_rope::line::ContentLine,
        last_content_line: hume_rope::line::ContentLine,
        primary_head_line: hume_rope::line::ContentLine,
        pane_ctx: &PaneRenderCtx,
        style: &mut super::StyleScratch,
    ) -> Self {
        debug_assert!(
            line_idx <= last_content_line,
            "display-line walk reached line {}, past the buffer's last \
             content line {}: `DisplayLineMap::last_line`, not \
             `visible.last_line_idx` (the phantom trailing-\\n line one past it)",
            line_idx.index(),
            last_content_line.index()
        );
        let tint = crate::style::rebuild_line_decorations(
            line_idx,
            pane_ctx.syntax,
            pane_ctx.providers,
            pane_ctx.rope,
            style,
        );
        let start_char = hume_rope::lines::line_start_char(pane_ctx.rope, line_idx.into());
        let end_char = hume_rope::lines::next_line_start(pane_ctx.rope, line_idx.into());
        Self {
            chars: ExclusiveRange::new(start_char, end_char),
            // Cursorline highlights only the primary cursor's line.
            is_head_line: line_idx == primary_head_line,
            tint,
        }
    }
}
