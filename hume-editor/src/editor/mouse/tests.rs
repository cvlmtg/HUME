use super::*;
use hume_engine::format::FormatScratch;
use hume_engine::pane::{ViewportState, WhitespaceConfig, WrapMode};
use ropey::Rope;

fn ws() -> WhitespaceConfig {
    WhitespaceConfig::default()
}
const SCROLL_LINES: usize = 3; // default from EditorSettings

// Build a rope with `n` content lines (each "line\n"), plus the structural trailing '\n'.
// total_lines() == n + 1 (ropey's phantom line).
fn rope_with_lines(n: usize) -> Rope {
    let mut s = String::new();
    for i in 0..n {
        s.push_str(&format!("line{}\n", i));
    }
    Rope::from_str(&s)
}

// ── scroll_viewport_down (no-wrap) ──────────────────────────────────────

#[test]
fn down_no_wrap_clamps_at_max_top() {
    // 10 content lines, viewport height 5 → max_top = 10 - 5 = 5.
    let rope = rope_with_lines(10);
    let total = rope.len_lines(); // 11 (10 content + phantom)
    let mut vp = ViewportState::new(80, 5);
    vp.top_line = 0;
    let mut scratch = FormatScratch::new();

    // Scroll far enough to hit the cap.
    for _ in 0..20 {
        scroll_viewport_down(
            &mut vp,
            &rope,
            &WrapMode::None,
            4,
            &ws(),
            total,
            SCROLL_LINES,
            &mut scratch,
        );
    }
    assert_eq!(vp.top_line, 5, "top_line must not exceed max_top=5");
}

#[test]
fn down_no_wrap_file_fits_no_movement() {
    // 3 content lines, viewport height 10 → max_top = 0 → no movement.
    let rope = rope_with_lines(3);
    let total = rope.len_lines();
    let mut vp = ViewportState::new(80, 10);
    let mut scratch = FormatScratch::new();

    scroll_viewport_down(
        &mut vp,
        &rope,
        &WrapMode::None,
        4,
        &ws(),
        total,
        SCROLL_LINES,
        &mut scratch,
    );
    assert_eq!(vp.top_line, 0, "viewport must not move when file fits");
}

#[test]
fn down_no_wrap_advances_by_scroll_lines() {
    let rope = rope_with_lines(20);
    let total = rope.len_lines();
    let mut vp = ViewportState::new(80, 5);
    let mut scratch = FormatScratch::new();

    scroll_viewport_down(
        &mut vp,
        &rope,
        &WrapMode::None,
        4,
        &ws(),
        total,
        SCROLL_LINES,
        &mut scratch,
    );
    assert_eq!(
        vp.top_line, SCROLL_LINES,
        "first scroll advances by SCROLL_LINES"
    );
}

// ── scroll_viewport_up (no-wrap) ────────────────────────────────────────

#[test]
fn up_no_wrap_clamps_at_zero() {
    let rope = rope_with_lines(10);
    let mut vp = ViewportState::new(80, 5);
    vp.top_line = 1; // only 1 above top
    let mut scratch = FormatScratch::new();

    scroll_viewport_up(
        &mut vp,
        &rope,
        &WrapMode::None,
        4,
        &ws(),
        SCROLL_LINES,
        &mut scratch,
    );
    assert_eq!(vp.top_line, 0, "saturating_sub must not underflow");
}

#[test]
fn up_no_wrap_decrements_by_scroll_lines() {
    let rope = rope_with_lines(20);
    let mut vp = ViewportState::new(80, 5);
    vp.top_line = 10;
    let mut scratch = FormatScratch::new();

    scroll_viewport_up(
        &mut vp,
        &rope,
        &WrapMode::None,
        4,
        &ws(),
        SCROLL_LINES,
        &mut scratch,
    );
    assert_eq!(vp.top_line, 10 - SCROLL_LINES);
}

#[test]
fn up_at_top_is_no_op() {
    let rope = rope_with_lines(10);
    let mut vp = ViewportState::new(80, 5);
    vp.top_line = 0;
    let mut scratch = FormatScratch::new();

    scroll_viewport_up(
        &mut vp,
        &rope,
        &WrapMode::None,
        4,
        &ws(),
        SCROLL_LINES,
        &mut scratch,
    );
    assert_eq!(vp.top_line, 0);
    assert_eq!(vp.top_row_offset, 0);
}

// ── scroll_viewport_down (wrap) ─────────────────────────────────────────

#[test]
fn down_wrap_file_fits_no_movement() {
    // 2 short lines in a wide viewport → all rows fit → no scroll.
    let rope = rope_with_lines(2);
    let total = rope.len_lines();
    let mut vp = ViewportState::new(80, 10);
    let wrap = WrapMode::Soft { width: 80 };
    let mut scratch = FormatScratch::new();

    scroll_viewport_down(
        &mut vp,
        &rope,
        &wrap,
        4,
        &ws(),
        total,
        SCROLL_LINES,
        &mut scratch,
    );
    assert_eq!(vp.top_line, 0, "no scroll when file fits in viewport");
    assert_eq!(vp.top_row_offset, 0);
}
