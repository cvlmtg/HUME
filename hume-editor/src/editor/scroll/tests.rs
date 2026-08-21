use super::*;
use hume_engine::format::FormatScratch;
use hume_engine::pane::{ViewportState, WhitespaceConfig, WrapMode};
use hume_engine::providers::ProviderSet;
use ropey::Rope;

use crate::editor::cursor;

fn viewport(top: usize, height: u16, width: u16) -> ViewportState {
    let mut v = ViewportState::new(width, height);
    v.top_line = top;
    v
}

fn rope(text: &str) -> Rope {
    Rope::from_str(text)
}

fn map<'a>(
    rope: &'a Rope,
    wrap: WrapMode,
    providers: &'a ProviderSet,
    content_width: u16,
    scratch: &'a mut FormatScratch,
) -> RowMap<'a> {
    RowMap::new(
        rope,
        wrap,
        4,
        WhitespaceConfig::default(),
        providers,
        content_width,
        scratch,
    )
}

/// No decoration source registered — every line's block reduces to its
/// content rows, matching every test's virtual-line-unaware expectations
/// exactly.
fn no_providers() -> ProviderSet {
    ProviderSet::new()
}

// ── ensure_cursor_visible (no-wrap) ──────────────────────────────────────

#[test]
fn no_wrap_cursor_visible_no_scroll_needed() {
    let r = rope("a\nb\nc\nd\ne\n");
    let mut v = viewport(0, 10, 80);
    let providers = no_providers();
    let mut s = FormatScratch::new();
    ensure_cursor_visible(
        &mut v,
        &mut map(&r, WrapMode::None, &providers, 80, &mut s),
        RowPos::new(2, 0),
        3,
    );
    assert_eq!(v.top_line, 0);
}

#[test]
fn no_wrap_cursor_below_viewport_scrolls_down() {
    let r = rope("a\nb\nc\nd\ne\nf\ng\nh\n");
    let mut v = viewport(0, 5, 80);
    let providers = no_providers();
    let mut s = FormatScratch::new();
    ensure_cursor_visible(
        &mut v,
        &mut map(&r, WrapMode::None, &providers, 80, &mut s),
        RowPos::new(7, 0),
        3,
    );
    let cursor_line = 7usize;
    assert!(cursor_line >= v.top_line);
    assert!(cursor_line < v.top_line + v.height as usize);
}

#[test]
fn no_wrap_cursor_above_viewport_scrolls_up() {
    let r = rope("a\nb\nc\nd\ne\nf\ng\nh\n");
    let mut v = viewport(5, 5, 80);
    let providers = no_providers();
    let mut s = FormatScratch::new();
    ensure_cursor_visible(
        &mut v,
        &mut map(&r, WrapMode::None, &providers, 80, &mut s),
        RowPos::new(1, 0),
        3,
    );
    let cursor_line = 1usize;
    assert!(cursor_line >= v.top_line);
    assert!(cursor_line < v.top_line + v.height as usize);
}

/// A `scrolloff` at or above half the viewport height (`:set scrolloff=999`'s
/// "always center" idiom, at an even height) used to leave the "no scroll
/// needed" window empty: the two correction arms disagreed about where the
/// cursor should land and rescrolled every single frame. Calling
/// `ensure_cursor_visible` again with the cursor unmoved must be a no-op —
/// it wasn't, before capping the margin at `(height - 1) / 2`.
#[test]
fn no_wrap_huge_scrolloff_at_even_height_settles_after_one_scroll() {
    let text: String = (0..50).map(|i| format!("line{i}\n")).collect();
    let r = rope(&text);
    let mut v = viewport(0, 24, 80);
    let providers = no_providers();
    let cursor_pos = RowPos::new(20, 0);

    let mut s = FormatScratch::new();
    ensure_cursor_visible(
        &mut v,
        &mut map(&r, WrapMode::None, &providers, 80, &mut s),
        cursor_pos,
        999,
    );
    let top_after_first = v.top_line;

    let mut s = FormatScratch::new();
    ensure_cursor_visible(
        &mut v,
        &mut map(&r, WrapMode::None, &providers, 80, &mut s),
        cursor_pos,
        999,
    );

    assert_eq!(
        v.top_line, top_after_first,
        "ensure_cursor_visible must be a fixed point once the cursor is already visible"
    );
}

