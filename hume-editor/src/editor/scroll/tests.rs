use super::*;
use hume_engine::pane::{ViewportState, WhitespaceConfig, WrapMode};
use ropey::Rope;

fn viewport(top: usize, height: u16, width: u16) -> ViewportState {
    let mut v = ViewportState::new(width, height);
    v.top_line = top;
    v
}

fn rope(text: &str) -> Rope {
    Rope::from_str(text)
}

/// No `VirtualLineSource` registered — `display_rows_for_line` reduces
/// to `RowsBreakdown { before: 0, content, after: 0 }` for every line,
/// matching every test's virtual-line-unaware expectations exactly.
fn no_providers() -> ProviderSet {
    ProviderSet::new()
}

// ── ensure_cursor_visible (no-wrap) ──────────────────────────────────────

#[test]
fn no_wrap_cursor_visible_no_scroll_needed() {
    let r = rope("a\nb\nc\nd\ne\n");
    let mut v = viewport(0, 10, 80);
    ensure_cursor_visible(
        &mut v,
        &r,
        r.line_to_char(2),
        &WrapMode::None,
        4,
        &WhitespaceConfig::default(),
        &mut FormatScratch::new(),
        3,
        &no_providers(),
        80,
    );
    assert_eq!(v.top_line, 0);
}

#[test]
fn no_wrap_cursor_below_viewport_scrolls_down() {
    let r = rope("a\nb\nc\nd\ne\nf\ng\nh\n");
    let mut v = viewport(0, 5, 80);
    ensure_cursor_visible(
        &mut v,
        &r,
        r.line_to_char(7),
        &WrapMode::None,
        4,
        &WhitespaceConfig::default(),
        &mut FormatScratch::new(),
        3,
        &no_providers(),
        80,
    );
    let cursor_line = 7usize;
    assert!(cursor_line >= v.top_line);
    assert!(cursor_line < v.top_line + v.height as usize);
}

#[test]
fn no_wrap_cursor_above_viewport_scrolls_up() {
    let r = rope("a\nb\nc\nd\ne\nf\ng\nh\n");
    let mut v = viewport(5, 5, 80);
    ensure_cursor_visible(
        &mut v,
        &r,
        r.line_to_char(1),
        &WrapMode::None,
        4,
        &WhitespaceConfig::default(),
        &mut FormatScratch::new(),
        3,
        &no_providers(),
        80,
    );
    let cursor_line = 1usize;
    assert!(cursor_line >= v.top_line);
    assert!(cursor_line < v.top_line + v.height as usize);
}

// ── cursor_sub_row ───────────────────────────────────────────────────────

#[test]
fn cursor_sub_row_no_wrap() {
    // With a WrapMode::None, the whole line is one row, sub-row 0.
    let r = rope("hello world\n");
    let mut scratch = FormatScratch::new();
    let sub = cursor::sub_row(
        &r,
        0,
        5,
        &WrapMode::None,
        4,
        &WhitespaceConfig::default(),
        &mut scratch,
    );
    assert_eq!(sub, 0);
}

#[test]
fn cursor_sub_row_wrapped() {
    // "abcdefgh" with Soft { width: 4 } → 2 rows: "abcd" / "efgh".
    let r = rope("abcdefgh\n");
    let mut scratch = FormatScratch::new();
    // Cursor at char 0 → sub-row 0.
    let sub0 = cursor::sub_row(
        &r,
        0,
        0,
        &WrapMode::Soft { width: 4 },
        4,
        &WhitespaceConfig::default(),
        &mut scratch,
    );
    assert_eq!(sub0, 0);
    // Cursor at char 4 → sub-row 1.
    let sub1 = cursor::sub_row(
        &r,
        0,
        4,
        &WrapMode::Soft { width: 4 },
        4,
        &WhitespaceConfig::default(),
        &mut scratch,
    );
    assert_eq!(sub1, 1);
}

