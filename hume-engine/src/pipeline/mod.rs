use hume_grid::{Grid, Position, Rect, Rgb};
use hume_rope::line::ContentLine;
use rustc_hash::FxHashMap;
use std::sync::atomic::{AtomicU64, Ordering};

use slotmap::{SecondaryMap, SlotMap, new_key_type};

use crate::pane::Pane;
use crate::providers::{ChromeProvider, DEFAULT_GUTTER_SCOPE, OverlayProvider, SyntaxSpans};
use crate::style::StyleScratch;
use crate::theme::{ScopeRegistry, Theme};
use crate::types::{EditorMode, ResolvedStyle, ScopeId};

mod geometry;
mod layout;
mod pane_pool;
mod pane_render;
#[cfg(test)]
mod tests;

use geometry::DrawnLayout;
pub use geometry::{ChromeRows, ContentWidth, PaneHit, PaneSize, ScreenLayout};
use layout::{
    ARM_E, ARM_N, ARM_S, ARM_W, collect_seam_arms, focused_pane_corners, focused_seam_segment,
    junction_glyph,
};
pub use layout::{
    Cycle, DetachedPane, Direction, FocusedLayout, LayoutTree, Rotation, Seam, Side, SinglePane,
};
pub use pane_pool::{PanePool, UnattachedPane};
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

/// Per-frame scratch storage for the Style and gutter-layout stages.
/// Cleared at the start of each pane render. After a few frames, all `Vec`s
/// have stabilised capacity and no more heap allocations occur.
///
/// The Format stage's buffers are not here. They live on each
/// [`Pane`], one [`crate::format::LineFormat`] per line
/// visited, since sharing them between every walk of a pane is the whole
/// point of that store (see `display_lines::line_store`'s module doc).
pub struct FrameScratch {
    /// Buffers for the Style stage (Stage 3).
    pub style: StyleScratch,
    /// Pre-computed gutter lane widths used by the render stage.
    pub lane_widths: Vec<u16>,
}

impl FrameScratch {
    pub fn new() -> Self {
        Self {
            style: StyleScratch::new(),
            lane_widths: Vec::new(),
        }
    }

    /// Reset all buffers to empty, retaining allocated capacity.
    pub fn clear(&mut self) {
        self.style.clear();
        self.lane_widths.clear();
    }
}

impl Default for FrameScratch {
    fn default() -> Self {
        Self::new()
    }
}

/// What one [`EngineView::render`] call draws from, built by the caller for
/// that frame alone.
pub struct FrameInputs<'a, R, S> {
    /// Resolves a `BufferId` to the authoritative `&Rope` owned by the caller
    /// (typically the editor's `Document`). The borrow is used only inside
    /// the render call; no rope is stored in `EngineView`.
    pub get_rope: R,
    /// Resolves a `BufferId` to its syntax highlight span source, if any.
    /// Same per-frame-borrow contract as `get_rope`.
    pub get_syntax: S,
    pub statusline: &'a dyn ChromeProvider,
    /// Where the chrome and the panes go this frame. Its terminal area is
    /// the grid's size.
    pub layout: ScreenLayout,
}

// ---------------------------------------------------------------------------
// Render context: all per-frame scratch in one place
// ---------------------------------------------------------------------------

