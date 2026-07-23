use super::*;
use crate::pane::ViewportState;
use crate::theme::Theme;
use crate::types::{CellContent, DisplayRow, Grapheme, ResolvedStyle, RowKind};

fn make_test_buf(w: u16, h: u16) -> ratatui::buffer::Buffer {
    ratatui::buffer::Buffer::empty(ratatui::layout::Rect {
        x: 0,
        y: 0,
        width: w,
        height: h,
    })
}

fn simple_row(graphemes: std::ops::Range<usize>) -> DisplayRow {
    DisplayRow {
        kind: RowKind::LineStart { line_idx: 0 },
        graphemes,
    }
}

fn simple_grapheme(col: u16, byte_start: usize, ch_len: usize) -> Grapheme {
    Grapheme {
        byte_range: byte_start..byte_start + ch_len,
        // char_offset is not needed for render tests (selections handled in style stage).
        char_offset: byte_start,
        col,
        width: 1,
        content: CellContent::Grapheme,
        indent_depth: 0,
        scope: None,
    }
}

#[test]
fn renders_simple_text() {
    let graphemes = vec![simple_grapheme(0, 0, 1), simple_grapheme(1, 1, 1)];
    let rows = [simple_row(0..2)];
    let styles = vec![ResolvedStyle::default(); 2];
    let visible = VisibleRange {
        line_range: 0..1,
        top_skip_rows: 0,
        content_height: 5,
        content_width: 20,
        gutter_width: 0,
        last_line_idx: 0,
    };
    let viewport = ViewportState::new(20, 5);
    let pane_rect = ratatui::layout::Rect {
        x: 0,
        y: 0,
        width: 20,
        height: 5,
    };
    let mut buf = make_test_buf(20, 5);
    let theme = Theme::default();
    let col_widths: Vec<u16> = Vec::new();
    let rope = ropey::Rope::new();
    let ctx = ComposeCtx {
        gutter_columns: &[],
        visible: &visible,
        viewport: &viewport,
        mode: EditorMode::Normal,
        primary_head_line: 0,
        tab_width: 4,
        tilde_style: ratatui::style::Style::default(),
        indent_guide_style: ratatui::style::Style::default(),
        show_indent_guides: true,
        pane_rect,
        theme: &theme,
        pane_bg: None,
        rope: &rope,
    };
    let mut canvas = PaneCanvas::new(&mut buf, None);
    compose_row(
        &rows[0],
        &graphemes,
        &styles,
        "hi",
        "",
        0,
        &col_widths,
        &ctx,
        &mut canvas,
        None,
    );

    assert_eq!(
        buf.cell(ratatui::layout::Position { x: 0, y: 0 })
            .unwrap()
            .symbol(),
        "h"
    );
    assert_eq!(
        buf.cell(ratatui::layout::Position { x: 1, y: 0 })
            .unwrap()
            .symbol(),
        "i"
    );
}

#[test]
fn filler_rows_have_tilde() {
    // Only render_tilde_fillers (not compose_row) draws tildes — verify
    // it fills every requested row from the given start row onward.
    let visible = VisibleRange {
        line_range: 0..1,
        top_skip_rows: 0,
        content_height: 5, // 5 rows requested; caller already rendered row 0
        content_width: 20,
        gutter_width: 0,
        last_line_idx: 0,
    };
    let viewport = ViewportState::new(20, 5);
    let pane_rect = ratatui::layout::Rect {
        x: 0,
        y: 0,
        width: 20,
        height: 5,
    };
    let mut buf = make_test_buf(20, 5);
    let theme = Theme::default();
    let rope = ropey::Rope::new();
    let ctx = ComposeCtx {
        gutter_columns: &[],
        visible: &visible,
        viewport: &viewport,
        mode: EditorMode::Normal,
        primary_head_line: 0,
        tab_width: 4,
        tilde_style: ratatui::style::Style::default(),
        indent_guide_style: ratatui::style::Style::default(),
        show_indent_guides: true,
        pane_rect,
        theme: &theme,
        pane_bg: None,
        rope: &rope,
    };
    let mut canvas = PaneCanvas::new(&mut buf, None);
    render_tilde_fillers(1, &[], &ctx, &mut canvas);

    // Rows 1–4 should have '~'
    for r in 1..5u16 {
        assert_eq!(
            buf.cell(ratatui::layout::Position { x: 0, y: r })
                .unwrap()
                .symbol(),
            "~",
            "row {} should be tilde",
            r
        );
    }
}

/// Render one row via `compose_row` directly (stage isolation — no batch
/// orchestration) at screen row 0 and return the buffer.
#[allow(clippy::too_many_arguments)]
fn do_compose_row(
    line_str: &str,
    virtual_texts: &str,
    row: &DisplayRow,
    graphemes: &[Grapheme],
    styles: &[ResolvedStyle],
    visible: VisibleRange,
    viewport: ViewportState,
    tab_width: u8,
    w: u16,
    h: u16,
) -> ratatui::buffer::Buffer {
    let pane_rect = ratatui::layout::Rect {
        x: 0,
        y: 0,
        width: w,
        height: h,
    };
    let mut buf = make_test_buf(w, h);
    let theme = Theme::default();
    let col_widths: Vec<u16> = Vec::new();
    let rope = ropey::Rope::new();
    let ctx = ComposeCtx {
        gutter_columns: &[],
        visible: &visible,
        viewport: &viewport,
        mode: EditorMode::Normal,
        primary_head_line: 0,
        tab_width,
        tilde_style: ratatui::style::Style::default(),
        indent_guide_style: ratatui::style::Style::default(),
        show_indent_guides: true,
        pane_rect,
        theme: &theme,
        pane_bg: None,
        rope: &rope,
    };
    let mut canvas = PaneCanvas::new(&mut buf, None);
    compose_row(
        row,
        graphemes,
        styles,
        line_str,
        virtual_texts,
        0,
        &col_widths,
        &ctx,
        &mut canvas,
        None,
    );
    buf
}

