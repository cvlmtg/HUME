use std::collections::HashMap;
use std::sync::OnceLock;

use hume_engine::lock::SharedSlot;
use hume_engine::providers::ChromeProvider;
use hume_engine::render::Canvas;
use hume_engine::theme::{ScopeRegistry, Theme};
use hume_engine::types::{Modifiers, ResolvedStyle, ScopeId};
use hume_grid::{Grid, Rect, Rgb};
use hume_rope::spanned::SpannedText;

use super::super::menu_box::FRAME_EDGE;
use super::super::symbols_in;
use super::{DrawerRow, DrawerViewState, DrawerWidget, SpanScopes, band_height, band_visible_rows};

#[test]
fn band_height_is_capped_by_the_ceiling() {
    // 50 items and the top frame row want 51 rows; a 20-row terminal's 35%
    // ceiling is 7.
    assert_eq!(band_height(50, 20), 7);
}

/// The overflow-safety guard on the shared arithmetic itself lives in
/// `menu_box::tests` (`band_capacity_clamps_instead_of_overflowing_u16`).
/// This pins the drawer's own 1-row top frame against it instead of
/// re-testing the arithmetic.
#[test]
fn band_visible_rows_reserves_the_top_frame_row_below_the_cap() {
    assert_eq!(
        band_visible_rows(3, 20),
        3,
        "well under the cap: all 3 items fit"
    );
}

const DRAWER_FG: Rgb = Rgb(10, 0, 0);
const DRAWER_BG: Rgb = Rgb(20, 0, 0);
const SELECTED_FG: Rgb = Rgb(30, 0, 0);
const SELECTED_BG: Rgb = Rgb(40, 0, 0);

fn colors(fg: Rgb, bg: Rgb) -> ResolvedStyle {
    ResolvedStyle {
        fg: Some(fg),
        bg: Some(bg),
        ..Default::default()
    }
}

const SPAN_SCOPE: &str = "markup.italic";
const SELECTED_SPAN_SCOPE: &str = "markup.bold";
const ROW_SCOPE: &str = "diagnostic.warning.message";
const ROW_FG: Rgb = Rgb(70, 0, 0);
const ROW_BG: Rgb = Rgb(80, 0, 0);

fn italic() -> ResolvedStyle {
    ResolvedStyle {
        modifiers: Modifiers::ITALIC,
        ..Default::default()
    }
}

/// The ids [`drawer_theme`] interns its scopes under.
struct Ids {
    span: ScopeId,
    selected_span: ScopeId,
    row: ScopeId,
}

/// A theme styling the drawer, its selected row, [`SELECTED_SPAN_SCOPE`] as
/// bold, [`ROW_SCOPE`] in its own colors and, when `span_style` is set,
/// [`SPAN_SCOPE`]. It is baked against a registry holding the three scopes,
/// whose ids are returned with it.
fn drawer_theme(span_style: Option<ResolvedStyle>, default: ResolvedStyle) -> (Theme, Ids) {
    let mut styles = HashMap::from([
        ("ui.drawer", colors(DRAWER_FG, DRAWER_BG)),
        ("ui.menu.selected", colors(SELECTED_FG, SELECTED_BG)),
        (
            SELECTED_SPAN_SCOPE,
            ResolvedStyle {
                modifiers: Modifiers::BOLD,
                ..Default::default()
            },
        ),
        (ROW_SCOPE, colors(ROW_FG, ROW_BG)),
    ]);
    if let Some(style) = span_style {
        styles.insert(SPAN_SCOPE, style);
    }
    let mut registry = ScopeRegistry::new();
    let ids = Ids {
        span: registry.intern(SPAN_SCOPE),
        selected_span: registry.intern(SELECTED_SPAN_SCOPE),
        row: registry.intern(ROW_SCOPE),
    };
    let mut theme = Theme::new(styles, default);
    theme.bake(&registry);
    (theme, ids)
}

/// Scopes that style a span the same on every row.
/// A row of `text` whose `spans` (char offsets) are styled by `scopes`.
fn styled_row(text: &str, spans: Vec<(usize, usize, SpanScopes)>) -> DrawerRow {
    DrawerRow::styled(SpannedText::new(text.to_string(), spans).expect("spans fit their row"))
}

fn any_row(scope: ScopeId) -> SpanScopes {
    SpanScopes {
        scope,
        selected: None,
    }
}