/// All scratch buffers needed for one render pass.
///
/// Create once with `RenderContext::new()` and pass `&mut ctx` to
/// `EngineView::render()` and `cursor::content_pos()` each frame; its buffers
/// stabilise their capacity within a few frames.
pub struct RenderContext {
    /// Engine pipeline scratch (style, gutter lane widths).
    pub(crate) frame: FrameScratch,
    /// Per-pane render settings for the frame about to be drawn, filled by
    /// the caller through [`RenderContext::set_pane_settings`].
    ///
    /// Resolved ahead of [`EngineView::render`] rather than through a closure
    /// like its `get_rope`/`get_syntax`: resolving a pane's settings reads the
    /// pane itself, and the pane loop holds `&mut self.panes`, so a closure
    /// doing that lookup could not run while the loop is borrowing. It lives
    /// here rather than arriving as a parameter so its map is reused across
    /// frames instead of reallocated per frame, the same reason `pane_rects`
    /// and `seams` are here.
    pub(crate) pane_settings: SecondaryMap<PaneId, PaneRenderSettings>,
    /// Pane rects computed by the layout stage.
    pub(crate) pane_rects: Vec<(PaneId, Rect)>,
    /// Seam dividers computed alongside `pane_rects` each render: reused
    /// scratch storage, same rationale as `pane_rects`.
    pub(crate) seams: Vec<Seam>,
    /// Perpendicular-arm bits keyed by cell, computed from `seams` each
    /// render so junction glyphs (`┬ ┴ ├ ┤ ┼`) can be drawn where seams
    /// cross. Reused scratch storage, same rationale as `seams`.
    pub(crate) seam_arms: FxHashMap<(u16, u16), u8>,
    /// Where the focused pane's cursor landed within the pane's content area
    /// (pane-relative, *before* the gutter and pane origin are added, not a
    /// terminal-absolute screen cell), resolved by the scroll step that
    /// already had the display-line map open. `None` until that step runs, and reset
    /// every frame: a `RenderContext` outlives the frame that filled it, so
    /// a leftover value must never read as the current one. Callers that need
    /// the real screen cell add the gutter width and the pane rect's origin
    /// (see `lifecycle.rs`/`overlay_sync.rs` in `hume-editor`).
    pub cursor_content_pos: Option<(u16, u16)>,
}

impl RenderContext {
    pub fn new() -> Self {
        Self {
            frame: FrameScratch::new(),
            pane_settings: SecondaryMap::new(),
            pane_rects: Vec::new(),
            seams: Vec::new(),
            seam_arms: FxHashMap::default(),
            cursor_content_pos: None,
        }
    }

    /// Replace the settings [`EngineView::render`] will draw this frame's
    /// panes with. Every pane the next `render` call reaches must be in
    /// `settings`, which is why this replaces wholesale rather than merging:
    /// a pane left over from the last frame would otherwise be drawn with
    /// stale settings instead of failing.
    pub fn set_pane_settings(
        &mut self,
        settings: impl IntoIterator<Item = (PaneId, PaneRenderSettings)>,
    ) {
        self.pane_settings.clear();
        self.pane_settings.extend(settings);
    }
}

impl Default for RenderContext {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// Editor view: top-level owner
// ---------------------------------------------------------------------------

/// Permission to change which pane is focused, or which buffer a pane
/// shows. [`EngineView::new`] mints one per view, and every `EngineView`
/// method that makes either change takes it and panics on a key minted by
/// another view. The editor keeps its view's key beside the steps such a
/// change must run, so the change cannot skip them.
#[derive(Debug)]
pub struct FocusKey(u64);

static NEXT_FOCUS_KEY: AtomicU64 = AtomicU64::new(0);

/// How many bottom chrome bands the view stacks above the statusline. Every
/// per-band list is an array of this length, so a painter or row count
/// without its band does not compile.
pub const BOTTOM_BANDS: usize = 2;

/// The painters of the chrome around the panes, other than the statusline
/// (which borrows the editor per frame, see [`FrameInputs::statusline`]).
pub struct Chrome {
    /// Painted into the rows at the top of the terminal area.
    pub tabbar: Box<dyn ChromeProvider>,
    /// The bottom bands (the pick-list drawer, a docked popup), stacked
    /// upward from the statusline in array order. Several can be open at
    /// once.
    pub bottom_bands: [Box<dyn ChromeProvider>; BOTTOM_BANDS],
}

/// The root of the editor's rendering state.
pub struct EngineView {
    /// The active tab's layout and focused pane, private so every
    /// whole-layout replacement routes through [`Self::replace_layout`],
    /// which is `#[must_use]`: an outgoing layout dropped here unnoticed
    /// would leak every pane it still reaches, since a pooled pane no tree
    /// names is never freed.
    layout: FocusedLayout,
    /// The id of the [`FocusKey`] this view accepts.
    focus_key: u64,
    pub panes: PanePool,
    /// Pure `BufferId` allocator: a buffer's content, syntax, and rope all
    /// live in the editor's `Document`/`Buffer`; this slotmap only mints and
    /// validates IDs so `PaneId -> BufferId` references stay checkable.
    pub buffers: SlotMap<BufferId, ()>,
    pub theme: Theme,
    /// Session-wide scope registry. Providers intern their scopes here.
    /// See [`Theme::bake_if_stale`] for when it gets baked.
    pub registry: ScopeRegistry,
    /// `DEFAULT_GUTTER_SCOPE` interned once at construction, carried on
    /// `PaneRenderCtx`/`ComposeCtx` so gutter composition never falls back to
    /// a by-name lookup on the per-cell hot path.
    default_gutter_scope: ScopeId,
    /// `None` until [`Self::set_chrome`], when no chrome but the statusline
    /// can take rows.
    chrome: Option<Chrome>,
    /// Floating overlays (popups, menus, the picker) painted once over the
    /// whole pane area after every pane and seam. List order is z-order: the
    /// last one paints on top.
    pub overlays: Vec<Box<dyn OverlayProvider>>,
    /// The layout the last [`Self::render`] drew, written only there.
    drawn: DrawnLayout,
    /// The terminal's whole area, written only by [`Self::resize`]: the one
    /// stored input every layout is derived from when read
    /// ([`Self::screen_layout`]).
    last_terminal_area: Rect,
}

/// Blend fraction used to dim non-focused panes toward `ui.background`: 0.0
/// leaves colors untouched, 1.0 flattens them entirely to the background.
const PANE_DIM_FACTOR: f32 = 0.5;

impl EngineView {
    /// A view of a `terminal`-sized screen with no panes yet, and the one
    /// [`FocusKey`] for it.
    pub fn new(theme: Theme, terminal: Rect) -> (Self, FocusKey) {
        let panes = PanePool::with_key();
        let buffers = SlotMap::with_key();
        let mut registry = ScopeRegistry::new();
        let default_gutter_scope = registry.intern(DEFAULT_GUTTER_SCOPE.0);
        let focus_key = NEXT_FOCUS_KEY.fetch_add(1, Ordering::Relaxed);
        let view = Self {
            // Replaced before the first render.
            layout: FocusedLayout::placeholder(),
            focus_key,
            panes,
            buffers,
            theme,
            registry,
            default_gutter_scope,
            chrome: None,
            overlays: Vec::new(),
            drawn: DrawnLayout::default(),
            last_terminal_area: terminal,
        };
        (view, FocusKey(focus_key))
    }

