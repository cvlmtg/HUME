use slotmap::{SlotMap, new_key_type};

use crate::builtins::tree_sitter_hl::TreeSitterHighlighter;
use crate::format::FormatScratch;
use crate::pane::{Pane, WhitespaceConfig, WrapMode};
use crate::providers::{
    GutterCell, InlineInsert, StatuslineProvider, TabBarProvider, VirtualLineAnchor,
};
use crate::render::ComposeCtx;
use crate::style::StyleScratch;
use crate::theme::{ScopeRegistry, Theme};
use crate::types::{DisplayRow, EditorMode, ResolvedStyle, RowKind};

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
/// The rope and the syntax highlighter are intentionally absent — they live in
/// the editor's `Document` / `BufferSyntax` respectively, and are injected into
/// `EngineView::render()` via the `get_rope` / `get_syntax` closures at render
/// time. Keeping them here would couple the engine to editor-domain types and
/// require per-frame clones to stay in sync.
pub struct SharedBuffer {
    /// Incremental tree-sitter parse tree, rebuilt on each edit.
    ///
    /// Written by the parse worker (`install_parse_done`) and baked each frame
    /// by `bake_pending_edits`. `None` until the first parse result arrives.
    /// The renderer tolerates `None` — it just renders without highlights.
    pub tree: Option<tree_sitter::Tree>,
}

impl SharedBuffer {
    pub fn new() -> Self {
        Self { tree: None }
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
        self.format.display_rows.clear();
        self.format.graphemes.clear();
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
// Layout tree
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Direction {
    Horizontal,
    Vertical,
}

/// A single 1-cell divider segment drawn between two sibling panes at one
/// split node.
///
/// Emitted by `LayoutTree::collect_seams_into`, which walks the same
/// recursion as `collect_rects_into` off the same `split_rect` math, so seam
/// and leaf geometry can never drift apart.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Seam {
    pub rect: ratatui::layout::Rect,
    /// The split that produced this seam. `Horizontal` (a width split)
    /// carves a 1-column-wide, full-height seam — drawn as a vertical line
    /// (`│`). `Vertical` (a height split) carves a 1-row-tall, full-width
    /// seam — drawn as a horizontal line (`─`). Same axis inversion as
    /// `LayoutTree::Split` (see `split_focused_pane`'s doc comment).
    pub direction: Direction,
}

/// Recursive layout tree. Leaves reference panes; splits partition space.
#[derive(Clone, Debug, PartialEq)]
pub enum LayoutTree {
    Leaf(PaneId),
    Split {
        direction: Direction,
        /// Fraction (0.0–1.0) allocated to the first child.
        ratio: f32,
        children: Box<(LayoutTree, LayoutTree)>,
    },
}

impl LayoutTree {
    /// Compute (PaneId, Rect) pairs for the leaf panes given the total area.
    /// Results are appended to `out` (which is not cleared — caller must clear first).
    pub fn collect_rects_into(
        &self,
        area: ratatui::layout::Rect,
        out: &mut Vec<(PaneId, ratatui::layout::Rect)>,
    ) {
        match self {
            LayoutTree::Leaf(id) => out.push((*id, area)),
            LayoutTree::Split {
                direction,
                ratio,
                children,
            } => {
                let (r1, _seam, r2) = split_rect(area, *direction == Direction::Vertical, *ratio);
                children.0.collect_rects_into(r1, out);
                children.1.collect_rects_into(r2, out);
            }
        }
    }

    /// Compute the 1-cell seam divider drawn between sibling panes at every
    /// split node, given the total area. Walks the same recursion as
    /// `collect_rects_into`, off the same `split_rect` math, so seams always
    /// align with the leaf rects computed for the same `area` this frame.
    /// Results are appended to `out` (not cleared — caller must clear first).
    pub fn collect_seams_into(&self, area: ratatui::layout::Rect, out: &mut Vec<Seam>) {
        if let LayoutTree::Split {
            direction,
            ratio,
            children,
        } = self
        {
            let (r1, seam, r2) = split_rect(area, *direction == Direction::Vertical, *ratio);
            if seam.width > 0 && seam.height > 0 {
                out.push(Seam {
                    rect: seam,
                    direction: *direction,
                });
            }
            children.0.collect_seams_into(r1, out);
            children.1.collect_seams_into(r2, out);
        }
    }

