// `reload_buffer_in_place` (`:e!`) interacting with a scrolled-down viewport.

use hume_grid::Rect;
use hume_rope::line::ContentLine;

use super::*;

/// A reload (`:e!`) that shrinks the document past a scrolled-down
/// viewport's top must not leave that top permanently stale: once the next
/// frame's scroll/render pass resolves it (`Viewport::top_at`), it must land
/// inside the new, shorter document — `reload_buffer_in_place` itself no
/// longer touches the viewport at all, having no map to resolve it against.
///
/// Characterization test: passes unchanged whether or not `reload_buffer_in_place`
/// resolves the top itself, so long as *some* pass resolves it before this
/// assertion runs — `render_to_buf` is that pass either way.
#[test]
fn reload_shrinking_the_document_leaves_the_next_frames_top_inside_it() {
    let mut ed = editor_from("-[a]>\nb\nc\nd\ne\nf\ng\nh\ni\nj\n"); // 10 lines
    let bid = ed.focused_buffer_id();
    let pid = ed.state.focus.id();

    // Scroll down near the 10-line original's end — line 8 is valid there,
    // but past the end of the 2-line replacement below.
    ed.view.panes[pid]
        .viewport
        .seed_top_for_test(hume_engine::display_lines::DisplayLinePos::new(
            ContentLine::new(8),
            0,
        ));

    let replacement = Buffer::new(BufferText::from("x\ny\n"), SelectionSet::default());
    ed.reload_buffer_in_place(bid, replacement);

    // Whatever pass resolves the top next — the render/scroll pipeline here —
    // it must land inside the shrunken 2-line document, not past its end.
    ed.render_to_buf(Rect::new(0, 0, 40, 8));
    assert!(
        ed.view.panes[pid].viewport.top().line <= ContentLine::new(1),
        "top must land inside the shrunken 2-line document, not past its end (got {:?})",
        ed.view.panes[pid].viewport.top().line
    );
}
