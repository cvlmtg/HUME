use std::collections::HashMap;

use slotmap::{SlotMap, new_key_type};

use crate::format::FormatScratch;
use crate::pane::{Pane, WhitespaceConfig, WrapMode};
use crate::providers::{
    DrawerProvider, GutterCell, InlineInsert, StatuslineProvider, TabBarProvider,
};
use crate::style::StyleScratch;
use crate::syntax_layers::SyntaxLayers;
use crate::theme::{ScopeRegistry, Theme};
use crate::types::EditorMode;

mod layout;
mod pane_render;
#[cfg(test)]
mod tests;

use layout::{
    ARM_E, ARM_N, ARM_S, ARM_W, collect_seam_arms, focused_pane_corners, focused_seam_segment,
    junction_glyph,
};
pub use layout::{Direction, LayoutTree, Seam};
use pane_render::render_pane;

new_key_type! {
    /// Opaque handle to a buffer.
    pub struct BufferId;
    /// Opaque handle to a pane.
    pub struct PaneId;
}

// ---------------------------------------------------------------------------
// Shared buffer
// ---------------------------------------------------------------------------

/// State shared across all panes that view the same file.
///
/// The rope is intentionally absent — it lives in the editor's `Document`
/// and is injected into `EngineView::render()` via the `get_rope` closure at
/// render time. Keeping it here would couple the engine to editor-domain
/// types and require per-frame clones to stay in sync. The syntax layers
/// (parse trees + shared highlighters) are engine-owned, since both halves
/// are already engine types — see [`crate::syntax_layers`].
pub struct SharedBuffer {
    /// Tree-sitter syntax layers (root grammar + embedded-language
    /// injections), rebuilt on each edit.
    ///
    /// Written by the parse worker's install path and baked each frame by
    /// `bake_pending_edits`. `None` until the first parse result arrives.
    /// The renderer tolerates `None` — it just renders without highlights.
    pub syntax: Option<crate::syntax_layers::SyntaxLayers>,
}

impl SharedBuffer {
    pub fn new() -> Self {
        Self { syntax: None }
    }
}

impl Default for SharedBuffer {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// Frame scratch buffers
// ---------------------------------------------------------------------------

/// Per-frame scratch storage reused across all pipeline stages.
/// Cleared at the start of each pane render. After a few frames, all `Vec`s
/// have stabilised capacity and no more heap allocations occur.
pub struct FrameScratch {
    /// Buffers for the Format stage (Stage 2).
    pub format: FormatScratch,
    /// Buffers for the Style stage (Stage 3).
    pub style: StyleScratch,
    /// Inline inserts collected for the current buffer line. Kept separate from
    /// `format` so the fused pipeline can borrow `&inline_inserts` and
    /// `&mut format` simultaneously without a borrow conflict.
    pub inline_inserts: Vec<InlineInsert>,
    /// Scratch storage for gutter cells rendered per row.
    pub gutter_cells: Vec<GutterCell>,
    /// Pre-computed gutter column widths used by the render stage.
    pub col_widths: Vec<u16>,
}

impl FrameScratch {
    pub fn new() -> Self {
        Self {
            format: FormatScratch::new(),
            style: StyleScratch::new(),
            inline_inserts: Vec::new(),
            gutter_cells: Vec::new(),
            col_widths: Vec::new(),
        }
    }

    /// Reset all buffers to empty, retaining allocated capacity.
    pub fn clear(&mut self) {
        self.format.clear();
        self.style.clear();
        self.inline_inserts.clear();
        self.gutter_cells.clear();
        self.col_widths.clear();
    }