    /// Replace `Leaf(target)` with a `Split` of `(Leaf(target), Leaf(new_pane))`.
    /// Returns whether `target` was found.
    pub fn split_leaf(
        &mut self,
        target: PaneId,
        new_pane: PaneId,
        direction: Direction,
        ratio: f32,
    ) -> bool {
        match self {
            LayoutTree::Leaf(id) if *id == target => {
                *self = LayoutTree::Split {
                    direction,
                    ratio,
                    children: Box::new((LayoutTree::Leaf(target), LayoutTree::Leaf(new_pane))),
                };
                true
            }
            LayoutTree::Leaf(_) => false,
            LayoutTree::Split { children, .. } => {
                children.0.split_leaf(target, new_pane, direction, ratio)
                    || children.1.split_leaf(target, new_pane, direction, ratio)
            }
        }
    }

    /// Leftmost leaf, found by descending into the first child at each split.
    fn first_leaf(&self) -> PaneId {
        match self {
            LayoutTree::Leaf(id) => *id,
            LayoutTree::Split { children, .. } => children.0.first_leaf(),
        }
    }

    /// Prune `Leaf(target)`, collapsing its parent `Split` onto the sibling.
    /// Returns the leftmost leaf of the promoted sibling (the new focus target),
    /// or `None` if `self` is the sole leaf.
    pub fn remove_leaf(&mut self, target: PaneId) -> Option<PaneId> {
        match self {
            LayoutTree::Leaf(_) => None,
            LayoutTree::Split { children, .. } => {
                let hit0 = matches!(&children.0, LayoutTree::Leaf(id) if *id == target);
                let hit1 = matches!(&children.1, LayoutTree::Leaf(id) if *id == target);
                if hit0 || hit1 {
                    // Promote the sibling subtree; the placeholder left behind
                    // in `keep` is discarded when `*self` is overwritten below.
                    let keep = if hit0 {
                        &mut children.1
                    } else {
                        &mut children.0
                    };
                    let sibling = std::mem::replace(keep, LayoutTree::Leaf(PaneId::default()));
                    let survivor = sibling.first_leaf();
                    *self = sibling;
                    Some(survivor)
                } else {
                    children
                        .0
                        .remove_leaf(target)
                        .or_else(|| children.1.remove_leaf(target))
                }
            }
        }
    }
}

/// Split `area` into two child rects plus the 1-cell seam divider drawn
/// between them, reserving that cell from the split axis so `first`, `seam`,
/// and `second` together — not `first`/`second` alone — tile `area` exactly.
///
/// All arithmetic is saturating so degenerate areas (zero width/height, or a
/// ratio that leaves no room for a seam) clamp to empty rects rather than
/// panicking. Callers must not invoke this on an area too small to hold a
/// seam plus two minimal panes — `:split`/`:vsplit` guard that before
/// mutating the layout tree (see `split_focused_pane` in the editor crate).
fn split_rect(
    area: ratatui::layout::Rect,
    vertical: bool,
    ratio: f32,
) -> (
    ratatui::layout::Rect,
    ratatui::layout::Rect,
    ratatui::layout::Rect,
) {
    if vertical {
        let usable = area.height.saturating_sub(1);
        let h1 = ((usable as f32 * ratio) as u16).min(usable);
        let seam_h = area.height.saturating_sub(h1).min(1);
        let r1 = ratatui::layout::Rect { height: h1, ..area };
        let seam = ratatui::layout::Rect {
            y: area.y + h1,
            height: seam_h,
            ..area
        };
        let r2 = ratatui::layout::Rect {
            y: area.y + h1 + seam_h,
            height: area.height.saturating_sub(h1 + seam_h),
            ..area
        };
        (r1, seam, r2)
    } else {
        let usable = area.width.saturating_sub(1);
        let w1 = ((usable as f32 * ratio) as u16).min(usable);
        let seam_w = area.width.saturating_sub(w1).min(1);
        let r1 = ratatui::layout::Rect { width: w1, ..area };
        let seam = ratatui::layout::Rect {
            x: area.x + w1,
            width: seam_w,
            ..area
        };
        let r2 = ratatui::layout::Rect {
            x: area.x + w1 + seam_w,
            width: area.width.saturating_sub(w1 + seam_w),
            ..area
        };
        (r1, seam, r2)
    }
}

/// Whether `seam` sits on the boundary of `pane_rect` — used to pick the
/// focused-accent vs. muted seam color. A seam "touches" a rect if it's
/// immediately adjacent on the perpendicular axis and their spans on the
/// parallel axis overlap. Works for both seam orientations without needing
/// `Seam::direction`: a width-split seam has `width == 1` (checked by the
/// horizontal-adjacency arm), a height-split seam has `height == 1` (checked
/// by the vertical-adjacency arm).
fn seam_touches_rect(seam: ratatui::layout::Rect, pane: ratatui::layout::Rect) -> bool {
    let touches_horizontally = (seam.x == pane.x + pane.width || seam.x + seam.width == pane.x)
        && seam.y < pane.y + pane.height
        && pane.y < seam.y + seam.height;
    let touches_vertically = (seam.y == pane.y + pane.height || seam.y + seam.height == pane.y)
        && seam.x < pane.x + pane.width
        && pane.x < seam.x + seam.width;
    touches_horizontally || touches_vertically
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
    /// Session-wide scope registry. Providers intern their scopes here at
    /// construction time. Call `theme.bake(&registry)` once, after all
    /// providers are registered and before the first render, to make
    /// `theme.resolve(ScopeId)` an O(1) Vec index.
    pub registry: ScopeRegistry,
    /// Optional tab bar rendered at the top of the terminal area.
    pub tabbar: Option<Box<dyn TabBarProvider>>,
    /// Last-rendered pane rects (DFS pre-order). Populated by the editor's
    /// `prepare_frame` each frame; read by pane-focus commands + cursor
    /// placement. Empty until the first `prepare_frame`.
    pub pane_rects: Vec<(PaneId, ratatui::layout::Rect)>,
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
            pane_rects: Vec::new(),
        }
    }

    /// Partition `area` into the pane-content rect, reserving a tab-bar row at
    /// the top (if `self.tabbar` is set) and a statusline row at the bottom (if
    /// `has_statusline`). Single source of truth for chrome layout — `render`
    /// and the editor's `prepare_frame` (which caches pane rects a frame ahead
    /// of `render`) both partition through this method so the cache and the
    /// draw never disagree.
    pub fn pane_area(
        &self,
        area: ratatui::layout::Rect,
        has_statusline: bool,
    ) -> ratatui::layout::Rect {
        let tabbar_height: u16 = if self.tabbar.is_some() { 1 } else { 0 };
        let statusline_height: u16 = if has_statusline { 1 } else { 0 };
        let chrome_height = tabbar_height + statusline_height;

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
    /// (if present) occupies the bottom row. Panes fill the remaining area.
    ///
    /// `focused_pane_id` and `draw_dividers` drive the pane-seam / dimming
    /// chrome (see the module-level notes on `Seam`): when `draw_dividers` is
    /// `false` this behaves exactly as before — panes tile with no gap.
    #[allow(clippy::too_many_arguments)]
    pub fn render<'rope, 'syn>(
        &self,
        area: ratatui::layout::Rect,
        buf: &mut ratatui::buffer::Buffer,
        get_rope: impl Fn(BufferId) -> Option<&'rope ropey::Rope>,
        get_syntax: impl Fn(BufferId) -> Option<&'syn TreeSitterHighlighter>,
        get_pane_settings: impl Fn(PaneId) -> PaneRenderSettings,
        statusline: Option<&dyn StatuslineProvider>,
        focused_pane_id: PaneId,
        draw_dividers: bool,
        ctx: &mut RenderContext,
    ) {
        let scratch = &mut ctx.frame;
        let pane_rects = &mut ctx.pane_rects;
        let pane_area = self.pane_area(area, statusline.is_some());

        // ── Render tab bar ────────────────────────────────────────────────────
        if let Some(ref tabbar) = self.tabbar {
            let tabbar_area = ratatui::layout::Rect {
                y: area.y,
                height: 1,
                ..area
            };
            tabbar.render(tabbar_area, &self.theme, buf);
        }

        // ── Render statusline ─────────────────────────────────────────────────
        if let Some(statusline) = statusline {
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
        self.layout.collect_rects_into(pane_area, pane_rects);

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
                tree: buffer.tree.as_ref(),
                syntax: get_syntax(pane.buffer_id),
                theme: &self.theme,
                rect,
                settings: get_pane_settings(pane_id),
            };
            render_pane(&pane_ctx, scratch, buf);

            // Dim non-focused panes so the active one reads clearly at a glance.
            // Skipped when `ui.background` has no explicit bg — there is no
            // defined blend target for custom themes that leave it unset.
            if draw_dividers
                && pane_id != focused_pane_id
                && let Some(bg) = self.theme.ui.background.bg
            {
                crate::render::dim_rect(buf, rect, bg, PANE_DIM_FACTOR);
            }
        }

        // ── Render seam dividers between panes ────────────────────────────────
        if draw_dividers {
            let focused_rect = pane_rects
                .iter()
                .find(|(pid, _)| *pid == focused_pane_id)
                .map(|(_, r)| *r);
            ctx.seams.clear();
            self.layout.collect_seams_into(pane_area, &mut ctx.seams);
            for seam in &ctx.seams {
                let is_focused = focused_rect.is_some_and(|fr| seam_touches_rect(seam.rect, fr));
                let win_style = if is_focused {
                    self.theme.ui.window_focused
                } else {
                    self.theme.ui.window
                };
                let style: ratatui::style::Style = self.theme.ui.background.layer(win_style).into();
                let glyph = if seam.direction == Direction::Horizontal {
                    "│"
                } else {
                    "─"
                };
                crate::render::draw_glyph_rect(buf, seam.rect, glyph, style);
            }
        }

        // ── Render overlays on top (may span panes) ───────────────────────────
        for (pane_id, _rect) in pane_rects.iter().copied() {
            let Some(pane) = self.panes.get(pane_id) else {
                continue;
            };
            for overlay in &pane.providers.overlays {
                if overlay.is_active() {
                    overlay.render(pane_area, &self.theme, buf);
                }
            }
        }
    }
}

