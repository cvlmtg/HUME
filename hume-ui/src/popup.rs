//! Cursor-anchored popup widget (`show-popup!`): a floating text panel used
//! by hover, signature help, and (as a menu) the selection menu, completion
//! menu and minibuffer `:` completion.
//!
//! Geometry rules:
//! - Preferred placement: below-right of the anchor cell. Flip above when the
//!   space below is smaller than the content and the space above is larger.
//! - Clamp horizontally to the pane's right edge.
//! - Max width `min(60, pane_width - 4)`, max height a third of the pane.
//!   Taller content is the caller's problem (hover overflows to the drawer).
//! - 1-cell frame themed `ui.popup` (`ui.menu` for menus), box-drawn when
//!   `popup-border` is on. Rendering lives in [`super::menu_box`].
//!
//! [`resolve_popup`]/[`resolve_menu`]/[`resolve_band`] resolve geometry and
//! the scroll window every frame, against the focused pane's current rect, so
//! a resize or scroll never leaves them stale. For a menu, [`menu_window`]
//! picks the window from counts alone and the caller materializes only those
//! rows, so width is measured over visible rows only. `PopupOverlay` and
//! `PopupBandWidget::render` paint the resolved result, clipped defensively to
//! `pane_rect`.

use hume_engine::types::ResolvedStyle;
use hume_grid::Rect;
use std::sync::Arc;

use hume_engine::lock::SharedSlot;

use hume_engine::providers::{BottomBandProvider, OverlayProvider};
use hume_engine::render::Canvas;
use hume_engine::theme::Theme;
use hume_engine::theme::ui_scopes;

use super::menu_box::{MenuBoxStyles, draw_menu_box};
use super::width::cell_width;

/// Maximum popup width in terminal columns, before any pane-width clamp.
/// Popup-only: a menu doesn't wrap, so it has no analogous cap here (it
/// uses [`super::menu_box::MAX_MENU_ROWS`], a row cap, instead).
const MAX_POPUP_WIDTH: u16 = 60;

/// Where a popup renders: same widget, same model, two placements.
/// `show-popup!`'s `#:anchor` kwarg selects between them (`hume-editor`'s
/// `PopupLayer::layout`).
pub enum PopupLayout {
    /// Floating, anchored near the focused pane's cursor (`#:anchor
    /// 'cursor`, the default), painted by `PopupOverlay`.
    Cursor,
    /// Docked as a full-width chrome band directly above the statusline,
    /// reserving pane space like the drawer (`#:anchor 'bottom`), painted
    /// by `PopupBandWidget`. Used for hover content too tall for the
    /// cursor layout; keeps popup semantics (plain scroll, no selection,
    /// close-on-any-other-key) rather than becoming a pick-list.
    Docked,
}

/// Popup text, wrapped on demand and memoized by width: a pure function of
/// `(source, width)`. Source is fixed at construction (a `show-popup!`
/// builds a fresh `PopupContent`; text/highlights never change during a
/// popup's lifetime), so `width` is the only invalidation key the cache
/// needs; a `:theme` switch does not need to invalidate it either: a
/// `Scrollable` popup closes on any non-scroll key (`Editor::popup_input`)
/// and a `Sticky` one dies with its mode layer as soon as Insert ends
/// (`EditorState::push_mode_layer`'s `clear_popups()` call) before
/// Command-mode input like `:theme` can run, so no popup survives to see a
/// stale highlight.
///
/// [`Self::plain`]/[`Self::styled`] both resolve through the same
/// `wrap_styled` call internally. There is one wrap algorithm, not two;
/// `plain` is exactly a single default-style run. `styled` takes
/// pre-resolved `(text, style)` runs rather than a grammar name: this crate
/// has no tree-sitter dependency of its own, so `hume-editor` (which does,
/// via `MarkupSyntax`) resolves spans against its own baked `Theme` before
/// handing them over: `Theme::resolve` debug-asserts on an unbaked
/// `ScopeId`, and this crate has no `ScopeRegistry` access to bake one.
pub struct PopupContent {
    source: Vec<(String, ResolvedStyle)>,
    /// `false` for plain text: [`resolve_popup`]/[`resolve_band`] then leave
    /// `PopupState::styled_rows`/`PopupBandState::styled_rows` `None`, so the
    /// box paints every row in one style instead of walking per-run.
    styled: bool,
    cache: Option<Wrapped>,
}