// ── ensure_cursor_visible (wrap) top/bottom margin enforcement ───────────
//
// 10 lines of "ab\n". Under Soft{width:2}, "ab" fills the wrap column
// exactly → 1 display row per line, exercising the wrapped code path
// (Soft is_wrapping=true) even though no line actually wraps. Viewport
// height=8, margin=2. Checks that scrolling lands the cursor exactly on
// the margin, both scrolling up (top) and down (bottom).

#[test]
fn wrap_cursor_within_top_margin_scrolls_up() {
    let r = rope(&"ab\n".repeat(10));
    let mut v = ViewportState::new(2, 8);
    v.top_line = 3;
    v.top_row_offset = 0;
    let cursor_char = r.line_to_char(3);
    ensure_cursor_visible(
        &mut v,
        &r,
        cursor_char,
        &WrapMode::Soft { width: 2 },
        4,
        &WhitespaceConfig::default(),
        &mut FormatScratch::new(),
        2,
        &no_providers(),
        2,
    );
    assert_eq!(v.top_line, 1);
    assert_eq!(v.top_row_offset, 0);
}

#[test]
fn wrap_cursor_within_bottom_margin_scrolls_down() {
    let r = rope(&"ab\n".repeat(10));
    let mut v = ViewportState::new(2, 8);
    v.top_line = 0;
    v.top_row_offset = 0;
    let cursor_char = r.line_to_char(7);
    ensure_cursor_visible(
        &mut v,
        &r,
        cursor_char,
        &WrapMode::Soft { width: 2 },
        4,
        &WhitespaceConfig::default(),
        &mut FormatScratch::new(),
        2,
        &no_providers(),
        2,
    );
    assert_eq!(v.top_line, 2);
    assert_eq!(v.top_row_offset, 0);
}

// ── zt / zb interaction with scrolloff ───────────────────────────────────
//
// `cmd_view_top` calls `scroll_cursor_to_row(target=0)`. That alone places
// the cursor at display row 0. `prepare_frame` then runs the standard
// `ensure_cursor_visible` with `scrolloff` and trims the cursor inward —
// vim's "smart scrolloff" semantics. This test pins the behaviour so a future
// change to either function can't silently break the contract.

#[test]
fn zt_then_scrolloff_trims_cursor_inward() {
    // 50 lines, no wrap, height=24, scrolloff=3. Cursor on line 25.
    let r = rope(&"a\n".repeat(50));
    let mut v = viewport(0, 24, 80);
    let cursor_char = r.line_to_char(25);

    // 1) `zt`: target_row = 0 → top_line = cursor_line.
    scroll_cursor_to_row(
        &mut v,
        &r,
        cursor_char,
        &WrapMode::None,
        4,
        &WhitespaceConfig::default(),
        &mut FormatScratch::new(),
        0,
        &no_providers(),
        80,
    );
    assert_eq!(v.top_line, 25, "zt places top at cursor line");

    // 2) Per-frame correction: scrolloff = 3 trims cursor inward by 3 rows.
    ensure_cursor_visible(
        &mut v,
        &r,
        cursor_char,
        &WrapMode::None,
        4,
        &WhitespaceConfig::default(),
        &mut FormatScratch::new(),
        3,
        &no_providers(),
        80,
    );
    assert_eq!(v.top_line, 22, "scrolloff trims top inward by margin (3)");
}