// ── cursor sub-row ───────────────────────────────────────────────────────

#[test]
fn cursor_sub_row_no_wrap() {
    // With a WrapMode::None, the whole line is one row, sub-row 0.
    let r = rope("hello world\n");
    let providers = no_providers();
    let mut s = FormatScratch::new();
    let mut rm = map(&r, WrapMode::None, &providers, 80, &mut s);
    let sub = rm.locate(5).0.row;
    assert_eq!(sub, 0);
}

#[test]
fn cursor_sub_row_wrapped() {
    // "abcdefgh" with Soft { width: 4 } → 2 rows: "abcd" / "efgh".
    let r = rope("abcdefgh\n");
    let providers = no_providers();
    let mut s = FormatScratch::new();
    let mut rm = map(&r, WrapMode::Soft { width: 4 }, &providers, 80, &mut s);
    // Cursor at char 0 → sub-row 0.
    assert_eq!(rm.locate(0).0.row, 0);
    // Cursor at char 4 → sub-row 1.
    assert_eq!(rm.locate(4).0.row, 1);
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
    let providers = no_providers();
    let mut s = FormatScratch::new();
    let mut rm = map(&r, WrapMode::Soft { width: 2 }, &providers, 2, &mut s);
    let cursor_pos = rm.locate_row(cursor_char);
    ensure_cursor_visible(&mut v, &mut rm, cursor_pos, 2);
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
    let providers = no_providers();
    let mut s = FormatScratch::new();
    let mut rm = map(&r, WrapMode::Soft { width: 2 }, &providers, 2, &mut s);
    let cursor_pos = rm.locate_row(cursor_char);
    ensure_cursor_visible(&mut v, &mut rm, cursor_pos, 2);
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
    let providers = no_providers();

    // 1) `zt`: target_row = 0 → top_line = cursor_line.
    let mut s = FormatScratch::new();
    scroll_cursor_to_row(
        &mut v,
        &mut map(&r, WrapMode::None, &providers, 80, &mut s),
        cursor_char,
        0,
    );
    assert_eq!(v.top_line, 25, "zt places top at cursor line");

    // 2) Per-frame correction: scrolloff = 3 trims cursor inward by 3 rows.
    let mut s = FormatScratch::new();
    ensure_cursor_visible(
        &mut v,
        &mut map(&r, WrapMode::None, &providers, 80, &mut s),
        RowPos::new(25, 0),
        3,
    );
    assert_eq!(v.top_line, 22, "scrolloff trims top inward by margin (3)");
}

#[test]
fn zb_then_scrolloff_trims_cursor_inward() {
    // height=24, scrolloff=3. Cursor on line 25, target = height-1 = 23.
    let r = rope(&"a\n".repeat(50));
    let mut v = viewport(0, 24, 80);
    let cursor_char = r.line_to_char(25);
    let providers = no_providers();

    let mut s = FormatScratch::new();
    scroll_cursor_to_row(
        &mut v,
        &mut map(&r, WrapMode::None, &providers, 80, &mut s),
        cursor_char,
        23,
    );
    assert_eq!(v.top_line, 2, "zb places cursor on display row 23");

    let mut s = FormatScratch::new();
    ensure_cursor_visible(
        &mut v,
        &mut map(&r, WrapMode::None, &providers, 80, &mut s),
        RowPos::new(25, 0),
        3,
    );
    // cursor_line=25, top=2, height=24, margin=3 → cursor at row 23 = height-margin-1.
    // bottom branch fires: top_line = 25 - (24-3-1) = 25 - 20 = 5.
    assert_eq!(v.top_line, 5, "scrolloff trims top up by margin (3)");
}

// ── Virtual-line-aware scrolling (synthetic provider) ───────────────

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
                    base_scope: None,
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

/// Emits `self.1` distinct `Before(self.0)` rows, texted "1".."9".
struct MultiBeforeLine(usize, usize);