#[test]
fn horizontal_scroll_clips_left_columns() {
    let graphemes: Vec<Grapheme> = (0..5u16)
        .map(|i| Grapheme {
            byte_range: (i as usize)..(i as usize + 1),
            char_offset: i as usize,
            col: i,
            width: 1,
            content: CellContent::Grapheme,
            indent_depth: 0,
            scope: None,
        })
        .collect();
    let rows = [simple_row(0..5)];
    let styles = vec![ResolvedStyle::default(); 5];
    let visible = VisibleRange {
        line_range: 0..1,
        top_skip_rows: 0,
        content_height: 5,
        content_width: 20,
        gutter_width: 0,
        last_line_idx: 0,
    };
    let mut viewport = ViewportState::new(20, 5);
    viewport.horizontal_offset = 2; // skip columns 0 and 1
    let buf = do_compose_row(
        "abcde", "", &rows[0], &graphemes, &styles, visible, viewport, 4, 20, 5,
    );
    // With h_offset=2, screen col 0 shows 'c' (buf col 2).
    assert_eq!(
        buf.cell(ratatui::layout::Position { x: 0, y: 0 })
            .unwrap()
            .symbol(),
        "c"
    );
    assert_eq!(
        buf.cell(ratatui::layout::Position { x: 1, y: 0 })
            .unwrap()
            .symbol(),
        "d"
    );
}

// ── Double-width straddle at the h-scroll edge (B9) ─────────────────

#[test]
fn double_width_char_straddling_scroll_edge_renders_space_not_shifted_glyph() {
    // "中X": '中' is width 2 at col 0 (+ a WidthContinuation at col 1);
    // 'X' is width 1 at col 2. With h_offset=1, '中' straddles the edge
    // (col 0 < 1 < col 0 + width 2) — its right half is the only
    // visible cell. Before the fix, `visible_col` clamped to 0 and drew
    // the *whole* glyph at screen col 0, shifting 'X' to look like it
    // was still at col 1 instead of col 0.
    let graphemes = vec![
        Grapheme {
            byte_range: 0..3,
            char_offset: 0,
            col: 0,
            width: 2,
            content: CellContent::Grapheme,
            indent_depth: 0,
            scope: None,
        },
        Grapheme {
            byte_range: 0..3,
            char_offset: 0,
            col: 2,
            width: 0,
            content: CellContent::WidthContinuation,
            indent_depth: 0,
            scope: None,
        },
        Grapheme {
            byte_range: 3..4,
            char_offset: 1,
            col: 2,
            width: 1,
            content: CellContent::Grapheme,
            indent_depth: 0,
            scope: None,
        },
    ];
    let rows = [simple_row(0..3)];
    let styles = vec![ResolvedStyle::default(); 3];
    let visible = VisibleRange {
        line_range: 0..1,
        top_skip_rows: 0,
        content_height: 5,
        content_width: 20,
        gutter_width: 0,
        last_line_idx: 0,
    };
    let mut viewport = ViewportState::new(20, 5);
    viewport.horizontal_offset = 1;
    let buf = do_compose_row(
        "中X", "", &rows[0], &graphemes, &styles, visible, viewport, 4, 20, 5,
    );
    assert_eq!(
        buf.cell(ratatui::layout::Position { x: 0, y: 0 })
            .unwrap()
            .symbol(),
        " ",
        "straddling half of '中' renders as a space, not the glyph"
    );
    assert_eq!(
        buf.cell(ratatui::layout::Position { x: 1, y: 0 })
            .unwrap()
            .symbol(),
        "X",
        "'X' lands at its correct scrolled column"
    );
}

#[test]
fn indent_guide_drawn_at_inner_tab_stops() {
    // A line with indent_depth=2 and tab_width=4 should show a guide at col 4.
    // (guides at k*tab_width for k in 1..depth, so k=1 => col 4)
    let graphemes: Vec<Grapheme> = (0..11u16)
        .map(|i| Grapheme {
            byte_range: (i as usize)..(i as usize + 1),
            char_offset: i as usize,
            col: i,
            width: 1,
            content: CellContent::Grapheme,
            indent_depth: 2, // 8 spaces / 4 tab_width = depth 2
            scope: None,
        })
        .collect();
    let rows = [DisplayRow {
        kind: RowKind::LineStart { line_idx: 0 },
        graphemes: 0..11,
    }];
    let styles = vec![ResolvedStyle::default(); 11];
    let visible = VisibleRange {
        line_range: 0..1,
        top_skip_rows: 0,
        content_height: 5,
        content_width: 20,
        gutter_width: 0,
        last_line_idx: 0,
    };
    let viewport = ViewportState::new(20, 5);
    let buf = do_compose_row(
        "        foo", // 8 spaces + "foo"
        "",
        &rows[0],
        &graphemes,
        &styles,
        visible,
        viewport,
        4,
        20,
        5,
    );
    // A guide should appear at screen col 4 (k=1, tw=4).
    assert_eq!(
        buf.cell(ratatui::layout::Position { x: 4, y: 0 })
            .unwrap()
            .symbol(),
        INDENT_GUIDE_GLYPH
    );
    // Col 0 has the space content (no guide at depth boundary).
    assert_ne!(
        buf.cell(ratatui::layout::Position { x: 0, y: 0 })
            .unwrap()
            .symbol(),
        INDENT_GUIDE_GLYPH
    );
    // Col 8 is where content starts — no guide there.
    assert_ne!(
        buf.cell(ratatui::layout::Position { x: 8, y: 0 })
            .unwrap()
            .symbol(),
        INDENT_GUIDE_GLYPH
    );
}

