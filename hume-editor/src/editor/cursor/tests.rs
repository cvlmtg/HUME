use super::*;
use hume_engine::format::FormatScratch;
use hume_engine::pane::{ViewportState, WhitespaceConfig, WrapMode};
use ropey::Rope;

fn vp(top_line: usize, width: u16, height: u16) -> ViewportState {
    let mut v = ViewportState::new(width, height);
    v.top_line = top_line;
    v
}

fn ws() -> WhitespaceConfig {
    WhitespaceConfig::default()
}

/// No `VirtualLineSource` registered — `display_rows_for_line` reduces
/// to content-only `RowsBreakdown`s, matching every test below's
/// virtual-line-unaware expectations exactly.
fn no_providers() -> ProviderSet {
    ProviderSet::new()
}

// ── screen_to_char_offset (no-wrap) ──────────────────────────────────────

/// Click on column 0 of line 0, no gutter → char 0.
#[test]
fn nowrap_click_first_char() {
    // "abc\ndef\n": chars 0-2 = 'a','b','c', char 3 = '\n', chars 4-6 = 'd','e','f'
    let rope = Rope::from_str("abc\ndef\n");
    let v = vp(0, 80, 10);
    let mut s = FormatScratch::new();
    let got = screen_to_char_offset(
        0,
        0,
        0,
        &v,
        &rope,
        &WrapMode::None,
        4,
        &ws(),
        &mut s,
        &no_providers(),
        80,
    );
    assert_eq!(got, Some(0));
}

/// Click on column 2 of line 0 → char 2.
#[test]
fn nowrap_click_mid_first_line() {
    let rope = Rope::from_str("abc\ndef\n");
    let v = vp(0, 80, 10);
    let mut s = FormatScratch::new();
    let got = screen_to_char_offset(
        2,
        0,
        0,
        &v,
        &rope,
        &WrapMode::None,
        4,
        &ws(),
        &mut s,
        &no_providers(),
        80,
    );
    assert_eq!(got, Some(2));
}

/// Click on screen row 1, column 0 → start of second line (char 4).
#[test]
fn nowrap_click_second_line() {
    let rope = Rope::from_str("abc\ndef\n");
    let v = vp(0, 80, 10);
    let mut s = FormatScratch::new();
    let got = screen_to_char_offset(
        0,
        1,
        0,
        &v,
        &rope,
        &WrapMode::None,
        4,
        &ws(),
        &mut s,
        &no_providers(),
        80,
    );
    assert_eq!(got, Some(4)); // 'd' is char 4
}

/// Click in the gutter (screen_x < gutter_w) returns None.
#[test]
fn nowrap_gutter_click_returns_none() {
    let rope = Rope::from_str("abc\n");
    let v = vp(0, 80, 10);
    let mut s = FormatScratch::new();
    // gutter_w = 4; click at column 2 is inside the gutter.
    let got = screen_to_char_offset(
        2,
        0,
        4,
        &v,
        &rope,
        &WrapMode::None,
        4,
        &ws(),
        &mut s,
        &no_providers(),
        80,
    );
    assert_eq!(got, None);
}

/// Click past end of line returns the newline char at the end of the line.
///
/// In HUME's inclusive selection model, the newline char is a valid cursor
/// position (end-of-line). "hi\n" has chars: h=0, i=1, \n=2.
#[test]
fn nowrap_click_past_line_end() {
    let rope = Rope::from_str("hi\n");
    let v = vp(0, 80, 10);
    let mut s = FormatScratch::new();
    // Click at column 99, way past "hi" — lands at '\n' (char 2), the eol marker.
    let got = screen_to_char_offset(
        99,
        0,
        0,
        &v,
        &rope,
        &WrapMode::None,
        4,
        &ws(),
        &mut s,
        &no_providers(),
        80,
    );
    assert_eq!(got, Some(2));
}

/// Viewport scrolled down: screen_y=0 refers to top_line, not line 0.
#[test]
fn nowrap_viewport_scrolled() {
    // Lines: 0=a, 1=b, 2=c, 3=d. top_line=2 → screen row 0 is line 2 = 'c'.
    let rope = Rope::from_str("a\nb\nc\nd\n");
    let v = vp(2, 80, 10); // top_line = 2
    let mut s = FormatScratch::new();
    // Line 2 starts at char 4 ('c'). Screen row 0, col 0 → char 4.
    let got = screen_to_char_offset(
        0,
        0,
        0,
        &v,
        &rope,
        &WrapMode::None,
        4,
        &ws(),
        &mut s,
        &no_providers(),
        80,
    );
    assert_eq!(got, Some(4));
}

