use super::*;
use hume_engine::format::FormatScratch;
use hume_engine::pane::{ViewportState, WhitespaceConfig, WrapMode};
use hume_engine::providers::ProviderSet;
use hume_engine::rows::RowMap;
use ropey::Rope;

fn vp(top_line: usize, width: u16, height: u16) -> ViewportState {
    let mut v = ViewportState::new(width, height);
    v.top_line = top_line;
    v
}

fn ws() -> WhitespaceConfig {
    WhitespaceConfig::default()
}

fn map<'a>(
    rope: &'a Rope,
    wrap: WrapMode,
    providers: &'a ProviderSet,
    content_width: u16,
    scratch: &'a mut FormatScratch,
) -> RowMap<'a> {
    RowMap::new(rope, wrap, 4, ws(), providers, content_width, scratch)
}

/// No decoration source registered — every line's block reduces to its
/// content rows, matching every test below's virtual-line-unaware expectations
/// exactly.
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
    let providers = no_providers();
    let mut s = FormatScratch::new();
    let got = screen_to_char_offset(
        0,
        0,
        0,
        &v,
        &mut map(&rope, WrapMode::None, &providers, 80, &mut s),
    );
    assert_eq!(got, Some(0));
}

/// Click on column 2 of line 0 → char 2.
#[test]
fn nowrap_click_mid_first_line() {
    let rope = Rope::from_str("abc\ndef\n");
    let v = vp(0, 80, 10);
    let providers = no_providers();
    let mut s = FormatScratch::new();
    let got = screen_to_char_offset(
        2,
        0,
        0,
        &v,
        &mut map(&rope, WrapMode::None, &providers, 80, &mut s),
    );
    assert_eq!(got, Some(2));
}

/// Click on screen row 1, column 0 → start of second line (char 4).
#[test]
fn nowrap_click_second_line() {
    let rope = Rope::from_str("abc\ndef\n");
    let v = vp(0, 80, 10);
    let providers = no_providers();
    let mut s = FormatScratch::new();
    let got = screen_to_char_offset(
        0,
        1,
        0,
        &v,
        &mut map(&rope, WrapMode::None, &providers, 80, &mut s),
    );
    assert_eq!(got, Some(4)); // 'd' is char 4
}

