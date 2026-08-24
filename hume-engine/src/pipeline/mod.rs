use rustc_hash::FxHashMap;

use slotmap::{SlotMap, new_key_type};

use crate::format::FormatScratch;
use crate::pane::{Pane, WhitespaceConfig, WrapMode};
use crate::providers::{
    BottomBandProvider, DEFAULT_GUTTER_SCOPE, StatuslineProvider, SyntaxSpans, TabBarProvider,
};
use crate::style::StyleScratch;
use crate::theme::{ScopeRegistry, Theme};
use crate::types::{EditorMode, ScopeId};

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
    /// Pre-computed gutter lane widths used by the render stage.
    pub lane_widths: Vec<u16>,
}

impl FrameScratch {
    pub fn new() -> Self {
        Self {
            format: FormatScratch::new(),
            style: StyleScratch::new(),
            lane_widths: Vec::new(),
        }
    }

    /// Reset all buffers to empty, retaining allocated capacity.
    pub fn clear(&mut self) {
        self.format.clear();
        self.style.clear();
        self.lane_widths.clear();
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
/// `EngineView::render()` and `cursor::content_pos()` each frame. After a few
/// frames all internal `Vec`s have stabilised capacity and no further heap
/// allocations occur.
pub struct RenderContext {
    /// Engine pipeline scratch (format, style, gutter lane widths).
    pub(crate) frame: FrameScratch,
    /// Pane rects computed by the layout stage.
    pub(crate) pane_rects: Vec<(PaneId, ratatui::layout::Rect)>,
    /// Seam dividers computed alongside `pane_rects` each render — reused
    /// scratch storage, same rationale as `pane_rects`.
    pub(crate) seams: Vec<Seam>,
    /// Perpendicular-arm bits keyed by cell, computed from `seams` each
    /// render so junction glyphs (`┬ ┴ ├ ┤ ┼`) can be drawn where seams
    /// cross. Reused scratch storage, same rationale as `seams`.
    pub(crate) seam_arms: FxHashMap<(u16, u16), u8>,
    /// Scratch for cursor-position computation (`cursor::content_pos` and scroll).
    /// Distinct from `frame.format` — used outside the render pipeline, where
    /// borrowing `frame` simultaneously would conflict.
    pub cursor_format: FormatScratch,
    /// Where the focused pane's cursor landed within the pane's content area
    /// (pane-relative, *before* the gutter and pane origin are added — not a
    /// terminal-absolute screen cell), resolved by the scroll step that
    /// already had the row map open. `None` until that step runs, and reset
    /// every frame — a `RenderContext` outlives the frame that filled it, so
    /// a leftover value must never read as the current one. Callers that need
    /// the real screen cell add the gutter width and the pane rect's origin
    /// (see `lifecycle.rs`/`overlay_sync.rs` in `hume-editor`).
    pub cursor_content_pos: Option<(u16, u16)>,
}

impl RenderContext {
    pub fn new() -> Self {
        Self {
            frame: FrameScratch::new(),
            pane_rects: Vec::new(),
            seams: Vec::new(),
            seam_arms: FxHashMap::default(),
            cursor_format: FormatScratch::new(),
            cursor_content_pos: None,
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
    /// Pure `BufferId` allocator: a buffer's content, syntax, and rope all
    /// live in the editor's `Document`/`Buffer` — this slotmap only mints and
    /// validates IDs so `PaneId -> BufferId` references stay checkable.
    pub buffers: SlotMap<BufferId, ()>,
    pub theme: Theme,
    /// Session-wide scope registry. Providers intern their scopes here.
    /// `Editor::prepare_frame` calls `theme.bake_if_stale(&registry)` twice
    /// per frame — before its own steps run and again after — so a scope
    /// interned by one of those steps (extra highlights, a newly attached
    /// grammar, ...) is still baked before `render_into` resolves anything.
    /// No other call site needs to bake manually after interning.
    pub registry: ScopeRegistry,
    /// `DEFAULT_GUTTER_SCOPE` interned once at construction — carried on
    /// `PaneRenderCtx`/`ComposeCtx` so gutter composition never falls back to
    /// a by-name lookup on the per-cell hot path.
    default_gutter_scope: ScopeId,
    /// Optional tab bar rendered at the top of the terminal area.
    pub tabbar: Option<Box<dyn TabBarProvider>>,
    /// Bottom chrome bands, stacked directly above the statusline —
    /// currently the pick-list drawer and the docked hover popup. Only one
    /// is ever non-empty in practice, but the list carries both rather than
    /// special-casing which one "owns" the band.
    pub bottom_bands: Vec<Box<dyn BottomBandProvider>>,
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
        let mut registry = ScopeRegistry::new();
        let default_gutter_scope = registry.intern(DEFAULT_GUTTER_SCOPE.0);
        Self {
            // Placeholder layout — will be replaced before the first render.
            layout: LayoutTree::Leaf(PaneId::default()),
            panes,
            buffers,
            theme,
            registry,
            default_gutter_scope,
            tabbar: None,
            bottom_bands: Vec::new(),
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
        self.layout
            .find_rect(pid, self.last_pane_area, self.reserve_seam)
    }

    /// Partition `area` into the pane-content rect, reserving a tab-bar row at
    /// the top (if `self.tabbar` is set), the bottom chrome bands directly
    /// above the statusline (`self.bottom_bands`), and a statusline row at
    /// the bottom (always). Single source of truth for chrome layout —
    /// `render` and the editor's `prepare_frame` both partition through this
    /// method so pane geometry is computed identically wherever it's needed.
    pub fn pane_area(&self, area: ratatui::layout::Rect) -> ratatui::layout::Rect {
        let tabbar_height: u16 = if self.tabbar.is_some() { 1 } else { 0 };
        let bands_height: u16 = self
            .bottom_bands
            .iter()
            .map(|b| b.height(area.height / 2))
            .sum();
        let chrome_height = tabbar_height + 1 + bands_height;

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
    /// inside this call — no rope is stored in `EngineView`.
    ///
    /// `get_syntax` resolves a `BufferId` to its syntax highlight span
    /// source, if any — same per-frame-borrow contract as `get_rope`.
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
        get_syntax: impl Fn(BufferId) -> Option<&'rope dyn SyntaxSpans>,
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
        // line into — providers write text via `write_text_run`, whose
        // `set_cell` bounds-checks against the buffer rect rather than
        // panicking, but a chrome row drawn at an out-of-bounds `y` is still
        // silently lost, so skip both chrome rows entirely rather than
        // handing them a `Rect` claiming a row that doesn't exist.
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

            // ── Render bottom bands ──────────────────────────────────────────────
            // Stacked directly above the statusline row, in registration
            // order — derived from `area` directly (not `pane_area`),
            // matching the tab bar/statusline's own convention: chrome
            // claims its band even when the terminal is too small to also
            // fit pane content (`pane_area`'s degenerate branch already
            // collapses `height` to 0 there).
            let mut bottom_edge = (area.y + area.height).saturating_sub(1);
            for band in &self.bottom_bands {
                let band_height = band.height(area.height / 2);
                if band_height == 0 {
                    continue;
                }
                let band_y = bottom_edge.saturating_sub(band_height);
                let available = bottom_edge.saturating_sub(area.y);
                let band_area = ratatui::layout::Rect {
                    y: band_y.max(area.y),
                    height: band_height.min(available),
                    ..area
                };
                band.render(band_area, &self.theme, buf);
                bottom_edge = band_y.max(area.y);
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
            if self.buffers.get(pane.buffer_id).is_none() {
                continue;
            }
            // Resolve the rope from the caller — zero-copy, no clone needed.
            let Some(rope) = get_rope(pane.buffer_id) else {
                continue;
            };

            scratch.clear();

            let pane_ctx = PaneRenderCtx {
                pane,
                rope,
                syntax: get_syntax(pane.buffer_id),
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
                default_gutter_scope: self.default_gutter_scope,
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

            // Clamped once per seam, ahead of `canvas` below — it needs an
            // immutable `&Buffer` and the canvas holds an exclusive `&mut`
            // for the whole seam pass.
            let seam_bounds: Vec<(u16, u16, u16, u16)> = ctx
                .seams
                .iter()
                .map(|seam| crate::render::clamp_rect_to_buf(buf, seam.rect))
                .collect();
            let mut canvas = crate::render::Canvas::new(buf, &self.theme, None);
            for (seam, &(x0, y0, x1, y1)) in ctx.seams.iter().zip(seam_bounds.iter()) {
                let base = match seam.direction {
                    Direction::Horizontal => ARM_N | ARM_S,
                    Direction::Vertical => ARM_E | ARM_W,
                };
                // The slice of this seam adjacent to the focused pane — drawn
                // in the accent color. Computed once per seam so the per-cell
                // loop below only needs a bounds check, not a repeated call.
                let accent_rect = focused_rect.and_then(|fr| focused_seam_segment(seam.rect, fr));

                for y in y0..y1 {
                    for x in x0..x1 {
                        let arms = ctx.seam_arms.get(&(x, y)).copied().unwrap_or(0);
                        let glyph = junction_glyph(base | arms);
                        let in_accent = accent_rect.is_some_and(|r| {
                            x >= r.x && x < r.x + r.width && y >= r.y && y < r.y + r.height
                        }) || corners.is_some_and(|cs| cs.contains(&Some((x, y))));
                        let style = if in_accent { accent } else { muted };
                        // One junction glyph per seam cell. Through the same
                        // writer as everything else rather than a raw cell
                        // poke, bounded to this single cell. A box-drawing
                        // glyph never needs the placeholder substitution, but
                        // `write_text_run` takes no exemption from it.
                        canvas.write_text_run(x, y, glyph, style, x + 1);
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
/// panes on the same buffer may wrap differently, once `:wrap`/`:set pane
/// wrap-mode=…` pins one) — the editor resolves pane override → buffer
/// override → global default (see `commands::effective_wrap_mode`) and
/// copies the result through here alongside the document facts so the
/// render pipeline has one bundle to read. `mode` is a per-focus fact, not a
/// document fact: the editor resolves
/// it to the live editor mode only for the focused pane (whose fake cursor
/// must yield to the real terminal cursor in bar-cursor modes) and to a
/// block-cursor mode for every other pane.
#[derive(Clone)]
pub struct PaneRenderSettings {
    pub mode: EditorMode,
    pub wrap_mode: WrapMode,
    pub tab_width: u8,
    pub whitespace: WhitespaceConfig,
    pub show_indent_guides: bool,
}

/// Transient bundle of borrows needed to render one pane. Avoids passing a
/// dozen separate parameters through the call stack.
pub(crate) struct PaneRenderCtx<'a> {
    pub pane: &'a Pane,
    /// Rope borrowed from the caller's `Document` for this frame only.
    pub rope: &'a ropey::Rope,
    /// Syntax highlight span source borrowed from the caller via
    /// `get_syntax`, if a language with a grammar is configured.
    pub syntax: Option<&'a dyn SyntaxSpans>,
    pub theme: &'a Theme,
    pub rect: ratatui::layout::Rect,
    pub settings: PaneRenderSettings,
    /// `Some` for non-focused panes — blend every written cell's fg/bg toward
    /// this target by `factor`. `None` for the focused pane.
    pub dim: Option<(ratatui::style::Color, f32)>,
    /// See `EngineView::default_gutter_scope`.
    pub default_gutter_scope: ScopeId,
}