struct Wrapped {
    width: u16,
    lines: Arc<Vec<String>>,
    /// Widest line's display width, measured once per wrap, not
    /// re-measured every frame an unchanged wrap is reused, via
    /// [`super::menu_box::widest`].
    inner_width: u16,
    styled_rows: Option<Arc<Vec<StyledRow>>>,
}

impl PopupContent {
    pub fn plain(text: &str) -> Self {
        Self {
            source: vec![(text.to_string(), ResolvedStyle::default())],
            styled: false,
            cache: None,
        }
    }

    /// `runs`: pre-highlighted `(text, style)` spans in source order. See
    /// this type's own doc for why the caller resolves these rather than
    /// handing over a grammar name.
    pub fn styled(runs: Vec<(String, ResolvedStyle)>) -> Self {
        Self {
            source: runs,
            styled: true,
            cache: None,
        }
    }

    /// Wrapped lines + (for [`Self::styled`] content) per-run styles at
    /// `width`, recomputing only when `width` differs from the cached one:
    /// an unchanged width across frames is O(1), not a re-wrap. `lines`/
    /// `styled_rows` are `Arc`-wrapped here so the *cache* itself is a
    /// pointer, not a copy, across frames; [`resolve_popup`]/[`resolve_band`]
    /// hand that same `Arc` straight to [`PopupState`]/[`PopupBandState`]
    /// (an `Arc` clone, never a copy, regardless of scroll position) along
    /// with a `visible` range into it. The window is a view, not a slice
    /// taken up front.
    fn wrapped(&mut self, width: u16) -> &Wrapped {
        let stale = self.cache.as_ref().is_none_or(|c| c.width != width);
        if stale {
            let rows = wrap_styled(&self.source, width);
            let lines: Vec<String> = rows
                .iter()
                .map(|row| row.iter().map(|(s, _)| s.as_str()).collect())
                .collect();
            let styled_rows = self.styled.then(|| Arc::new(rows));
            let inner_width = super::menu_box::widest(lines.iter().map(String::as_str));
            self.cache = Some(Wrapped {
                width,
                lines: Arc::new(lines),
                inner_width,
                styled_rows,
            });
        }
        self.cache.as_ref().expect("populated above")
    }
}

/// Fully-resolved popup/menu content and position, computed once per frame
/// by the write side; the overlay only paints.
pub struct PopupState {
    /// For a popup (`resolve_popup`): the *full* wrapped row list,
    /// `Arc`-shared with the source [`PopupContent`]'s cache. `visible`
    /// carries the window into it, so a scrolled popup taller than its box
    /// still costs a refcount bump per frame, not a fresh `Vec` copy of
    /// whatever's on screen. For a menu (`resolve_menu`): already just the
    /// visible window, composed fresh every frame from the only rows the
    /// resolver was ever handed; `visible` is then `0..lines.len()`, a
    /// no-op slice.
    pub lines: Arc<Vec<String>>,
    /// The window into `lines` that's actually on screen: `draw_menu_box`
    /// paints `&lines[visible]`, never `lines` whole. Same length as `rect`'s
    /// inner height (or less, for the last partial window at the end of a
    /// short list).
    pub visible: std::ops::Range<usize>,
    /// Outer footprint (including the 1-cell frame), top-left corner already
    /// flipped/clamped by the write side. Each caller's row cap differs
    /// (hover: ⅓ pane height; menus/LSP completion: `MAX_MENU_ROWS` with a
    /// scroll window). Carrying the resolved size here, rather than
    /// re-deriving it from `lines.len()`
    /// at render time, keeps the painted box and the positioned box the
    /// same box.
    pub rect: Rect,
    /// The highlighted row index, for menus: window-relative (an index
    /// into `visible`, not into the full filtered/ranked list). `None` for a
    /// plain popup.
    pub selected: Option<usize>,
    /// The full, unwindowed row count `lines`/`visible` were sliced from;
    /// together with `scroll`, what the scrollbar thumb sizes and places
    /// itself against; for a menu, `lines.len()` alone can't stand in for it
    /// since `lines` is only ever the visible window there.
    pub total_rows: usize,
    /// The visible window's own start within the full list: equal to
    /// `visible.start` for a popup, but a distinct value for a menu, whose
    /// `visible` indexes into the small per-frame `lines` rather than the
    /// full filtered/ranked list `total_rows` counts. For a plain popup
    /// (`selected.is_none()`), the resolved counterpart of
    /// `PopupLayer::scroll`; for a menu, wherever [`menu_window`] centered
    /// the window around `selected`. Used only by the scrollbar thumb.
    pub scroll: usize,
    /// Whether to draw box-drawing border glyphs around the popup (vs. a
    /// plain background-filled 1-cell margin). Fed from the `popup-border`
    /// setting.
    pub border: bool,
    /// Per-run styled counterpart of `lines`, same length and same text when
    /// flattened. `Some` only for a [`PopupContent::styled`] popup. `None`
    /// for every other popup and for menus, which paint `lines` in one style
    /// regardless. Sliced by `visible` the same way `lines` is.
    pub styled_rows: Option<Arc<Vec<StyledRow>>>,
}