#[test]
fn indent_guide_hidden_when_show_indent_guides_is_false() {
    // Same fixture as indent_guide_drawn_at_inner_tab_stops (depth=2,
    // tab_width=4, guide expected at col 4) but with the setting off —
    // proves ComposeCtx::show_indent_guides actually gates the draw loop,
    // not just that the glyph can appear under default settings.
    let graphemes: Vec<Grapheme> = (0..11u16)
        .map(|i| Grapheme {
            byte_range: (i as usize)..(i as usize + 1),
            char_offset: i as usize,
            col: i,
            width: 1,
            content: CellContent::Grapheme,
            indent_depth: 2,
            scope: None,
        })
        .collect();
    let rows = [DisplayRow {
        kind: RowKind::LineStart { line_idx: 0 },
        graphemes: 0..11,
    }];
    let styles = vec![ResolvedStyle::default(); 11];
    let visible = VisibleRange {
        line_range: 0..1,
        top_skip_rows: 0,
        content_height: 5,
        content_width: 20,
        gutter_width: 0,
        last_line_idx: 0,
    };
    let viewport = ViewportState::new(20, 5);
    let pane_rect = ratatui::layout::Rect {
        x: 0,
        y: 0,
        width: 20,
        height: 5,
    };
    let mut buf = make_test_buf(20, 5);
    let theme = Theme::default();
    let col_widths: Vec<u16> = Vec::new();
    let rope = ropey::Rope::new();
    let ctx = ComposeCtx {
        gutter_columns: &[],
        visible: &visible,
        viewport: &viewport,
        mode: EditorMode::Normal,
        primary_head_line: 0,
        tab_width: 4,
        tilde_style: ratatui::style::Style::default(),
        indent_guide_style: ratatui::style::Style::default(),
        show_indent_guides: false,
        pane_rect,
        theme: &theme,
        pane_bg: None,
        rope: &rope,
    };
    let mut canvas = PaneCanvas::new(&mut buf, None);
    compose_row(
        &rows[0],
        &graphemes,
        &styles,
        "        foo",
        "",
        0,
        &col_widths,
        &ctx,
        &mut canvas,
        None,
    );
    // No guide anywhere on the row, including the col-4 tab stop that
    // indent_guide_drawn_at_inner_tab_stops proves is drawn when enabled.
    for x in 0..11 {
        assert_ne!(
            buf.cell(ratatui::layout::Position { x, y: 0 })
                .unwrap()
                .symbol(),
            INDENT_GUIDE_GLYPH,
            "no indent guide should render at col {x} when show_indent_guides is false"
        );
    }
}

#[test]
fn indent_guide_not_drawn_on_wrap_rows() {
    // depth=1 means no inner guides (guides at k in 1..1 — empty range)
    // in general, but this test specifically pins that a Wrap row draws
    // no guide even when it would otherwise qualify — so render only the
    // Wrap row (a continuation of line 0, graphemes 4..8 of "    text").
    let graphemes: Vec<Grapheme> = (0..8u16)
        .map(|i| Grapheme {
            byte_range: (i as usize)..(i as usize + 1),
            char_offset: i as usize,
            col: i,
            width: 1,
            content: CellContent::Grapheme,
            indent_depth: 1,
            scope: None,
        })
        .collect();
    let rows = [DisplayRow {
        kind: RowKind::Wrap {
            line_idx: 0,
            wrap_row: 1,
        },
        graphemes: 4..8,
    }];
    let styles = vec![ResolvedStyle::default(); 8];
    let visible = VisibleRange {
        line_range: 0..1,
        top_skip_rows: 0,
        content_height: 5,
        content_width: 20,
        gutter_width: 0,
        last_line_idx: 0,
    };
    let viewport = ViewportState::new(20, 5);
    let buf = do_compose_row(
        "    text", "", &rows[0], &graphemes, &styles, visible, viewport, 4, 20, 5,
    );
    assert_ne!(
        buf.cell(ratatui::layout::Position { x: 0, y: 0 })
            .unwrap()
            .symbol(),
        INDENT_GUIDE_GLYPH
    );
}

#[test]
fn indicator_content_fills_tab_width() {
    // A tab indicator with width=4 should write the indicator char at col 0
    // and spaces at cols 1-3.
    let graphemes = vec![Grapheme {
        byte_range: 0..1,
        char_offset: 0,
        col: 0,
        width: 4,
        content: CellContent::Indicator { start: 0, len: 3 }, // "→" is 3 bytes
        indent_depth: 0,
        scope: None,
    }];
    let rows = [simple_row(0..1)];
    let styles = vec![ResolvedStyle::default()];
    let visible = VisibleRange {
        line_range: 0..1,
        top_skip_rows: 0,
        content_height: 5,
        content_width: 20,
        gutter_width: 0,
        last_line_idx: 0,
    };
    let viewport = ViewportState::new(20, 5);
    let buf = do_compose_row(
        "\t", "→", &rows[0], &graphemes, &styles, visible, viewport, 4, 20, 5,
    );
    assert_eq!(
        buf.cell(ratatui::layout::Position { x: 0, y: 0 })
            .unwrap()
            .symbol(),
        "→"
    );
    assert_eq!(
        buf.cell(ratatui::layout::Position { x: 1, y: 0 })
            .unwrap()
            .symbol(),
        " "
    );
    assert_eq!(
        buf.cell(ratatui::layout::Position { x: 2, y: 0 })
            .unwrap()
            .symbol(),
        " "
    );
    assert_eq!(
        buf.cell(ratatui::layout::Position { x: 3, y: 0 })
            .unwrap()
            .symbol(),
        " "
    );
}

// ── Virtual/Indicator content arena (G1) ────────────────────────────