/// Per-pane render settings supplied by the editor at render time.
///
/// Mode, wrap, tab-width, and whitespace are editor-domain facts (they depend
/// on global settings and per-buffer overrides). Storing them on the engine
/// `Pane` would duplicate state and require frame-by-frame sync. Instead the
/// editor resolves them once per frame and passes them via this bundle.
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
    /// Tree-sitter parse tree from `SharedBuffer`, if available.
    pub tree: Option<&'a tree_sitter::Tree>,
    /// Tree-sitter syntax highlighter from `SharedBuffer`, if language is configured.
    pub syntax: Option<&'a TreeSitterHighlighter>,
    pub theme: &'a Theme,
    pub rect: ratatui::layout::Rect,
    pub settings: PaneRenderSettings,
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
    use crate::{layout, render};

    // ── Stage 1: Layout ───────────────────────────────────────────────────
    let visible = layout::compute_viewport(
        pane_ctx.rope,
        &pane_ctx.pane.viewport,
        &pane_ctx.settings.wrap_mode,
        &pane_ctx.pane.providers.gutter_columns,
    );

    // ── Pre-render: per-frame constant setup ──────────────────────────────

    // Selections arrive pre-sorted from the editor; copy once, reuse every row.
    scratch
        .style
        .populate_sorted_sels(&pane_ctx.pane.selections, pane_ctx.pane.primary_idx);

    // Pre-collect virtual lines from all providers; sort by anchor.
    scratch.format.virtual_lines.clear();
    for provider in &pane_ctx.pane.providers.virtual_lines {
        provider.virtual_lines(
            visible.line_range.clone(),
            visible.content_width,
            &mut scratch.format.virtual_lines,
        );
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
            .map(|c| c.width(visible.last_line_idx) as u16),
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
        pane_rect: pane_ctx.rect,
        theme: pane_ctx.theme,
        pane_bg: pane_ctx.theme.ui.background.bg,
    };

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
            buf,
        );
        if vc.is_full() {
            break;
        }

        render_buffer_line(pane_ctx, line_idx, &mut vc, scratch, &compose_ctx, buf);
        if vc.is_full() {
            break;
        }

        drain_virtual_lines(
            VirtualLineAnchor::After(line_idx),
            &mut vc,
            scratch,
            &compose_ctx,
            buf,
        );
        if vc.is_full() {
            break;
        }
    }

    render::render_tilde_fillers(vc.screen_row, &compose_ctx, buf);
}