    /// Reset only the per-line buffers reused between buffer lines in the fused pipeline.
    pub(crate) fn clear_line(&mut self) {
        self.format.clear_line_bufs();
        self.style.styles.clear();
    }
}

impl Default for FrameScratch {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// Render context — all per-frame scratch in one place
// ---------------------------------------------------------------------------

/// All scratch buffers needed for one render pass.
///
/// Create once with `RenderContext::new()` and pass `&mut ctx` to
/// `EngineView::render()` and `cursor::screen_pos()` each frame. After a few
/// frames all internal `Vec`s have stabilised capacity and no further heap
/// allocations occur.
pub struct RenderContext {
    /// Engine pipeline scratch (format, style, inline inserts, gutter cells).
    pub(crate) frame: FrameScratch,
    /// Pane rects computed by the layout stage.
    pub(crate) pane_rects: Vec<(PaneId, ratatui::layout::Rect)>,
    /// Seam dividers computed alongside `pane_rects` each render — reused
    /// scratch storage, same rationale as `pane_rects`.
    pub(crate) seams: Vec<Seam>,
    /// Perpendicular-arm bits keyed by cell, computed from `seams` each
    /// render so junction glyphs (`┬ ┴ ├ ┤ ┼`) can be drawn where seams
    /// cross. Reused scratch storage, same rationale as `seams`.
    pub(crate) seam_arms: HashMap<(u16, u16), u8>,
    /// Scratch for cursor-position computation (`cursor::screen_pos` and scroll).
    /// Distinct from `frame.format` — used outside the render pipeline, where
    /// borrowing `frame` simultaneously would conflict.
    pub cursor_format: FormatScratch,
}

impl RenderContext {
    pub fn new() -> Self {
        Self {
            frame: FrameScratch::new(),
            pane_rects: Vec::new(),
            seams: Vec::new(),
            seam_arms: HashMap::new(),
            cursor_format: FormatScratch::new(),
        }
    }
}

impl Default for RenderContext {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// Editor view — top-level owner
// ---------------------------------------------------------------------------

/// The root of the editor's rendering state.
pub struct EngineView {
    pub layout: LayoutTree,
    pub panes: SlotMap<PaneId, Pane>,
    pub buffers: SlotMap<BufferId, SharedBuffer>,
    pub theme: Theme,
    /// Session-wide scope registry. Providers intern their scopes here.
    /// `Editor::prepare_frame` calls `theme.bake_if_stale(&registry)` once per
    /// frame, before every render, so newly interned scopes are always baked —
    /// no other call site needs to bake manually after interning.
    pub registry: ScopeRegistry,
    /// Optional tab bar rendered at the top of the terminal area.
    pub tabbar: Option<Box<dyn TabBarProvider>>,
    /// Optional bottom drawer, rendered directly above the statusline.
    pub drawer: Option<Box<dyn DrawerProvider>>,
    /// Terminal area available to panes as of the last `prepare_frame`.
    /// Pane-focus/split commands run between frames with no terminal handle
    /// of their own, so they recompute geometry from this plus `layout`
    /// (see `pane_rects`/`pane_rect`) rather than trusting a stored rect
    /// list — a handful of panes makes the DFS cheap enough that there is no
    /// cache to go stale when a command mutates `layout` mid-frame. Zero
    /// area until the first `prepare_frame`.
    pub last_pane_area: ratatui::layout::Rect,
    /// Raw terminal area (before chrome subtraction) as of the last
    /// `prepare_frame` — the same `area` passed to `pane_area`/`render`.
    /// Distinct from `last_pane_area`: chrome that reserves rows off a
    /// fraction of this raw height (the drawer's `max` ceiling) needs the
    /// *un-subtracted* figure, since `last_pane_area` already has the
    /// drawer's own reserved rows folded out of it.
    pub last_terminal_area: ratatui::layout::Rect,
    /// Whether pane splits reserve a 1-cell seam column/row — mirrors the
    /// `pane-dividers` setting. Set alongside `last_pane_area`; consulted by
    /// the same recompute helpers.
    pub reserve_seam: bool,
}

/// Blend fraction used to dim non-focused panes toward `ui.background` — 0.0
/// leaves colors untouched, 1.0 flattens them entirely to the background.
const PANE_DIM_FACTOR: f32 = 0.5;

impl EngineView {
    pub fn new(theme: Theme) -> Self {
        let panes = SlotMap::with_key();
        let buffers = SlotMap::with_key();
        Self {
            // Placeholder layout — will be replaced before the first render.
            layout: LayoutTree::Leaf(PaneId::default()),
            panes,
            buffers,
            theme,
            registry: ScopeRegistry::new(),
            tabbar: None,
            drawer: None,
            last_pane_area: ratatui::layout::Rect::default(),
            last_terminal_area: ratatui::layout::Rect::default(),
            reserve_seam: true,
        }
    }