#[test]
fn virtual_cell_wider_than_one_column_renders_from_the_arena() {
    // A decoration whose text is more than one byte/column ("AB", width
    // 2) must round-trip through the arena correctly, and the following
    // real grapheme must land at the column the insert's width shifted
    // it to (col 2, not col 1).
    let arena = "AB";
    let graphemes = vec![
        Grapheme {
            byte_range: 0..0,
            char_offset: usize::MAX,
            col: 0,
            width: 2,
            content: CellContent::Virtual { start: 0, len: 2 },
            indent_depth: 0,
            scope: None,
        },
        simple_grapheme(2, 0, 1), // real 'c', shifted right by the insert's width
    ];
    let rows = [simple_row(0..2)];
    let styles = vec![ResolvedStyle::default(); 2];
    let visible = VisibleRange {
        line_range: 0..1,
        top_skip_rows: 0,
        content_height: 5,
        content_width: 20,
        gutter_width: 0,
        last_line_idx: 0,
    };
    let viewport = ViewportState::new(20, 5);
    let buf = do_compose_row(
        "c", arena, &rows[0], &graphemes, &styles, visible, viewport, 4, 20, 5,
    );
    assert_eq!(
        buf.cell(ratatui::layout::Position { x: 0, y: 0 })
            .unwrap()
            .symbol(),
        "AB",
        "insert text resolved from the arena, not truncated to its first byte"
    );
    assert_eq!(
        buf.cell(ratatui::layout::Position { x: 2, y: 0 })
            .unwrap()
            .symbol(),
        "c",
        "real grapheme shifted right by the insert's width"
    );
}

// ── Gutter text overflow (B8) ────────────────────────────────────────

struct OverlongGutter;
impl GutterColumn for OverlongGutter {
    fn width(&self, _: usize) -> u8 {
        4
    }
    fn render_row_cells(
        &self,
        _: RowKind,
        _: &crate::providers::GutterRowCtx,
    ) -> Vec<crate::providers::GutterCell> {
        vec![crate::providers::GutterCell {
            content: crate::providers::GutterCellContent::Text(std::borrow::Cow::Borrowed(
                "TOOLONG",
            )),
            scope: crate::types::Scope("ui.linenr").into(),
        }]
    }
    fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
        self
    }
}

#[test]
fn gutter_text_wider_than_column_is_truncated_not_bled_into_content() {
    // Gutter column width() = 4 → usable = 3. Cell text "TOOLONG" (7
    // cols) must truncate to "TOO", not spill "LONG" into the content
    // area (which starts right after the gutter, at x=4).
    let graphemes = vec![simple_grapheme(0, 0, 1)];
    let rows = [simple_row(0..1)];
    let styles = vec![ResolvedStyle::default()];
    let gutter_columns: Vec<(ProviderId, Box<dyn GutterColumn>)> =
        vec![(0, Box::new(OverlongGutter))];
    let visible = VisibleRange {
        line_range: 0..1,
        top_skip_rows: 0,
        content_height: 1,
        content_width: 6,
        gutter_width: 4,
        last_line_idx: 0,
    };
    let viewport = ViewportState::new(10, 1);
    let pane_rect = ratatui::layout::Rect {
        x: 0,
        y: 0,
        width: 10,
        height: 1,
    };
    let mut buf = make_test_buf(10, 1);
    let theme = Theme::default();
    let col_widths = vec![4u16];
    let rope = ropey::Rope::new();
    let ctx = ComposeCtx {
        gutter_columns: &gutter_columns,
        visible: &visible,
        viewport: &viewport,
        mode: EditorMode::Normal,
        primary_head_line: 0,
        tab_width: 4,
        tilde_style: ratatui::style::Style::default(),
        indent_guide_style: ratatui::style::Style::default(),
        show_indent_guides: true,
        pane_rect,
        theme: &theme,
        pane_bg: None,
        rope: &rope,
    };
    let mut canvas = PaneCanvas::new(&mut buf, None);
    compose_row(
        &rows[0],
        &graphemes,
        &styles,
        "X",
        "",
        0,
        &col_widths,
        &ctx,
        &mut canvas,
        None,
    );

    let sym = |x: u16| {
        buf.cell(ratatui::layout::Position { x, y: 0 })
            .unwrap()
            .symbol()
            .to_string()
    };
    assert_eq!(sym(0), "T");
    assert_eq!(sym(1), "O");
    assert_eq!(sym(2), "O");
    assert_eq!(sym(3), " ", "separator cell, not 'L'");
    // Content area starts at x=4 (gutter_width). Must show the real
    // grapheme 'X', never a straggler from "LONG".
    assert_eq!(sym(4), "X");
    assert_ne!(sym(4), "L");
    assert_ne!(sym(5), "O");
    assert_ne!(sym(6), "N");
}

#[test]
fn gutter_overflow_does_not_bleed_into_neighbouring_pane() {
    // Same overlong-gutter setup as above, but with a narrow pane_rect
    // (width 5 = gutter(4) + 1 content col) simulating a second pane
    // starting immediately at x=5 in the same shared buffer. Pre-seed
    // the whole buffer with a marker glyph so any write past this pane's
    // own right edge is directly observable.
    let graphemes = vec![simple_grapheme(0, 0, 1)];
    let rows = [simple_row(0..1)];
    let styles = vec![ResolvedStyle::default()];
    let gutter_columns: Vec<(ProviderId, Box<dyn GutterColumn>)> =
        vec![(0, Box::new(OverlongGutter))];
    let visible = VisibleRange {
        line_range: 0..1,
        top_skip_rows: 0,
        content_height: 1,
        content_width: 1,
        gutter_width: 4,
        last_line_idx: 0,
    };
    let viewport = ViewportState::new(5, 1);
    let pane_rect = ratatui::layout::Rect {
        x: 0,
        y: 0,
        width: 5,
        height: 1,
    };
    let mut buf = make_test_buf(11, 1);
    for x in 0..11u16 {
        set_cell(&mut buf, x, 0, "Z", ratatui::style::Style::default());
    }
    let theme = Theme::default();
    let col_widths = vec![4u16];
    let rope = ropey::Rope::new();
    let ctx = ComposeCtx {
        gutter_columns: &gutter_columns,
        visible: &visible,
        viewport: &viewport,
        mode: EditorMode::Normal,
        primary_head_line: 0,
        tab_width: 4,
        tilde_style: ratatui::style::Style::default(),
        indent_guide_style: ratatui::style::Style::default(),
        show_indent_guides: true,
        pane_rect,
        theme: &theme,
        pane_bg: None,
        rope: &rope,
    };
    let mut canvas = PaneCanvas::new(&mut buf, None);
    compose_row(
        &rows[0],
        &graphemes,
        &styles,
        "X",
        "",
        0,
        &col_widths,
        &ctx,
        &mut canvas,
        None,
    );

    // x=5..10 belongs to the "next pane" — must remain untouched ('Z').
    for x in 5..11u16 {
        assert_eq!(
            buf.cell(ratatui::layout::Position { x, y: 0 })
                .unwrap()
                .symbol(),
            "Z",
            "neighbouring pane's column {x} must be untouched"
        );
    }
}