// ---------------------------------------------------------------------------
// Per-line helpers
// ---------------------------------------------------------------------------

/// Emit all virtual lines whose anchor matches `anchor`, advancing `vc`.
///
/// Stops early if the viewport fills up. After returning, `vc.vl_cursor`
/// points past the last consumed virtual line.
fn drain_virtual_lines(
    anchor: VirtualLineAnchor,
    vc: &mut ViewportCursor,
    scratch: &mut FrameScratch,
    compose_ctx: &ComposeCtx,
    buf: &mut ratatui::buffer::Buffer,
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
            buf,
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
    buf: &mut ratatui::buffer::Buffer,
) {
    use crate::{format, render, style};

    scratch.format.line_texts.clear();

    if line_idx < pane_ctx.rope.len_lines() {
        // Collect and sort inline decorations for this line.
        scratch.inline_inserts.clear();
        for provider in &pane_ctx.pane.providers.inline_decorations {
            provider.decorations_for_line(line_idx, &mut scratch.inline_inserts);
        }
        scratch.inline_inserts.sort_by_key(|i| i.byte_offset);

        // Stage 2 (per line): format into scratch.format.display_rows + scratch.format.graphemes.
        // `inline_inserts` is kept outside `scratch.format` to allow simultaneous
        // `&scratch.inline_inserts` and `&mut scratch.format` without a borrow conflict.
        format::format_buffer_line(
            pane_ctx.rope,
            line_idx,
            pane_ctx.settings.tab_width,
            &pane_ctx.settings.whitespace,
            &pane_ctx.settings.wrap_mode,
            &scratch.inline_inserts,
            &mut scratch.format,
        );

        // Stage 3 (per line): build highlight intervals for this buffer line.
        style::rebuild_tier_bufs(
            line_idx,
            pane_ctx.syntax,
            &pane_ctx.pane.providers.highlights,
            pane_ctx.rope,
            pane_ctx.tree,
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
        // line_str borrows scratch.format.line_texts; must not clear it inside the loop.
        let line_str = scratch.format.line_texts.as_str();

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
                vc.screen_row,
                &scratch.col_widths,
                compose_ctx,
                buf,
                row_bg,
            );
            vc.screen_row += 1;
        }
    } else {
        // Past EOF: emit a single Filler row.
        scratch.format.display_rows.push(DisplayRow {
            kind: RowKind::Filler,
            graphemes: 0..0,
        });

        if !vc.try_skip() && !vc.is_full() {
            render::compose_row(
                &scratch.format.display_rows[0],
                &scratch.format.graphemes,
                &scratch.style.styles,
                "",
                vc.screen_row,
                &scratch.col_widths,
                compose_ctx,
                buf,
                None,
            );
            vc.screen_row += 1;
        }
    }

    scratch.clear_line();
}