impl hume_engine::providers::DecorationSource for MultiBeforeLine {
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
                        anchor: hume_engine::providers::VirtualLineAnchor::Before(self.0),
                        provider_id: 0,
                        text: (i + 1).to_string(),
                        segments: Vec::new(),
                        base_scope: None,
                    },
                ));
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
/// left untested here.
#[test]
fn ensure_cursor_visible_accounts_for_a_stolen_virtual_row() {
    let r = rope("a\nb\nc\nd\n");
    let mut v = viewport(0, 2, 80);
    let wrap = WrapMode::Soft { width: 80 };
    let providers = providers_with_before_line(2);
    let cursor_char = r.line_to_char(3);

    let mut s = FormatScratch::new();
    let mut rm = map(&r, wrap, &providers, 80, &mut s);
    let cursor_pos = rm.locate_row(cursor_char);
    ensure_cursor_visible(&mut v, &mut rm, cursor_pos, 0);

    let mut s = FormatScratch::new();
    let pos = cursor::screen_pos(&v, &mut map(&r, wrap, &providers, 80, &mut s), cursor_char);
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

    let mut s = FormatScratch::new();
    let mut rm = map(&r, wrap, &providers, 80, &mut s);
    let cursor_pos = rm.locate_row(cursor_char);
    ensure_cursor_visible(&mut v, &mut rm, cursor_pos, 0);

    let mut s = FormatScratch::new();
    let pos = cursor::screen_pos(&v, &mut map(&r, wrap, &providers, 80, &mut s), cursor_char);
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
/// the backward walk past line 1, past line 0's own content, and into line
/// 0's 3-row `Before` block — landing at `top_line == 0, top_row_offset ==
/// 0` (the block's very first row), the correct top-of-buffer terminal
/// state, rather than stopping short or underflowing.
#[test]
fn scroll_backward_from_cursor_reaches_into_before_line_0() {
    let r = rope("a\nb\nc\n");
    let mut providers = ProviderSet::new();
    providers.add_decoration_source(Box::new(MultiBeforeLine(0, 3)));
    let cursor_char = r.line_to_char(2);

    for wrap in [WrapMode::None, WrapMode::Soft { width: 80 }] {
        // Height 20 (not 10): `ensure_cursor_visible` caps the margin at
        // `(height - 1) / 2`, so the margin needs enough headroom from the
        // height alone to stay "far larger" than the 5-row walk back to
        // line 0's Before block, regardless of the `v_margin` passed below.
        let mut v = viewport(2, 20, 80);
        let mut s = FormatScratch::new();
        let mut rm = map(&r, wrap, &providers, 80, &mut s);
        let cursor_pos = rm.locate_row(cursor_char);
        ensure_cursor_visible(
            &mut v, &mut rm, cursor_pos,
            20, // margin far larger than the 2 real rows between top and cursor
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

/// `clamp_viewport_top` must shrink an out-of-range offset (as `recall_scroll`
/// or an LSP jump could leave behind) down to the top line's actual current
/// block size, in either wrap mode.
#[test]
fn clamp_top_row_offset_shrinks_stale_offset() {
    let r = rope("a\nb\n");
    let providers = providers_with_before_line(0); // Before(0): 1 row + content: 1 row = total 2

    for wrap in [WrapMode::None, WrapMode::Soft { width: 80 }] {
        let mut v = viewport(0, 5, 80);
        v.top_row_offset = 200; // wildly stale — e.g. a resize shrank the block since it was set
        let mut s = FormatScratch::new();
        clamp_viewport_top(&mut v, &mut map(&r, wrap, &providers, 80, &mut s));
        assert_eq!(
            v.top_row_offset, 1,
            "clamped to the block's last valid row (total 2, so max offset 1) ({wrap:?})"
        );
    }
}

/// `clamp_viewport_top` must leave an already-valid offset untouched.
#[test]
fn clamp_top_row_offset_is_a_noop_when_already_valid() {
    let r = rope("a\nb\n");
    let providers = providers_with_before_line(0);
    let mut v = viewport(0, 5, 80);
    v.top_row_offset = 1;
    let mut s = FormatScratch::new();
    clamp_viewport_top(&mut v, &mut map(&r, WrapMode::None, &providers, 80, &mut s));
    assert_eq!(v.top_row_offset, 1);
}

// ── ensure_cursor_visible_horizontal ─────────────────────────────────────

/// `locate`'s column is content-relative (gutter already subtracted), so the
/// margin check must compare it against the map's own `content_width`, not
/// `viewport.width` (still gutter-inclusive). Viewport width 80, content
/// width 72 (an 8-column gutter): cursor at column 70 is inside the margin
/// measured against content width (70 >= 72 - 5) but not against the wider
/// viewport width (70 < 80 - 5) — so a scroll fires here only if the fix is
/// in place.
#[test]
fn horizontal_scroll_margin_uses_content_width_not_viewport_width() {
    let r = rope(&("a".repeat(100) + "\n"));
    let mut v = viewport(0, 10, 80);
    let providers = no_providers();
    let mut s = FormatScratch::new();
    let cursor_char = 70;

    let mut rm = map(&r, WrapMode::None, &providers, 72, &mut s);
    let cursor_col = rm.locate(cursor_char).1;
    ensure_cursor_visible_horizontal(&mut v, &mut rm, cursor_col);

    assert_eq!(
        v.horizontal_offset, 4,
        "cursor_col(70) - (content_width(72) - margin(5) - 1) = 4"
    );
}

/// Same cursor position, no scroll needed once the (correct) content width
/// is wide enough that the cursor sits outside the margin — sanity check
/// that the assertion above isn't just "always scrolls".
#[test]
fn horizontal_scroll_margin_no_scroll_when_within_content_width() {
    let r = rope(&("a".repeat(100) + "\n"));
    let mut v = viewport(0, 10, 80);
    let providers = no_providers();
    let mut s = FormatScratch::new();
    let cursor_char = 70;

    let mut rm = map(&r, WrapMode::None, &providers, 80, &mut s);
    let cursor_col = rm.locate(cursor_char).1;
    ensure_cursor_visible_horizontal(&mut v, &mut rm, cursor_col);

    assert_eq!(v.horizontal_offset, 0, "70 < content_width(80) - margin(5)");
}

/// A cursor past column 65535 on a huge unwrapped line must scroll to its
/// true (unclamped) column, not a `u16`-truncated one — regression guard
/// for the `u16` → `u32` display-column widening. Independent oracle: every
/// char is 1 column wide, so `cursor_col == cursor_char`, and the expected
/// offset is plain arithmetic on that value.
#[test]
fn horizontal_scroll_reaches_past_former_u16_column_ceiling() {
    let r = rope(&("a".repeat(70_000) + "\n"));
    let mut v = viewport(0, 10, 80);
    let providers = no_providers();
    let mut s = FormatScratch::new();
    let cursor_char = 69_999; // last 'a', column 69_999 — past u16::MAX (65_535)

    // The column is resolved through `locate`, not passed in as a literal:
    // the narrowing this guards against would live in that resolution, and a
    // hand-written column would step over the very code under test.
    let mut rm = map(&r, WrapMode::None, &providers, 80, &mut s);
    let cursor_col = rm.locate(cursor_char).1;
    ensure_cursor_visible_horizontal(&mut v, &mut rm, cursor_col);

    assert_eq!(
        v.horizontal_offset, 69_925,
        "cursor_col(69_999) - (content_width(80) - margin(5) - 1) = 69_925"
    );
    assert!(
        v.horizontal_offset > u16::MAX as u32,
        "offset must exceed the former u16 ceiling, not wrap/truncate into it"
    );
}

// ── One cursor resolution per frame ──────────────────────────────────────
//
// `lifecycle::scroll_into_view` resolves the cursor once and hands the result
// to the draw path, which used to rebuild a row map and re-derive it. The two
// tests below cover the halves of that claim: the row it reports is the row a
// forward walk finds, and resolving it costs one format.

/// Counts formats of `line`: `RowMap::format_line` queries every registered
/// INLINE-kind `DecorationSource` exactly once per format and nowhere else,
/// so this counts `format_buffer_line` runs without depending on anything
/// the row map reports about itself.
struct CountFormatsOf(usize, std::rc::Rc<std::cell::Cell<usize>>);

impl hume_engine::providers::DecorationSource for CountFormatsOf {
    fn kinds(&self) -> hume_engine::providers::DecorationKinds {
        hume_engine::providers::DecorationKinds::INLINE
    }
    fn decorations_for_line(
        &self,
        line_idx: usize,
        _out: &mut Vec<hume_engine::providers::Decoration>,
    ) {
        if line_idx == self.0 {
            self.1.set(self.1.get() + 1);
        }
    }
}

/// `ensure_cursor_visible` reports the cursor's screen row from the rows
/// `scroll_back_from` stepped *backward* (or, in its stable arm, from the
/// distance it already measured). `screen_pos` derives the same row by walking
/// *forward* from the settled viewport top. The draw path may only skip that
/// forward walk if the two always agree — so sweep the shapes where they could
/// diverge: both wrap modes, a viewport shorter than the virtual block, a top
/// above and below the cursor, and every line including the phantom last one.
#[test]
fn reported_screen_row_agrees_with_a_forward_walk() {
    let r = rope("a\nb\nc\nd\ne\nf\ng\nh\ni\nj\n");
    let mut providers = ProviderSet::new();
    providers.add_decoration_source(Box::new(MultiBeforeLine(3, 2)));

    for wrap in [WrapMode::None, WrapMode::Soft { width: 80 }] {
        for height in [1u16, 2, 5, 8] {
            for top in [0usize, 2, 5, 9] {
                for line in hume_rope::content_lines_range(&r) {
                    let cursor_char = r.line_to_char(line);
                    let mut v = viewport(top, height, 80);

                    let mut s = FormatScratch::new();
                    let mut rm = map(&r, wrap, &providers, 80, &mut s);
                    clamp_viewport_top(&mut v, &mut rm);
                    let cursor_pos = rm.locate_row(cursor_char);
                    let reported = ensure_cursor_visible(&mut v, &mut rm, cursor_pos, 2);

                    let mut s = FormatScratch::new();
                    let walked = cursor::screen_pos(
                        &v,
                        &mut map(&r, wrap, &providers, 80, &mut s),
                        cursor_char,
                    );
                    assert_eq!(
                        reported.map(|row| row as u16),
                        walked.map(|(_, row)| row),
                        "{wrap:?}, height {height}, top {top}, line {line}"
                    );
                }
            }
        }
    }
}

/// One frame resolves the cursor's line once, shared between the scroll step
/// (`lifecycle::scroll_into_view`) and the terminal-cursor placement the draw
/// path asks for — both read the same `RowMap` rather than each formatting
/// their own. In `WrapMode::None` `block` never formats, so `locate` is
/// the only thing that can move the counter — making 1 a derived expectation,
/// not a measured one.
#[test]
fn a_frame_formats_the_cursors_line_once_in_no_wrap() {
    let r = rope(&("a".repeat(5_000) + "\n"));
    let formats = std::rc::Rc::new(std::cell::Cell::new(0));
    let mut providers = ProviderSet::new();
    providers.add_decoration_source(Box::new(CountFormatsOf(0, std::rc::Rc::clone(&formats))));
    let mut v = viewport(0, 10, 80);
    let cursor_char = 4_000;

    let mut s = FormatScratch::new();
    let mut rm = map(&r, WrapMode::None, &providers, 80, &mut s);
    clamp_viewport_top(&mut v, &mut rm);
    let (cursor_pos, cursor_col) = rm.locate(cursor_char);
    let row = ensure_cursor_visible(&mut v, &mut rm, cursor_pos, 3).expect("height is 10");
    ensure_cursor_visible_horizontal(&mut v, &mut rm, cursor_col);
    let placed = cursor::place(&v, cursor_col, row);

    assert_eq!(
        formats.get(),
        1,
        "the scroll step must resolve the cursor with a single format"
    );

    // ...and the cell it produced is the one a *second* row map would also
    // compute, so nothing is traded away by sharing it. Runs after the count
    // above: re-deriving is exactly the second format being ruled out, so it
    // has to stay on this side of the assertion.
    let mut s = FormatScratch::new();
    let walked = cursor::screen_pos(
        &v,
        &mut map(&r, WrapMode::None, &providers, 80, &mut s),
        cursor_char,
    );
    assert_eq!(
        Some(placed),
        walked,
        "placement must match a full re-derivation"
    );
}
