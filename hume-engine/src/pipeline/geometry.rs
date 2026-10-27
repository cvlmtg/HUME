//! The screen's partition into chrome and panes, derived from the terminal
//! size, the rows the chrome takes and the pane tree whenever it is read.

use std::num::NonZeroU16;

use hume_grid::{Position, Rect};

use super::BOTTOM_BANDS;
use super::layout::{Direction, FocusedLayout, Side};
use crate::pane::ViewGeometry;
use hume_core::PaneId;

/// Rows each piece of chrome takes, as the editor computes them from its
/// models.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub struct ChromeRows {
    /// Rows at the top of the terminal.
    pub tabbar: u16,
    /// Rows of each bottom band, stacked upward from the statusline in array
    /// order.
    pub bands: [u16; BOTTOM_BANDS],
}

/// Where the chrome and the panes sit on a terminal. Built only by
/// [`super::EngineView::screen_layout`].
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct ScreenLayout {
    terminal: Rect,
    rows: ChromeRows,
    reserve_seam: bool,
}

impl ScreenLayout {
    pub(super) fn new(terminal: Rect, rows: ChromeRows, reserve_seam: bool) -> Self {
        Self {
            terminal,
            rows,
            reserve_seam,
        }
    }

    /// The whole terminal.
    pub fn terminal_area(&self) -> Rect {
        self.terminal
    }

    /// The rows each piece of chrome takes.
    pub(crate) fn chrome_rows(&self) -> ChromeRows {
        self.rows
    }

    /// Whether pane splits reserve a 1-cell seam for a divider.
    pub(crate) fn reserves_seam(&self) -> bool {
        self.reserve_seam
    }

    /// The statusline's rect: the terminal's bottom row, `height: 0` on a
    /// terminal with no rows. The tab bar, the bottom bands and the pane area
    /// all end above it.
    pub fn statusline_area(&self) -> Rect {
        let area = self.terminal;
        let height = area.height.min(1);
        Rect {
            y: area.bottom() - height,
            height,
            ..area
        }
    }

    /// The tab bar's rect: its rows at the top of the terminal, `height: 0`
    /// when hidden. Clamped to end at [`Self::statusline_area`]: on a
    /// terminal too short for both, the tab bar yields the bottom row to the
    /// statusline, so painting and the click hit test agree on who owns it.
    pub fn tabbar_area(&self) -> Rect {
        let area = self.terminal;
        Rect {
            y: area.y,
            height: self.rows.tabbar.min(self.statusline_area().y - area.y),
            ..area
        }
    }

    /// The rows between the tab bar and the statusline: the panes and every
    /// bottom band. A box anchored to the statusline row, such as the command
    /// line's completion menu, stays inside it and covers the bands.
    pub fn area_above_statusline(&self) -> Rect {
        let top = self.tabbar_area().bottom();
        Rect {
            y: top,
            height: self.statusline_area().y - top,
            ..self.terminal
        }
    }

    /// The rows left to the panes once the bands take theirs, directly
    /// above them. On a terminal too small for the chrome and any content
    /// it is empty, with `y` left at the terminal's top rather than below
    /// the tab bar: a caller asking whether a row belongs to the tab bar
    /// uses [`Self::tabbar_area`].
    pub fn pane_area(&self) -> Rect {
        let above_statusline = self.area_above_statusline();
        let bands_height: u16 = self.rows.bands.iter().sum();
        if bands_height < above_statusline.height {
            Rect {
                height: above_statusline.height - bands_height,
                ..above_statusline
            }
        } else {
            Rect {
                height: 0,
                ..self.terminal
            }
        }
    }

    /// The size of pane `pid` of `layout`, `None` when `layout` has no such
    /// pane.
    pub fn pane_size(&self, layout: &FocusedLayout, pid: PaneId) -> Option<PaneSize> {
        layout
            .tree()
            .find_rect(pid, self.pane_area(), self.reserve_seam)
            .map(PaneSize::of)
    }

    /// The focused pane's rect.
    pub fn focused_rect(&self, layout: &FocusedLayout) -> Rect {
        layout.focused_rect(self.pane_area(), self.reserve_seam)
    }

    /// The focused pane's size.
    pub fn focused_size(&self, layout: &FocusedLayout) -> PaneSize {
        PaneSize::of(self.focused_rect(layout))
    }

    /// The focused pane's rect after a split on `direction`, without
    /// splitting.
    pub fn focused_split_rect(&self, layout: &FocusedLayout, direction: Direction) -> Rect {
        layout.predicted_split_rect(self.pane_area(), self.reserve_seam, direction)
    }

    /// The nearest pane on `side` of the focused one on screen, `None` when
    /// no pane lies that way.
    pub fn focused_neighbor(&self, layout: &FocusedLayout, side: Side) -> Option<PaneId> {
        layout.focused_neighbor(self.pane_area(), self.reserve_seam, side)
    }
}

