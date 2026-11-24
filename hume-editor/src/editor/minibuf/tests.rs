use hume_grid::Rect;

use super::{MiniBuffer, flatten_single_line};
use crate::editor::tests::key_left;

/// The input area of a 10-cell statusline row: its one-cell pad, then nine
/// cells.
const AREA: Rect = Rect {
    x: 1,
    y: 0,
    width: 9,
    height: 1,
};

/// A `:` line holding `abcdefghijklmnop`, shown once with its cursor at the
/// end, then with the cursor moved `lefts` graphemes back, one frame per key.
fn scrolled_line(lefts: usize) -> MiniBuffer {
    let mut mb = MiniBuffer::with_prefill(":", "abcdefghijklmnop".to_owned());
    mb.statusline_view(AREA);
    for _ in 0..lefts {
        let _ = mb.line.edit(key_left());
        mb.statusline_view(AREA);
    }
    mb
}

/// The cursor on the last cell, with `jklmnop` before it.
#[test]
fn a_line_wider_than_the_row_shows_its_tail_before_a_cursor_at_its_end() {
    let view = scrolled_line(0).statusline_view(AREA);
    assert_eq!((view.text(), view.cursor_x()), (":jklmnop", 9));
}

/// The cursor on `n`, after `:jklm`.
#[test]
fn the_cursor_moves_left_across_a_scrolled_line_and_the_text_stays() {
    let view = scrolled_line(3).statusline_view(AREA);
    assert_eq!((view.text(), view.cursor_x()), (":jklmnop", 6));
}

/// The cursor on `i`, the window's first cell after `:`.
#[test]
fn the_cursor_moved_before_the_window_scrolls_it_one_cluster() {
    let view = scrolled_line(8).statusline_view(AREA);
    assert_eq!((view.text(), view.cursor_x()), (":ijklmnop", 2));
}

/// Byte 0 (`a`) scrolled off the left lands on the first input cell, and an
/// offset after the cursor on `n` lands on the cursor's cell.
#[test]
fn an_offset_off_the_window_lands_on_its_first_cell_or_on_the_cursor() {
    let view = scrolled_line(3).statusline_view(AREA);
    assert_eq!((view.x_at(0), view.x_at(16)), (2, 6));
}

/// The input area's own column counts: the text starts at `area.x`.
#[test]
fn the_view_starts_at_the_input_area_column() {
    let mut mb = MiniBuffer::with_prefill(":", "ab".to_owned());
    let view = mb.statusline_view(Rect { x: 5, ..AREA });
    assert_eq!((view.x_at(0), view.cursor_x()), (6, 8));
}

#[test]
fn flatten_drops_trailing_newline() {
    assert_eq!(flatten_single_line("foo\n"), "foo");
}

#[test]
fn flatten_turns_interior_newline_into_space() {
    assert_eq!(flatten_single_line("foo\nbar"), "foo bar");
}

#[test]
fn flatten_drops_multiple_trailing_newlines() {
    assert_eq!(flatten_single_line("foo\n\n"), "foo");
}
