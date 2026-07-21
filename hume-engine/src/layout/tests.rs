use super::*;
use crate::providers::GutterCell;
use crate::providers::GutterColumn;
use crate::providers::GutterRowCtx;
use crate::types::{RowKind, Scope};

struct _NoGutter;
impl GutterColumn for _NoGutter {
    fn width(&self, _: usize) -> u8 {
        0
    }
    fn render_row_cells(&self, _: RowKind, _: &GutterRowCtx) -> Vec<GutterCell> {
        vec![GutterCell::blank(Scope("ui.linenr"))]
    }
    fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
        self
    }
}

#[test]
fn no_wrap_basic_range() {
    let rope = Rope::from_str("line1\nline2\nline3\nline4\nline5\n");
    let viewport = ViewportState::new(80, 3);
    let visible = compute_viewport(&rope, &viewport, &WrapMode::None, std::iter::empty(), 4);
    assert_eq!(visible.line_range.start, 0);
    assert!(visible.line_range.end <= 5);
    assert_eq!(visible.gutter_width, 0);
    assert_eq!(visible.content_width, 80);
}

#[test]
fn no_wrap_clamped_to_total_lines() {
    // Trailing '\n' per the buffer invariant (see compute_viewport's doc
    // comment, B12) — without it the rope's one real content line would
    // be excluded from line_range as if it were the phantom trailing line.
    let rope = Rope::from_str("only one line\n");
    let viewport = ViewportState::new(80, 50);
    let visible = compute_viewport(&rope, &viewport, &WrapMode::None, std::iter::empty(), 4);
    // 2 ropey lines (content + phantom trailing); line_range excludes
    // the phantom, so it must stop at 1 despite the 50-row viewport.
    assert_eq!(visible.line_range, 0..1);
}

#[test]
fn soft_wrap_includes_lookahead() {
    let rope = Rope::from_str("a\nb\nc\nd\ne\nf\ng\n");
    let viewport = ViewportState::new(80, 3);
    let visible = compute_viewport(
        &rope,
        &viewport,
        &WrapMode::Soft { width: 80 },
        std::iter::empty(),
        4,
    );
    // Should have at least 3 + lookahead lines
    assert!(visible.line_range.len() >= 3);
}

// ── estimate_line_rows width-awareness (B6) ─────────────────────────

#[test]
fn estimate_line_rows_cjk_width_aware() {
    // 40 '中' chars, each display width 2 → true width 80. At
    // content_width 40 that's exactly 2 rows. Char-count-only (the old
    // proxy) would see 40 chars / 40 cols = 1 row — half the true count.
    let text: String = "中".repeat(40);
    let rope = Rope::from_str(&text);
    assert_eq!(estimate_line_rows(&rope, 0, 40, 4), 2);
}

#[test]
fn estimate_line_rows_tabs_width_aware() {
    // 10 tabs, tab_width 4 → each tab contributes a full 4-column
    // expansion (its 1-column `unicode_width` placeholder is swapped for
    // the real tab-stop width) → true width 40. At content_width 20
    // that's exactly 2 rows. Char-count-only would see 10 chars / 20
    // cols = 1 row.
    let text = "\t".repeat(10);
    let rope = Rope::from_str(&text);
    assert_eq!(estimate_line_rows(&rope, 0, 20, 4), 2);
}

#[test]
fn estimate_line_rows_empty_line_is_one() {
    let rope = Rope::from_str("");
    assert_eq!(estimate_line_rows(&rope, 0, 40, 4), 1);
}

#[test]
fn estimate_line_rows_zero_content_width_is_one() {
    let rope = Rope::from_str("abc");
    assert_eq!(estimate_line_rows(&rope, 0, 0, 4), 1);
}