/// Generic overlay that paints a `PopupState` snapshot. Used directly for
/// hover-style popups (`show-popup!`) and, via a second registration with
/// its own `Arc`, for the selection menu and completion menu.
pub(crate) struct PopupOverlay {
    pub(crate) data: SharedSlot<Option<PopupState>>,
    /// Root scope for the background/text fill (`ui.popup` for hover popups,
    /// `ui.menu` for menus); `MenuBoxStyles::resolve` derives the
    /// highlighted-row and scrollbar-thumb styles from it.
    pub(crate) scope: &'static str,
}

impl OverlayProvider for PopupOverlay {
    fn is_active(&self) -> bool {
        self.data.read().is_some()
    }

    fn render(&self, pane_rect: Rect, theme: &Theme, canvas: &mut Canvas) {
        let guard = self.data.read();
        let Some(state) = guard.as_ref() else { return };
        if state.lines.is_empty() {
            return;
        }

        // Defensive clip: the write side computed this rect against this
        // same pane's rect this same frame, so this should never trigger;
        // see `fits_inside`'s doc.
        if !super::menu_box::fits_inside(state.rect, pane_rect) {
            return;
        }
        draw_menu_box(
            canvas,
            state.rect,
            &state.lines[state.visible.clone()],
            state.selected,
            state.total_rows,
            state.scroll,
            state.border,
            MenuBoxStyles::resolve(theme, self.scope),
            state
                .styled_rows
                .as_ref()
                .map(|rows| &rows[state.visible.clone()]),
        );
    }
}

/// Fully-resolved content for a **docked** popup (`PopupLayout::Docked`):
/// the `PopupBandWidget` counterpart of [`PopupState`]. No position/size
/// is stored here: unlike the floating popup, a bottom band's geometry is
/// resolved by the engine at render time from `height(max)` and the chrome
/// area (the same contract the drawer already follows), not pre-computed by
/// the write side.
pub struct PopupBandState {
    /// Word-wrapped to the band's width (the write side, `Editor::
    /// sync_popup_band_view`, wraps against `last_terminal_area`, the same
    /// raw area the engine will render the band into): the *full* wrapped
    /// list, `Arc`-shared with the source `PopupContent`'s cache; `visible`
    /// carries the window into it. See [`PopupState::lines`]'s doc, same
    /// contract (a docked popup has no menu counterpart, so this is always
    /// the shared-cache case).
    pub lines: Arc<Vec<String>>,
    /// See [`PopupState::visible`].
    pub visible: std::ops::Range<usize>,
    /// See [`PopupState::total_rows`].
    pub total_rows: usize,
    pub scroll: usize,
    pub border: bool,
    pub styled_rows: Option<Arc<Vec<StyledRow>>>,
}

