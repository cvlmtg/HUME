use std::collections::HashMap;

use ratatui::style::Color;

use hume_engine::types::ResolvedStyle;

use super::*;

fn rect(x: u16, y: u16, w: u16, h: u16) -> Rect {
    Rect::new(x, y, w, h)
}

fn style() -> Style {
    Style::default()
}

/// Rows of `area` as plain symbols, trailing spaces trimmed per row —
/// mirrors `menu_box/tests.rs`'s helper of the same name.
fn symbols_in(buf: &ScreenBuf, area: Rect) -> String {
    (area.y..area.y + area.height)
        .map(|y| {
            let row: String = (area.x..area.x + area.width)
                .map(|x| buf[(x, y)].symbol())
                .collect();
            row.trim_end().to_string()
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn state(query: &str, rows: &[&str], selected_row: Option<usize>, geo: &PanelGeometry) -> PickerViewState {
    PickerViewState {
        query: query.to_string(),
        rows: rows.iter().map(|s| s.to_string()).collect(),
        selected_row,
        matched: rows.len(),
        total: rows.len(),
        x: geo.x,
        y: geo.y,
        width: geo.width,
        height: geo.height,
        border: true,
    }
}

// ── panel_geometry ────────────────────────────────────────────────────────

#[test]
fn geometry_caps_at_80_percent_width_and_100_cols() {
    let geo = panel_geometry(rect(0, 0, 200, 100)).expect("viable region");
    assert_eq!(geo.width, 100, "80% of 200 is 160, capped to 100");
    assert_eq!(geo.x, 50, "centered: (200 - 100) / 2");
}

#[test]
fn geometry_caps_at_60_percent_height_and_30_rows() {
    let geo = panel_geometry(rect(0, 0, 200, 100)).expect("viable region");
    assert_eq!(geo.height, 30, "60% of 100 is 60, capped to 30");
    assert_eq!(geo.y, 35, "centered: (100 - 30) / 2");
    assert_eq!(geo.list_rows, 27, "height - 3 (borders + input row)");
}

#[test]
fn geometry_uses_percentage_when_below_caps() {
    let geo = panel_geometry(rect(0, 0, 50, 20)).expect("viable region");
    assert_eq!(geo.width, 40, "80% of 50, under the 100-col cap");
    assert_eq!(geo.height, 12, "60% of 20, under the 30-row cap");
    assert_eq!(geo.x, 5);
    assert_eq!(geo.y, 4);
}

#[test]
fn geometry_clamps_to_a_tiny_region_without_panicking() {
    let geo = panel_geometry(rect(0, 0, 4, 10)).expect("just viable: width 3, height 6");
    assert_eq!(geo.width, 3);
    assert_eq!(geo.height, 6);
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

// ── draw_picker_panel ───────────────────────────────────────────────────

#[test]
fn draw_picker_panel_border_input_and_rows_snapshot() {
    let mut buf = ScreenBuf::empty(Rect::new(0, 0, 20, 20));
    let geo = PanelGeometry {
        x: 2,
        y: 3,
        width: 10,
        height: 6,
        list_rows: 3,
    };
    let s = state("ab", &["item0", "item1"], Some(0), &geo);
    let outer = Rect::new(geo.x, geo.y, geo.width, geo.height);
    draw_picker_panel(&mut buf, &s, style(), style(), style());

    insta::assert_snapshot!(symbols_in(&buf, outer), @r"
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
    let mut buf = ScreenBuf::empty(Rect::new(0, 0, 20, 20));
    let geo = PanelGeometry {
        x: 2,
        y: 3,
        width: 10,
        height: 6,
        list_rows: 3,
    };
    let mut s = state("ab", &["item0"], Some(0), &geo);
    s.border = false;
    let outer = Rect::new(geo.x, geo.y, geo.width, geo.height);
    draw_picker_panel(&mut buf, &s, style(), style(), style());

    insta::assert_snapshot!(symbols_in(&buf, outer), @r"

    ab   1/1
    item0

    ");
}

#[test]
fn draw_picker_panel_highlights_selected_row_full_width() {
    let mut buf = ScreenBuf::empty(Rect::new(0, 0, 20, 20));
    let geo = PanelGeometry {
        x: 0,
        y: 0,
        width: 10,
        height: 6,
        list_rows: 3,
    };
    let selected_style = Style::default().bg(Color::Red);
    let s = state("", &["item0", "item1"], Some(1), &geo);
    draw_picker_panel(&mut buf, &s, style(), selected_style, style());

    // Row 1 ("item1") is the selected row — its whole inner width (cols
    // 1..=8) must carry the selected background, not just the text cells.
    for x in 1..=8u16 {
        assert_eq!(
            buf[(x, 3)].style().bg,
            Some(Color::Red),
            "selected row must be highlighted across the full inner width at x={x}"
        );
    }
    // The non-selected row above it must not be.
    for x in 1..=8u16 {
        assert_ne!(buf[(x, 2)].style().bg, Some(Color::Red));
    }
}

#[test]
fn draw_picker_panel_does_not_rewindow_rows() {
    // The store already scrolled `rows` to fit `list_rows`; the widget must
    // only ever paint up to its own row capacity, ignoring any overflow
    // rather than trying to re-window it.
    let mut buf = ScreenBuf::empty(Rect::new(0, 0, 20, 20));
    let geo = PanelGeometry {
        x: 0,
        y: 0,
        width: 12,
        height: 6, // list_rows = 3
        list_rows: 3,
    };
    let s = state(
        "",
        &["item0", "item1", "item2", "item3", "item4"],
        None,
        &geo,
    );
    draw_picker_panel(&mut buf, &s, style(), style(), style());

    let outer = Rect::new(geo.x, geo.y, geo.width, geo.height);
    let painted = symbols_in(&buf, outer);
    assert!(painted.contains("item2"), "last row within capacity");
    assert!(!painted.contains("item3"), "beyond list_rows must not paint");
}

#[test]
fn draw_picker_panel_counts_shown_when_room_and_dropped_when_narrow() {
    let geo_wide = PanelGeometry {
        x: 0,
        y: 0,
        width: 20,
        height: 4,
        list_rows: 1,
    };
    let mut buf = ScreenBuf::empty(Rect::new(0, 0, 20, 20));
    let mut s = state("q", &[], None, &geo_wide);
    s.matched = 3;
    s.total = 42;
    draw_picker_panel(&mut buf, &s, style(), style(), style());
    let outer = Rect::new(geo_wide.x, geo_wide.y, geo_wide.width, geo_wide.height);
    assert!(symbols_in(&buf, outer).contains("3/42"));

    let geo_narrow = PanelGeometry {
        x: 0,
        y: 0,
        width: 6,
        height: 4,
        list_rows: 1,
    };
    let mut buf2 = ScreenBuf::empty(Rect::new(0, 0, 20, 20));
    let mut s2 = state("q", &[], None, &geo_narrow);
    s2.matched = 3;
    s2.total = 42;
    draw_picker_panel(&mut buf2, &s2, style(), style(), style());
    let outer2 = Rect::new(geo_narrow.x, geo_narrow.y, geo_narrow.width, geo_narrow.height);
    assert!(
        !symbols_in(&buf2, outer2).contains("3/42"),
        "counts must be dropped rather than overlap the query/cursor"
    );
}

#[test]
fn draw_picker_panel_truncates_query_tail_keeping_cursor_visible() {
    let mut buf = ScreenBuf::empty(Rect::new(0, 0, 20, 20));
    let geo = PanelGeometry {
        x: 0,
        y: 0,
        width: 10, // inner_width = 8; counts "0/0" leaves a 3-col query budget
        height: 4,
        list_rows: 1,
    };
    let s = state("abcdefghij", &[], None, &geo); // matched = total = 0
    draw_picker_panel(&mut buf, &s, style(), style(), style());

    let inner_x = geo.x + 1;
    let row: String = (inner_x..inner_x + 8)
        .map(|x| buf[(x, 1)].symbol().to_string())
        .collect();
    // Query truncated to its tail ("hij") so the cursor cell right after it
    // stays inside the input row, instead of the head of the query.
    assert!(row.starts_with("hij"), "expected tail truncation, got {row:?}");
}

#[test]
fn draw_picker_panel_empty_state_is_blank_with_zero_counts() {
    let mut buf = ScreenBuf::empty(Rect::new(0, 0, 20, 20));
    let geo = PanelGeometry {
        x: 0,
        y: 0,
        width: 12,
        height: 6,
        list_rows: 3,
    };
    let s = state("", &[], None, &geo);
    draw_picker_panel(&mut buf, &s, style(), style(), style());
    let outer = Rect::new(geo.x, geo.y, geo.width, geo.height);
    let painted = symbols_in(&buf, outer);
    assert!(painted.contains("0/0"));
}

#[test]
fn draw_picker_panel_degenerate_rect_does_not_panic_or_paint() {
    let mut buf = ScreenBuf::empty(Rect::new(0, 0, 20, 20));
    let before = buf.clone();
    let geo = PanelGeometry {
        x: 0,
        y: 0,
        width: 2,
        height: 2,
        list_rows: 0,
    };
    let s = state("q", &["item0"], Some(0), &geo);
    draw_picker_panel(&mut buf, &s, style(), style(), style());
    assert_eq!(buf, before, "sub-3x4 outer must not panic or paint");
}

// ── PickerOverlay ─────────────────────────────────────────────────────────

#[test]
fn overlay_is_inactive_when_data_is_none() {
    let overlay = PickerOverlay {
        data: Arc::new(RwLock::new(None)),
    };
    assert!(!overlay.is_active());
}

#[test]
fn overlay_clips_state_outside_pane_rect() {
    let geo = PanelGeometry {
        x: 100,
        y: 100,
        width: 10,
        height: 6,
        list_rows: 3,
    };
    let s = state("q", &["item0"], Some(0), &geo);
    let overlay = PickerOverlay {
        data: Arc::new(RwLock::new(Some(s))),
    };
    assert!(overlay.is_active());

    let mut buf = ScreenBuf::empty(Rect::new(0, 0, 20, 20));
    let before = buf.clone();
    let theme = Theme::new(HashMap::new(), ResolvedStyle::default());
    overlay.render(Rect::new(0, 0, 20, 20), &theme, &mut buf);
    assert_eq!(buf, before, "state positioned outside pane_rect must not paint");
}

// ── picker_styles fallback aliasing ────────────────────────────────────────

fn resolved(fg: Color) -> ResolvedStyle {
    ResolvedStyle {
        fg: Some(fg),
        ..Default::default()
    }
}

#[test]
fn picker_styles_alias_to_menu_family_when_absent() {
    let mut m = HashMap::new();
    m.insert("ui.menu", resolved(Color::Blue));
    m.insert("ui.menu.selected", resolved(Color::Green));
    let theme = Theme::new(m, ResolvedStyle::default());

    let styles = picker_styles(&theme);
    assert_eq!(styles.base.fg, Some(Color::Blue), "ui.picker absent -> ui.menu");
    assert_eq!(
        styles.selected.fg,
        Some(Color::Green),
        "ui.picker.selected absent -> ui.menu.selected"
    );
    assert_eq!(styles.input.fg, Some(Color::Blue), "ui.picker.input absent, ui.picker absent -> ui.menu");
}

#[test]
fn picker_styles_prefer_own_scopes_when_present() {
    let mut m = HashMap::new();
    m.insert("ui.menu", resolved(Color::Blue));
    m.insert("ui.picker", resolved(Color::Yellow));
    m.insert("ui.picker.selected", resolved(Color::Magenta));
    let theme = Theme::new(m, ResolvedStyle::default());

    let styles = picker_styles(&theme);
    assert_eq!(styles.base.fg, Some(Color::Yellow));
    assert_eq!(styles.selected.fg, Some(Color::Magenta));
}

#[test]
fn picker_styles_input_falls_back_to_picker_before_menu() {
    let mut m = HashMap::new();
    m.insert("ui.menu", resolved(Color::Blue));
    m.insert("ui.picker", resolved(Color::Yellow));
    // No "ui.picker.input" entry.
    let theme = Theme::new(m, ResolvedStyle::default());

    let styles = picker_styles(&theme);
    assert_eq!(
        styles.input.fg,
        Some(Color::Yellow),
        "ui.picker.input absent, but ui.picker present -> ui.picker, not ui.menu"
    );
}