    /// Recompute the current pane partition from `layout` and
    /// `last_pane_area`. Cheap even with several splits open — recomputing
    /// beats keeping a cross-frame cache in sync with every layout mutation.
    pub fn pane_rects(&self) -> Vec<(PaneId, ratatui::layout::Rect)> {
        let mut out = Vec::new();
        self.layout
            .collect_rects_into(self.last_pane_area, self.reserve_seam, &mut out);
        out
    }

    /// The current on-screen rect of `pid`, or `None` if `last_pane_area` has
    /// no area yet (before the first `prepare_frame`).
    pub fn pane_rect(&self, pid: PaneId) -> Option<ratatui::layout::Rect> {
        self.pane_rects()
            .into_iter()
            .find(|(p, _)| *p == pid)
            .map(|(_, r)| r)
    }

    /// Partition `area` into the pane-content rect, reserving a tab-bar row at
    /// the top (if `self.tabbar` is set), a drawer band directly above the
    /// statusline (if `self.drawer` is set), and a statusline row at the
    /// bottom (always). Single source of truth for chrome layout — `render`
    /// and the editor's `prepare_frame` both partition through this method so
    /// pane geometry is computed identically wherever it's needed.
    pub fn pane_area(&self, area: ratatui::layout::Rect) -> ratatui::layout::Rect {
        let tabbar_height: u16 = if self.tabbar.is_some() { 1 } else { 0 };
        let drawer_height = self
            .drawer
            .as_ref()
            .map_or(0, |d| d.height(area.height / 2));
        let chrome_height = tabbar_height + 1 + drawer_height;

        if chrome_height < area.height {
            ratatui::layout::Rect {
                y: area.y + tabbar_height,
                height: area.height - chrome_height,
                ..area
            }
        } else {
            // Degenerate: terminal too small to fit chrome + content.
            ratatui::layout::Rect { height: 0, ..area }
        }
    }