#[test]
fn zb_then_scrolloff_trims_cursor_inward() {
    // height=24, scrolloff=3. Cursor on line 25, target = height-1 = 23.
    let r = rope(&"a\n".repeat(50));
    let mut v = viewport(0, 24, 80);
    let cursor_char = r.line_to_char(25);

    scroll_cursor_to_row(
        &mut v,
        &r,
        cursor_char,
        &WrapMode::None,
        4,
        &WhitespaceConfig::default(),
        &mut FormatScratch::new(),
        23,
        &no_providers(),
        80,
    );
    assert_eq!(v.top_line, 2, "zb places cursor on display row 23");

    ensure_cursor_visible(
        &mut v,
        &r,
        cursor_char,
        &WrapMode::None,
        4,
        &WhitespaceConfig::default(),
        &mut FormatScratch::new(),
        3,
        &no_providers(),
        80,
    );
    // cursor_line=25, top=2, height=24, margin=3 → cursor at row 23 = height-margin-1.
    // bottom branch fires: top_line = 25 - (24-3-1) = 25 - 20 = 5.
    assert_eq!(v.top_line, 5, "scrolloff trims top up by margin (3)");
}

// ── Virtual-line-aware scrolling (synthetic provider) ───────────────

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

/// Emits `self.1` distinct `Before(self.0)` rows, texted "1".."9".
struct MultiBeforeLine(usize, usize);

impl hume_engine::providers::VirtualLineSource for MultiBeforeLine {
    fn virtual_lines(
        &self,
        visible_lines: std::ops::Range<usize>,
        _content_width: u16,
        out: &mut Vec<hume_engine::providers::VirtualLine>,
    ) {
        if visible_lines.contains(&self.0) {
            for i in 0..self.1 {
                out.push(hume_engine::providers::VirtualLine {
                    anchor: hume_engine::providers::VirtualLineAnchor::Before(self.0),
                    provider_id: 0,
                    text: (i + 1).to_string(),
                    segments: Vec::new(),
                });
            }
        }
    }
}

/// A virtual row anchored between the viewport's top and the cursor
/// "steals" a row from the lines below it — `ensure_cursor_visible` must
/// still scroll far enough to bring the cursor fully into view, not just
/// far enough for the content-only row count.
///
/// Checks the *robust* invariant (cursor lands inside the viewport,
/// verified through `screen_pos` the same way the render pipeline would
/// place the terminal cursor), not exact `top_line`/`top_row_offset`
/// values — landing precision exactly at a virtual block's boundary is
/// left untested here, deferred until a real `VirtualLineSource` exists.
#[test]
fn ensure_cursor_visible_accounts_for_a_stolen_virtual_row() {
    let r = rope("a\nb\nc\nd\n");
    let mut v = viewport(0, 2, 80);
    let wrap = WrapMode::Soft { width: 80 };
    let providers = providers_with_before_line(2);
    let cursor_char = r.line_to_char(3);

    ensure_cursor_visible(
        &mut v,
        &r,
        cursor_char,
        &wrap,
        4,
        &WhitespaceConfig::default(),
        &mut FormatScratch::new(),
        0,
        &providers,
        80,
    );

    let mut ctx = hume_engine::pipeline::RenderContext::new();
    let pos = cursor::screen_pos(
        &v,
        &r,
        cursor_char,
        &wrap,
        4,
        &WhitespaceConfig::default(),
        &mut ctx,
        &providers,
        80,
    );
    let (_, row) = pos.expect("cursor must be visible after ensure_cursor_visible");
    assert!(
        (row as usize) < v.height as usize,
        "cursor row {row} must be inside the {}-row viewport",
        v.height
    );
}

/// No-wrap mirror of `ensure_cursor_visible_accounts_for_a_stolen_virtual_row`
/// — row math is wrap-mode-agnostic.
#[test]
fn ensure_cursor_visible_accounts_for_a_stolen_virtual_row_no_wrap() {
    let r = rope("a\nb\nc\nd\n");
    let mut v = viewport(0, 2, 80);
    let wrap = WrapMode::None;
    let providers = providers_with_before_line(2);
    let cursor_char = r.line_to_char(3);

    ensure_cursor_visible(
        &mut v,
        &r,
        cursor_char,
        &wrap,
        4,
        &WhitespaceConfig::default(),
        &mut FormatScratch::new(),
        0,
        &providers,
        80,
    );

    let mut ctx = hume_engine::pipeline::RenderContext::new();
    let pos = cursor::screen_pos(
        &v,
        &r,
        cursor_char,
        &wrap,
        4,
        &WhitespaceConfig::default(),
        &mut ctx,
        &providers,
        80,
    );
    let (_, row) = pos.expect("cursor must be visible after ensure_cursor_visible");
    assert!(
        (row as usize) < v.height as usize,
        "cursor row {row} must be inside the {}-row viewport, no-wrap too",
        v.height
    );
}