    fn check_key(&self, key: &FocusKey) {
        assert_eq!(key.0, self.focus_key, "FocusKey from another EngineView");
    }

    /// The pane one step from the focused one in layout order, wrapping at
    /// either end; `None` when the focused pane is the only one.
    pub fn pane_cycled_from_focused(&self, cycle: Cycle) -> Option<PaneId> {
        self.layout.pane_cycled_from_focused(cycle)
    }

    /// Install the chrome's painters.
    pub fn set_chrome(&mut self, chrome: Chrome) {
        self.chrome = Some(chrome);
    }

    /// The raw terminal area `render` draws into, zero until the first
    /// [`Self::resize`].
    pub fn terminal_area(&self) -> Rect {
        self.last_terminal_area
    }

    /// Record the terminal's area.
    pub fn resize(&mut self, terminal: Rect) {
        self.last_terminal_area = terminal;
    }

    /// The layout of [`Self::terminal_area`] with the chrome taking `rows`,
    /// which the caller computes from the models the chrome shows. Its one
    /// caller is the editor's `EditorState::screen_layout`; `clippy.toml`
    /// bans every other.
    pub fn screen_layout(&self, rows: ChromeRows, reserve_seam: bool) -> ScreenLayout {
        assert!(
            self.chrome.is_some() || rows == ChromeRows::default(),
            "chrome rows without chrome painters"
        );
        ScreenLayout::new(self.last_terminal_area, rows, reserve_seam)
    }

    /// The active tab's tree.
    pub fn layout(&self) -> &LayoutTree {
        self.layout.tree()
    }

    /// The active tab's layout, for sizing its panes through a
    /// [`ScreenLayout`].
    pub fn focused_layout(&self) -> &FocusedLayout {
        &self.layout
    }

    /// The tab bar's rect as the last [`Self::render`] drew it, for its hit
    /// test.
    pub fn drawn_tabbar_area(&self) -> Rect {
        self.drawn.tabbar_area()
    }

    /// The pane the last [`Self::render`] drew at `pos`, `None` on a miss or
    /// when that pane has left the active tab since.
    pub fn pane_hit(&self, pos: Position) -> Option<PaneHit> {
        self.drawn
            .pane_at(pos)
            .filter(|hit| self.layout.tree().contains_leaf(hit.pane()))
    }