/// Width-6 column returning 4 single-char cells — `(6 - 1) / 4 == 1`
/// (integer division), leaving a 1-cell remainder that the exact-fill
/// `SignColumn`/`LineNumberColumn` never produce on their own (see
/// `sign_column.rs`: `max_signs == width - 1` always divides evenly).
/// Exists purely to reproduce a leftover in a *non-first* gutter column.
struct LeftoverGutter;
impl GutterColumn for LeftoverGutter {
    fn width(&self, _: usize) -> u8 {
        6
    }
    fn render_row_cells(
        &self,
        _: RowKind,
        _: &crate::providers::GutterRowCtx,
    ) -> Vec<crate::providers::GutterCell> {
        ["1", "2", "3", "4"]
            .iter()
            .map(|s| crate::providers::GutterCell {
                content: crate::providers::GutterCellContent::Text(std::borrow::Cow::Borrowed(s)),
                scope: crate::types::Scope("ui.linenr").into(),
            })
            .collect()
    }
    fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
        self
    }
}

/// Single-cell width-2 column — exact-fills like the shipped
/// `LineNumberColumn`, placed first so the leftover column below it is the
/// *second* column, which is what exposes the bug (see next test's doc).
struct ExactFillGutter;
impl GutterColumn for ExactFillGutter {
    fn width(&self, _: usize) -> u8 {
        2
    }
    fn render_row_cells(
        &self,
        _: RowKind,
        _: &crate::providers::GutterRowCtx,
    ) -> Vec<crate::providers::GutterCell> {
        vec![crate::providers::GutterCell {
            content: crate::providers::GutterCellContent::Text(std::borrow::Cow::Borrowed("N")),
            scope: crate::types::Scope("ui.linenr").into(),
        }]
    }
    fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
        self
    }
}

#[test]
fn second_column_leftover_is_painted_and_next_column_starts_on_boundary() {
    // `compose_gutter`'s leftover-fill bound must be the column's real right
    // edge, not `pane_rect.x + col_width` — that's only correct for the
    // *first* column (where col_start == pane_rect.x). For any column after
    // the first, a bound that small leaves a leftover cell in a non-first
    // column unpainted (stale glyph shows through) and `gutter_x` falls
    // short of the column boundary, shifting every following column left.
    //
    // Column 0 (ExactFillGutter, width 2) exact-fills, landing gutter_x
    // at col_start=2 for column 1 (LeftoverGutter, width 6, 4 cells):
    // usable_per_cell = (6-1)/4 = 1, so the per-cell loop only advances
    // gutter_x to 2+5=7, one short of the column's right edge at 8.
    let graphemes = vec![simple_grapheme(0, 0, 1)];
    let rows = [simple_row(0..1)];
    let styles = vec![ResolvedStyle::default()];
    let gutter_columns: Vec<(ProviderId, Box<dyn GutterColumn>)> = vec![
        (0, Box::new(ExactFillGutter)),
        (1, Box::new(LeftoverGutter)),
    ];
    let visible = VisibleRange {
        line_range: 0..1,
        top_skip_rows: 0,
        content_height: 1,
        content_width: 2,
        gutter_width: 8, // 2 (ExactFillGutter) + 6 (LeftoverGutter)
        last_line_idx: 0,
    };
    let viewport = ViewportState::new(10, 1);
    let pane_rect = ratatui::layout::Rect {
        x: 0,
        y: 0,
        width: 10,
        height: 1,
    };
    let mut buf = make_test_buf(10, 1);
    for x in 0..10u16 {
        set_cell(&mut buf, x, 0, "Z", ratatui::style::Style::default());
    }
    let theme = Theme::default();
    let col_widths = vec![2u16, 6u16];
    let rope = ropey::Rope::new();
    let ctx = ComposeCtx {
        gutter_columns: &gutter_columns,
        visible: &visible,
        viewport: &viewport,
        mode: EditorMode::Normal,
        primary_head_line: 0,
        tab_width: 4,
        tilde_style: ratatui::style::Style::default(),
        indent_guide_style: ratatui::style::Style::default(),
        show_indent_guides: true,
        pane_rect,
        theme: &theme,
        pane_bg: None,
        rope: &rope,
    };
    let mut canvas = PaneCanvas::new(&mut buf, None);
    compose_row(
        &rows[0],
        &graphemes,
        &styles,
        "X",
        "",
        0,
        &col_widths,
        &ctx,
        &mut canvas,
        None,
    );

    let sym = |x: u16| {
        buf.cell(ratatui::layout::Position { x, y: 0 })
            .unwrap()
            .symbol()
            .to_string()
    };
    assert_eq!(sym(0), "N", "column 0's only cell");
    assert_eq!(sym(1), " ", "column 0's separator");
    assert_eq!(sym(2), "1");
    assert_eq!(sym(3), "2");
    assert_eq!(sym(4), "3");
    assert_eq!(sym(5), "4");
    assert_eq!(sym(6), " ", "column 1's separator, after its last cell");
    assert_eq!(
        sym(7),
        " ",
        "column 1's leftover cell (gutter_width 8 - col_start 2 - 5 rendered = 1 leftover) \
         must be painted blank, not left as the stale 'Z' marker"
    );
    assert_eq!(
        sym(8),
        "X",
        "content must start exactly at gutter_width=8, not shifted left by the dropped leftover cell"
    );
}

