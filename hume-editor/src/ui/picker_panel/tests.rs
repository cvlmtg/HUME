use hume_engine::types::ResolvedStyle;
use hume_grid::{Grid, Rect, Rgb};
use std::collections::HashMap;

use super::super::symbols_in;
use super::*;

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
        rows: rows.iter().map(|s| s.to_string()).collect(),
        selected_row,
        matched: rows.len(),
        total: rows.len(),
        pending: false,
        rect: geo.rect,
        border: true,
        truncate: TruncateEnd::Head,
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
    // A row wider than the panel (e.g. a deep file path from `gf`) must be
    // clipped to inner_width, keeping its tail behind a … marker, instead of
    // bleeding past the right border.
    let mut buf = Grid::new(40, 20);
    let theme = Theme::default();
    let mut canvas = Canvas::new(&mut buf, &theme, None);
    let geo = PanelGeometry {
        rect: rect(0, 0, 12, 4), // inner_width = 10
        list_rows: 1,
    };
    let s = state("", &["hume-editor/src/ui/picker_panel.rs"], None, &geo);
    draw_picker_panel(&mut canvas, &s, styles());

    // Right border must still be an unbroken │ column, not overrun text.
    let right = geo.rect.x + geo.rect.width - 1;
    assert_eq!(
        buf[(right, 2)].text(),
        "│",
        "right border must survive an overlong row"
    );
    let painted = symbols_in(&buf, geo.rect);
    assert!(
        painted.contains("…_panel.rs"),
        "clipped row must lead with … and keep the tail (basename), got:\n{painted}"
    );
}

#[test]
fn draw_picker_panel_truncate_tail_clips_overlong_row_keeping_head() {
    // A grep-style row (path in front, line preview trailing) with
    // `#:truncate 'tail` must clip the *end*, not the path — the mirror of
    // the head-cut test above.
    let mut buf = Grid::new(40, 20);
    let theme = Theme::default();
    let mut canvas = Canvas::new(&mut buf, &theme, None);
    let geo = PanelGeometry {
        rect: rect(0, 0, 23, 4), // inner_width = 21: exactly "src/editor/picker.rs" (20) + …
        list_rows: 1,
    };
    let mut s = state(
        "",
        &["src/editor/picker.rs:412:9:  fn push(&mut self)"],
        None,
        &geo,
    );
    s.truncate = TruncateEnd::Tail;
    draw_picker_panel(&mut canvas, &s, styles());

    let right = geo.rect.x + geo.rect.width - 1;
    assert_eq!(
        buf[(right, 2)].text(),
        "│",
        "right border must survive an overlong row"
    );
    let painted = symbols_in(&buf, geo.rect);
    assert!(
        painted.contains("src/editor/picker.rs…"),
        "clipped row must trail with … and keep the head (path), got:\n{painted}"
    );
}

// ── truncate_marked ─────────────────────────────────────────────────────

#[test]
fn truncate_marked_leaves_short_strings_unchanged() {
    assert_eq!(truncate_marked("hello", 8, TruncateEnd::Head), "hello");
    assert_eq!(
        truncate_marked("hello", 5, TruncateEnd::Head),
        "hello",
        "exact fit needs no marker"
    );
}

#[test]
fn truncate_marked_cut_head_prefixes_ellipsis_and_keeps_tail() {
    let source = "hume-editor/src/ui/picker_panel.rs";
    let out = truncate_marked(source, 12, TruncateEnd::Head);
    assert_eq!(unicode_width::UnicodeWidthStr::width(out.as_ref()), 12);
    let kept = out
        .strip_prefix('…')
        .unwrap_or_else(|| panic!("clipped string must lead with …, got {out:?}"));
    assert!(
        source.ends_with(kept),
        "the kept part must be a genuine tail of the source string, got {out:?}"
    );
}

#[test]
fn truncate_marked_cut_tail_appends_ellipsis_and_keeps_head() {
    let source = "src/editor/picker.rs:412:9:  fn push(&mut self)";
    let out = truncate_marked(source, 12, TruncateEnd::Tail);
    assert_eq!(unicode_width::UnicodeWidthStr::width(out.as_ref()), 12);
    let kept = out
        .strip_suffix('…')
        .unwrap_or_else(|| panic!("clipped string must trail with …, got {out:?}"));
    assert!(
        source.starts_with(kept),
        "the kept part must be a genuine head of the source string, got {out:?}"
    );
}

#[test]
fn truncate_marked_never_splits_a_grapheme_cluster() {
    use unicode_segmentation::UnicodeSegmentation;

    // "é" here is e + combining acute (U+0065 U+0301) — a cut landing inside
    // this cluster would emit a bare combining mark with no base character
    // ahead of it, which re-segmenting the output would catch: a bare mark
    // opens its own (invalid) cluster instead of extending the previous one.
    let source = "cafe\u{0301} bar";
    for cut in [TruncateEnd::Head, TruncateEnd::Tail] {
        // Every clip width up to the source's own — including one that lands
        // mid-cluster if the code were byte-counting instead of grapheme-aware.
        for budget in 0..=8 {
            let out = truncate_marked(source, budget, cut);
            assert!(
                out.graphemes(true).all(|g| g != "\u{0301}"),
                "cut={cut:?} budget={budget}: combining accent split from its base, got {out:?}"
            );
        }
    }
}

#[test]
fn truncate_marked_zero_budget_is_empty() {
    assert_eq!(truncate_marked("anything", 0, TruncateEnd::Head), "");
    assert_eq!(truncate_marked("anything", 0, TruncateEnd::Tail), "");
}

// ── draw_picker_panel ───────────────────────────────────────────────────