/// Emit one virtual line: push graphemes into scratch, compose, then clear.
fn emit_virtual_row(
    vl_idx: usize,
    line_idx: usize,
    screen_row: u16,
    scratch: &mut FrameScratch,
    compose_ctx: &ComposeCtx,
    buf: &mut ratatui::buffer::Buffer,
) {
    use crate::render;

    // Field-split: read virtual_lines, write graphemes — different sub-struct fields.
    let g_start = scratch.format.graphemes.len();
    scratch
        .format
        .graphemes
        .extend_from_slice(&scratch.format.virtual_lines[vl_idx].graphemes);
    let provider_id = scratch.format.virtual_lines[vl_idx].provider_id;

    scratch.format.display_rows.push(DisplayRow {
        kind: RowKind::Virtual {
            provider_id,
            anchor_line: line_idx,
        },
        graphemes: g_start..scratch.format.graphemes.len(),
    });
    // Virtual rows keep default styles (the style stage skips them).
    scratch
        .style
        .styles
        .resize(scratch.format.graphemes.len(), ResolvedStyle::default());

    let row_idx = scratch.format.display_rows.len() - 1;
    render::compose_row(
        &scratch.format.display_rows[row_idx],
        &scratch.format.graphemes,
        &scratch.style.styles,
        "",
        screen_row,
        &scratch.col_widths,
        compose_ctx,
        buf,
        None,
    );

    scratch.clear_line();
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::layout::Rect;

    fn rect(x: u16, y: u16, w: u16, h: u16) -> Rect {
        Rect {
            x,
            y,
            width: w,
            height: h,
        }
    }

    // ── split_rect ───────────────────────────────────────────────────────

    #[test]
    fn split_rect_horizontal_half() {
        // width=100 → usable=99, w1 = (99*0.5) as u16 = 49 (truncates, not rounds).
        let (a, seam, b) = split_rect(rect(0, 0, 100, 50), false, 0.5);
        assert_eq!(a, rect(0, 0, 49, 50));
        assert_eq!(seam, rect(49, 0, 1, 50));
        assert_eq!(b, rect(50, 0, 50, 50));
    }

    #[test]
    fn split_rect_vertical_half() {
        // height=50 → usable=49, h1 = (49*0.5) as u16 = 24 (truncates, not rounds).
        let (a, seam, b) = split_rect(rect(0, 0, 100, 50), true, 0.5);
        assert_eq!(a, rect(0, 0, 100, 24));
        assert_eq!(seam, rect(0, 24, 100, 1));
        assert_eq!(b, rect(0, 25, 100, 25));
    }

    #[test]
    fn split_rect_ratio_zero_gives_remainder_to_second() {
        // ratio 0.0 → first gets nothing; second gets everything but the seam.
        let (a, seam, b) = split_rect(rect(0, 0, 100, 50), false, 0.0);
        assert_eq!(a.width, 0);
        assert_eq!(seam.width, 1);
        assert_eq!(b.width, 99);
    }

    #[test]
    fn split_rect_ratio_one_gives_remainder_to_first() {
        // ratio 1.0 → first gets everything but the seam; second gets nothing.
        let (a, seam, b) = split_rect(rect(0, 0, 100, 50), false, 1.0);
        assert_eq!(a.width, 99);
        assert_eq!(seam.width, 1);
        assert_eq!(b.width, 0);
    }

    #[test]
    fn split_rect_zero_area_no_panic() {
        let (a, seam, b) = split_rect(rect(0, 0, 0, 0), false, 0.5);
        assert_eq!(a.width, 0);
        assert_eq!(seam.width, 0);
        assert_eq!(b.width, 0);
    }

    #[test]
    fn split_rect_children_and_seam_tile_parent() {
        let area = rect(10, 5, 100, 40);
        let (a, seam, b) = split_rect(area, false, 0.3);
        assert_eq!(a.x, area.x);
        assert_eq!(seam.x, a.x + a.width);
        assert_eq!(b.x, seam.x + seam.width);
        assert_eq!(a.width + seam.width + b.width, area.width);
        assert_eq!(a.height, area.height);
        assert_eq!(seam.height, area.height);
        assert_eq!(b.height, area.height);
    }

    // ── LayoutTree ───────────────────────────────────────────────────────

    #[test]
    fn layout_tree_leaf_returns_single_rect() {
        let tree = LayoutTree::Leaf(PaneId::default());
        let mut out = Vec::new();
        tree.collect_rects_into(rect(0, 0, 80, 24), &mut out);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].1, rect(0, 0, 80, 24));
    }

    #[test]
    fn layout_tree_horizontal_split() {
        let id_a = PaneId::default();
        let id_b = PaneId::default();
        let tree = LayoutTree::Split {
            direction: Direction::Horizontal,
            ratio: 0.5,
            children: Box::new((LayoutTree::Leaf(id_a), LayoutTree::Leaf(id_b))),
        };
        let mut out = Vec::new();
        tree.collect_rects_into(rect(0, 0, 100, 50), &mut out);
        assert_eq!(out.len(), 2);
        // Seam eats 1 column from the first child; the second child's
        // geometry is unaffected (see split_rect_horizontal_half).
        assert_eq!(out[0].1.width, 49);
        assert_eq!(out[1].1.x, 50);
        assert_eq!(out[1].1.width, 50);
    }

    #[test]
    fn layout_tree_vertical_split() {
        let tree = LayoutTree::Split {
            direction: Direction::Vertical,
            ratio: 0.5,
            children: Box::new((
                LayoutTree::Leaf(PaneId::default()),
                LayoutTree::Leaf(PaneId::default()),
            )),
        };
        let mut out = Vec::new();
        tree.collect_rects_into(rect(0, 0, 100, 50), &mut out);
        assert_eq!(out.len(), 2);
        // Seam eats 1 row from the first child; the second child's geometry
        // is unaffected (see split_rect_vertical_half).
        assert_eq!(out[0].1.height, 24);
        assert_eq!(out[1].1.y, 25);
        assert_eq!(out[1].1.height, 25);
    }

    #[test]
    fn layout_tree_collect_appends_without_clearing() {
        let tree = LayoutTree::Leaf(PaneId::default());
        let mut out = vec![(PaneId::default(), rect(99, 99, 1, 1))]; // pre-existing entry
        tree.collect_rects_into(rect(0, 0, 80, 24), &mut out);
        assert_eq!(out.len(), 2); // appended, not replaced
    }

    // ── Seams ────────────────────────────────────────────────────────────

    #[test]
    fn collect_seams_leaf_has_no_seams() {
        let tree = LayoutTree::Leaf(PaneId::default());
        let mut out = Vec::new();
        tree.collect_seams_into(rect(0, 0, 80, 24), &mut out);
        assert!(out.is_empty());
    }

    #[test]
    fn collect_seams_one_split_yields_one_seam() {
        let tree = LayoutTree::Split {
            direction: Direction::Horizontal,
            ratio: 0.5,
            children: Box::new((
                LayoutTree::Leaf(PaneId::default()),
                LayoutTree::Leaf(PaneId::default()),
            )),
        };
        let mut out = Vec::new();
        tree.collect_seams_into(rect(0, 0, 100, 50), &mut out);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].rect, rect(49, 0, 1, 50));
        assert_eq!(out[0].direction, Direction::Horizontal);
    }

    #[test]
    fn collect_seams_nested_splits_yield_one_seam_per_split_node() {
        let [a, b, c] = pane_ids();
        let tree = LayoutTree::Split {
            direction: Direction::Horizontal,
            ratio: 0.5,
            children: Box::new((
                LayoutTree::Leaf(a),
                LayoutTree::Split {
                    direction: Direction::Vertical,
                    ratio: 0.5,
                    children: Box::new((LayoutTree::Leaf(b), LayoutTree::Leaf(c))),
                },
            )),
        };
        let mut out = Vec::new();
        tree.collect_seams_into(rect(0, 0, 100, 100), &mut out);
        assert_eq!(out.len(), 2, "one seam per split node — root + nested");
    }

    #[test]
    fn seam_touches_rect_detects_horizontal_adjacency() {
        // Two panes side by side with a 1-col seam between them at x=49.
        let seam = rect(49, 0, 1, 50);
        let left_pane = rect(0, 0, 49, 50);
        let right_pane = rect(50, 0, 50, 50);
        assert!(seam_touches_rect(seam, left_pane));
        assert!(seam_touches_rect(seam, right_pane));
    }

    #[test]
    fn seam_touches_rect_detects_vertical_adjacency() {
        // Two panes stacked with a 1-row seam between them at y=24.
        let seam = rect(0, 24, 100, 1);
        let top_pane = rect(0, 0, 100, 24);
        let bottom_pane = rect(0, 25, 100, 25);
        assert!(seam_touches_rect(seam, top_pane));
        assert!(seam_touches_rect(seam, bottom_pane));
    }

    #[test]
    fn seam_touches_rect_false_for_non_adjacent_pane() {
        // A 3-pane row: a | seam | b | seam | c. The first seam does not
        // touch the third pane.
        let first_seam = rect(32, 0, 1, 50);
        let pane_c = rect(66, 0, 34, 50);
        assert!(!seam_touches_rect(first_seam, pane_c));
    }

    /// `PaneId::default()` is the slotmap null key — every default is equal,
    /// so tests that assert on distinct ids mint real ones off a throwaway map.
    fn pane_ids<const N: usize>() -> [PaneId; N] {
        let mut sm: SlotMap<PaneId, ()> = SlotMap::with_key();
        std::array::from_fn(|_| sm.insert(()))
    }

    #[test]
    fn split_leaf_on_root() {
        let [a, b] = pane_ids();
        let mut tree = LayoutTree::Leaf(a);
        assert!(tree.split_leaf(a, b, Direction::Vertical, 0.5));
        assert_eq!(
            tree,
            LayoutTree::Split {
                direction: Direction::Vertical,
                ratio: 0.5,
                children: Box::new((LayoutTree::Leaf(a), LayoutTree::Leaf(b))),
            }
        );
    }

    #[test]
    fn split_leaf_missing_target_is_noop() {
        let [a, b, missing] = pane_ids();
        let mut tree = LayoutTree::Leaf(a);
        assert!(!tree.split_leaf(missing, b, Direction::Vertical, 0.5));
        assert_eq!(tree, LayoutTree::Leaf(a));
    }

    #[test]
    fn split_leaf_on_nested_target() {
        let [a, b, c] = pane_ids();
        let mut tree = LayoutTree::Split {
            direction: Direction::Horizontal,
            ratio: 0.5,
            children: Box::new((LayoutTree::Leaf(a), LayoutTree::Leaf(b))),
        };
        assert!(tree.split_leaf(b, c, Direction::Vertical, 0.5));
        assert_eq!(
            tree,
            LayoutTree::Split {
                direction: Direction::Horizontal,
                ratio: 0.5,
                children: Box::new((
                    LayoutTree::Leaf(a),
                    LayoutTree::Split {
                        direction: Direction::Vertical,
                        ratio: 0.5,
                        children: Box::new((LayoutTree::Leaf(b), LayoutTree::Leaf(c))),
                    },
                )),
            }
        );
        let mut out = Vec::new();
        tree.collect_rects_into(rect(0, 0, 100, 100), &mut out);
        assert_eq!(out.len(), 3);
    }

    #[test]
    fn remove_leaf_collapses_parent() {
        let [a, b] = pane_ids();
        let mut tree = LayoutTree::Split {
            direction: Direction::Horizontal,
            ratio: 0.5,
            children: Box::new((LayoutTree::Leaf(a), LayoutTree::Leaf(b))),
        };
        assert_eq!(tree.remove_leaf(a), Some(b));
        assert_eq!(tree, LayoutTree::Leaf(b));
    }

    #[test]
    fn remove_leaf_promotes_subtree_and_returns_its_leftmost_leaf() {
        let [a, b, c] = pane_ids();
        let mut tree = LayoutTree::Split {
            direction: Direction::Horizontal,
            ratio: 0.5,
            children: Box::new((
                LayoutTree::Leaf(a),
                LayoutTree::Split {
                    direction: Direction::Vertical,
                    ratio: 0.5,
                    children: Box::new((LayoutTree::Leaf(b), LayoutTree::Leaf(c))),
                },
            )),
        };
        assert_eq!(tree.remove_leaf(a), Some(b));
        assert_eq!(
            tree,
            LayoutTree::Split {
                direction: Direction::Vertical,
                ratio: 0.5,
                children: Box::new((LayoutTree::Leaf(b), LayoutTree::Leaf(c))),
            }
        );
    }

    #[test]
    fn remove_leaf_sole_leaf_returns_none() {
        let [a] = pane_ids();
        let mut tree = LayoutTree::Leaf(a);
        assert_eq!(tree.remove_leaf(a), None);
        assert_eq!(tree, LayoutTree::Leaf(a));
    }

    #[test]
    fn remove_leaf_missing_target_is_noop() {
        let [a, b, missing] = pane_ids();
        let mut tree = LayoutTree::Split {
            direction: Direction::Horizontal,
            ratio: 0.5,
            children: Box::new((LayoutTree::Leaf(a), LayoutTree::Leaf(b))),
        };
        let before = tree.clone();
        assert_eq!(tree.remove_leaf(missing), None);
        assert_eq!(tree, before);
    }

    // ── FrameScratch ─────────────────────────────────────────────────────

    #[test]
    fn frame_scratch_clear_retains_capacity() {
        let mut s = FrameScratch::new();
        for _ in 0..100 {
            s.format.graphemes.push(crate::types::Grapheme {
                byte_range: 0..1,
                char_offset: 0,
                col: 0,
                width: 1,
                content: crate::types::CellContent::Empty,
                indent_depth: 0,
            });
        }
        let cap_before = s.format.graphemes.capacity();
        s.clear();
        assert_eq!(s.format.graphemes.len(), 0);
        assert!(s.format.graphemes.capacity() >= cap_before);
    }
}
