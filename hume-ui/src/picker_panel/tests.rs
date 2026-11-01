use hume_engine::lock::SharedSlot;
use hume_engine::types::ResolvedStyle;
use hume_grid::{Grid, Rect, Rgb};
use std::collections::HashMap;

use super::super::symbols_in;
use super::*;
use crate::list_row::SpanScopes;
use hume_engine::theme::ScopeRegistry;
use hume_engine::types::{Modifiers, ScopeId};
use hume_rope::spanned::SpannedText;

fn rect(x: u16, y: u16, w: u16, h: u16) -> Rect {
    Rect::new(x, y, w, h)
}

fn style() -> ResolvedStyle {
    ResolvedStyle::default()
}

fn styles() -> PickerStyles {
    PickerStyles {
        background: style(),
        text: style(),
        selected: style(),
        cursor: style(),
    }
}

fn state(
    query: &str,
    rows: &[&str],
    selected_row: Option<usize>,
    geo: &PanelGeometry,
) -> PickerViewState {
    PickerViewState {
        prompt: String::new(),
        query: query.to_string(),
        rows: rows
            .iter()
            .map(|s| Some(Arc::new(ListRow::plain(s.to_string()))))
            .collect(),
        selected_row,
        matched: rows.len(),
        total: rows.len(),
        pending: false,
        rect: geo.rect,
        list_rows: geo.list_rows,
        border: true,
    }
}

// ── panel_geometry ────────────────────────────────────────────────────────

#[test]
fn geometry_caps_at_80_percent_width_and_100_cols() {
    let geo = panel_geometry(rect(0, 0, 200, 100)).expect("viable region");
    assert_eq!(geo.rect.width, 100, "80% of 200 is 160, capped to 100");
    assert_eq!(geo.rect.x, 50, "centered: (200 - 100) / 2");
}

#[test]
fn geometry_caps_at_60_percent_height_and_30_rows() {
    let geo = panel_geometry(rect(0, 0, 200, 100)).expect("viable region");
    assert_eq!(geo.rect.height, 30, "60% of 100 is 60, capped to 30");
    assert_eq!(geo.rect.y, 35, "centered: (100 - 30) / 2");
    assert_eq!(geo.list_rows, 27, "height - 3 (borders + input row)");
}

#[test]
fn geometry_uses_percentage_when_below_caps() {
    let geo = panel_geometry(rect(0, 0, 50, 20)).expect("viable region");
    assert_eq!(geo.rect.width, 40, "80% of 50, under the 100-col cap");
    assert_eq!(geo.rect.height, 12, "60% of 20, under the 30-row cap");
    assert_eq!(geo.rect.x, 5);
    assert_eq!(geo.rect.y, 4);
}

#[test]
fn geometry_clamps_to_a_tiny_region_without_panicking() {
    let geo = panel_geometry(rect(0, 0, 4, 10)).expect("just viable: width 3, height 6");
    assert_eq!(geo.rect.width, 3);
    assert_eq!(geo.rect.height, 6);
    assert_eq!(geo.list_rows, 3);
}

#[test]
fn geometry_none_when_height_below_minimum() {
    // width = 8 (viable), height = 60% of 5 = 3, below the 4-row minimum.
    assert!(panel_geometry(rect(0, 0, 10, 5)).is_none());
}

#[test]
fn geometry_none_when_width_below_minimum() {
    // width = 80% of 3 = 2, below the 3-col minimum.
    assert!(panel_geometry(rect(0, 0, 3, 20)).is_none());
}