/// Engine-facing bottom-band provider for a docked popup. Mirrors
/// [`super::drawer::DrawerWidget`]'s shape (chrome, not per-pane), but paints
/// through `draw_menu_box` so a docked hover keeps the popup's framed,
/// `ui.popup`-scoped look rather than the drawer's plain list rows.
pub(crate) struct PopupBandWidget {
    pub(crate) data: SharedSlot<Option<PopupBandState>>,
}

/// The frame's top/bottom cells, always reserved, even with `popup-border`
/// off (a plain background margin still takes the row, see `draw_menu_box`'s
/// doc on `border`).
const POPUP_FRAME_ROWS: u16 = 2;

/// Rows a docked popup shows at once, given `lines` wrapped lines and the
/// band's row ceiling `max` (35% of the last-rendered *terminal* height,
/// mirroring `PopupBandWidget::height`'s own `max`): the number
/// `Editor::scroll_popup` pages against, agreeing with what the engine will
/// next paint since both derive from
/// `super::menu_box::band_capacity`.
pub fn band_visible_rows(lines: usize, max: u16) -> usize {
    super::menu_box::band_visible_rows(lines, POPUP_FRAME_ROWS, max)
}

impl BottomBandProvider for PopupBandWidget {
    fn height(&self, max: u16) -> u16 {
        let guard = self.data.read();
        guard.as_ref().map_or(0, |s| {
            super::menu_box::band_capacity(s.total_rows, POPUP_FRAME_ROWS, max)
        })
    }

    fn render(&self, area: Rect, theme: &Theme, canvas: &mut Canvas) {
        if area.height == 0 {
            return;
        }
        let guard = self.data.read();
        let Some(state) = guard.as_ref() else { return };
        draw_menu_box(
            canvas,
            area,
            &state.lines[state.visible.clone()],
            None,
            state.total_rows,
            state.scroll,
            state.border,
            MenuBoxStyles::resolve(theme, ui_scopes::POPUP),
            state
                .styled_rows
                .as_ref()
                .map(|rows| &rows[state.visible.clone()]),
        );
    }
}

/// Where a cursor-anchored box sits: the anchor cell (absolute screen
/// coords), the pane it must stay inside, and that pane's text-column
/// budget. Three facts about the cursor, no per-widget size cap here: a
/// popup wraps to ⅓ pane height and `MAX_POPUP_WIDTH`, a menu doesn't wrap
/// at all and uses `MAX_MENU_ROWS` instead, so folding either onto this
/// shared value would leave the other caller discarding it.
#[derive(Clone, Copy)]
pub struct PopupPlacement {
    pub anchor: (u16, u16),
    pub pane_rect: Rect,
    pub content_width: u16,
}

/// Resolve a cursor-anchored popup's content + position for this frame.
/// `scroll` is the model's raw scroll value; the result clamps it to the
/// resolved content height and windows `lines`/`styled_rows` down to it.
pub fn resolve_popup(
    content: &mut PopupContent,
    placement: PopupPlacement,
    scroll: usize,
    border: bool,
) -> PopupState {
    // Reserve 2 cells on each axis for the popup's 1-cell frame, so
    // content and border together fit inside the envelope.
    let max_width = MAX_POPUP_WIDTH
        .min(placement.content_width.saturating_sub(4))
        .saturating_sub(2);
    let max_height = (placement.pane_rect.height / 3)
        .max(1)
        .saturating_sub(2)
        .max(1);

    let wrapped = content.wrapped(max_width);
    let total_rows = wrapped.lines.len();
    let (outer_w, outer_h) =
        super::menu_box::outer_dims_from_width(wrapped.inner_width, total_rows, max_height);
    let (x, y, outer_w, outer_h) =
        resolve_popup_geometry(outer_w, outer_h, placement.anchor, placement.pane_rect);
    let inner_h = outer_h.saturating_sub(2) as usize;
    let scroll = scroll.min(total_rows.saturating_sub(inner_h));
    let visible = super::menu_box::window_range(total_rows, scroll, inner_h);
    PopupState {
        lines: Arc::clone(&wrapped.lines),
        visible,
        rect: Rect::new(x, y, outer_w, outer_h),
        selected: None,
        total_rows,
        scroll,
        styled_rows: wrapped.styled_rows.clone(),
        border,
    }
}

