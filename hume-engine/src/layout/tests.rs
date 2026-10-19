use super::*;
use crate::providers::GutterCell;
use crate::providers::GutterColumn;
use crate::providers::GutterCtx;
use crate::types::{DisplayLineKind, ScopeId};

/// A gutter column three columns wide, whatever the line count.
struct FixedWidthGutter;

impl GutterColumn for FixedWidthGutter {
    fn width(&self, _: hume_rope::line::RopeyLine, _: crate::providers::GutterInputs) -> u8 {
        3
    }
    fn render_cells(&self, _: DisplayLineKind, _: &GutterCtx) -> Vec<GutterCell> {
        vec![GutterCell::blank(ScopeId(0))]
    }
}

#[test]
fn geometry_without_a_gutter_is_the_whole_viewport() {
    let rope = Rope::from_str("line1\nline2\nline3\n");
    let size = crate::pipeline::PaneSize::new(80, 3);
    let visible = compute_viewport(
        &rope,
        size,
        std::iter::empty(),
        crate::providers::GutterInputs::default(),
    );
    assert_eq!(visible.gutter_width, 0);
    assert_eq!(visible.content_width.get().get(), 80);
}

#[test]
fn a_gutter_takes_its_width_out_of_the_content_area() {
    let rope = Rope::from_str("line1\n");
    let size = crate::pipeline::PaneSize::new(80, 3);
    let lane: Box<dyn GutterColumn> = Box::new(FixedWidthGutter);
    let visible = compute_viewport(
        &rope,
        size,
        std::iter::once(lane.as_ref()),
        crate::providers::GutterInputs::default(),
    );
    assert_eq!(visible.gutter_width, 3);
    assert_eq!(visible.content_width.get().get(), 77);
}

#[test]
fn content_width_never_reaches_zero() {
    // A gutter wider than the pane would otherwise leave nothing to format
    // into, which the formatter's wrap arithmetic cannot represent.
    let rope = Rope::from_str("line1\n");
    let size = crate::pipeline::PaneSize::new(2, 3);
    let lane: Box<dyn GutterColumn> = Box::new(FixedWidthGutter);
    let visible = compute_viewport(
        &rope,
        size,
        std::iter::once(lane.as_ref()),
        crate::providers::GutterInputs::default(),
    );
    assert_eq!(visible.content_width.get().get(), 1);
}

#[test]
fn last_line_idx_includes_the_phantom_trailing_line() {
    // `GutterColumn::width` sizes a line-number column from this, and ropey
    // reports one line past the content for the buffer's structural '\n'.
    let rope = Rope::from_str("a\nb\nc\n");
    let size = crate::pipeline::PaneSize::new(80, 3);
    let visible = compute_viewport(
        &rope,
        size,
        std::iter::empty(),
        crate::providers::GutterInputs::default(),
    );
    assert_eq!(visible.last_line_idx.index(), 3);
}