/// Width-20 single-cell column — `signcolumn` accepts up to 127 slots with
/// no clamp against pane width anywhere in `layout.rs`; this simulates a
/// configured gutter wider than the pane itself (e.g. `signcolumn
/// always:40` in a narrow vsplit).
struct HugeGutter;
impl GutterColumn for HugeGutter {
    fn width(&self, _: usize) -> u8 {
        20
    }
    fn render_row_cells(
        &self,
        _: RowKind,
        _: &crate::providers::GutterRowCtx,
    ) -> Vec<crate::providers::GutterCell> {
        vec![crate::providers::GutterCell {
            content: crate::providers::GutterCellContent::Text(std::borrow::Cow::Borrowed("N")),
            scope: crate::types::Scope("ui.linenr").into(),
        }]
    }
    fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
        self
    }
}

#[test]
fn gutter_wider_than_pane_does_not_bleed_past_the_pane_right_edge() {
    // Regression: nothing clamped a gutter column's configured width against
    // the pane's actual width, so a gutter wider than the pane (reachable
    // via `signcolumn always:N` in a narrow vsplit) wrote straight through
    // the pane's right edge into whatever the shared terminal buffer holds
    // next to it — typically a neighbouring pane.
    let graphemes = vec![simple_grapheme(0, 0, 1)];
    let rows = [simple_row(0..1)];
    let styles = vec![ResolvedStyle::default()];
    let gutter_columns: Vec<(ProviderId, Box<dyn GutterColumn>)> = vec![(0, Box::new(HugeGutter))];
    let visible = VisibleRange {
        line_range: 0..1,
        top_skip_rows: 0,
        content_height: 1,
        content_width: 1,
        gutter_width: 20,
        last_line_idx: 0,
    };
    let viewport = ViewportState::new(6, 1);
    let pane_rect = ratatui::layout::Rect {
        x: 0,
        y: 0,
        width: 6,
        height: 1,
    };
    let mut buf = make_test_buf(12, 1);
    for x in 0..12u16 {
        set_cell(&mut buf, x, 0, "Z", ratatui::style::Style::default());
    }
    let theme = Theme::default();
    let col_widths = vec![20u16];
    let rope = ropey::Rope::new();
    let ctx = ComposeCtx {
        gutter_columns: &gutter_columns,
        visible: &visible,
        viewport: &viewport,
        mode: EditorMode::Normal,
        primary_head_line: 0,
        tab_width: 4,
        tilde_style: ratatui::style::Style::default(),
        indent_guide_style: ratatui::style::Style::default(),
        show_indent_guides: true,
        pane_rect,
        theme: &theme,
        pane_bg: None,
        rope: &rope,
    };
    let mut canvas = PaneCanvas::new(&mut buf, None);
    compose_row(
        &rows[0],
        &graphemes,
        &styles,
        "X",
        "",
        0,
        &col_widths,
        &ctx,
        &mut canvas,
        None,
    );

    let sym = |x: u16| {
        buf.cell(ratatui::layout::Position { x, y: 0 })
            .unwrap()
            .symbol()
            .to_string()
    };
    for x in 6..12u16 {
        assert_eq!(
            sym(x),
            "Z",
            "column x={x} is past the pane's right edge (width 6) and must be untouched"
        );
    }
}

/// Gutter column returning a runtime-computed `Cow::Owned` icon — the
/// shape a Steel-configured gutter icon would take.
struct OwnedIconGutter;
impl GutterColumn for OwnedIconGutter {
    fn width(&self, _: usize) -> u8 {
        3
    }
    fn render_row_cells(
        &self,
        _: RowKind,
        _: &crate::providers::GutterRowCtx,
    ) -> Vec<crate::providers::GutterCell> {
        vec![crate::providers::GutterCell {
            // Built at call time (e.g. `format!`) rather than a literal —
            // exercises the `Cow::Owned` path, not `Cow::Borrowed`.
            content: crate::providers::GutterCellContent::Text(std::borrow::Cow::Owned(
                "AB".to_string(),
            )),
            scope: crate::types::Scope("ui.linenr").into(),
        }]
    }
    fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
        self
    }
}

/// Same gutter column, but the `'static` literal is borrowed directly
/// (`Cow::Borrowed`). Renders through the identical `compose_gutter`
/// path; `Cow::Owned` must produce the same output.
struct StaticIconGutter;
impl GutterColumn for StaticIconGutter {
    fn width(&self, _: usize) -> u8 {
        3
    }
    fn render_row_cells(
        &self,
        _: RowKind,
        _: &crate::providers::GutterRowCtx,
    ) -> Vec<crate::providers::GutterCell> {
        vec![crate::providers::GutterCell {
            content: crate::providers::GutterCellContent::Text(std::borrow::Cow::Borrowed("AB")),
            scope: crate::types::Scope("ui.linenr").into(),
        }]
    }
    fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
        self
    }
}

#[test]
fn owned_gutter_icon_renders_identically_to_static_one() {
    fn render_with(col: Box<dyn GutterColumn>) -> ratatui::buffer::Buffer {
        let graphemes = vec![simple_grapheme(0, 0, 1)];
        let rows = [simple_row(0..1)];
        let styles = vec![ResolvedStyle::default()];
        let gutter_columns: Vec<(ProviderId, Box<dyn GutterColumn>)> = vec![(0, col)];
        let visible = VisibleRange {
            line_range: 0..1,
            top_skip_rows: 0,
            content_height: 1,
            content_width: 4,
            gutter_width: 3,
            last_line_idx: 0,
        };
        let viewport = ViewportState::new(7, 1);
        let pane_rect = ratatui::layout::Rect {
            x: 0,
            y: 0,
            width: 7,
            height: 1,
        };
        let mut buf = make_test_buf(7, 1);
        let theme = Theme::default();
        let col_widths = vec![3u16];
        let rope = ropey::Rope::new();
        let ctx = ComposeCtx {
            gutter_columns: &gutter_columns,
            visible: &visible,
            viewport: &viewport,
            mode: EditorMode::Normal,
            primary_head_line: 0,
            tab_width: 4,
            tilde_style: ratatui::style::Style::default(),
            indent_guide_style: ratatui::style::Style::default(),
            show_indent_guides: true,
            pane_rect,
            theme: &theme,
            pane_bg: None,
            rope: &rope,
        };
        let mut canvas = PaneCanvas::new(&mut buf, None);
        compose_row(
            &rows[0],
            &graphemes,
            &styles,
            "X",
            "",
            0,
            &col_widths,
            &ctx,
            &mut canvas,
            None,
        );
        buf
    }

    let owned_buf = render_with(Box::new(OwnedIconGutter));
    let static_buf = render_with(Box::new(StaticIconGutter));
    for x in 0..7u16 {
        assert_eq!(
            owned_buf
                .cell(ratatui::layout::Position { x, y: 0 })
                .unwrap()
                .symbol(),
            static_buf
                .cell(ratatui::layout::Position { x, y: 0 })
                .unwrap()
                .symbol(),
            "column {x}: Cow::Owned must render identically to Cow::Borrowed"
        );
    }
}