/// Resolve a docked popup's content for this frame: the [`PopupLayout::
/// Docked`] counterpart of [`resolve_popup`]. No anchor or pane rect: unlike
/// the floating popup, a bottom band's position/height is resolved by the
/// engine at render time from `height(max)` and the chrome area (mirroring
/// the drawer's own contract), so this only resolves content. `area_width`
/// is the raw, un-subtracted terminal width (the band spans the full
/// terminal, not just the panes region); `max_rows` bounds how many rows the
/// band may claim (mirrors `PopupBandWidget::height`'s own `max`).
pub fn resolve_band(
    content: &mut PopupContent,
    area_width: u16,
    max_rows: u16,
    scroll: usize,
    border: bool,
) -> PopupBandState {
    let max_width = area_width.saturating_sub(2);
    let wrapped = content.wrapped(max_width);
    let total_rows = wrapped.lines.len();
    let inner_h = band_visible_rows(total_rows, max_rows);
    let scroll = scroll.min(total_rows.saturating_sub(inner_h));
    let visible = super::menu_box::window_range(total_rows, scroll, inner_h);
    PopupBandState {
        lines: Arc::clone(&wrapped.lines),
        visible,
        total_rows,
        scroll,
        styled_rows: wrapped.styled_rows.clone(),
        border,
    }
}

/// One menu row: a primary label, plus an optional second part right-aligned
/// against it (an LSP completion candidate's `detail`, e.g.). See
/// [`resolve_menu`] for how the two are laid out into one painted string.
/// `String`, not `Arc<str>`: `compose_menu_row` always paints into a fresh
/// `String` regardless (`truncate_marked` copies), so an `Arc<str>` here
/// would only ever save a caller's own construction-time clone, and the
/// caller with the most rows to build (`CompletionItem::menu_row`, one call
/// per parsed item) never reaches the visible window most of them are built
/// for; see that method's own doc.
#[derive(Clone)]
pub struct MenuRow {
    pub main: String,
    pub trailing: Option<String>,
}

impl MenuRow {
    /// A single-column row, `trailing` absent.
    pub fn plain(main: String) -> Self {
        Self {
            main,
            trailing: None,
        }
    }
}

/// The blank column separator between a row's `main` and `trailing` parts.
const MENU_COLUMN_GAP: u16 = 2;

/// Which rows of a menu are on screen this frame, resolved by
/// [`menu_window`] from *counts alone*, before any row is materialized. A
/// caller slices its own list to `range`, hands exactly those rows to
/// [`resolve_menu`], and the rest of the list never reaches the resolver at
/// all. Every field but `range` is private and there is no constructor
/// besides `menu_window`, so a window can't be faked or widened by hand.
pub struct MenuWindow {
    /// The `[start, end)` slice of the full ranked list that's visible.
    pub range: std::ops::Range<usize>,
    /// The highlighted row, window-relative; `None` for an empty list.
    selected_rel: Option<usize>,
    /// Outer row count including the 1-cell frame, already clamped to the
    /// pane's height.
    outer_h: u16,
    /// The full list's length `range` was cut from: the scrollbar thumb's
    /// denominator.
    total_rows: usize,
}