/// What the last render drew: the tab bar's rect and each pane's rect. Hit
/// tests read only this, so a click lands in the pane that was on screen
/// under it even when a band or a pane has changed since. Before the first
/// render every hit misses.
#[derive(Clone, Debug, Default)]
pub(crate) struct DrawnLayout {
    tabbar: Rect,
    panes: Vec<(PaneId, Rect)>,
}

impl DrawnLayout {
    /// Start recording a frame drawn with `screen`, keeping the pane list's
    /// allocation.
    pub(crate) fn begin(&mut self, screen: &ScreenLayout) {
        self.tabbar = screen.tabbar_area();
        self.panes.clear();
    }

    /// Record pane `id`, drawn into `rect`.
    pub(crate) fn record_pane(&mut self, id: PaneId, rect: Rect) {
        self.panes.push((id, rect));
    }

    /// The tab bar's rect as drawn.
    pub(crate) fn tabbar_area(&self) -> Rect {
        self.tabbar
    }

    /// The pane drawn at `pos`, whether or not it still exists.
    pub(crate) fn pane_at(&self, pos: Position) -> Option<PaneHit> {
        let &(pane, rect) = self.panes.iter().find(|(_, rect)| rect.contains(pos))?;
        Some(PaneHit {
            pane,
            x: pos.x - rect.x,
            y: pos.y - rect.y,
            size: PaneSize::of(rect),
        })
    }
}

/// A position on a drawn pane, in that pane's own cells, with the size the
/// pane was drawn at. Built only by [`super::EngineView::pane_hit`].
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct PaneHit {
    pane: PaneId,
    x: u16,
    y: u16,
    size: PaneSize,
}

impl PaneHit {
    pub fn pane(self) -> PaneId {
        self.pane
    }

    /// The column within the pane's rect, gutter included.
    pub fn x(self) -> u16 {
        self.x
    }

    /// The row within the pane's rect.
    pub fn y(self) -> u16 {
        self.y
    }

    pub fn size(self) -> PaneSize {
        self.size
    }
}

/// A pane's size in terminal cells, gutter and content together. Built only
/// from a [`ScreenLayout`], so a size always matches the layout it was read
/// from.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct PaneSize {
    width: u16,
    height: u16,
}

impl PaneSize {
    pub(crate) fn new(width: u16, height: u16) -> Self {
        Self { width, height }
    }

    pub(crate) fn of(rect: Rect) -> Self {
        Self::new(rect.width, rect.height)
    }

    /// A size no layout produced, for a test that sizes a pane directly.
    #[cfg(any(test, feature = "test-util"))]
    pub fn for_test(width: u16, height: u16) -> Self {
        Self::new(width, height)
    }

    pub fn width(self) -> u16 {
        self.width
    }

    pub fn height(self) -> u16 {
        self.height
    }
    /// The scroll-margin geometry of a pane this size, or `None` at zero
    /// height. Every scroll verb takes a [`ViewGeometry`] rather than a raw
    /// height, so a collapsed pane is one early return here instead of a
    /// `height == 0` check repeated inside each verb. The sole constructor,
    /// so `margin`/`target` can never disagree between callers.
    pub fn geometry(self, scroll_margin: usize) -> Option<ViewGeometry> {
        if self.height == 0 {
            return None;
        }
        let height = self.height as usize;
        // `(height - 1) / 2`, not `height / 2`: at an even height, a margin
        // of `height / 2` leaves the stable middle window empty (its
        // bounds `margin..height-margin` collapse to a single point), so the
        // two correction arms that use this margin would fight over that one
        // display line and rescroll every frame.
        let margin = scroll_margin.min(height.saturating_sub(1) / 2);
        let target = height.saturating_sub(margin).saturating_sub(1);
        Some(ViewGeometry {
            height,
            margin,
            target,
        })
    }
}

/// The columns a pane leaves for text once its gutter takes its own, at
/// least one. Built only from a [`PaneSize`], by
/// [`compute_viewport`](crate::layout::compute_viewport).
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub struct ContentWidth(NonZeroU16);

impl ContentWidth {
    pub(crate) fn of(size: PaneSize, gutter_width: u16) -> Self {
        Self(NonZeroU16::new(size.width.saturating_sub(gutter_width)).unwrap_or(NonZeroU16::MIN))
    }

    /// A width no pane produced, for a test that formats against it directly.
    #[cfg(any(test, feature = "test-util"))]
    pub fn for_test(width: u16) -> Self {
        Self(NonZeroU16::new(width).expect("a content width is at least one column"))
    }

    pub fn get(self) -> NonZeroU16 {
        self.0
    }
}