/// Horizontal scroll: content_col = screen_x - gutter_w + h_offset.
#[test]
fn nowrap_horizontal_scroll() {
    // "abcde\n" with h_offset=2: screen col 0 maps to content col 2 = 'c' (char 2).
    let rope = Rope::from_str("abcde\n");
    let mut v = vp(0, 80, 10);
    v.horizontal_offset = 2;
    let mut s = FormatScratch::new();
    let got = screen_to_char_offset(
        0,
        0,
        0,
        &v,
        &rope,
        &WrapMode::None,
        4,
        &ws(),
        &mut s,
        &no_providers(),
        80,
    );
    assert_eq!(got, Some(2));
}

// ── screen_to_char_offset (wrap) ─────────────────────────────────────────

/// With Soft { width: 4 }, "abcdefgh\n" wraps: row 0 = "abcd", row 1 = "efgh".
/// Click at screen (0, 0) → char 0 ('a').
/// Click at screen (0, 1) → char 4 ('e').
#[test]
fn wrap_click_first_and_second_visual_row() {
    let rope = Rope::from_str("abcdefgh\n");
    let v = vp(0, 10, 10);
    let wrap = WrapMode::Soft { width: 4 };
    let mut s = FormatScratch::new();

    let row0 = screen_to_char_offset(
        0,
        0,
        0,
        &v,
        &rope,
        &wrap,
        4,
        &ws(),
        &mut s,
        &no_providers(),
        10,
    );
    assert_eq!(row0, Some(0));

    let row1 = screen_to_char_offset(
        0,
        1,
        0,
        &v,
        &rope,
        &wrap,
        4,
        &ws(),
        &mut s,
        &no_providers(),
        10,
    );
    assert_eq!(row1, Some(4));
}

/// Click on column 2 in the second wrap row → char 6 ('g').
#[test]
fn wrap_click_mid_second_row() {
    let rope = Rope::from_str("abcdefgh\n");
    let v = vp(0, 10, 10);
    let wrap = WrapMode::Soft { width: 4 };
    let mut s = FormatScratch::new();

    let got = screen_to_char_offset(
        2,
        1,
        0,
        &v,
        &rope,
        &wrap,
        4,
        &ws(),
        &mut s,
        &no_providers(),
        10,
    );
    assert_eq!(got, Some(6)); // 'g' is char 6
}

/// Click below the last line is clamped to the last real line.
#[test]
fn wrap_click_below_last_line_clamped() {
    let rope = Rope::from_str("hi\n");
    let v = vp(0, 80, 10);
    let wrap = WrapMode::Soft { width: 40 };
    let mut s = FormatScratch::new();
    // Screen row 99 is past the end — should return something in line 0.
    let got = screen_to_char_offset(
        0,
        99,
        0,
        &v,
        &rope,
        &wrap,
        4,
        &ws(),
        &mut s,
        &no_providers(),
        80,
    );
    assert!(got.is_some());
}

// ── Virtual-line-aware row counting (synthetic provider) ────────────

/// A `VirtualLineSource` double that emits exactly one `Before(line)`
/// virtual row when queried for `line`, and nothing for any other line.
struct OneBeforeLine(usize);

impl hume_engine::providers::VirtualLineSource for OneBeforeLine {
    fn virtual_lines(
        &self,
        visible_lines: std::ops::Range<usize>,
        _content_width: u16,
        out: &mut Vec<hume_engine::providers::VirtualLine>,
    ) {
        if visible_lines.contains(&self.0) {
            out.push(hume_engine::providers::VirtualLine {
                anchor: hume_engine::providers::VirtualLineAnchor::Before(self.0),
                provider_id: 0,
                text: "V".to_string(),
                segments: Vec::new(),
            });
        }
    }
}

fn providers_with_before_line(line: usize) -> ProviderSet {
    let mut p = ProviderSet::new();
    p.add_virtual_line_source(Box::new(OneBeforeLine(line)));
    p
}