/// Phase 1 of resolving a menu (selection menu or completion menu): the
/// visible window, from `total_rows`/`selected`/the pane's height only.
/// Height doesn't depend on width, and the pane clamp on it can be applied
/// directly here: `resolve_popup_geometry` (inside [`resolve_menu`])
/// reclamps it again with the final width, but a value already `<=` the
/// pane's height is unaffected by a second `.min`.
///
/// A session narrowed to zero matches still resolves a (tiny, unpainted:
/// `PopupOverlay`/`PopupBandWidget` both bail on an empty `lines`) box:
/// geometry stays a pure function of `(total_rows, pane)`, with no
/// special-cased early return, so a caller never sees a stale rect from
/// the *previous* nonempty frame linger into this one.
pub fn menu_window(total_rows: usize, selected: usize, pane_rect: Rect) -> MenuWindow {
    let raw_outer_h = super::menu_box::outer_rows(total_rows, super::menu_box::MAX_MENU_ROWS);
    let outer_h = raw_outer_h.min(pane_rect.height);
    let inner_h = outer_h.saturating_sub(2) as usize;
    let (selected_rel, range) = if total_rows == 0 {
        (None, 0..0)
    } else {
        let selected = selected.min(total_rows - 1);
        let range = super::menu_box::window_range(
            total_rows,
            selected.saturating_sub(inner_h / 2),
            inner_h,
        );
        (Some(selected - range.start), range)
    };
    MenuWindow {
        range,
        selected_rel,
        outer_h,
        total_rows,
    }
}

/// Phase 2: a menu's content + position for this frame, from `rows`, which
/// are exactly `window.range`'s rows, nothing more. No wrapping (menu
/// entries are short labels, not prose), so, unlike [`resolve_popup`], there
/// is no fixed width budget: the box is exactly as wide as `rows` need, up
/// to the pane clamp. Because a caller can only ever hand over the window's
/// own rows, one long candidate scrolled out of view *cannot* inflate the
/// box. That guarantee is the reason the resolver is split in two, not a
/// property this function has to remember to uphold. Column widths
/// (`main`/`trailing`) are resolved from `rows` and composed into each row
/// before painting: `draw_menu_box` receives plain, pre-aligned strings,
/// unaware rows ever had two parts.
pub fn resolve_menu(
    rows: &[MenuRow],
    window: MenuWindow,
    placement: PopupPlacement,
    border: bool,
) -> PopupState {
    debug_assert_eq!(
        rows.len(),
        window.range.len(),
        "resolve_menu: `rows` must be exactly the window's own slice"
    );

    let max_inner = placement.pane_rect.width.saturating_sub(2);
    let main_w = super::menu_box::widest(rows.iter().map(|r| r.main.as_ref()));
    let trail_w = super::menu_box::widest(rows.iter().filter_map(|r| r.trailing.as_deref()));
    let main_col = main_w.min(max_inner);
    // `trail_w == 0` (no row has a `trailing` at all) already collapses to
    // `trail_col == 0` through the `.min` below. A real budget-of-zero and
    // an absent trailing column are indistinguishable to this arithmetic,
    // and that's fine: both mean "reserve no trailing column".
    let trail_col = trail_w.min(
        max_inner
            .saturating_sub(main_col)
            .saturating_sub(MENU_COLUMN_GAP),
    );
    let inner_w = main_col
        + if trail_col > 0 {
            MENU_COLUMN_GAP + trail_col
        } else {
            0
        };

    let lines: Vec<String> = rows
        .iter()
        .map(|row| compose_menu_row(row, main_col, trail_col))
        .collect();

    let (x, y, outer_w, outer_h) = resolve_popup_geometry(
        inner_w + 2,
        window.outer_h,
        placement.anchor,
        placement.pane_rect,
    );
    let visible_len = lines.len();
    PopupState {
        lines: Arc::new(lines),
        visible: 0..visible_len,
        rect: Rect::new(x, y, outer_w, outer_h),
        selected: window.selected_rel,
        total_rows: window.total_rows,
        scroll: window.range.start,
        styled_rows: None, // menus never highlight per-span, only per-row
        border,
    }
}