    /// The active tab's focused pane, always a leaf of [`Self::layout`].
    pub fn focused_pane(&self) -> PaneId {
        self.layout.focused()
    }

    /// Replace the whole active-tab layout, returning what was there before.
    /// `#[must_use]`: the outgoing layout may still reach live panes, which
    /// would leak once no tree names them. A caller discarding one with no
    /// real pane in it says so explicitly with `let _ = `.
    ///
    /// The incoming panes' line stores are rewound: they may date from an
    /// older frame (see [`Self::begin_frame`]).
    #[must_use]
    pub fn replace_layout(&mut self, key: &FocusKey, layout: FocusedLayout) -> FocusedLayout {
        self.check_key(key);
        let outgoing = std::mem::replace(&mut self.layout, layout);
        self.begin_frame();
        outgoing
    }

    /// Make pooled pane `pid`, active or in a background tab, show `target`
    /// (see `Pane::show_buffer`).
    pub fn show_buffer(
        &mut self,
        key: &FocusKey,
        pid: PaneId,
        target: BufferId,
        last_content_line: ContentLine,
    ) {
        self.check_key(key);
        self.panes[pid].show_buffer(target, last_content_line);
    }

    /// Focus `pid`, which must be a leaf of the active tab's tree.
    pub fn focus(&mut self, key: &FocusKey, pid: PaneId) {
        self.check_key(key);
        self.layout.focus(pid);
    }

    /// Splice a freshly inserted, not-yet-attached pane into the active
    /// tab's tree beside the focused pane, which keeps focus.
    pub fn split_focused(&mut self, pane: UnattachedPane, direction: Direction) -> PaneId {
        self.layout.split_focused(pane, direction)
    }

    /// Prune the focused pane from the active tab's tree. Focus moves to the
    /// leftmost leaf of the sibling that takes its place.
    pub fn close_focused(&mut self, key: &FocusKey) -> Result<DetachedPane, SinglePane> {
        self.check_key(key);
        self.layout.close_focused()
    }

    /// The refusal [`Self::close_focused`] would give, without closing
    /// anything: for a caller whose own checks come after this one.
    pub fn refuse_close_focused(&self) -> Result<(), SinglePane> {
        self.layout.refuse_single_pane()
    }

    /// Rotate the row or column the focused pane sits in by `steps` places.
    /// Focus stays on the same pane.
    pub fn rotate_focused(&mut self, rotation: Rotation, steps: usize) -> Result<(), SinglePane> {
        self.layout.rotate_focused(rotation, steps)
    }

    /// Insert a pane into the pool, returning the [`UnattachedPane`] token
    /// proving it isn't yet reachable from any layout tree. Splice it in
    /// with [`Self::split_focused`] or [`FocusedLayout::new`].
    pub fn insert_pane(&mut self, pane: Pane) -> UnattachedPane {
        self.panes.insert(pane)
    }

    /// Remove a pane the caller has already detached from every layout tree
    /// that could reach it (see [`DetachedPane`]'s own doc).
    pub fn remove_pane(&mut self, detached: DetachedPane) {
        self.panes.remove(detached);
    }

    /// Every pane `self.layout` (the active tab's) currently reaches. The
    /// working set every per-frame sync step operates on, in place of the
    /// full `panes` pool: an inactive tab's pane is excluded,
    /// since nothing sizes, decorates, mirrors, or scrolls it while its tab
    /// isn't active, so its state reflects whatever its own tab last left it
    /// at rather than this frame's. Decorating, mirroring, or scrolling it
    /// against this frame's state would be wrong the moment the terminal has
    /// since been resized. It re-enters this set, and catches up, the moment
    /// its tab is switched back to.
    pub fn active_pane_ids(&self) -> Vec<PaneId> {
        self.layout.tree().leaves()
    }

    /// Drop the line-store entries of every active pane (see
    /// [`Self::active_pane_ids`]), keeping the allocations behind them.
    ///
    /// Once per frame, and a correctness requirement rather than hygiene: see
    /// `display_lines::line_store`'s module doc. A pane created since the last frame
    /// starts empty, which is what a rewind would produce anyway, so there is
    /// no ordering hazard against `:split`.
    pub fn begin_frame(&mut self) {
        for pid in self.active_pane_ids() {
            self.panes[pid].line_store.rewind();
        }
    }