/// Click in the gutter (screen_x < gutter_w) returns None.
#[test]
fn nowrap_gutter_click_returns_none() {
    let rope = Rope::from_str("abc\n");
    let v = vp(0, 80, 10);
    let providers = no_providers();
    let mut s = FormatScratch::new();
    // gutter_w = 4; click at column 2 is inside the gutter.
    let got = screen_to_char_offset(
        2,
        0,
        4,
        &v,
        &mut map(&rope, WrapMode::None, &providers, 80, &mut s),
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
    let providers = no_providers();
    let mut s = FormatScratch::new();
    // Click at column 99, way past "hi" — lands at '\n' (char 2), the eol marker.
    let got = screen_to_char_offset(
        99,
        0,
        0,
        &v,
        &mut map(&rope, WrapMode::None, &providers, 80, &mut s),
    );
    assert_eq!(got, Some(2));
}

/// Viewport scrolled down: screen_y=0 refers to top_line, not line 0.
#[test]
fn nowrap_viewport_scrolled() {
    // Lines: 0=a, 1=b, 2=c, 3=d. top_line=2 → screen row 0 is line 2 = 'c'.
    let rope = Rope::from_str("a\nb\nc\nd\n");
    let v = vp(2, 80, 10); // top_line = 2
    let providers = no_providers();
    let mut s = FormatScratch::new();
    // Line 2 starts at char 4 ('c'). Screen row 0, col 0 → char 4.
    let got = screen_to_char_offset(
        0,
        0,
        0,
        &v,
        &mut map(&rope, WrapMode::None, &providers, 80, &mut s),
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
    let providers = no_providers();
    let mut s = FormatScratch::new();
    let got = screen_to_char_offset(
        0,
        0,
        0,
        &v,
        &mut map(&rope, WrapMode::None, &providers, 80, &mut s),
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
    let providers = no_providers();
    let mut s = FormatScratch::new();

    let row0 = screen_to_char_offset(0, 0, 0, &v, &mut map(&rope, wrap, &providers, 10, &mut s));
    assert_eq!(row0, Some(0));

    let row1 = screen_to_char_offset(0, 1, 0, &v, &mut map(&rope, wrap, &providers, 10, &mut s));
    assert_eq!(row1, Some(4));
}

/// Click on column 2 in the second wrap row → char 6 ('g').
#[test]
fn wrap_click_mid_second_row() {
    let rope = Rope::from_str("abcdefgh\n");
    let v = vp(0, 10, 10);
    let wrap = WrapMode::Soft { width: 4 };
    let providers = no_providers();
    let mut s = FormatScratch::new();

    let got = screen_to_char_offset(2, 1, 0, &v, &mut map(&rope, wrap, &providers, 10, &mut s));
    assert_eq!(got, Some(6)); // 'g' is char 6
}

/// Click below the last line is clamped to the last real line.
#[test]
fn wrap_click_below_last_line_clamped() {
    let rope = Rope::from_str("hi\n");
    let v = vp(0, 80, 10);
    let wrap = WrapMode::Soft { width: 40 };
    let providers = no_providers();
    let mut s = FormatScratch::new();
    // Screen row 99 is past the end — should return something in line 0.
    let got = screen_to_char_offset(0, 99, 0, &v, &mut map(&rope, wrap, &providers, 80, &mut s));
    assert!(got.is_some());
}

// ── Virtual-line-aware row counting (synthetic provider) ────────────

/// A `DecorationSource` double that emits exactly one `Before(line)`
/// virtual row when queried for `line`, and nothing for any other line.
struct OneBeforeLine(usize);

impl hume_engine::providers::DecorationSource for OneBeforeLine {
    fn kinds(&self) -> hume_engine::providers::DecorationKinds {
        hume_engine::providers::DecorationKinds::VIRTUAL_LINE
    }
    fn decorations_for_line(
        &self,
        line_idx: usize,
        out: &mut Vec<hume_engine::providers::Decoration>,
    ) {
        if line_idx == self.0 {
            out.push(hume_engine::providers::Decoration::VirtualLine(
                hume_engine::providers::VirtualLine {
                    anchor: hume_engine::providers::VirtualLineAnchor::Before(self.0),
                    provider_id: 0,
                    text: "V".to_string(),
                    segments: Vec::new(),
                },
            ));
        }
    }
}

fn providers_with_before_line(line: usize) -> ProviderSet {
    let mut p = ProviderSet::new();
    p.add_decoration_source(Box::new(OneBeforeLine(line)));
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
    // Cursor at char 2 = start of line 1 ('b').
    let cursor_char = rope.line_to_char(1);

    let bare = no_providers();
    let mut s = FormatScratch::new();
    let with_none = screen_pos(&v, &mut map(&rope, wrap, &bare, 80, &mut s), cursor_char);
    assert_eq!(
        with_none,
        Some((0, 1)),
        "sanity: no provider — cursor at row 1"
    );

    let providers = providers_with_before_line(1);
    let mut s = FormatScratch::new();
    let with_virtual = screen_pos(
        &v,
        &mut map(&rope, wrap, &providers, 80, &mut s),
        cursor_char,
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
    let providers = providers_with_before_line(1);
    let mut s = FormatScratch::new();

    // Row layout: 0 = line 0 ('a'), 1 = virtual-before(line 1),
    // 2 = line 1's own content ('b'), 3 = line 2 ('c').
    let on_virtual_row =
        screen_to_char_offset(0, 1, 0, &v, &mut map(&rope, wrap, &providers, 80, &mut s));
    assert_eq!(
        on_virtual_row,
        Some(rope.line_to_char(1)),
        "a click on the virtual row clamps to line 1's own first char"
    );

    let on_pushed_down_content =
        screen_to_char_offset(0, 2, 0, &v, &mut map(&rope, wrap, &providers, 80, &mut s));
    assert_eq!(
        on_pushed_down_content,
        Some(rope.line_to_char(1)),
        "row 2 must resolve to line 1 (pushed down by the virtual row), not line 2"
    );

    let on_next_line =
        screen_to_char_offset(0, 3, 0, &v, &mut map(&rope, wrap, &providers, 80, &mut s));
    assert_eq!(
        on_next_line,
        Some(rope.line_to_char(2)),
        "row 3 must resolve to line 2, correctly accounting for the stolen row"
    );
}

/// No-wrap mirror of `screen_pos_accounts_for_a_virtual_before_line_on_the_cursors_line`
/// — row math is wrap-mode-agnostic (a line occupies exactly one content row
/// with wrapping off), so the same virtual-row accounting must hold.
#[test]
fn screen_pos_accounts_for_a_virtual_before_line_on_the_cursors_line_no_wrap() {
    let rope = Rope::from_str("a\nb\nc\n");
    let v = vp(0, 80, 10);
    let wrap = WrapMode::None;
    let cursor_char = rope.line_to_char(1);

    let bare = no_providers();
    let mut s = FormatScratch::new();
    let with_none = screen_pos(&v, &mut map(&rope, wrap, &bare, 80, &mut s), cursor_char);
    assert_eq!(
        with_none,
        Some((0, 1)),
        "sanity: no provider — cursor at row 1"
    );

    let providers = providers_with_before_line(1);
    let mut s = FormatScratch::new();
    let with_virtual = screen_pos(
        &v,
        &mut map(&rope, wrap, &providers, 80, &mut s),
        cursor_char,
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
    let providers = providers_with_before_line(1);
    let mut s = FormatScratch::new();

    let on_virtual_row =
        screen_to_char_offset(0, 1, 0, &v, &mut map(&rope, wrap, &providers, 80, &mut s));
    assert_eq!(
        on_virtual_row,
        Some(rope.line_to_char(1)),
        "a click on the virtual row clamps to line 1's own first char"
    );

    let on_pushed_down_content =
        screen_to_char_offset(0, 2, 0, &v, &mut map(&rope, wrap, &providers, 80, &mut s));
    assert_eq!(
        on_pushed_down_content,
        Some(rope.line_to_char(1)),
        "row 2 must resolve to line 1 (pushed down by the virtual row), not line 2"
    );

    let on_next_line =
        screen_to_char_offset(0, 3, 0, &v, &mut map(&rope, wrap, &providers, 80, &mut s));
    assert_eq!(
        on_next_line,
        Some(rope.line_to_char(2)),
        "row 3 must resolve to line 2, correctly accounting for the stolen row"
    );
}

// ── Buffer-edge virtual blocks: Before(0) and After(last_line) ──────────

/// A `DecorationSource` double that emits `self.1` distinct `After(line)`
/// rows, texted "1".."9", when queried for `line`.
struct MultiAfterLine(usize, usize);

impl hume_engine::providers::DecorationSource for MultiAfterLine {
    fn kinds(&self) -> hume_engine::providers::DecorationKinds {
        hume_engine::providers::DecorationKinds::VIRTUAL_LINE
    }
    fn decorations_for_line(
        &self,
        line_idx: usize,
        out: &mut Vec<hume_engine::providers::Decoration>,
    ) {
        if line_idx == self.0 {
            for i in 0..self.1 {
                out.push(hume_engine::providers::Decoration::VirtualLine(
                    hume_engine::providers::VirtualLine {
                        anchor: hume_engine::providers::VirtualLineAnchor::After(self.0),
                        provider_id: 0,
                        text: (i + 1).to_string(),
                        segments: Vec::new(),
                    },
                ));
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
    providers.add_decoration_source(Box::new(OneBeforeLine(0)));

    for wrap in [WrapMode::None, WrapMode::Soft { width: 80 }] {
        let mut s = FormatScratch::new();
        let pos = screen_pos(
            &v,
            &mut map(&rope, wrap, &providers, 80, &mut s),
            cursor_char,
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
    providers.add_decoration_source(Box::new(MultiAfterLine(1, 3)));

    for wrap in [WrapMode::None, WrapMode::Soft { width: 80 }] {
        let v = vp(0, 80, 10);
        let mut s = FormatScratch::new();
        let pos = screen_pos(
            &v,
            &mut map(&rope, wrap, &providers, 80, &mut s),
            cursor_char,
        );
        assert_eq!(
            pos,
            Some((0, 1)),
            "After(1) trails the cursor's own line — no effect on its row ({wrap:?})"
        );
    }
}

/// `screen_pos` must clamp a stale `top_row_offset` the same way
/// `pane_render.rs`'s row walk clamps its own start address (`RowMap::clamp`)
/// before drawing — a write site that never validates the offset against the
/// block it addresses (`recall_scroll`, an LSP jump) can leave it pointing
/// past a line's current block, e.g. after a `Before` block shrinks.
///
/// Line 0's block is `Before(0)` (1 row) + content (1 row) = 2 rows total,
/// valid addresses 0..2. `top_row_offset = 2` is one past the end — stale,
/// as if the block used to be taller. The cursor sits on line 0's own
/// content row (address `(0, 1)`), which the clamped top (`(0, 1)`) resolves
/// to directly (distance 0); walking forward from the raw, unclamped
/// address `(0, 2)` immediately steps to line 1 (`next` only checks
/// `row + 1 < total`, so any row `>= total` jumps a whole line at once) and
/// permanently overshoots the cursor, since `distance` only walks forward —
/// yielding `None` instead of the clamped answer.
#[test]
fn screen_pos_clamps_a_top_row_offset_past_the_lines_current_block() {
    let rope = Rope::from_str("a\nb\n");
    let mut v = vp(0, 80, 10);
    v.top_row_offset = 2; // past line 0's 2-row block (before=1, content=1)
    let cursor_char = 0; // 'a' — line 0's own content row, address (0, 1)
    let providers = providers_with_before_line(0);
    let mut s = FormatScratch::new();

    let pos = screen_pos(
        &v,
        &mut map(&rope, WrapMode::None, &providers, 80, &mut s),
        cursor_char,
    );
    assert_eq!(
        pos,
        Some((0, 0)),
        "clamped top (0,1) sits exactly on the cursor's row — distance 0"
    );
}

/// A zero-height viewport (a pane collapsed to nothing mid-resize) has no
/// row to place the cursor on.
#[test]
fn screen_pos_zero_height_returns_none() {
    let rope = Rope::from_str("a\nb\nc\n");
    let v = vp(0, 80, 0);
    let providers = no_providers();
    let mut s = FormatScratch::new();

    let pos = screen_pos(
        &v,
        &mut map(&rope, WrapMode::None, &providers, 80, &mut s),
        0,
    );
    assert_eq!(pos, None);
}

/// A cursor more rows below the viewport's top than the viewport is tall —
/// the case `ensure_cursor_visible` is supposed to prevent, but `screen_pos`
/// must still answer `None` rather than a row past the visible window.
#[test]
fn screen_pos_cursor_below_viewport_returns_none() {
    let rope = Rope::from_str("a\nb\nc\nd\ne\nf\n");
    let v = vp(0, 80, 2); // only rows for lines 0-1 are visible
    let providers = no_providers();
    let mut s = FormatScratch::new();
    let cursor_char = rope.line_to_char(5); // 'f' — 5 rows below the top

    let pos = screen_pos(
        &v,
        &mut map(&rope, WrapMode::None, &providers, 80, &mut s),
        cursor_char,
    );
    assert_eq!(pos, None);
}