/// Compose one [`MenuRow`] into a single painted string: `main` left-aligned
/// and padded to `main_col`, then (when `trail_col > 0`) [`MENU_COLUMN_GAP`]
/// blank cells and `trailing` right-aligned and padded to `trail_col`. A
/// row with no `trailing` of its own still reserves that column as blank, so
/// every row's second part lines up under the others'. Both parts are
/// truncated (never wrapped) to their column width, since a menu row is
/// exactly one screen line.
fn compose_menu_row(row: &MenuRow, main_col: u16, trail_col: u16) -> String {
    use hume_engine::types::TruncateEnd;

    let main = super::width::truncate_marked(&row.main, main_col as usize, TruncateEnd::Tail);
    if trail_col == 0 {
        return main.into_owned();
    }
    let main_pad = main_col as usize - super::width::text_width(&main);
    let trailing = super::width::truncate_marked(
        row.trailing.as_deref().unwrap_or(""),
        trail_col as usize,
        TruncateEnd::Tail,
    );
    let trail_pad = trail_col as usize - super::width::text_width(&trailing);
    let mut out =
        String::with_capacity(main_col as usize + MENU_COLUMN_GAP as usize + trail_col as usize);
    out.push_str(&main);
    for _ in 0..(main_pad + MENU_COLUMN_GAP as usize + trail_pad) {
        out.push(' ');
    }
    out.push_str(&trailing);
    out
}

/// Resolve the top-left corner and clamped size for a `width` × `height` box
/// (the outer footprint, including any frame) anchored near `anchor`
/// (cursor cell, absolute screen coords) within `pane_rect`. Callers pass
/// their content size plus the 2-cell frame reserved for the border.
///
/// `width`/`height` are clamped to `pane_rect`'s size before the position is
/// resolved, so the returned box always fits inside the pane. Callers must
/// use the returned size, not their original request, when painting.
/// `PopupOverlay`'s bounds check is a defensive backstop, not a substitute
/// for this: without the clamp, a box wider or taller than the pane can
/// never satisfy that check, and the whole popup silently fails to render.
fn resolve_popup_geometry(
    width: u16,
    height: u16,
    anchor: (u16, u16),
    pane_rect: Rect,
) -> (u16, u16, u16, u16) {
    let (width, height) = clamp_size_to_pane(width, height, pane_rect);
    let (anchor_x, anchor_y) = anchor;
    let space_below = pane_rect.bottom().saturating_sub(anchor_y + 1);
    let space_above = anchor_y.saturating_sub(pane_rect.y);

    let y = if space_below >= height || space_below >= space_above {
        anchor_y + 1
    } else {
        anchor_y.saturating_sub(height)
    };
    let y = y
        .max(pane_rect.y)
        .min(pane_rect.bottom().saturating_sub(height));

    let x = clamp_x_to_pane(anchor_x, width, pane_rect);

    (x, y, width, height)
}

/// Clamp `width`×`height` to fit inside `pane_rect`: the size half of a
/// box-in-pane placement.
fn clamp_size_to_pane(width: u16, height: u16, pane_rect: Rect) -> (u16, u16) {
    (width.min(pane_rect.width), height.min(pane_rect.height))
}

/// Clamp `x` so a `width`-wide box starting there never crosses `pane_rect`'s
/// left or right edge: the horizontal-position half of a box-in-pane
/// placement, split out from [`resolve_popup_geometry`] so its vertical
/// counterpart isn't tangled up with this axis.
fn clamp_x_to_pane(x: u16, width: u16, pane_rect: Rect) -> u16 {
    x.max(pane_rect.x)
        .min(pane_rect.right().saturating_sub(width))
}

/// One wrapped popup row's content, as contiguous same-style runs: the
/// styled counterpart of a plain-wrapped row (a `Vec<StyledRun>` instead of
/// a bare `String`).
pub(in crate::popup) type StyledRun = (String, ResolvedStyle);
pub type StyledRow = Vec<StyledRun>;

/// Merge adjacent `(text, style)` pairs sharing the same `ResolvedStyle`:
/// [`coalesce_atoms`]'s own primitive, merging wrapped *output* graphemes
/// back down to runs.
fn push_run(runs: &mut Vec<(String, ResolvedStyle)>, text: &str, style: ResolvedStyle) {
    if text.is_empty() {
        return;
    }
    match runs.last_mut() {
        Some((last_text, last_style)) if *last_style == style => last_text.push_str(text),
        _ => runs.push((text.to_string(), style)),
    }
}

/// Merge adjacent atoms sharing the same `ResolvedStyle` into `StyledRun`s.
fn coalesce_atoms(atoms: Vec<(&str, ResolvedStyle)>) -> StyledRow {
    let mut out: StyledRow = Vec::new();
    for (g, style) in atoms {
        push_run(&mut out, g, style);
    }
    out
}

