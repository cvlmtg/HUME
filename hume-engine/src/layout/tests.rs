use super::*;
use crate::providers::GutterCell;
use crate::providers::GutterColumn;
use crate::providers::GutterRowCtx;
use crate::types::{RowKind, ScopeId};

/// A gutter column three columns wide, whatever the line count.
struct FixedWidthGutter;

impl GutterColumn for FixedWidthGutter {
    fn width(&self, _: usize) -> u8 {
        3
    }
    fn render_row_cells(&self, _: RowKind, _: &GutterRowCtx) -> Vec<GutterCell> {
        vec![GutterCell::blank(ScopeId(0))]
    }
    fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
        self
    }
}

#[test]
fn geometry_without_a_gutter_is_the_whole_viewport() {
    let rope = Rope::from_str("line1\nline2\nline3\n");
    let viewport = ViewportState::new(80, 3);
    let visible = compute_viewport(&rope, &viewport, std::iter::empty());
    assert_eq!(visible.gutter_width, 0);
    assert_eq!(visible.content_width, 80);
    assert_eq!(visible.content_height, 3);
}

#[test]
fn a_gutter_takes_its_width_out_of_the_content_area() {
    let rope = Rope::from_str("line1\n");
    let viewport = ViewportState::new(80, 3);
    let lane: Box<dyn GutterColumn> = Box::new(FixedWidthGutter);
    let visible = compute_viewport(&rope, &viewport, std::iter::once(lane.as_ref()));
    assert_eq!(visible.gutter_width, 3);
    assert_eq!(visible.content_width, 77);
}

#[test]
fn content_width_never_reaches_zero() {
    // A gutter wider than the pane would otherwise leave nothing to format
    // into, which the formatter's wrap arithmetic cannot represent.
    let rope = Rope::from_str("line1\n");
    let viewport = ViewportState::new(2, 3);
    let lane: Box<dyn GutterColumn> = Box::new(FixedWidthGutter);
    let visible = compute_viewport(&rope, &viewport, std::iter::once(lane.as_ref()));
    assert_eq!(visible.content_width, 1);
}

#[test]
fn last_line_idx_includes_the_phantom_trailing_line() {
    // `GutterColumn::width` sizes a line-number column from this, and ropey
    // reports one line past the content for the buffer's structural '\n'.
    let rope = Rope::from_str("a\nb\nc\n");
    let viewport = ViewportState::new(80, 3);
    let visible = compute_viewport(&rope, &viewport, std::iter::empty());
    assert_eq!(visible.last_line_idx, 3);
}