    /// Render all panes and chrome into `grid`, as laid out by
    /// [`FrameInputs::layout`], whose terminal area `grid` must match in
    /// size, and record that layout for [`Self::pane_hit`] and
    /// [`Self::drawn_tabbar_area`].
    ///
    /// `ctx`'s [`pane_settings`](RenderContext::set_pane_settings) must carry
    /// an entry for every pane this call will draw.
    pub fn render<'rope, R, S>(
        &mut self,
        grid: &mut Grid,
        input: FrameInputs<'_, R, S>,
        ctx: &mut RenderContext,
    ) where
        R: Fn(BufferId) -> Option<&'rope ropey::Rope>,
        S: Fn(BufferId) -> Option<&'rope dyn SyntaxSpans>,
    {
        let FrameInputs {
            get_rope,
            get_syntax,
            statusline,
            layout,
        } = input;
        let area = layout.terminal_area();
        assert_eq!(
            grid.size(),
            (area.width, area.height),
            "grid sized for another terminal area"
        );
        self.drawn.begin(&layout);
        let pane_area = layout.pane_area();
        let draw_dividers = layout.reserves_seam();
        let focused_pane_id = self.layout.focused();
        let scratch = &mut ctx.frame;
        let pane_settings = &ctx.pane_settings;
        let pane_rects = &mut ctx.pane_rects;

        // A degenerate area (e.g. a terminal reporting height 0 during early
        // startup, or a tiny window) has no row to draw a chrome
        // line into. Providers write text via `write_text_run`, which
        // bounds-checks against the grid rather than panicking, but a chrome
        // row drawn at an out-of-bounds `y` is still
        // silently lost, so skip both chrome rows entirely rather than
        // handing them a `Rect` claiming a row that doesn't exist.
        if area.height > 0 {
            // One canvas for every chrome row. Chrome is never dimmed, so
            // they all share the same `dim: None`; a canvas per provider
            // would only re-derive that.
            let mut canvas = crate::render::Canvas::new(grid, self.theme.ui.invisible, None);

            if let Some(chrome) = &self.chrome {
                // ── Render tab bar ────────────────────────────────────────────
                let tabbar_area = layout.tabbar_area();
                if tabbar_area.height > 0 {
                    chrome.tabbar.render(tabbar_area, &self.theme, &mut canvas);
                }

                // ── Render bottom bands ───────────────────────────────────────
                // Stacked directly above the statusline row, in array order,
                // derived from `area` directly (not `pane_area`), matching
                // the tab bar/statusline's own convention: chrome claims its
                // band even when the terminal is too small to also fit pane
                // content (the pane area's degenerate branch already
                // collapses `height` to 0 there).
                let mut bottom_edge = layout.statusline_area().y;
                for (painter, &rows) in chrome.bottom_bands.iter().zip(&layout.chrome_rows().bands)
                {
                    if rows == 0 {
                        continue;
                    }
                    let band_y = bottom_edge.saturating_sub(rows);
                    let available = bottom_edge.saturating_sub(area.y);
                    let band_area = Rect {
                        y: band_y.max(area.y),
                        height: rows.min(available),
                        ..area
                    };
                    painter.render(band_area, &self.theme, &mut canvas);
                    bottom_edge = band_y.max(area.y);
                }
            }

            // ── Render statusline ───────────────────────────────────────────────
            statusline.render(layout.statusline_area(), &self.theme, &mut canvas);
        }

        // ── Compute pane rects once; reuse for panes and overlays ─────────────
        pane_rects.clear();
        self.layout
            .tree()
            .collect_rects_into(pane_area, draw_dividers, pane_rects);

        // ── Render panes ──────────────────────────────────────────────────────
        for (pane_id, rect) in pane_rects.iter().copied() {
            let Some(pane) = self.panes.get_mut(pane_id) else {
                continue;
            };
            let buffer_id = pane.buffer_id;
            if self.buffers.get(buffer_id).is_none() {
                continue;
            }
            // Resolve the rope from the caller: zero-copy, no clone needed.
            let Some(rope) = get_rope(buffer_id) else {
                continue;
            };
            // Unlike the three skips above (a pane, buffer or rope that
            // may be gone by the time the frame draws), a missing
            // entry here is the caller having resolved settings for a
            // different set of panes than it is now asking to draw. Drawing
            // the pane with stale settings, or skipping it and leaving its
            // rect blank, would both hide that.
            let settings = pane_settings
                .get(pane_id)
                .expect("no render settings for a live pane, see RenderContext::set_pane_settings");

            scratch.clear();

            // The one place that can say these are disjoint parts of one pane:
            // the render pass reads three of its fields while writing two:
            // `line_store`, and `viewport`'s own resolved top (`Viewport::top_at`).
            let Pane {
                viewport,
                providers,
                selections,
                line_store,
                ..
            } = pane;

            let mut pane_ctx = PaneRenderCtx {
                viewport,
                providers,
                selections: selections
                    .as_ref()
                    .expect("the host writes the selection mirror before a pane's first frame"),
                rope,
                syntax: get_syntax(buffer_id),
                theme: &self.theme,
                rect,
                settings,
                // Dim non-focused panes so the active one reads clearly at a
                // glance, independent of `draw_dividers`, which only controls
                // the seam glyph. Skipped when `ui.background` has no explicit
                // bg: there is no defined blend target for custom themes that
                // leave it unset.
                dim: (pane_id != focused_pane_id)
                    .then_some(self.theme.ui.background.bg)
                    .flatten()
                    .map(|bg| (bg, PANE_DIM_FACTOR)),
                default_gutter_scope: self.default_gutter_scope,
            };
            render_pane(&mut pane_ctx, scratch, line_store, grid);
            self.drawn.record_pane(pane_id, rect);
        }

        // ── Render seam dividers between panes ────────────────────────────────
        if draw_dividers {
            let focused_rect = pane_rects
                .iter()
                .find(|(pid, _)| *pid == focused_pane_id)
                .map(|(_, r)| *r);
            ctx.seams.clear();
            self.layout
                .tree()
                .collect_seams_into(pane_area, &mut ctx.seams);
            ctx.seam_arms.clear();
            collect_seam_arms(&ctx.seams, &mut ctx.seam_arms);

            // `theme.default` (ui.text's color) is the base layer here for the
            // same reason it's the base for a content display line
            // (`style_display_line`'s `base_style`) and a virtual display
            // line (`base_scope` in `pane_render.rs`): a scope that leaves
            // `fg` unset should fall
            // back to the theme's own base text color everywhere, not just on
            // some rendered surfaces: the seam is chrome HUME draws itself,
            // it isn't Helix's own border (which leaves an unset fg as
            // whatever the terminal already shows), so there's no reason for
            // it to be the one exception.
            //
            // Only `ui.background`'s *background* is layered, never its whole
            // style, the same constraint `style_display_line`'s Tier 4 and
            // `render.rs`'s `row_bg` apply. A background scope has no business
            // contributing a foreground, and letting it would give a theme
            // that sets `ui.background = { fg, bg }` a seam glyph in that fg
            // and nowhere else, contradicting the base-layer rule above.
            let seam_base = self.theme.default.layer(ResolvedStyle {
                bg: self.theme.ui.background.bg,
                ..ResolvedStyle::default()
            });
            let muted = seam_base.layer(self.theme.ui.window);
            let accent = seam_base.layer(self.theme.ui.window_focused);

            // Junction cells at the focused pane's corners are missed by the
            // per-seam accent test (see `focused_pane_corners`); precompute
            // them once so the per-cell loop only does a membership check.
            let corners = focused_rect.map(focused_pane_corners);

            // The grid's size, read once ahead of `canvas` below. `Grid` is
            // origin-free, so this `(u16, u16)` is all a per-seam clamp
            // needs, and it's `Copy` rather than a borrow, so it can be read
            // before the canvas takes its exclusive `&mut Grid` for the whole
            // seam pass.
            let (grid_w, grid_h) = grid.size();
            let mut canvas = crate::render::Canvas::new(grid, self.theme.ui.invisible, None);
            for seam in &ctx.seams {
                let (x0, y0, x1, y1) =
                    crate::render::clamp_rect_to_grid((grid_w, grid_h), seam.rect);
                let base = match seam.direction {
                    Direction::Horizontal => ARM_N | ARM_S,
                    Direction::Vertical => ARM_E | ARM_W,
                };
                // The slice of this seam adjacent to the focused pane, drawn
                // in the accent color. Computed once per seam so the per-cell
                // loop below only needs a bounds check, not a repeated call.
                let accent_rect = focused_rect.and_then(|fr| focused_seam_segment(seam.rect, fr));

                for y in y0..y1 {
                    for x in x0..x1 {
                        let arms = ctx.seam_arms.get(&(x, y)).copied().unwrap_or(0);
                        let glyph = junction_glyph(base | arms);
                        let in_accent = accent_rect
                            .is_some_and(|r| r.contains(Position::new(x, y)))
                            || corners.is_some_and(|cs| cs.contains(&Some((x, y))));
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
        // One canvas for all of them, for the same reason chrome shares one:
        // an overlay is never dimmed either.
        let mut canvas = crate::render::Canvas::new(grid, self.theme.ui.invisible, None);
        for overlay in &self.overlays {
            if overlay.is_active() {
                overlay.render(&self.theme, &mut canvas);
            }
        }
    }
}

/// Per-pane render settings supplied by the editor at render time.
///
/// `format` bundles everything the render pass's `DisplayLineMap` resolves a line's
/// layout from (wrap mode, tab width, whitespace config, buffer identity)
/// as one [`FormatKey`](crate::display_lines::line_store::FormatKey), unresolved (the
/// pane's own `content_width` resolves it, inside `DisplayLineMap::new`). `tab_width`
/// and `whitespace` inside it are document facts, resolved from per-buffer
/// overrides against global settings, identical for every pane viewing the
/// same buffer, while `wrap_mode` is per-pane (two panes on the
/// same buffer may wrap differently, once `:wrap`/`:set pane wrap-mode=…`
/// pins one); the editor resolves pane override → buffer override → global
/// default (see `commands::effective_wrap_mode`) and folds the result into
/// the same key alongside the document facts, since the render pass's
/// `DisplayLineMap` and the scroll pass's must resolve a bit-identical key to share
/// this pane's line store; see `FormatKey`'s own doc. `mode` and
/// `cursor_is_block` are both per-focus facts rather than document facts;
/// see `Editor::resolve_pane_settings` (`hume-editor`) for exactly what each
/// resolves to and why. `mode` picks which cursor-scope ladder applies;
/// `cursor_is_block` gates both selection heads in `style::style_display_line`'s
/// Tier 1/0.
#[derive(Copy, Clone)]
pub struct PaneRenderSettings {
    pub mode: EditorMode,
    pub format: crate::display_lines::line_store::FormatKey,
    pub indent_guides: bool,
    pub cursor_is_block: bool,
    pub gutter: crate::providers::GutterInputs,
}

/// Transient bundle of borrows needed to render one pane. Avoids passing a
/// dozen separate parameters through the call stack.
pub(crate) struct PaneRenderCtx<'a> {
    /// The pane's own fields rather than `&Pane`, so the render loop can hand
    /// `render_pane` a `&mut` on that same pane's line store alongside this:
    /// one `&mut Pane` split into disjoint field borrows, which only the loop
    /// that owns the pane can say is sound.
    ///
    /// `&'a mut`, not `&'a`: the display-line walk resolves `viewport.top()`
    /// against its own `DisplayLineMap` via `Viewport::top_at` before
    /// walking from it, and writes the resolved address back. See that
    /// method's doc for why a stale top must never reach the walk.
    pub viewport: &'a mut crate::pane::Viewport,
    pub providers: &'a crate::providers::ProviderSet,
    pub selections: &'a crate::types::PaintedSelections,
    /// Rope borrowed from the caller's `Document` for this frame only.
    pub rope: &'a ropey::Rope,
    /// Syntax highlight span source borrowed from the caller via
    /// `get_syntax`, if a language with a grammar is configured.
    pub syntax: Option<&'a dyn SyntaxSpans>,
    pub theme: &'a Theme,
    pub rect: Rect,
    /// Borrowed from the caller's `RenderContext`, which owns the map these
    /// come out of for the whole frame.
    pub settings: &'a PaneRenderSettings,
    /// `Some` for non-focused panes: blend every written cell's fg/bg toward
    /// this target by `factor`. `None` for the focused pane.
    pub dim: Option<(Rgb, f32)>,
    /// See `EngineView::default_gutter_scope`.
    pub default_gutter_scope: ScopeId,
}