#[test]
fn draw_picker_panel_border_input_and_rows_snapshot() {
    let mut buf = Grid::new(20, 20);
    let theme = Theme::default();
    let mut canvas = Canvas::new(&mut buf, &theme, None);
    let geo = PanelGeometry {
        rect: rect(2, 3, 10, 6),
        list_rows: 3,
    };
    let s = state("ab", &["item0", "item1"], Some(0), &geo);
    draw_picker_panel(&mut canvas, &s, styles());

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
    let mut canvas = Canvas::new(&mut buf, &theme, None);
    let geo = PanelGeometry {
        rect: rect(2, 3, 10, 6),
        list_rows: 3,
    };
    let mut s = state("ab", &["item0"], Some(0), &geo);
    s.border = false;
    draw_picker_panel(&mut canvas, &s, styles());

    insta::assert_snapshot!(symbols_in(&buf, geo.rect), @r"

    ab   1/1
    item0

    ");
}

#[test]
fn draw_picker_panel_highlights_selected_row_full_width() {
    let mut buf = Grid::new(20, 20);
    let theme = Theme::default();
    let mut canvas = Canvas::new(&mut buf, &theme, None);
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
    );

    // Row 1 ("item1") is the selected row — its whole inner width (cols
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
    let mut canvas = Canvas::new(&mut buf, &theme, None);
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
    draw_picker_panel(&mut canvas, &s, styles());

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
    let mut canvas = Canvas::new(&mut buf, &theme, None);
    let geo = PanelGeometry {
        rect: rect(2, 3, 14, 6),
        list_rows: 3,
    };
    let mut s = state("ab", &[], None, &geo);
    s.pending = true;
    draw_picker_panel(&mut canvas, &s, styles());

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
    let mut canvas = Canvas::new(&mut buf, &theme, None);
    let mut s = state("q", &[], None, &geo_wide);
    s.matched = 3;
    s.total = 42;
    draw_picker_panel(&mut canvas, &s, styles());
    assert!(symbols_in(&buf, geo_wide.rect).contains("3/42"));

    let geo_narrow = PanelGeometry {
        rect: rect(0, 0, 6, 4),
        list_rows: 1,
    };
    let mut buf2 = Grid::new(20, 20);
    let mut canvas2 = Canvas::new(&mut buf2, &theme, None);
    let mut s2 = state("q", &[], None, &geo_narrow);
    s2.matched = 3;
    s2.total = 42;
    draw_picker_panel(&mut canvas2, &s2, styles());
    assert!(
        !symbols_in(&buf2, geo_narrow.rect).contains("3/42"),
        "counts must be dropped rather than overlap the query/cursor"
    );
}

#[test]
fn draw_picker_panel_truncates_query_tail_keeping_cursor_visible() {
    let mut buf = Grid::new(20, 20);
    let theme = Theme::default();
    let mut canvas = Canvas::new(&mut buf, &theme, None);
    let geo = PanelGeometry {
        rect: rect(0, 0, 10, 4), // inner_width = 8; counts "0/0" leaves a 3-col query budget
        list_rows: 1,
    };
    let s = state("abcdefghij", &[], None, &geo); // matched = total = 0
    draw_picker_panel(&mut canvas, &s, styles());

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
    let mut canvas = Canvas::new(&mut buf, &theme, None);
    let geo = PanelGeometry {
        rect: rect(0, 0, 12, 4), // inner_width = 10; counts "0/0" leaves a 2-col query budget after "f: "
        list_rows: 1,
    };
    let mut s = state("ab", &[], None, &geo); // matched = total = 0
    s.prompt = "f: ".to_string();
    draw_picker_panel(&mut canvas, &s, styles());

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
    let mut canvas = Canvas::new(&mut buf, &theme, None);
    let geo = PanelGeometry {
        rect: rect(0, 0, 7, 4), // inner_width = 5, narrower than the prompt alone
        list_rows: 1,
    };
    let mut s = state("query text", &["row"], Some(0), &geo);
    s.prompt = "much longer prompt than the panel".to_string();

    // Must not panic even though the prompt alone exceeds inner_width, and
    // must leave the query with zero budget rather than overflow the row.
    draw_picker_panel(&mut canvas, &s, styles());

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
    let mut canvas = Canvas::new(&mut buf, &theme, None);
    let geo = PanelGeometry {
        rect: rect(0, 0, 12, 6),
        list_rows: 3,
    };
    let s = state("", &[], None, &geo);
    draw_picker_panel(&mut canvas, &s, styles());
    let painted = symbols_in(&buf, geo.rect);
    assert!(painted.contains("0/0"));
}

#[test]
fn draw_picker_panel_degenerate_rect_does_not_panic_or_paint() {
    let mut buf = Grid::new(20, 20);
    let before = buf.clone();
    let theme = Theme::default();
    let mut canvas = Canvas::new(&mut buf, &theme, None);
    let geo = PanelGeometry {
        rect: rect(0, 0, 2, 2),
        list_rows: 0,
    };
    let s = state("q", &["item0"], Some(0), &geo);
    draw_picker_panel(&mut canvas, &s, styles());
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
        rect: rect(100, 100, 10, 6),
        list_rows: 3,
    };
    let s = state("q", &["item0"], Some(0), &geo);
    let overlay = PickerOverlay {
        data: Arc::new(RwLock::new(Some(s))),
    };
    assert!(overlay.is_active());

    let mut buf = Grid::new(20, 20);
    let before = buf.clone();
    let theme = Theme::new(HashMap::new(), ResolvedStyle::default());
    overlay.render(
        Rect::new(0, 0, 20, 20),
        &theme,
        &mut Canvas::new(&mut buf, &theme, None),
    );
    assert_eq!(
        buf, before,
        "state positioned outside pane_rect must not paint"
    );
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