fn render(theme: &Theme, rows: Vec<DrawerRow>, border: bool, grid: &mut Grid, area: Rect) {
    let widget = DrawerWidget {
        data: SharedSlot::new(Some(DrawerViewState {
            rows: rows.into_iter().map(OnceLock::from).collect(),
            selected: 0,
            scroll: 0,
            border,
        })),
    };
    let mut canvas = Canvas::new(grid, theme.ui.invisible, None);
    widget.render(area, theme, &mut canvas);
}

#[test]
fn span_cells_layer_the_span_style_over_the_row_style() {
    let (theme, Ids { span: id, .. }) = drawer_theme(Some(italic()), ResolvedStyle::default());
    let row = || styled_row("abcd", vec![(1, 3, any_row(id))]);
    let area = Rect::new(0, 0, 10, 3);
    let mut grid = Grid::new(10, 3);
    render(&theme, vec![row(), row()], false, &mut grid, area);

    // The text starts after the frame's left edge, on the row below its top.
    let x0 = area.x + FRAME_EDGE;
    let (selected_y, plain_y) = (area.y + FRAME_EDGE, area.y + FRAME_EDGE + 1);
    let span = [x0 + 1, x0 + 2];
    let outside = [x0, x0 + 3];
    let italic = |x: u16, y: u16| grid[(x, y)].style().modifiers.contains(Modifiers::ITALIC);

    for x in span {
        let style = grid[(x, selected_y)].style();
        assert_eq!(style.fg, Some(SELECTED_FG), "selected span fg at x={x}");
        assert_eq!(style.bg, Some(SELECTED_BG), "selected span bg at x={x}");
        assert!(italic(x, selected_y), "selected span italic at x={x}");
        assert_eq!(
            grid[(x, plain_y)].style().fg,
            Some(DRAWER_FG),
            "span fg at x={x}"
        );
        assert!(italic(x, plain_y), "span italic at x={x}");
    }
    for x in outside {
        assert_eq!(
            grid[(x, selected_y)].style().fg,
            Some(SELECTED_FG),
            "selected fg at x={x}"
        );
        assert!(
            !italic(x, selected_y),
            "selected cell outside the span at x={x}"
        );
        assert_eq!(
            grid[(x, plain_y)].style().fg,
            Some(DRAWER_FG),
            "fg at x={x}"
        );
        assert!(!italic(x, plain_y), "cell outside the span at x={x}");
    }
}

#[test]
fn a_span_with_a_selected_scope_takes_it_on_the_selected_row_only() {
    let (
        theme,
        Ids {
            span: id,
            selected_span: selected_id,
            ..
        },
    ) = drawer_theme(Some(italic()), ResolvedStyle::default());
    let scopes = SpanScopes {
        scope: id,
        selected: Some(selected_id),
    };
    let row = || styled_row("abcd", vec![(1, 3, scopes)]);
    let area = Rect::new(0, 0, 10, 3);
    let mut grid = Grid::new(10, 3);
    render(&theme, vec![row(), row()], false, &mut grid, area);

    let x0 = area.x + FRAME_EDGE;
    let (selected_y, plain_y) = (area.y + FRAME_EDGE, area.y + FRAME_EDGE + 1);
    for x in [x0 + 1, x0 + 2] {
        let selected = grid[(x, selected_y)].style();
        assert_eq!(
            selected.modifiers,
            Modifiers::BOLD,
            "selected span at x={x}"
        );
        assert_eq!(selected.bg, Some(SELECTED_BG), "selected span bg at x={x}");
        let plain = grid[(x, plain_y)].style();
        assert_eq!(plain.modifiers, Modifiers::ITALIC, "span at x={x}");
        assert_eq!(plain.bg, Some(DRAWER_BG), "span bg at x={x}");
    }
}