/// `screen_pos` must count a virtual-`Before` row anchored to the
/// cursor's own line as occupying a screen row above it — the cursor
/// must land one row lower than it would with zero providers.
///
/// See the `_no_wrap` sibling below — row math is wrap-mode-agnostic.
#[test]
fn screen_pos_accounts_for_a_virtual_before_line_on_the_cursors_line() {
    let rope = Rope::from_str("a\nb\nc\n");
    let v = vp(0, 80, 10);
    let wrap = WrapMode::Soft { width: 80 };
    let mut ctx = RenderContext::new();
    // Cursor at char 2 = start of line 1 ('b').
    let cursor_char = rope.line_to_char(1);

    let with_none = screen_pos(
        &v,
        &rope,
        cursor_char,
        &wrap,
        4,
        &ws(),
        &mut ctx,
        &no_providers(),
        80,
    );
    assert_eq!(
        with_none,
        Some((0, 1)),
        "sanity: no provider — cursor at row 1"
    );

    let with_virtual = screen_pos(
        &v,
        &rope,
        cursor_char,
        &wrap,
        4,
        &ws(),
        &mut ctx,
        &providers_with_before_line(1),
        80,
    );
    assert_eq!(
        with_virtual,
        Some((0, 2)),
        "a virtual row before line 1 must push the cursor down one more row"
    );
}

/// `screen_to_char_offset` must account for a virtual row stealing a
/// screen row from the lines below it: with a virtual-before row
/// inserted above line 1, screen row 2 is line 1's own content (pushed
/// down by the virtual row), not line 2's — a virtual-row-unaware
/// implementation would misidentify this row as line 2's.
///
/// Also covers a click that lands *on* the virtual row itself (screen
/// row 1): clamped to line 1's own first content sub-row (precise
/// anchor-line mapping isn't implemented yet). See the `_no_wrap` sibling
/// below — row math is wrap-mode-agnostic.
#[test]
fn screen_to_char_offset_accounts_for_a_stolen_virtual_row() {
    let rope = Rope::from_str("a\nb\nc\n");
    let v = vp(0, 80, 10);
    let wrap = WrapMode::Soft { width: 80 };
    let mut s = FormatScratch::new();
    let providers = providers_with_before_line(1);

    // Row layout: 0 = line 0 ('a'), 1 = virtual-before(line 1),
    // 2 = line 1's own content ('b'), 3 = line 2 ('c').
    let on_virtual_row =
        screen_to_char_offset(0, 1, 0, &v, &rope, &wrap, 4, &ws(), &mut s, &providers, 80);
    assert_eq!(
        on_virtual_row,
        Some(rope.line_to_char(1)),
        "a click on the virtual row clamps to line 1's own first char"
    );

    let on_pushed_down_content =
        screen_to_char_offset(0, 2, 0, &v, &rope, &wrap, 4, &ws(), &mut s, &providers, 80);
    assert_eq!(
        on_pushed_down_content,
        Some(rope.line_to_char(1)),
        "row 2 must resolve to line 1 (pushed down by the virtual row), not line 2"
    );

    let on_next_line =
        screen_to_char_offset(0, 3, 0, &v, &rope, &wrap, 4, &ws(), &mut s, &providers, 80);
    assert_eq!(
        on_next_line,
        Some(rope.line_to_char(2)),
        "row 3 must resolve to line 2, correctly accounting for the stolen row"
    );
}

/// No-wrap mirror of `screen_pos_accounts_for_a_virtual_before_line_on_the_cursors_line`
/// — row math is wrap-mode-agnostic (`display_rows_for_line` returns
/// `content: 1` for `WrapMode::None`), so the same virtual-row accounting
/// must hold with wrapping off.
#[test]
fn screen_pos_accounts_for_a_virtual_before_line_on_the_cursors_line_no_wrap() {
    let rope = Rope::from_str("a\nb\nc\n");
    let v = vp(0, 80, 10);
    let wrap = WrapMode::None;
    let mut ctx = RenderContext::new();
    let cursor_char = rope.line_to_char(1);

    let with_none = screen_pos(
        &v,
        &rope,
        cursor_char,
        &wrap,
        4,
        &ws(),
        &mut ctx,
        &no_providers(),
        80,
    );
    assert_eq!(
        with_none,
        Some((0, 1)),
        "sanity: no provider — cursor at row 1"
    );

    let with_virtual = screen_pos(
        &v,
        &rope,
        cursor_char,
        &wrap,
        4,
        &ws(),
        &mut ctx,
        &providers_with_before_line(1),
        80,
    );
    assert_eq!(
        with_virtual,
        Some((0, 2)),
        "a virtual row before line 1 must push the cursor down one more row, no-wrap too"
    );
}

