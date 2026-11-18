// A pane's visible line range under soft wrap: the lines with a row on
// screen, as `(viewport-range)` reads it between frames and as the frame
// records it for `on-viewport-change`.

use super::*;
use hume_core::Side;
use hume_engine::pane::WrapMode;
use hume_engine::test_doubles::VirtualLineBlock;
use hume_rope::line::ContentLine;

const WIDTH: u16 = 40;

/// 11 rows leave the pane 10 under the statusline.
const HEIGHT: u16 = 11;

/// `lines` lines of `len` chars, wrapping. A 40-column terminal leaves
/// between 27 and 40 text columns whatever the gutter takes, so a line of 50
/// chars takes 2 rows and one of 80 takes 3 at any of them.
fn wrapped(len: usize, lines: usize) -> Editor {
    let text = format!("{}\n", "a".repeat(len)).repeat(lines);
    let mut ed = unwrapped_editor(&text, 0);
    pin_wrap(&mut ed, WrapMode::Soft { width: 0 });
    ed
}

/// The focused pane's range read between frames, then the range the frame
/// recorded.
fn ranges(ed: &mut Editor) -> [std::ops::Range<usize>; 2] {
    frame(ed, WIDTH, HEIGHT);
    let pid = ed.view.focused_pane();
    let settled = crate::editor::geometry::shown_lines(&mut ed.state, &mut ed.view, pid).visible;
    let framed = ed.last_shown_lines[&pid].1.visible;
    [settled, framed].map(|range| range.start.index()..range.end.index())
}

#[test]
fn lines_of_two_rows_fill_ten_rows_with_five_lines() {
    let mut ed = wrapped(50, 100);
    assert_eq!(ranges(&mut ed), [0..5, 0..5]);
}

#[test]
fn a_line_cut_off_at_the_bottom_is_shown() {
    let mut ed = wrapped(80, 100);
    assert_eq!(ranges(&mut ed), [0..4, 0..4]);
}

/// Lines 0 to 3 take rows 0 to 7, and line 4's two virtual lines rows 8 and
/// 9, so none of line 4's text shows.
#[test]
fn a_line_shown_only_by_its_virtual_lines_is_shown() {
    let mut ed = wrapped(50, 100);
    let pid = ed.view.focused_pane();
    ed.view.panes[pid]
        .providers
        .add_decoration_source(Box::new(VirtualLineBlock::uniform(
            Side::Before,
            ContentLine::new(4),
            2,
            "V",
        )));
    assert_eq!(ranges(&mut ed), [0..5, 0..5]);
}
