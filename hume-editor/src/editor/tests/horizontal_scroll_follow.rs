use super::*;
use pretty_assertions::assert_eq;

// ── Horizontal scroll tracks the cursor independently of the vertical gate ─
//
// `scroll_into_view` (`frame.rs`) runs the vertical `Viewport::reveal`
// correction only when `PaneBufferState::reveal_pending` is set — false for
// a view-led scroll (mouse wheel, `Ctrl-d`) that carries the cursor past a
// virtual-line block without being able to place it, so the block is passed
// without being snapped back. `Viewport::reveal_horizontal` has no such gate:
// it runs unconditionally, every frame, before that branch — so a cursor
// move *within* one display line (`l` along a long unwrapped line), which
// changes the display column without changing the display line at all, still
// scrolls horizontally regardless of what the vertical branch does.

#[test]
fn horizontal_scroll_follows_a_same_display_line_cursor_move() {
    // One long unwrapped line, well past a narrow pane's content width.
    let content = "x".repeat(200) + "\n";
    let mut ed = unwrapped_editor(&content, 0);
    let rect = hume_grid::Rect::new(0, 0, 20, 5); // narrow content width

    ed.render_to_buf(rect);
    assert_eq!(
        ed.viewport().horizontal_offset(),
        hume_rope::column::DisplayLineCol::new(0),
        "sanity: no horizontal scroll needed yet"
    );

    // Move the cursor to column 150 with a single edit rather than 150 key
    // presses — same display line and slot throughout (`WrapMode::None` is
    // always one display line per buffer line), only the column changes.
    set_cursor(&mut ed, 150);
    ed.render_to_buf(rect);

    assert!(
        ed.viewport().horizontal_offset().get() > 0,
        "the view must scroll horizontally to keep column 150 visible, even though \
         the cursor's display line never changed"
    );
}