/// No-wrap mirror of `screen_to_char_offset_accounts_for_a_stolen_virtual_row`.
#[test]
fn screen_to_char_offset_accounts_for_a_stolen_virtual_row_no_wrap() {
    let rope = Rope::from_str("a\nb\nc\n");
    let v = vp(0, 80, 10);
    let wrap = WrapMode::None;
    let mut s = FormatScratch::new();
    let providers = providers_with_before_line(1);

    let on_virtual_row =
        screen_to_char_offset(0, 1, 0, &v, &rope, &wrap, 4, &ws(), &mut s, &providers, 80);
    assert_eq!(
        on_virtual_row,
        Some(rope.line_to_char(1)),
        "a click on the virtual row clamps to line 1's own first char"
    );

    let on_pushed_down_content =
        screen_to_char_offset(0, 2, 0, &v, &rope, &wrap, 4, &ws(), &mut s, &providers, 80);
    assert_eq!(
        on_pushed_down_content,
        Some(rope.line_to_char(1)),
        "row 2 must resolve to line 1 (pushed down by the virtual row), not line 2"
    );

    let on_next_line =
        screen_to_char_offset(0, 3, 0, &v, &rope, &wrap, 4, &ws(), &mut s, &providers, 80);
    assert_eq!(
        on_next_line,
        Some(rope.line_to_char(2)),
        "row 3 must resolve to line 2, correctly accounting for the stolen row"
    );
}

// ── Buffer-edge virtual blocks: Before(0) and After(last_line) ──────────

/// A `VirtualLineSource` double that emits `self.1` distinct `After(line)`
/// rows, texted "1".."9", when queried for `line`.
struct MultiAfterLine(usize, usize);

impl hume_engine::providers::VirtualLineSource for MultiAfterLine {
    fn virtual_lines(
        &self,
        visible_lines: std::ops::Range<usize>,
        _content_width: u16,
        out: &mut Vec<hume_engine::providers::VirtualLine>,
    ) {
        if visible_lines.contains(&self.0) {
            for i in 0..self.1 {
                out.push(hume_engine::providers::VirtualLine {
                    anchor: hume_engine::providers::VirtualLineAnchor::After(self.0),
                    provider_id: 0,
                    text: (i + 1).to_string(),
                    segments: Vec::new(),
                });
            }
        }
    }
}

/// `screen_pos` for a cursor on buffer line 0 must count a `Before(0)`
/// block above it exactly like any other line's `before` block — no
/// special-casing at the very top of the buffer, in either wrap mode.
#[test]
fn screen_pos_accounts_for_before_line_0() {
    let rope = Rope::from_str("a\nb\n");
    let v = vp(0, 80, 10);
    let cursor_char = 0; // start of line 0
    let mut providers = ProviderSet::new();
    providers.add_virtual_line_source(Box::new(OneBeforeLine(0)));

    for wrap in [WrapMode::None, WrapMode::Soft { width: 80 }] {
        let mut ctx = RenderContext::new();
        let pos = screen_pos(
            &v,
            &rope,
            cursor_char,
            &wrap,
            4,
            &ws(),
            &mut ctx,
            &providers,
            80,
        );
        assert_eq!(
            pos,
            Some((0, 1)),
            "Before(0) pushes the cursor on line 0 down one row ({wrap:?})"
        );
    }
}

/// `screen_pos` for a cursor on the last real buffer line must not be
/// thrown off by an `After(last_line)` block anchored to that same line —
/// the block is *after* the cursor's row, so it must not affect the
/// cursor's own screen row at all (only rows below it).
#[test]
fn screen_pos_unaffected_by_after_on_cursors_own_last_line() {
    let rope = Rope::from_str("a\nb\n");
    let cursor_char = rope.line_to_char(1); // start of the last real line
    let mut providers = ProviderSet::new();
    providers.add_virtual_line_source(Box::new(MultiAfterLine(1, 3)));

    for wrap in [WrapMode::None, WrapMode::Soft { width: 80 }] {
        let v = vp(0, 80, 10);
        let mut ctx = RenderContext::new();
        let pos = screen_pos(
            &v,
            &rope,
            cursor_char,
            &wrap,
            4,
            &ws(),
            &mut ctx,
            &providers,
            80,
        );
        assert_eq!(
            pos,
            Some((0, 1)),
            "After(1) trails the cursor's own line — no effect on its row ({wrap:?})"
        );
    }
}