#[test]
fn draw_picker_panel_clips_overlong_row_to_inner_width() {
    // Rows arrive already cut to the list width. One that is still wider
    // must stop at the inner edge rather than bleed past the right border.
    let mut buf = Grid::new(40, 20);
    let theme = Theme::default();
    let mut canvas = Canvas::new(&mut buf, theme.ui.invisible, None);
    let geo = PanelGeometry {
        rect: rect(0, 0, 12, 4), // inner_width = 10
        list_rows: 1,
    };
    let s = state("", &["hume-ui/src/picker_panel.rs"], None, &geo);
    draw_picker_panel(&mut canvas, &s, styles(), &theme);

    // Right border must still be an unbroken │ column, not overrun text.
    let right = geo.rect.x + geo.rect.width - 1;
    assert_eq!(
        buf[(right, 2)].text(),
        "│",
        "right border must survive an overlong row"
    );
    insta::assert_snapshot!(symbols_in(&buf, Rect::new(0, 2, 12, 1)), @"│hume-ui/sr│");
}

// ── draw_picker_panel ───────────────────────────────────────────────────

#[test]
fn draw_picker_panel_border_input_and_rows_snapshot() {
    let mut buf = Grid::new(20, 20);
    let theme = Theme::default();
    let mut canvas = Canvas::new(&mut buf, theme.ui.invisible, None);
    let geo = PanelGeometry {
        rect: rect(2, 3, 10, 6),
        list_rows: 3,
    };
    let s = state("ab", &["item0", "item1"], Some(0), &geo);
    draw_picker_panel(&mut canvas, &s, styles(), &theme);

    insta::assert_snapshot!(symbols_in(&buf, geo.rect), @r"
    ┌────────┐
    │ab   2/2│
    │item0   │
    │item1   │
    │        │
    └────────┘
    ");
}

#[test]
fn draw_picker_panel_no_border_leaves_plain_margin() {
    let mut buf = Grid::new(20, 20);
    let theme = Theme::default();
    let mut canvas = Canvas::new(&mut buf, theme.ui.invisible, None);
    let geo = PanelGeometry {
        rect: rect(2, 3, 10, 6),
        list_rows: 3,
    };
    let mut s = state("ab", &["item0"], Some(0), &geo);
    s.border = false;
    draw_picker_panel(&mut canvas, &s, styles(), &theme);

    insta::assert_snapshot!(symbols_in(&buf, geo.rect), @r"

    ab   1/1
    item0

    ");
}

#[test]
fn draw_picker_panel_highlights_selected_row_full_width() {
    let mut buf = Grid::new(20, 20);
    let theme = Theme::default();
    let mut canvas = Canvas::new(&mut buf, theme.ui.invisible, None);
    let geo = PanelGeometry {
        rect: rect(0, 0, 10, 6),
        list_rows: 3,
    };
    let selected_style = ResolvedStyle {
        bg: Some(Rgb(255, 0, 0)),
        ..Default::default()
    };
    let s = state("", &["item0", "item1"], Some(1), &geo);
    draw_picker_panel(
        &mut canvas,
        &s,
        PickerStyles {
            selected: selected_style,
            ..styles()
        },
        &theme,
    );

    // Row 1 ("item1") is the selected row: its whole inner width (cols
    // 1..=8) must carry the selected background, not just the text cells.
    for x in 1..=8u16 {
        assert_eq!(
            buf[(x, 3)].style().bg,
            Some(Rgb(255, 0, 0)),
            "selected row must be highlighted across the full inner width at x={x}"
        );
    }
    // The non-selected row above it must not be.
    for x in 1..=8u16 {
        assert_ne!(buf[(x, 2)].style().bg, Some(Rgb(255, 0, 0)));
    }
}

#[test]
fn draw_picker_panel_does_not_rewindow_rows() {
    // The store already scrolled `rows` to fit `list_rows`; the widget must
    // only ever paint up to its own row capacity, ignoring any overflow
    // rather than trying to re-window it.
    let mut buf = Grid::new(20, 20);
    let theme = Theme::default();
    let mut canvas = Canvas::new(&mut buf, theme.ui.invisible, None);
    let geo = PanelGeometry {
        rect: rect(0, 0, 12, 6), // list_rows = 3
        list_rows: 3,
    };
    let s = state(
        "",
        &["item0", "item1", "item2", "item3", "item4"],
        None,
        &geo,
    );
    draw_picker_panel(&mut canvas, &s, styles(), &theme);

    let painted = symbols_in(&buf, geo.rect);
    assert!(painted.contains("item2"), "last row within capacity");
    assert!(
        !painted.contains("item3"),
        "beyond list_rows must not paint"
    );
}

#[test]
fn draw_picker_panel_pending_marks_the_counter_snapshot() {
    let mut buf = Grid::new(20, 20);
    let theme = Theme::default();
    let mut canvas = Canvas::new(&mut buf, theme.ui.invisible, None);
    let geo = PanelGeometry {
        rect: rect(2, 3, 14, 6),
        list_rows: 3,
    };
    let mut s = state("ab", &[], None, &geo);
    s.pending = true;
    draw_picker_panel(&mut canvas, &s, styles(), &theme);

    insta::assert_snapshot!(symbols_in(&buf, geo.rect), @r"
    ┌────────────┐
    │ab     0/0 …│
    │            │
    │            │
    │            │
    └────────────┘
    ");
}

#[test]
fn draw_picker_panel_counts_shown_when_room_and_dropped_when_narrow() {
    let geo_wide = PanelGeometry {
        rect: rect(0, 0, 20, 4),
        list_rows: 1,
    };
    let mut buf = Grid::new(20, 20);
    let theme = Theme::default();
    let mut canvas = Canvas::new(&mut buf, theme.ui.invisible, None);
    let mut s = state("q", &[], None, &geo_wide);
    s.matched = 3;
    s.total = 42;
    draw_picker_panel(&mut canvas, &s, styles(), &theme);
    assert!(symbols_in(&buf, geo_wide.rect).contains("3/42"));

    let geo_narrow = PanelGeometry {
        rect: rect(0, 0, 6, 4),
        list_rows: 1,
    };
    let mut buf2 = Grid::new(20, 20);
    let mut canvas2 = Canvas::new(&mut buf2, theme.ui.invisible, None);
    let mut s2 = state("q", &[], None, &geo_narrow);
    s2.matched = 3;
    s2.total = 42;
    draw_picker_panel(&mut canvas2, &s2, styles(), &theme);
    assert!(
        !symbols_in(&buf2, geo_narrow.rect).contains("3/42"),
        "counts must be dropped rather than overlap the query/cursor"
    );
}

#[test]
fn draw_picker_panel_truncates_query_tail_keeping_cursor_visible() {
    let mut buf = Grid::new(20, 20);
    let theme = Theme::default();
    let mut canvas = Canvas::new(&mut buf, theme.ui.invisible, None);
    let geo = PanelGeometry {
        rect: rect(0, 0, 10, 4), // inner_width = 8; counts "0/0" leaves a 3-col query budget
        list_rows: 1,
    };
    let s = state("abcdefghij", &[], None, &geo); // matched = total = 0
    draw_picker_panel(&mut canvas, &s, styles(), &theme);

    let inner_x = geo.rect.x + 1;
    let row: String = (inner_x..inner_x + 8)
        .map(|x| buf[(x, 1)].text().to_string())
        .collect();
    // Query truncated to its tail ("hij") so the cursor cell right after it
    // stays inside the input row, instead of the head of the query.
    assert!(
        row.starts_with("hij"),
        "expected tail truncation, got {row:?}"
    );
}

#[test]
fn draw_picker_panel_renders_prompt_before_query() {
    let mut buf = Grid::new(20, 20);
    let theme = Theme::default();
    let mut canvas = Canvas::new(&mut buf, theme.ui.invisible, None);
    let geo = PanelGeometry {
        rect: rect(0, 0, 12, 4), // inner_width = 10; counts "0/0" leaves a 2-col query budget after "f: "
        list_rows: 1,
    };
    let mut s = state("ab", &[], None, &geo); // matched = total = 0
    s.prompt = "f: ".to_string();
    draw_picker_panel(&mut canvas, &s, styles(), &theme);

    let inner_x = geo.rect.x + 1;
    let row: String = (inner_x..inner_x + 10)
        .map(|x| buf[(x, 1)].text().to_string())
        .collect();
    assert!(
        row.starts_with("f: ab"),
        "expected prompt painted before the query, got {row:?}"
    );
}

#[test]
fn draw_picker_panel_prompt_wider_than_panel_clips_without_panic() {
    let mut buf = Grid::new(20, 20);
    let theme = Theme::default();
    let mut canvas = Canvas::new(&mut buf, theme.ui.invisible, None);
    let geo = PanelGeometry {
        rect: rect(0, 0, 7, 4), // inner_width = 5, narrower than the prompt alone
        list_rows: 1,
    };
    let mut s = state("query text", &["row"], Some(0), &geo);
    s.prompt = "much longer prompt than the panel".to_string();

    // Must not panic even though the prompt alone exceeds inner_width, and
    // must leave the query with zero budget rather than overflow the row.
    draw_picker_panel(&mut canvas, &s, styles(), &theme);

    let inner_x = geo.rect.x + 1;
    let row: String = (inner_x..inner_x + 5)
        .map(|x| buf[(x, 1)].text().to_string())
        .collect();
    assert_eq!(
        row, "much ",
        "prompt clipped to the inner width, query dropped"
    );
}

#[test]
fn draw_picker_panel_empty_state_is_blank_with_zero_counts() {
    let mut buf = Grid::new(20, 20);
    let theme = Theme::default();
    let mut canvas = Canvas::new(&mut buf, theme.ui.invisible, None);
    let geo = PanelGeometry {
        rect: rect(0, 0, 12, 6),
        list_rows: 3,
    };
    let s = state("", &[], None, &geo);
    draw_picker_panel(&mut canvas, &s, styles(), &theme);
    let painted = symbols_in(&buf, geo.rect);
    assert!(painted.contains("0/0"));
}

#[test]
fn draw_picker_panel_degenerate_rect_does_not_panic_or_paint() {
    let mut buf = Grid::new(20, 20);
    let before = buf.clone();
    let theme = Theme::default();
    let mut canvas = Canvas::new(&mut buf, theme.ui.invisible, None);
    let geo = PanelGeometry {
        rect: rect(0, 0, 2, 2),
        list_rows: 0,
    };
    let s = state("q", &["item0"], Some(0), &geo);
    draw_picker_panel(&mut canvas, &s, styles(), &theme);
    assert_eq!(buf, before, "sub-3x4 outer must not panic or paint");
}

// ── PickerOverlay ─────────────────────────────────────────────────────────

#[test]
fn overlay_is_inactive_when_data_is_none() {
    let overlay = PickerOverlay {
        data: SharedSlot::new(None),
    };
    assert!(!overlay.is_active());
}

// ── picker_styles: direct Helix-scope resolution ────────────────────────────

fn resolved(fg: Rgb) -> ResolvedStyle {
    ResolvedStyle {
        fg: Some(fg),
        ..Default::default()
    }
}

#[test]
fn picker_styles_resolve_from_helix_scopes() {
    let mut m = HashMap::new();
    m.insert("ui.background", resolved(Rgb(0, 0, 0)));
    m.insert("ui.text", resolved(Rgb(0, 0, 255)));
    m.insert("ui.text.focus", resolved(Rgb(0, 255, 0)));
    m.insert("ui.cursor.primary", resolved(Rgb(255, 255, 0)));
    let theme = Theme::new(m, ResolvedStyle::default());

    let styles = picker_styles(&theme);
    assert_eq!(styles.background.fg, Some(Rgb(0, 0, 0)), "ui.background");
    assert_eq!(styles.text.fg, Some(Rgb(0, 0, 255)), "ui.text");
    assert_eq!(styles.selected.fg, Some(Rgb(0, 255, 0)), "ui.text.focus");
    assert_eq!(
        styles.cursor.fg,
        Some(Rgb(255, 255, 0)),
        "ui.cursor.primary"
    );
}

#[test]
fn picker_styles_default_when_scopes_absent() {
    // No custom aliasing: an empty theme resolves every picker scope to the
    // theme's plain `default`, exactly like any other unset scope.
    let theme = Theme::new(HashMap::new(), resolved(Rgb(255, 0, 0)));

    let styles = picker_styles(&theme);
    assert_eq!(styles.background.fg, Some(Rgb(255, 0, 0)));
    assert_eq!(styles.text.fg, Some(Rgb(255, 0, 0)));
    assert_eq!(styles.selected.fg, Some(Rgb(255, 0, 0)));
    assert_eq!(styles.cursor.fg, Some(Rgb(255, 0, 0)));
}

#[test]
fn draw_picker_panel_reversed_cursor_swaps_against_the_panels_own_text_color() {
    // `ui.cursor.primary` carrying only `modifiers = ["reversed"]` (no fg/bg
    // of its own, as upstream Helix's gruvbox_light defines it) needs a
    // concrete color already painted under the cursor cell to swap against.
    // `ui.background` (the panel fill) is bg-only by Helix convention, so
    // the panel's own `ui.text` fg is what the swap must land on.
    let mut buf = Grid::new(40, 20);
    let theme = Theme::default();
    let mut canvas = Canvas::new(&mut buf, theme.ui.invisible, None);
    let geo = PanelGeometry {
        rect: rect(0, 0, 20, 4),
        list_rows: 1,
    };
    let s = state("", &[], None, &geo);
    let text_fg = Rgb(10, 20, 30);
    let background_bg = Rgb(1, 1, 1);
    let styles = PickerStyles {
        background: ResolvedStyle {
            bg: Some(background_bg),
            ..Default::default()
        },
        text: ResolvedStyle {
            fg: Some(text_fg),
            ..Default::default()
        },
        selected: style(),
        cursor: ResolvedStyle {
            modifiers: hume_engine::types::Modifiers::REVERSED,
            ..Default::default()
        },
    };
    draw_picker_panel(&mut canvas, &s, styles, &theme);

    // Query cursor: row 1, right after the empty query at the inner left edge.
    let cursor_cell = buf[(geo.rect.x + 1, geo.rect.y + 1)].style();
    assert_eq!(cursor_cell.fg, Some(background_bg));
    assert_eq!(cursor_cell.bg, Some(text_fg));
    assert!(
        !cursor_cell
            .modifiers
            .contains(hume_engine::types::Modifiers::REVERSED)
    );
}

#[test]
fn a_rendered_rows_span_layers_over_ui_text_focus() {
    let mut registry = ScopeRegistry::new();
    let span: ScopeId = registry.intern("span");
    let focus_bg = Rgb(9, 9, 9);
    let mut theme = Theme::new(
        HashMap::from([(
            "span",
            ResolvedStyle {
                modifiers: Modifiers::ITALIC,
                ..Default::default()
            },
        )]),
        ResolvedStyle::default(),
    );
    theme.bake(&registry);
    let mut buf = Grid::new(20, 20);
    let mut canvas = Canvas::new(&mut buf, theme.ui.invisible, None);
    let geo = PanelGeometry {
        rect: rect(0, 0, 10, 5),
        list_rows: 2,
    };
    let scopes = SpanScopes {
        scope: span,
        selected: None,
    };
    let row = ListRow::styled(SpannedText::new("abcd".to_string(), vec![(1, 3, scopes)]).unwrap());
    let mut s = state("", &[], Some(0), &geo);
    s.rows = vec![Some(Arc::new(row))];
    draw_picker_panel(
        &mut canvas,
        &s,
        PickerStyles {
            selected: ResolvedStyle {
                bg: Some(focus_bg),
                ..Default::default()
            },
            ..styles()
        },
        &theme,
    );

    // The list's first row sits below the input row, inside the frame.
    let italic = |x: u16| buf[(x, 2)].style().modifiers.contains(Modifiers::ITALIC);
    for x in [2, 3] {
        assert_eq!(buf[(x, 2)].style().bg, Some(focus_bg), "x={x}");
        assert!(italic(x), "span italic at x={x}");
    }
    assert!(!italic(1) && !italic(4), "outside the span");
}