// ── GutterColumn gets buffer context (G2) ───────────────────────────

/// Gutter column that reads the first character of the row's own buffer
/// line straight out of `ctx.rope` — exercises the `GutterRowCtx`
/// plumbing end to end through `compose_gutter`.
struct FirstCharGutter;
impl GutterColumn for FirstCharGutter {
    fn width(&self, _: usize) -> u8 {
        2
    }
    fn render_row_cells(
        &self,
        kind: RowKind,
        ctx: &crate::providers::GutterRowCtx,
    ) -> Vec<crate::providers::GutterCell> {
        let cell = match kind {
            RowKind::LineStart { line_idx } => {
                let first_char = ctx.rope.line(line_idx).chars().next().unwrap_or(' ');
                crate::providers::GutterCell {
                    content: crate::providers::GutterCellContent::Text(std::borrow::Cow::Owned(
                        first_char.to_string(),
                    )),
                    scope: crate::types::Scope("ui.linenr").into(),
                }
            }
            _ => crate::providers::GutterCell::blank(crate::types::Scope("ui.linenr")),
        };
        vec![cell]
    }
    fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
        self
    }
}

#[test]
fn gutter_column_reads_rope_via_ctx() {
    // "apple\nbanana\n": rendering the row for line 1 must show 'b' —
    // proving the column reached the buffer through `GutterRowCtx.rope`,
    // not some pre-owned/stale copy.
    let rope = ropey::Rope::from_str("apple\nbanana\n");
    let graphemes = vec![simple_grapheme(0, 0, 1)];
    let rows = [DisplayRow {
        kind: RowKind::LineStart { line_idx: 1 },
        graphemes: 0..1,
    }];
    let styles = vec![ResolvedStyle::default()];
    let gutter_columns: Vec<(ProviderId, Box<dyn GutterColumn>)> =
        vec![(0, Box::new(FirstCharGutter))];
    let visible = VisibleRange {
        line_range: 0..2,
        top_skip_rows: 0,
        content_height: 2,
        content_width: 10,
        gutter_width: 2,
        last_line_idx: 1,
    };
    let viewport = ViewportState::new(12, 2);
    let pane_rect = ratatui::layout::Rect {
        x: 0,
        y: 0,
        width: 12,
        height: 2,
    };
    let mut buf = make_test_buf(12, 2);
    let theme = Theme::default();
    let col_widths = vec![2u16];
    let ctx = ComposeCtx {
        gutter_columns: &gutter_columns,
        visible: &visible,
        viewport: &viewport,
        mode: EditorMode::Normal,
        primary_head_line: 0,
        tab_width: 4,
        tilde_style: ratatui::style::Style::default(),
        indent_guide_style: ratatui::style::Style::default(),
        show_indent_guides: true,
        pane_rect,
        theme: &theme,
        pane_bg: None,
        rope: &rope,
    };
    let mut canvas = PaneCanvas::new(&mut buf, None);
    compose_row(
        &rows[0],
        &graphemes,
        &styles,
        "X",
        "",
        0,
        &col_widths,
        &ctx,
        &mut canvas,
        None,
    );
    assert_eq!(
        buf.cell(ratatui::layout::Position { x: 0, y: 0 })
            .unwrap()
            .symbol(),
        "b",
        "gutter column resolved 'banana' (line 1) via ctx.rope"
    );
}

#[test]
fn set_cell_out_of_bounds_no_panic() {
    let mut buf = make_test_buf(10, 5);
    // Call with coordinates well beyond the buffer area — must not panic.
    set_cell(&mut buf, 100, 100, "x", ratatui::style::Style::default());
    set_cell(&mut buf, 10, 0, "x", ratatui::style::Style::default()); // exactly at boundary
}

#[test]
fn clear_row_span_fills_with_blank() {
    let mut buf = make_test_buf(10, 3);
    // Write something so we can confirm clearing works.
    for x in 0..10 {
        set_cell(&mut buf, x, 1, "X", ratatui::style::Style::default());
    }
    // Clear the middle 4 columns of row 1.
    clear_row_span(&mut buf, 3, 7, 1);
    for x in 0..10 {
        let sym = buf
            .cell(ratatui::layout::Position { x, y: 1 })
            .unwrap()
            .symbol();
        if (3..7).contains(&x) {
            assert_eq!(sym, " ", "col {x} should be blank");
        } else {
            assert_eq!(sym, "X", "col {x} should be untouched");
        }
    }
}

#[test]
fn clear_row_span_clips_right_edge() {
    let mut buf = make_test_buf(10, 3);
    for x in 0..10 {
        set_cell(&mut buf, x, 0, "X", ratatui::style::Style::default());
    }
    // x_end extends past the buffer's right edge — should clip, not panic.
    clear_row_span(&mut buf, 8, 20, 0);
    for x in 0..10 {
        let sym = buf
            .cell(ratatui::layout::Position { x, y: 0 })
            .unwrap()
            .symbol();
        if x >= 8 {
            assert_eq!(sym, " ");
        } else {
            assert_eq!(sym, "X");
        }
    }
}

