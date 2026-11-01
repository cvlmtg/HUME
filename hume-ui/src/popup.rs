//! Cursor-anchored popup widget (`show-popup!`): a floating text panel used
//! by hover, signature help, and (as a menu) the selection menu, completion
//! menu and minibuffer `:` completion.
//!
//! Geometry rules:
//! - Preferred placement: below-right of the anchor cell. Flip above when the
//!   space below is smaller than the content and the space above is larger.
//! - Clamp horizontally to the pane's right edge.
//! - Max width `min(120, pane_width - 4)`, max height a third of the pane.
//!   Taller content is the caller's problem (hover overflows to the drawer).
//! - 1-cell frame themed `ui.popup` (`ui.menu` for menus), box-drawn when
//!   `popup-border` is on. Rendering lives in [`super::menu_box`].
//! - A side popup ([`resolve_side_popup`]) sits beside another box rather
//!   than below or above an anchor cell.
//!
//! [`resolve_popup`]/[`resolve_menu`]/[`resolve_band`] resolve geometry and
//! the scroll window every frame, against the focused pane's current rect, so
//! a resize or scroll never leaves them stale. For a menu, [`menu_window`]
//! picks the window from counts alone and the caller materializes only those
//! rows, so width is measured over visible rows only. `PopupOverlay`,
//! `MenuOverlay` and `PopupBandWidget::render` paint the resolved result,
//! clipped defensively to `pane_rect`.

use hume_engine::types::ResolvedStyle;
use hume_grid::Rect;
use std::cell::{Ref, RefCell};
use std::sync::Arc;

use hume_engine::lock::SharedSlot;

use hume_engine::providers::{ChromeProvider, OverlayProvider};
use hume_engine::render::Canvas;
use hume_engine::theme::Theme;
use hume_rope::width::{chrome_cluster_width, chrome_width};

use super::list_row::ListRow;
use super::menu_box::{BoxRows, FRAME_CELLS, MenuBoxStyles, draw_menu_box};

/// Maximum popup width in terminal columns, before any pane-width clamp.
/// Popup-only: a menu doesn't wrap, so it has no analogous cap here (it
/// uses [`super::menu_box::MAX_MENU_ROWS`], a row cap, instead).
const MAX_POPUP_WIDTH: u16 = 120;

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
/// `plain` is a single default-style run. `styled` takes
/// pre-resolved `(text, style)` runs rather than a grammar name: this crate
/// has no tree-sitter dependency of its own, so `hume-editor` (which does,
/// via `MarkupSyntax`) resolves spans against its own baked `Theme` before
/// handing them over: `Theme::resolve` debug-asserts on an unbaked
/// `ScopeId`, and this crate has no `ScopeRegistry` access to bake one.
pub struct PopupContent {
    source: PopupSource,
    cache: RefCell<Option<Wrapped>>,
}

/// A popup's text: plain, painted in one style, or styled runs.
enum PopupSource {
    Plain(String),
    Styled(Vec<(String, ResolvedStyle)>),
}

struct Wrapped {
    width: u16,
    lines: Arc<WrappedLines>,
    /// Widest line's display width, measured once per wrap, not
    /// re-measured every frame an unchanged wrap is reused, via
    /// [`super::menu_box::widest`].
    inner_width: u16,
}

impl Wrapped {
    /// The popup showing these rows in the box at `rect`, scrolled to
    /// `scroll` clamped to what its inner height leaves scrollable.
    fn popup_state(&self, rect: Rect, scroll: usize, border: bool) -> PopupState {
        let total_rows = self.lines.len();
        let inner_h = inner_rows(rect.height);
        let scroll = scroll.min(total_rows.saturating_sub(inner_h));
        PopupState {
            rows: PopupRows {
                lines: Arc::clone(&self.lines),
                visible: super::menu_box::window_range(total_rows, scroll, inner_h),
                total_rows,
                scroll,
                border,
            },
            rect,
        }
    }
}

/// A popup's wrapped lines: a plain popup's text, or a
/// [`PopupContent::styled`] popup's runs.
pub enum WrappedLines {
    Plain(Vec<String>),
    Styled(Vec<StyledRow>),
}