    /// Render all panes into `buf` for the given terminal area.
    ///
    /// `get_rope` resolves a `BufferId` to the authoritative `&Rope` owned by
    /// the caller (typically the editor's `Document`). The borrow is used only
    /// inside this call — no rope is stored in `SharedBuffer`.
    ///
    /// Layout: the tab bar (if present) occupies the top row, the statusline
    /// always occupies the bottom row. Panes fill the remaining area.
    ///
    /// `focused_pane_id` drives seam-accent styling and non-focused dimming.
    /// `draw_dividers` mirrors the `pane-dividers` setting: `true` reserves
    /// and paints a 1-cell seam between sibling panes; `false` tiles panes
    /// edge-to-edge with no reserved gap. Non-focused panes are dimmed
    /// either way — dimming is the focus cue, the seam glyph is a separate
    /// cosmetic choice.
    #[allow(clippy::too_many_arguments)]
    pub fn render<'rope>(
        &self,
        area: ratatui::layout::Rect,
        buf: &mut ratatui::buffer::Buffer,
        get_rope: impl Fn(BufferId) -> Option<&'rope ropey::Rope>,
        get_pane_settings: impl Fn(PaneId) -> PaneRenderSettings,
        statusline: &dyn StatuslineProvider,
        focused_pane_id: PaneId,
        draw_dividers: bool,
        ctx: &mut RenderContext,
    ) {
        let scratch = &mut ctx.frame;
        let pane_rects = &mut ctx.pane_rects;
        let pane_area = self.pane_area(area);

        // A degenerate area (e.g. a terminal reporting height 0 during early
        // startup, or a genuinely tiny window) has no row to draw a chrome
        // line into — providers write text via `Buffer::set_string`, which
        // panics on an out-of-bounds row (unlike a background fill, which
        // clamps), so skip both chrome rows entirely rather than handing them
        // a `Rect` claiming a row that doesn't exist.
        if area.height > 0 {
            // ── Render tab bar ────────────────────────────────────────────────
            if let Some(ref tabbar) = self.tabbar {
                let tabbar_area = ratatui::layout::Rect {
                    y: area.y,
                    height: 1,
                    ..area
                };
                tabbar.render(tabbar_area, &self.theme, buf);
            }

            // ── Render drawer ────────────────────────────────────────────────────
            // Sits directly above the statusline row — derived from `area`
            // directly (not `pane_area`), matching the tab bar/statusline's
            // own convention: chrome claims its band even when the terminal
            // is too small to also fit pane content (`pane_area`'s
            // degenerate branch already collapses `height` to 0 there).
            if let Some(ref drawer) = self.drawer {
                let drawer_height = drawer.height(area.height / 2);
                if drawer_height > 0 {
                    let drawer_y = (area.y + area.height).saturating_sub(1 + drawer_height);
                    let drawer_area = ratatui::layout::Rect {
                        y: drawer_y.max(area.y),
                        height: drawer_height.min(area.height.saturating_sub(1)),
                        ..area
                    };
                    drawer.render(drawer_area, &self.theme, buf);
                }
            }

            // ── Render statusline ───────────────────────────────────────────────
            let sl_y = area.y + area.height.saturating_sub(1);
            let sl_area = ratatui::layout::Rect {
                y: sl_y,
                height: 1,
                ..area
            };
            statusline.render(sl_area, &self.theme, buf);
        }

        // ── Compute pane rects once; reuse for panes and overlays ─────────────
        pane_rects.clear();
        self.layout
            .collect_rects_into(pane_area, draw_dividers, pane_rects);

        // ── Render panes ──────────────────────────────────────────────────────
        for (pane_id, rect) in pane_rects.iter().copied() {
            let Some(pane) = self.panes.get(pane_id) else {
                continue;
            };
            let Some(buffer) = self.buffers.get(pane.buffer_id) else {
                continue;
            };
            // Resolve the rope from the caller — zero-copy, no clone needed.
            let Some(rope) = get_rope(pane.buffer_id) else {
                continue;
            };

            scratch.clear();

            let pane_ctx = PaneRenderCtx {
                pane,
                rope,
                syntax: buffer.syntax.as_ref(),
                theme: &self.theme,
                rect,
                settings: get_pane_settings(pane_id),
                // Dim non-focused panes so the active one reads clearly at a
                // glance — independent of `draw_dividers`, which only controls
                // the seam glyph. Skipped when `ui.background` has no explicit
                // bg — there is no defined blend target for custom themes that
                // leave it unset. Non-RGB targets no-op inside the compose path.
                dim: (pane_id != focused_pane_id)
                    .then_some(self.theme.ui.background.bg)
                    .flatten()
                    .map(|bg| (bg, PANE_DIM_FACTOR)),
            };
            render_pane(&pane_ctx, scratch, buf);
        }

        // ── Render seam dividers between panes ────────────────────────────────
        if draw_dividers {
            let focused_rect = pane_rects
                .iter()
                .find(|(pid, _)| *pid == focused_pane_id)
                .map(|(_, r)| *r);
            ctx.seams.clear();
            self.layout.collect_seams_into(pane_area, &mut ctx.seams);
            ctx.seam_arms.clear();
            collect_seam_arms(&ctx.seams, &mut ctx.seam_arms);

            let muted: ratatui::style::Style =
                self.theme.ui.background.layer(self.theme.ui.window).into();
            let accent: ratatui::style::Style = self
                .theme
                .ui
                .background
                .layer(self.theme.ui.window_focused)
                .into();

            // Junction cells at the focused pane's corners are missed by the
            // per-seam accent test (see `focused_pane_corners`); precompute
            // them once so the per-cell loop only does a membership check.
            let corners = focused_rect.map(focused_pane_corners);

            for seam in &ctx.seams {
                let base = match seam.direction {
                    Direction::Horizontal => ARM_N | ARM_S,
                    Direction::Vertical => ARM_E | ARM_W,
                };
                // The slice of this seam adjacent to the focused pane — drawn
                // in the accent color. Computed once per seam so the per-cell
                // loop below only needs a bounds check, not a repeated call.
                let accent_rect = focused_rect.and_then(|fr| focused_seam_segment(seam.rect, fr));

                let (x0, y0, x1, y1) = crate::render::clamp_rect_to_buf(buf, seam.rect);
                for y in y0..y1 {
                    for x in x0..x1 {
                        let arms = ctx.seam_arms.get(&(x, y)).copied().unwrap_or(0);
                        let glyph = junction_glyph(base | arms);
                        let in_accent = accent_rect.is_some_and(|r| {
                            x >= r.x && x < r.x + r.width && y >= r.y && y < r.y + r.height
                        }) || corners.is_some_and(|cs| cs.contains(&Some((x, y))));
                        let style = if in_accent { accent } else { muted };
                        buf[(x, y)].set_symbol(glyph).set_style(style);
                    }
                }
            }
        }

        // ── Render overlays on top (may span panes) ───────────────────────────
        for (pane_id, _rect) in pane_rects.iter().copied() {
            let Some(pane) = self.panes.get(pane_id) else {
                continue;
            };
            for (_, overlay) in &pane.providers.overlays {
                if overlay.is_active() {
                    overlay.render(pane_area, &self.theme, buf);
                }
            }
        }
    }
}

/// Per-pane render settings supplied by the editor at render time.
///
/// `tab_width` and `whitespace` are document facts — resolved from per-buffer
/// overrides against global settings, identical for every pane viewing the
/// same buffer. Caching them on the engine `Pane` would duplicate state and
/// require frame-by-frame sync, so the editor resolves them fresh each frame
/// and passes them via this bundle. `wrap_mode` is genuinely per-pane (two
/// panes on the same buffer may wrap differently) — its SSOT is
/// `Pane::wrap_mode`; the editor just copies the resolved value through here
/// alongside the document facts so the render pipeline has one bundle to
/// read. `mode` is a per-focus fact, not a document fact: the editor resolves
/// it to the live editor mode only for the focused pane (whose fake cursor
/// must yield to the real terminal cursor in bar-cursor modes) and to a
/// block-cursor mode for every other pane.
#[derive(Clone)]
pub struct PaneRenderSettings {
    pub mode: EditorMode,
    pub wrap_mode: WrapMode,
    pub tab_width: u8,
    pub whitespace: WhitespaceConfig,
}

impl Default for PaneRenderSettings {
    fn default() -> Self {
        Self {
            mode: EditorMode::Normal,
            wrap_mode: WrapMode::None,
            tab_width: 4,
            whitespace: WhitespaceConfig::default(),
        }
    }
}

/// Transient bundle of borrows needed to render one pane. Avoids passing a
/// dozen separate parameters through the call stack.
pub(crate) struct PaneRenderCtx<'a> {
    pub pane: &'a Pane,
    /// Rope borrowed from the caller's `Document` for this frame only.
    pub rope: &'a ropey::Rope,
    /// Tree-sitter syntax layers from `SharedBuffer`, if a language with a
    /// grammar is configured.
    pub syntax: Option<&'a SyntaxLayers>,
    pub theme: &'a Theme,
    pub rect: ratatui::layout::Rect,
    pub settings: PaneRenderSettings,
    /// `Some` for non-focused panes — blend every written cell's fg/bg toward
    /// this target by `factor`. `None` for the focused pane.
    pub dim: Option<(ratatui::style::Color, f32)>,
}

// ---------------------------------------------------------------------------
// Viewport cursor — tracks skip / emit / full state across all row sources
// ---------------------------------------------------------------------------

/// Mutable progress state for the fused render loop.
///
/// Centralises the three pieces of state that every row source (virtual lines,
/// buffer lines, filler rows) must consult before emitting a display row.
struct ViewportCursor {
    /// Next screen row to write to.
    screen_row: u16,
    /// Maximum screen rows available for content.
    viewport_height: u16,
    /// Rows still to skip at the top (viewport scrolled into a wrapped line).
    top_skip_remaining: usize,
    /// Index into the sorted `virtual_lines` scratch buffer.
    vl_cursor: usize,
}

impl ViewportCursor {
    fn is_full(&self) -> bool {
        self.screen_row >= self.viewport_height
    }

    /// If rows remain to be skipped, decrement the counter and return `true`.
    fn try_skip(&mut self) -> bool {
        if self.top_skip_remaining > 0 {
            self.top_skip_remaining -= 1;
            true
        } else {
            false
        }
    }
}