#[test]
fn clear_row_span_empty_range_no_panic() {
    let mut buf = make_test_buf(10, 3);
    // x_start == x_end and x_start > x_end should both be no-ops.
    clear_row_span(&mut buf, 5, 5, 0);
    clear_row_span(&mut buf, 7, 3, 0);
}

// ── fused dim (compose path) ───────────────────────────────────────

/// `dim` on `PaneCanvas` must blend each written cell's fg/bg toward the
/// target inline. Verifies the same lerp oracle (255→0 at 0.5 ⇒ 128) holds
/// through `compose_row`.
#[test]
fn compose_row_dims_cells_inline() {
    use ratatui::style::Color;
    let graphemes = vec![simple_grapheme(0, 0, 1)];
    let rows = [simple_row(0..1)];
    let styles = vec![ResolvedStyle {
        fg: Some(Color::Rgb(255, 255, 255)),
        bg: Some(Color::Rgb(0, 0, 0)),
        ..Default::default()
    }];
    let visible = VisibleRange {
        line_range: 0..1,
        top_skip_rows: 0,
        content_height: 1,
        content_width: 2,
        gutter_width: 0,
        last_line_idx: 0,
    };
    let viewport = ViewportState::new(2, 1);
    let pane_rect = ratatui::layout::Rect {
        x: 0,
        y: 0,
        width: 2,
        height: 1,
    };
    let mut buf = make_test_buf(2, 1);
    let theme = Theme::default();
    let col_widths: Vec<u16> = Vec::new();
    let rope = ropey::Rope::new();
    let ctx = ComposeCtx {
        gutter_columns: &[],
        visible: &visible,
        viewport: &viewport,
        mode: EditorMode::Normal,
        primary_head_line: 0,
        tab_width: 4,
        tilde_style: ratatui::style::Style::default(),
        indent_guide_style: ratatui::style::Style::default(),
        show_indent_guides: true,
        pane_rect,
        theme: &theme,
        pane_bg: Some(Color::Rgb(0, 0, 0)),
        rope: &rope,
    };
    let mut canvas = PaneCanvas::new(&mut buf, Some((Color::Rgb(0, 0, 0), 0.5)));
    compose_row(
        &rows[0],
        &graphemes,
        &styles,
        "x",
        "",
        0,
        &col_widths,
        &ctx,
        &mut canvas,
        None,
    );
    let cell = buf.cell(ratatui::layout::Position { x: 0, y: 0 }).unwrap();
    // Independent oracle: 255 lerp 0 at 0.5 ⇒ 127.5, rounds to 128.
    assert_eq!(cell.fg, Color::Rgb(128, 128, 128));
    // bg already at target ⇒ blend is a no-op.
    assert_eq!(cell.bg, Color::Rgb(0, 0, 0));
}

/// A non-RGB dim target must be a no-op (cell keeps its original colours),
/// matching the prior `dim_rect` semantics.
#[test]
fn compose_row_non_rgb_dim_target_is_noop() {
    use ratatui::style::Color;
    let graphemes = vec![simple_grapheme(0, 0, 1)];
    let rows = [simple_row(0..1)];
    let styles = vec![ResolvedStyle {
        fg: Some(Color::Rgb(255, 255, 255)),
        ..Default::default()
    }];
    let visible = VisibleRange {
        line_range: 0..1,
        top_skip_rows: 0,
        content_height: 1,
        content_width: 2,
        gutter_width: 0,
        last_line_idx: 0,
    };
    let viewport = ViewportState::new(2, 1);
    let pane_rect = ratatui::layout::Rect {
        x: 0,
        y: 0,
        width: 2,
        height: 1,
    };
    let mut buf = make_test_buf(2, 1);
    let theme = Theme::default();
    let col_widths: Vec<u16> = Vec::new();
    let rope = ropey::Rope::new();
    let ctx = ComposeCtx {
        gutter_columns: &[],
        visible: &visible,
        viewport: &viewport,
        mode: EditorMode::Normal,
        primary_head_line: 0,
        tab_width: 4,
        tilde_style: ratatui::style::Style::default(),
        indent_guide_style: ratatui::style::Style::default(),
        show_indent_guides: true,
        pane_rect,
        theme: &theme,
        pane_bg: Some(Color::Rgb(0, 0, 0)),
        rope: &rope,
    };
    let mut canvas = PaneCanvas::new(&mut buf, Some((Color::Reset, 0.5)));
    compose_row(
        &rows[0],
        &graphemes,
        &styles,
        "x",
        "",
        0,
        &col_widths,
        &ctx,
        &mut canvas,
        None,
    );
    assert_eq!(
        buf.cell(ratatui::layout::Position { x: 0, y: 0 })
            .unwrap()
            .fg,
        Color::Rgb(255, 255, 255)
    );
}

#[test]
fn fill_rect_bg_clears_stale_modifiers() {
    // Reproduces the completion-popup bleed: an opaque overlay painted over a
    // cell the pane already wrote in italic/bold must not leave those
    // modifiers on the cell. A plain (no-modifier) ResolvedStyle converts to
    // a ratatui Style whose sub_modifier clears everything not explicitly
    // enabled — see the `From<ResolvedStyle> for ratatui::style::Style`
    // impl in types.rs.
    let mut buf = make_test_buf(4, 1);
    buf[(0, 0)].set_style(
        ratatui::style::Style::default()
            .add_modifier(ratatui::style::Modifier::ITALIC | ratatui::style::Modifier::BOLD),
    );
    assert!(
        buf[(0, 0)]
            .modifier
            .contains(ratatui::style::Modifier::ITALIC)
    );

    let menu_style: ratatui::style::Style = ResolvedStyle::default().into();
    fill_rect_bg(
        &mut buf,
        ratatui::layout::Rect {
            x: 0,
            y: 0,
            width: 4,
            height: 1,
        },
        menu_style,
    );

    assert!(
        !buf[(0, 0)]
            .modifier
            .contains(ratatui::style::Modifier::ITALIC)
    );
    assert!(
        !buf[(0, 0)]
            .modifier
            .contains(ratatui::style::Modifier::BOLD)
    );
}