impl WrappedLines {
    pub fn len(&self) -> usize {
        match self {
            Self::Plain(lines) => lines.len(),
            Self::Styled(rows) => rows.len(),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Each line's text, a styled line's runs joined.
    pub fn texts(&self) -> Vec<String> {
        match self {
            Self::Plain(lines) => lines.clone(),
            Self::Styled(rows) => rows.iter().map(|row| row_text(row)).collect(),
        }
    }

    /// Each line's runs, `None` for a plain popup.
    pub fn styled(&self) -> Option<&[StyledRow]> {
        match self {
            Self::Plain(_) => None,
            Self::Styled(rows) => Some(rows),
        }
    }

    /// The widest line's display width.
    fn widest(&self) -> u16 {
        match self {
            Self::Plain(lines) => super::menu_box::widest(lines.iter().map(String::as_str)),
            Self::Styled(rows) => rows
                .iter()
                .map(|row| chrome_width(&row_text(row)))
                .max()
                .unwrap_or(0) as u16,
        }
    }

    /// The lines in `visible`, as the box paints them.
    fn box_rows(&self, visible: std::ops::Range<usize>) -> BoxRows<'_> {
        match self {
            Self::Plain(lines) => BoxRows::Plain(&lines[visible]),
            Self::Styled(rows) => BoxRows::Styled(&rows[visible]),
        }
    }
}

/// A wrapped row's text, its runs joined.
fn row_text(row: &StyledRow) -> String {
    row.iter().map(|(text, _)| text.as_str()).collect()
}

impl PopupContent {
    pub fn plain(text: &str) -> Self {
        Self {
            source: PopupSource::Plain(text.to_string()),
            cache: RefCell::new(None),
        }
    }

    /// `runs`: pre-highlighted `(text, style)` spans in source order. See
    /// this type's own doc for why the caller resolves these rather than
    /// handing over a grammar name.
    pub fn styled(runs: Vec<(String, ResolvedStyle)>) -> Self {
        Self {
            source: PopupSource::Styled(runs),
            cache: RefCell::new(None),
        }
    }

    /// A plain popup's lines, or a [`Self::styled`] popup's runs, wrapped
    /// at `width`, recomputing only when `width` differs from the cached one:
    /// an unchanged width across frames is O(1), not a re-wrap. The lines
    /// are `Arc`-wrapped here so the *cache* itself is a pointer, not a
    /// copy, across frames; [`resolve_popup`]/[`resolve_band`]
    /// hand that same `Arc` straight to [`PopupRows`] (an `Arc` clone, never
    /// a copy, regardless of scroll position) along with a `visible` range
    /// into it. The window is a view, not a slice
    /// taken up front.
    fn wrapped(&self, width: u16) -> Ref<'_, Wrapped> {
        let stale = self
            .cache
            .borrow()
            .as_ref()
            .is_none_or(|c| c.width != width);
        if stale {
            let lines = match &self.source {
                PopupSource::Plain(text) => WrappedLines::Plain(
                    wrap_styled([(text.as_str(), ResolvedStyle::default())], width)
                        .iter()
                        .map(row_text)
                        .collect(),
                ),
                PopupSource::Styled(runs) => WrappedLines::Styled(wrap_styled(
                    runs.iter().map(|(text, style)| (text.as_str(), *style)),
                    width,
                )),
            };
            let inner_width = lines.widest();
            *self.cache.borrow_mut() = Some(Wrapped {
                width,
                lines: Arc::new(lines),
                inner_width,
            });
        }
        Ref::map(self.cache.borrow(), |cache| {
            cache.as_ref().expect("populated above")
        })
    }
}

/// The rows a framed popup paints and the scroll window over them: a
/// floating popup's (inside [`PopupState`]) or a docked popup's band.
pub struct PopupRows {
    /// The *full* wrapped row list, `Arc`-shared with the source
    /// [`PopupContent`]'s cache. `visible` carries the window into it, so a
    /// scrolled popup taller than its box still costs a refcount bump per
    /// frame, not a fresh `Vec` copy of whatever's on screen.
    pub lines: Arc<WrappedLines>,
    /// The window into `lines` that's actually on screen: `draw_menu_box`
    /// paints `&lines[visible]`, never `lines` whole. Same length as the
    /// box's inner height (or less, for the last partial window at the end
    /// of a short list).
    pub visible: std::ops::Range<usize>,
    /// The full row count `lines` holds; together with `scroll`, what the
    /// scrollbar thumb sizes and places itself against.
    pub total_rows: usize,
    /// The visible window's own start, `visible.start`: the resolved
    /// counterpart of `PopupLayer::scroll`. Used only by the scrollbar
    /// thumb.
    pub scroll: usize,
    /// Whether to draw box-drawing border glyphs around the box (vs. a
    /// plain background-filled 1-cell margin). Fed from the `popup-border`
    /// setting.
    pub border: bool,
}