/// A `Before(0)` block must be fully reachable by scrolling to the top of
/// the buffer — no special-casing hides or truncates it at the very start,
/// in either wrap mode. Mirrors the user-facing requirement behind this
/// fix: virtual lines anchored above buffer line 0 must be scrollable into
/// view just like any other `before` block.
///
/// Cursor starts on line 2 with the viewport already showing it; a margin
/// larger than the room available between `top_line` and the cursor forces
/// `scroll_backward_from_cursor` to walk past line 1, past line 0's own
/// content, and into line 0's 3-row `Before` block — landing at
/// `top_line == 0, top_row_offset == 0` (the block's very first row), the
/// correct top-of-buffer terminal state, rather than stopping short or
/// underflowing.
#[test]
fn scroll_backward_from_cursor_reaches_into_before_line_0() {
    let r = rope("a\nb\nc\n");
    let mut providers = ProviderSet::new();
    providers.add_virtual_line_source(Box::new(MultiBeforeLine(0, 3)));
    let cursor_char = r.line_to_char(2);

    for wrap in [WrapMode::None, WrapMode::Soft { width: 80 }] {
        let mut v = viewport(2, 10, 80);
        ensure_cursor_visible(
            &mut v,
            &r,
            cursor_char,
            &wrap,
            4,
            &WhitespaceConfig::default(),
            &mut FormatScratch::new(),
            10, // margin far larger than the 2 real rows between top and cursor
            &providers,
            80,
        );
        assert_eq!(
            v.top_line, 0,
            "walk reaches the very first buffer line ({wrap:?})"
        );
        assert_eq!(
            v.top_row_offset, 0,
            "walk reaches the first row of Before(0)'s block, not a partial offset ({wrap:?})"
        );
    }
}

/// `clamp_top_row_offset` must shrink an out-of-range offset (as `recall_scroll`
/// or an LSP jump could leave behind) down to the top line's actual current
/// block size, in either wrap mode.
#[test]
fn clamp_top_row_offset_shrinks_stale_offset() {
    let r = rope("a\nb\n");
    let providers = providers_with_before_line(0); // Before(0): 1 row + content: 1 row = total 2

    for wrap in [WrapMode::None, WrapMode::Soft { width: 80 }] {
        let mut v = viewport(0, 5, 80);
        v.top_row_offset = 200; // wildly stale — e.g. a resize shrank the block since it was set
        clamp_top_row_offset(
            &mut v,
            &r,
            &wrap,
            4,
            &WhitespaceConfig::default(),
            &mut FormatScratch::new(),
            &providers,
            80,
        );
        assert_eq!(
            v.top_row_offset, 1,
            "clamped to the block's last valid row (total 2, so max offset 1) ({wrap:?})"
        );
    }
}

/// `clamp_top_row_offset` must leave an already-valid offset untouched.
#[test]
fn clamp_top_row_offset_is_a_noop_when_already_valid() {
    let r = rope("a\nb\n");
    let providers = providers_with_before_line(0);
    let mut v = viewport(0, 5, 80);
    v.top_row_offset = 1;
    clamp_top_row_offset(
        &mut v,
        &r,
        &WrapMode::None,
        4,
        &WhitespaceConfig::default(),
        &mut FormatScratch::new(),
        &providers,
        80,
    );
    assert_eq!(v.top_row_offset, 1);
}
