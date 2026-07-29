use super::*;
use hume_engine::format::FormatScratch;
use hume_engine::pane::{ViewportState, WhitespaceConfig, WrapMode};
use hume_engine::providers::ProviderSet;
use ropey::Rope;

fn no_providers() -> ProviderSet {
    ProviderSet::new()
}
const SCROLL_LINES: usize = 3; // default from EditorSettings

fn map<'a>(
    rope: &'a Rope,
    wrap: WrapMode,
    providers: &'a ProviderSet,
    scratch: &'a mut FormatScratch,
) -> RowMap<'a> {
    RowMap::new(
        rope,
        wrap,
        4,
        WhitespaceConfig::default(),
        providers,
        80,
        scratch,
    )
}

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
fn down_no_wrap_clamps_at_last_real_line() {
    // 10 content lines (indices 0..9), viewport height 5. The clamp is the
    // last real content line, not a "keep the last line at the bottom"
    // max_top — same vim/helix "scrolling past EOF is allowed" convention
    // already documented on `scroll_cursor_to_row` (zz/zb), and required so
    // an `After(last_line)` virtual block anchored there can ever be
    // scrolled into view (a stricter "last line always at the bottom" clamp
    // would make such a block permanently unreachable).
    let rope = rope_with_lines(10);
    let mut vp = ViewportState::new(80, 5);
    vp.top_line = 0;
    let providers = no_providers();
    let mut scratch = FormatScratch::new();

    // Scroll far enough to hit the cap.
    for _ in 0..20 {
        scroll_viewport_down(
            &mut vp,
            &mut map(&rope, WrapMode::None, &providers, &mut scratch),
            SCROLL_LINES,
        );
    }
    assert_eq!(
        vp.top_line, 9,
        "top_line must not exceed the last real line (9)"
    );
    assert_eq!(
        vp.top_row_offset, 0,
        "no virtual rows — clamps to the line's only row"
    );
}

#[test]
fn down_no_wrap_file_fits_no_movement() {
    // 3 content lines, viewport height 10 → everything fits → no movement.
    let rope = rope_with_lines(3);
    let mut vp = ViewportState::new(80, 10);
    let providers = no_providers();
    let mut scratch = FormatScratch::new();

    scroll_viewport_down(
        &mut vp,
        &mut map(&rope, WrapMode::None, &providers, &mut scratch),
        SCROLL_LINES,
    );
    assert_eq!(vp.top_line, 0, "viewport must not move when file fits");
}