impl PopupRows {
    /// Paint the visible window into the box at `outer`.
    fn draw(&self, canvas: &mut Canvas, outer: Rect, theme: &Theme) {
        draw_menu_box(
            canvas,
            outer,
            self.lines.box_rows(self.visible.clone()),
            self.total_rows,
            self.scroll,
            self.border,
            MenuBoxStyles::popup(theme),
        );
    }
}

/// Fully-resolved popup content and position, computed once per frame by
/// the write side; the overlay only paints.
pub struct PopupState {
    pub rows: PopupRows,
    /// Outer footprint (including the 1-cell frame), top-left corner already
    /// flipped/clamped by the write side. Carrying the resolved size here,
    /// rather than re-deriving it from `lines.len()` at render time, keeps
    /// the painted box and the positioned box the same box.
    pub rect: Rect,
}

impl PopupState {
    /// Rows of content `rect` shows inside its frame.
    pub fn inner_height(&self) -> usize {
        inner_rows(self.rect.height)
    }
}

/// Rows of content a box `outer_h` rows tall shows inside its frame.
fn inner_rows(outer_h: u16) -> usize {
    outer_h.saturating_sub(FRAME_CELLS) as usize
}

/// Overlay that paints a `PopupState` snapshot in `ui.popup`: a hover-style
/// popup (`show-popup!`) or the completion documentation box.
pub(crate) struct PopupOverlay {
    pub(crate) data: SharedSlot<Option<PopupState>>,
}

impl OverlayProvider for PopupOverlay {
    fn is_active(&self) -> bool {
        self.data.read().is_some()
    }

    fn render(&self, theme: &Theme, canvas: &mut Canvas) {
        let guard = self.data.read();
        let Some(state) = guard.as_ref() else { return };
        if state.rows.lines.is_empty() {
            return;
        }
        state.rows.draw(canvas, state.rect, theme);
    }
}

/// Engine-facing bottom-band provider for a docked popup. Mirrors
/// [`super::drawer::DrawerWidget`]'s shape (chrome, not per-pane), but paints
/// through `draw_menu_box` so a docked hover keeps the popup's framed,
/// `ui.popup`-scoped look rather than the drawer's plain list rows.
pub(crate) struct PopupBandWidget {
    pub(crate) data: SharedSlot<Option<PopupRows>>,
}

/// A docked popup's `content` wrapped for a terminal `area_width` columns
/// wide.
fn band_wrapped(content: &PopupContent, area_width: u16) -> Ref<'_, Wrapped> {
    content.wrapped(super::menu_box::band_inner_width(area_width))
}

/// Wrapped lines a docked popup's `content` has on a terminal `area_width`
/// columns wide.
pub fn band_total_rows(content: &PopupContent, area_width: u16) -> usize {
    band_wrapped(content, area_width).lines.len()
}

/// Rows a docked popup of `lines` wrapped lines takes on a terminal
/// `terminal_height` rows tall.
pub fn band_height(lines: usize, terminal_height: u16) -> u16 {
    super::menu_box::band_capacity(lines, FRAME_CELLS, terminal_height)
}

/// Lines a docked popup shows at once inside [`band_height`]: the number
/// Ctrl-d/Ctrl-u page it against.
pub fn band_visible_rows(lines: usize, terminal_height: u16) -> usize {
    super::menu_box::band_visible_rows(lines, FRAME_CELLS, terminal_height)
}