#[test]
fn a_rows_selected_scope_replaces_the_selected_style_on_that_row_only() {
    let (theme, Ids { row: row_id, .. }) = drawer_theme(Some(italic()), ResolvedStyle::default());
    let row = || DrawerRow::plain("ab".into()).with_selected_scope(Some(row_id));
    let area = Rect::new(0, 0, 10, 3);
    let mut grid = Grid::new(10, 3);
    render(&theme, vec![row(), row()], false, &mut grid, area);

    // Past the text too: the scope paints the whole highlight band.
    let x0 = area.x + FRAME_EDGE;
    let (selected_y, plain_y) = (area.y + FRAME_EDGE, area.y + FRAME_EDGE + 1);
    for x in [x0, x0 + 1, x0 + 4] {
        assert_eq!(
            grid[(x, selected_y)].style().bg,
            Some(ROW_BG),
            "selected bg at x={x}"
        );
        assert_eq!(
            grid[(x, plain_y)].style().bg,
            Some(DRAWER_BG),
            "bg at x={x}"
        );
    }
    for x in [x0, x0 + 1] {
        assert_eq!(
            grid[(x, selected_y)].style().fg,
            Some(ROW_FG),
            "selected fg at x={x}"
        );
    }
}

#[test]
fn a_rows_selected_scope_the_theme_does_not_style_selects_in_the_default_style() {
    let default = colors(Rgb(50, 0, 0), Rgb(60, 0, 0));
    let (theme, Ids { span: unstyled, .. }) = drawer_theme(None, default);
    let row = DrawerRow::plain("ab".into()).with_selected_scope(Some(unstyled));
    let area = Rect::new(0, 0, 10, 2);
    let mut grid = Grid::new(10, 2);
    render(&theme, vec![row], false, &mut grid, area);

    let (x0, y) = (area.x + FRAME_EDGE, area.y + FRAME_EDGE);
    for x in [x0, x0 + 1, x0 + 4] {
        assert_eq!(grid[(x, y)].style().bg, default.bg, "bg at x={x}");
    }
}

#[test]
fn a_span_whose_scope_the_theme_does_not_style_keeps_the_row_style() {
    // The theme's default sets both colors, so a span that fell back to it
    // would repaint the selected row's highlight.
    let default = colors(Rgb(50, 0, 0), Rgb(60, 0, 0));
    let (theme, Ids { span: id, .. }) = drawer_theme(None, default);
    let row = styled_row("abcd", vec![(1, 3, any_row(id))]);
    let area = Rect::new(0, 0, 10, 2);
    let mut grid = Grid::new(10, 2);
    render(&theme, vec![row], false, &mut grid, area);

    let (x0, y) = (area.x + FRAME_EDGE, area.y + FRAME_EDGE);
    for x in [x0 + 1, x0 + 2] {
        let style = grid[(x, y)].style();
        assert_eq!(style.fg, Some(SELECTED_FG), "span fg at x={x}");
        assert_eq!(style.bg, Some(SELECTED_BG), "span bg at x={x}");
        assert_eq!(
            style.modifiers,
            Modifiers::empty(),
            "span modifiers at x={x}"
        );
    }
}

#[test]
fn a_styled_row_is_clipped_at_the_inner_right_edge() {
    // The span runs from inside the inner width to past its edge. The right
    // border has to survive, and nothing may be written past it.
    let (theme, Ids { span: id, .. }) = drawer_theme(Some(italic()), ResolvedStyle::default());
    let row = styled_row("abcdefghij", vec![(4, 8, any_row(id))]);
    let mut grid = Grid::new(20, 2);
    render(&theme, vec![row], true, &mut grid, Rect::new(2, 0, 8, 2));

    // From the drawer's left edge to the grid's right edge.
    insta::assert_snapshot!(symbols_in(&grid, Rect::new(2, 0, 18, 2)), @r"
    ┌──────┐
    │abcdef│
    ");
}

#[test]
fn text_between_two_spans_is_painted_once_in_the_row_style() {
    let (theme, Ids { span: id, .. }) = drawer_theme(Some(italic()), ResolvedStyle::default());
    let row = styled_row("abcdef", vec![(1, 2, any_row(id)), (3, 4, any_row(id))]);
    let area = Rect::new(0, 0, 10, 2);
    let mut grid = Grid::new(10, 2);
    render(&theme, vec![row], false, &mut grid, area);

    let (x0, y) = (area.x + FRAME_EDGE, area.y + FRAME_EDGE);
    insta::assert_snapshot!(symbols_in(&grid, Rect::new(x0, y, 10 - x0, 1)), @"abcdef");
    let italic = |x: u16| grid[(x, y)].style().modifiers.contains(Modifiers::ITALIC);
    assert!(italic(x0 + 1), "first span cell 'b'");
    assert!(!italic(x0 + 2), "gap cell 'c'");
    assert!(italic(x0 + 3), "second span cell 'd'");
}