/// Word-wrap `runs` (contiguous same-style chunks of source text; `\n` acts
/// as a paragraph delimiter, and may appear anywhere inside a run) to
/// `max_width` display columns, breaking on grapheme-cluster boundaries.
/// Unbounded height: [`PopupContent::wrapped`]'s caller windows the result
/// (`scroll`) rather than truncating it here; a `Scrollable` popup needs
/// every row reachable, not just the first screenful.
///
/// Operates on a flat per-grapheme stream, never on `runs`' original chunk
/// boundaries. A style change (e.g. a `**bold**` span) can land anywhere,
/// including mid-word, so wrapping must not coarsen past grapheme
/// granularity. Plain popup text is exactly a single default-style run, so
/// there is one wrap algorithm here, not a separate one for plain text.
fn wrap_styled(runs: &[(String, ResolvedStyle)], max_width: u16) -> Vec<StyledRow> {
    use unicode_segmentation::UnicodeSegmentation;

    let atoms: Vec<(&str, ResolvedStyle)> = runs
        .iter()
        .flat_map(|(text, style)| text.graphemes(true).map(move |g| (g, *style)))
        .collect();

    let max_width = max_width.max(1) as usize;
    let mut out: Vec<StyledRow> = Vec::new();
    let mut pos = 0;

    loop {
        let para_start = pos;
        while pos < atoms.len() && atoms[pos].0 != "\n" {
            pos += 1;
        }
        let paragraph = &atoms[para_start..pos];
        let had_newline = pos < atoms.len();
        if had_newline {
            pos += 1; // skip the "\n" atom itself
        }

        if paragraph.is_empty() {
            out.push(Vec::new());
        } else {
            let mut current: Vec<(&str, ResolvedStyle)> = Vec::new();
            let mut current_w = 0usize;
            let mut word_start = 0;
            loop {
                let mut word_end = word_start;
                while word_end < paragraph.len() && paragraph[word_end].0 != " " {
                    word_end += 1;
                }
                let word = &paragraph[word_start..word_end];
                let word_w: usize = word.iter().map(|(g, _)| cell_width(g)).sum();
                // Would-be width if `word` were appended to the current
                // line, recomputed fresh each iteration (never carried
                // across a break) so a line-break never leaves a stale
                // separator width behind.
                let would_be_w = if current.is_empty() {
                    word_w
                } else {
                    current_w + 1 + word_w
                };

                if would_be_w > max_width && !current.is_empty() {
                    out.push(coalesce_atoms(std::mem::take(&mut current)));
                    current_w = 0;
                }

                if word_w > max_width {
                    // A single word wider than the line: hard-break it on
                    // grapheme boundaries rather than overflow.
                    if !current.is_empty() {
                        out.push(coalesce_atoms(std::mem::take(&mut current)));
                    }
                    let mut piece: Vec<(&str, ResolvedStyle)> = Vec::new();
                    let mut piece_w = 0usize;
                    for &(g, style) in word {
                        let gw = cell_width(g);
                        if piece_w + gw > max_width && !piece.is_empty() {
                            out.push(coalesce_atoms(std::mem::take(&mut piece)));
                            piece_w = 0;
                        }
                        piece.push((g, style));
                        piece_w += gw;
                    }
                    current = piece;
                    current_w = piece_w;
                } else {
                    if !current.is_empty() {
                        // The synthetic separator carries the *next* word's
                        // style. It's a single blank cell either way; this
                        // just keeps it from spuriously splitting an
                        // otherwise-uniform run in two.
                        let sep_style = word
                            .first()
                            .map_or_else(ResolvedStyle::default, |&(_, s)| s);
                        current.push((" ", sep_style));
                        current_w += 1;
                    }
                    current.extend_from_slice(word);
                    current_w += word_w;
                }

                if word_end >= paragraph.len() {
                    break;
                }
                word_start = word_end + 1; // skip the delimiting space atom
            }
            out.push(coalesce_atoms(current));
        }

        if !had_newline {
            break;
        }
    }

    out
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests;