impl ChromeProvider for PopupBandWidget {
    fn render(&self, area: Rect, theme: &Theme, canvas: &mut Canvas) {
        if area.height == 0 {
            return;
        }
        let guard = self.data.read();
        let Some(rows) = guard.as_ref() else { return };
        rows.draw(canvas, area, theme);
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
/// resolved content height and windows its lines down to it.
pub fn resolve_popup(
    content: &PopupContent,
    placement: PopupPlacement,
    scroll: usize,
    border: bool,
) -> PopupState {
    // Reserve the frame's cells on each axis, so content and border
    // together fit inside the envelope.
    let max_width = MAX_POPUP_WIDTH
        .min(placement.content_width.saturating_sub(4))
        .saturating_sub(FRAME_CELLS);
    let max_height = (placement.pane_rect.height / 3)
        .max(1)
        .saturating_sub(FRAME_CELLS)
        .max(1);

    let wrapped = content.wrapped(max_width);
    let total_rows = wrapped.lines.len();
    let (outer_w, outer_h) =
        super::menu_box::outer_dims_from_width(wrapped.inner_width, total_rows, max_height);
    let (x, y, outer_w, outer_h) =
        resolve_popup_geometry(outer_w, outer_h, placement.anchor, placement.pane_rect);
    wrapped.popup_state(Rect::new(x, y, outer_w, outer_h), scroll, border)
}

/// Narrowest side popup worth drawing, frame included.
const MIN_SIDE_POPUP_WIDTH: u16 = 20;

/// Where a popup beside another box goes, from [`side_slot`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SideSlot {
    beside: Rect,
    /// Columns free on the chosen side, frame included.
    room: u16,
    right: bool,
    above: bool,
    max_outer_h: u16,
}

/// Room for a popup beside `beside`, another box's outer rect in
/// `pane_rect` (the completion menu's): to its right when at least
/// `MIN_SIDE_POPUP_WIDTH` cells fit there, else to its left, else `None`.
/// It may grow to the taller of `beside` and a third of the pane, away from
/// `anchor_row`, the row `beside` was placed against: below it, its top
/// edge is level with `beside`'s and it stops at the pane's bottom; above
/// it, its bottom edge is, and it stops at the pane's top. Either way it
/// never covers the anchor row. `None` too when that leaves no content row.
pub fn side_slot(beside: Rect, anchor_row: u16, pane_rect: Rect) -> Option<SideSlot> {
    let room_right = pane_rect.right().saturating_sub(beside.right());
    let room_left = beside.x.saturating_sub(pane_rect.x);
    let (room, right) = if room_right >= MIN_SIDE_POPUP_WIDTH {
        (room_right, true)
    } else if room_left >= MIN_SIDE_POPUP_WIDTH {
        (room_left, false)
    } else {
        return None;
    };
    let above = beside.bottom() <= anchor_row;
    let vertical_room = if above {
        beside.bottom().saturating_sub(pane_rect.y)
    } else {
        pane_rect.bottom().saturating_sub(beside.y)
    };
    let max_outer_h = beside.height.max(pane_rect.height / 3).min(vertical_room);
    if inner_rows(max_outer_h) == 0 {
        return None;
    }
    Some(SideSlot {
        beside,
        room,
        right,
        above,
        max_outer_h,
    })
}

/// Resolve a popup in `slot`, wrapped to the room there. `scroll` is
/// clamped the same way [`resolve_popup`] clamps it.
pub fn resolve_side_popup(
    content: &PopupContent,
    slot: &SideSlot,
    scroll: usize,
    border: bool,
) -> PopupState {
    let SideSlot {
        beside,
        room,
        right,
        above,
        max_outer_h,
    } = *slot;
    let wrapped = content.wrapped(room.min(MAX_POPUP_WIDTH) - FRAME_CELLS);
    let total_rows = wrapped.lines.len();
    let (outer_w, outer_h) = super::menu_box::outer_dims_from_width(
        wrapped.inner_width,
        total_rows,
        max_outer_h - FRAME_CELLS,
    );
    let x = if right {
        beside.right()
    } else {
        beside.x - outer_w
    };
    let y = if above {
        beside.bottom() - outer_h
    } else {
        beside.y
    };
    wrapped.popup_state(Rect::new(x, y, outer_w, outer_h), scroll, border)
}

/// Resolve a docked popup's content for this frame: the
/// [`hume_core::PopupLayout::Docked`] counterpart of [`resolve_popup`]. No anchor or pane rect: a
/// bottom band's rows come from the layout ([`band_height`]) and its
/// position from the engine at render time (mirroring the drawer's own
/// contract), so this only resolves content. `terminal` is the raw,
/// un-subtracted terminal area: the band spans the full terminal width, not
/// just the panes region, and its ceiling is a share of the full height.
pub fn resolve_band(
    content: &PopupContent,
    terminal: Rect,
    scroll: usize,
    border: bool,
) -> PopupRows {
    let wrapped = band_wrapped(content, terminal.width);
    let total_rows = wrapped.lines.len();
    let inner_h = band_visible_rows(total_rows, terminal.height);
    let scroll = scroll.min(total_rows.saturating_sub(inner_h));
    let visible = super::menu_box::window_range(total_rows, scroll, inner_h);
    PopupRows {
        lines: Arc::clone(&wrapped.lines),
        visible,
        total_rows,
        scroll,
        border,
    }
}

/// One menu row: a primary label, plus an optional plain second part
/// right-aligned against it (an LSP completion candidate's `detail`, e.g.).
/// See [`resolve_menu`] for how the two are laid out into one row.
/// `trailing` is a `String`, not an `Arc<str>`: `compose_menu_row` always
/// paints into a fresh row regardless, so an `Arc<str>` here would only
/// ever save a caller's own construction-time clone, and the caller with
/// the most rows to build (`CompletionItem::menu_row`, one call per parsed
/// item) never reaches the visible window most of them are built for; see
/// that method's own doc.
#[derive(Clone)]
pub struct MenuRow {
    pub main: ListRow,
    pub trailing: Option<String>,
}

impl MenuRow {
    /// A single-column row, `trailing` absent.
    pub fn plain(main: ListRow) -> Self {
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
/// caller slices its own list to `range`, hands those rows alone to
/// [`resolve_menu`], and the rest of the list never reaches the resolver at
/// all. Every field but `range` is private and there is no constructor
/// besides `menu_window`, so a window can't be faked or widened by hand.
pub struct MenuWindow {
    /// The `[start, end)` slice of the full ranked list that's visible.
    pub range: std::ops::Range<usize>,
    /// The highlighted row, window-relative; `None` for an empty list or with
    /// no row picked.
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
/// `draw_menu_box` bails on an empty window) box:
/// geometry stays a pure function of `(total_rows, pane)`, with no
/// special-cased early return, so a caller never sees a stale rect from
/// the *previous* nonempty frame linger into this one.
///
/// A `None` selection highlights nothing and windows from the top.
pub fn menu_window(total_rows: usize, selected: Option<usize>, pane_rect: Rect) -> MenuWindow {
    let raw_outer_h = super::menu_box::outer_rows(total_rows, super::menu_box::MAX_MENU_ROWS);
    let outer_h = raw_outer_h.min(pane_rect.height);
    let inner_h = inner_rows(outer_h);
    let (selected_rel, range) = if total_rows == 0 {
        (None, 0..0)
    } else {
        let selected = selected.map(|row| row.min(total_rows - 1));
        let range = super::menu_box::window_range(
            total_rows,
            selected.unwrap_or(0).saturating_sub(inner_h / 2),
            inner_h,
        );
        (selected.map(|row| row - range.start), range)
    };
    MenuWindow {
        range,
        selected_rel,
        outer_h,
        total_rows,
    }
}

/// Phase 2: a menu's content + position for this frame, from `rows`, which
/// are `window.range`'s rows, nothing more. No wrapping (menu
/// entries are short labels, not prose), so, unlike [`resolve_popup`], there
/// is no fixed width budget: the box is as wide as `rows` need, up
/// to the pane clamp. Because a caller can only ever hand over the window's
/// own rows, one long candidate scrolled out of view *cannot* inflate the
/// box. That guarantee is the reason the resolver is split in two, not a
/// property this function has to remember to uphold. Column widths
/// (`main`/`trailing`) are resolved from `rows` and composed into each row
/// before painting: `draw_menu_box` receives pre-aligned rows, unaware
/// rows ever had two parts.
pub fn resolve_menu(
    rows: &[MenuRow],
    window: MenuWindow,
    placement: PopupPlacement,
    border: bool,
) -> MenuState {
    debug_assert_eq!(
        rows.len(),
        window.range.len(),
        "resolve_menu: `rows` must be the window's own slice"
    );

    let max_inner = placement.pane_rect.width.saturating_sub(FRAME_CELLS);
    let main_w = super::menu_box::widest(rows.iter().map(|r| r.main.text()));
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

    let composed: Vec<ListRow> = rows
        .iter()
        .map(|row| compose_menu_row(row, main_col, trail_col))
        .collect();

    let (x, y, outer_w, outer_h) = resolve_popup_geometry(
        inner_w + FRAME_CELLS,
        window.outer_h,
        placement.anchor,
        placement.pane_rect,
    );
    MenuState {
        rows: composed,
        rect: Rect::new(x, y, outer_w, outer_h),
        selected: window.selected_rel,
        total_rows: window.total_rows,
        scroll: window.range.start,
        border,
    }
}

/// Compose one [`MenuRow`] into a single painted row: `main` left-aligned
/// and padded to `main_col`, then (when `trail_col > 0`) [`MENU_COLUMN_GAP`]
/// blank cells and `trailing` right-aligned and padded to `trail_col`. A
/// row with no `trailing` of its own still reserves that column as blank, so
/// every row's second part lines up under the others'. Both parts are
/// truncated (never wrapped) to their column width, since a menu row is
/// one screen line.
fn compose_menu_row(row: &MenuRow, main_col: u16, trail_col: u16) -> ListRow {
    use hume_core::TruncateEnd;

    let main = row.main.clone().fit(main_col as usize, TruncateEnd::Tail);
    if trail_col == 0 {
        return main;
    }
    let main_pad = main_col as usize - chrome_width(main.text());
    let trailing = hume_rope::width::truncate_marked(
        row.trailing.as_deref().unwrap_or(""),
        trail_col as usize,
        TruncateEnd::Tail,
    );
    let trail_pad = trail_col as usize - chrome_width(&trailing);
    let gap = main_pad + MENU_COLUMN_GAP as usize + trail_pad;
    main.append(&format!("{:gap$}{trailing}", ""))
}

/// A menu's content and position for this frame, resolved by
/// [`resolve_menu`]; `MenuOverlay` only paints it.
pub struct MenuState {
    /// The visible window's rows, each already laid out to the box's inner
    /// width.
    pub rows: Vec<ListRow>,
    /// Outer footprint (including the 1-cell frame), top-left corner already
    /// flipped/clamped to the pane.
    pub rect: Rect,
    /// The highlighted row, an index into `rows`; `None` with no row picked.
    pub selected: Option<usize>,
    /// The full list's row count and the window's start within it, for the
    /// scrollbar thumb.
    pub total_rows: usize,
    pub scroll: usize,
    /// Fed from the `popup-border` setting.
    pub border: bool,
}

/// Overlay that paints a [`MenuState`] in `ui.menu`: the selection menu, the
/// completion menu and the minibuffer's `:` completion, each from its own
/// slot.
pub(crate) struct MenuOverlay {
    pub(crate) data: SharedSlot<Option<MenuState>>,
}

impl OverlayProvider for MenuOverlay {
    fn is_active(&self) -> bool {
        self.data.read().is_some()
    }

    fn render(&self, theme: &Theme, canvas: &mut Canvas) {
        let guard = self.data.read();
        let Some(state) = guard.as_ref() else { return };
        draw_menu_box(
            canvas,
            state.rect,
            BoxRows::List {
                rows: &state.rows,
                selected: state.selected,
                theme,
            },
            state.total_rows,
            state.scroll,
            state.border,
            MenuBoxStyles::menu(theme),
        );
    }
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
/// never satisfy that check, and the whole popup is not drawn, with no error.
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
/// A line's leading spaces stay on its first row; continuation rows start
/// at column 0. Unbounded height: [`PopupContent::wrapped`]'s caller windows the result
/// (`scroll`) rather than truncating it here; a `Scrollable` popup needs
/// every row reachable, not just the first screenful.
///
/// Operates on a flat per-grapheme stream, never on `runs`' original chunk
/// boundaries. A style change (e.g. a `**bold**` span) can land anywhere,
/// including mid-word, so wrapping must not coarsen past grapheme
/// granularity. Plain popup text is a single default-style run, so
/// there is one wrap algorithm here, not a separate one for plain text.
fn wrap_styled<'a>(
    runs: impl IntoIterator<Item = (&'a str, ResolvedStyle)>,
    max_width: u16,
) -> Vec<StyledRow> {
    use unicode_segmentation::UnicodeSegmentation;

    let atoms: Vec<(&str, ResolvedStyle)> = runs
        .into_iter()
        .flat_map(|(text, style)| text.graphemes(true).map(move |g| (g, style)))
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
            // The line's leading spaces belong to its first word, so its
            // indentation stays on its first row.
            let mut word_end = paragraph.iter().take_while(|(g, _)| *g == " ").count();
            loop {
                while word_end < paragraph.len() && paragraph[word_end].0 != " " {
                    word_end += 1;
                }
                let word = &paragraph[word_start..word_end];
                let word_w: usize = word.iter().map(|(g, _)| chrome_cluster_width(g)).sum();
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
                        let gw = chrome_cluster_width(g);
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
                word_end = word_start;
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