#[test]
fn down_no_wrap_advances_by_scroll_lines() {
    let rope = rope_with_lines(20);
    let mut vp = ViewportState::new(80, 5);
    let providers = no_providers();
    let mut scratch = FormatScratch::new();

    scroll_viewport_down(
        &mut vp,
        &mut map(&rope, WrapMode::None, &providers, &mut scratch),
        SCROLL_LINES,
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
    let providers = no_providers();
    let mut scratch = FormatScratch::new();

    scroll_viewport_up(
        &mut vp,
        &mut map(&rope, WrapMode::None, &providers, &mut scratch),
        SCROLL_LINES,
    );
    assert_eq!(vp.top_line, 0, "stepping back must not underflow");
}

#[test]
fn up_no_wrap_decrements_by_scroll_lines() {
    let rope = rope_with_lines(20);
    let mut vp = ViewportState::new(80, 5);
    vp.top_line = 10;
    let providers = no_providers();
    let mut scratch = FormatScratch::new();

    scroll_viewport_up(
        &mut vp,
        &mut map(&rope, WrapMode::None, &providers, &mut scratch),
        SCROLL_LINES,
    );
    assert_eq!(vp.top_line, 10 - SCROLL_LINES);
}

#[test]
fn up_at_top_is_no_op() {
    let rope = rope_with_lines(10);
    let mut vp = ViewportState::new(80, 5);
    vp.top_line = 0;
    let providers = no_providers();
    let mut scratch = FormatScratch::new();

    scroll_viewport_up(
        &mut vp,
        &mut map(&rope, WrapMode::None, &providers, &mut scratch),
        SCROLL_LINES,
    );
    assert_eq!(vp.top_line, 0);
    assert_eq!(vp.top_row_offset, 0);
}

// ── scroll_viewport_down (wrap) ─────────────────────────────────────────

#[test]
fn down_wrap_file_fits_no_movement() {
    // 2 short lines in a wide viewport → all rows fit → no scroll.
    let rope = rope_with_lines(2);
    let mut vp = ViewportState::new(80, 10);
    let wrap = WrapMode::Soft { width: 80 };
    let providers = no_providers();
    let mut scratch = FormatScratch::new();

    scroll_viewport_down(
        &mut vp,
        &mut map(&rope, wrap, &providers, &mut scratch),
        SCROLL_LINES,
    );
    assert_eq!(vp.top_line, 0, "no scroll when file fits in viewport");
    assert_eq!(vp.top_row_offset, 0);
}

// ── EOF virtual block reachability (both wrap modes) ────────────────────

/// Emits `self.1` distinct `After(self.0)` rows, texted "1".."9".
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

/// A 3-row `After(last_line)` block must be reachable one row at a time by
/// repeated single-row wheel notches, in either wrap mode: `top_row_offset`
/// walks 0 → 1 → 2 → 3 (the block's last row) as `top_line` settles on the
/// last real line, then further scrolling stays clamped at 3 — it must not
/// reset back to 0 once the block's final row is reached (the EOF-overshoot
/// bug this fix corrects).
#[test]
fn down_reaches_every_row_of_an_after_last_line_block() {
    let rope = rope_with_lines(2); // last real line = index 1
    let mut providers = ProviderSet::new();
    providers.add_virtual_line_source(Box::new(MultiAfterLine(1, 3)));
    let mut scratch = FormatScratch::new();

    for wrap in [WrapMode::None, WrapMode::Soft { width: 80 }] {
        let mut vp = ViewportState::new(80, 2); // shorter than the 5-row total content
        let expected = [(0, 0), (1, 0), (1, 1), (1, 2), (1, 3)];
        for &(exp_line, exp_offset) in &expected {
            assert_eq!(
                (vp.top_line, vp.top_row_offset),
                (exp_line, exp_offset),
                "{wrap:?}"
            );
            scroll_viewport_down(&mut vp, &mut map(&rope, wrap, &providers, &mut scratch), 1);
        }
        // One more notch past the last row must stay clamped, not reset.
        scroll_viewport_down(&mut vp, &mut map(&rope, wrap, &providers, &mut scratch), 1);
        assert_eq!(
            (vp.top_line, vp.top_row_offset),
            (1, 3),
            "further scrolling past the block's last row must stay clamped there ({wrap:?})"
        );
    }
}

/// An overshooting notch (larger than what remains of the last line's
/// block) must clamp to the block's final row in one jump, not reset to 0
/// — direct regression for the EOF-overshoot bug: the old code did
/// `top_row_offset = 0; top_line += 1` unconditionally on overshoot, which
/// at `top_line == last_line` (no next line to advance into) snapped back
/// to the top of the block instead of clamping.
#[test]
fn down_overshoot_past_after_last_line_clamps_not_resets() {
    let rope = rope_with_lines(2);
    let mut providers = ProviderSet::new();
    providers.add_virtual_line_source(Box::new(MultiAfterLine(1, 3)));
    let mut scratch = FormatScratch::new();

    for wrap in [WrapMode::None, WrapMode::Soft { width: 80 }] {
        let mut vp = ViewportState::new(80, 2);
        vp.top_line = 1;
        vp.top_row_offset = 1; // already partway into the After block
        // A large notch overshoots well past the block's remaining rows.
        scroll_viewport_down(&mut vp, &mut map(&rope, wrap, &providers, &mut scratch), 10);
        assert_eq!(vp.top_line, 1, "{wrap:?}");
        assert_eq!(
            vp.top_row_offset, 3,
            "must clamp to the block's last row (3), not reset to 0 ({wrap:?})"
        );
    }
}
