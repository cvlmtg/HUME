//! Terminal cursor placement logic.
//!
//! The terminal cursor is an editor-level concern: the engine only styles the
//! grapheme at each selection head. Both directions of the screen ↔ buffer
//! mapping go through `DisplayLineMap`, so neither can disagree with the
//! renderer about which display line a position is on.

use hume_engine::display_lines::{DisplayColTarget, DisplayLineMap};
use hume_engine::pane::Viewport;
use hume_engine::scope_headers::header_display_line;
use hume_rope::cluster::ClusterStart;
use hume_rope::column::DisplayLineCol;
use hume_rope::line::ContentLine;

/// The `(x, row)` of `cursor` within the pane content area: after the
/// gutter, not a terminal-absolute cell. `x` accounts for
/// `viewport.horizontal_offset`.
///
/// `None` when the position is outside the viewport. That is a steady state:
/// the cursor occupies only content display lines, so a pure view scroll
/// into a virtual-line block (an inline diff's ghost lines) can carry the
/// viewport past where the cursor can follow. The caret stays hidden until a
/// cursor motion resyncs the view.
pub(in crate::editor) fn content_pos(
    size: hume_engine::pipeline::PaneSize,
    viewport: &mut Viewport,
    dlm: &mut DisplayLineMap<'_>,
    cursor: hume_rope::cluster::ClusterStart,
) -> Option<(u16, u16)> {
    let height = size.height();
    if height == 0 {
        return None;
    }
    let (cursor_pos, cursor_display_col) = dlm.locate(cursor);
    if cursor_display_col < viewport.horizontal_offset() {
        // Off the viewport horizontally. The live cursor never is, but the
        // completion menu's anchor stays at the token's start while the
        // cursor and the scroll it drives move on.
        return None;
    }
    // Resolve the top as the render pass does, or the walk would disagree
    // with the renderer about which display line is on screen.
    let top = viewport.top_at(dlm);
    // Capping the walk one row short of the viewport's height makes an
    // off-screen cursor a `None` rather than a row past the last one; a cursor
    // scrolled off the *top* is likewise unreachable walking forward.
    let screen_row = dlm.distance(top, cursor_pos, height as usize - 1)?;
    Some(place(viewport, cursor_display_col, screen_row))
}

/// Turn a resolved document display column and screen row into a
/// pane-content-relative cell. The scroll step, which has already resolved
/// both, calls this directly; it is the one place the horizontal-offset
/// subtraction and the `u16` narrowing happen.
pub(in crate::editor) fn place(
    viewport: &Viewport,
    cursor_display_col: DisplayLineCol,
    screen_row: usize,
) -> (u16, u16) {
    // Saturating, so a column left of the offset cannot panic here even when
    // a caller has not screened it out as `content_pos` does.
    let content_x = cursor_display_col.cells_since_saturating(viewport.horizontal_offset());
    // `Viewport::reveal_horizontal` keeps the cursor within one viewport
    // width of `horizontal_offset`, so the offset fits a `u16`.
    debug_assert!(
        u16::try_from(content_x).is_ok(),
        "on-screen cursor column {content_x} exceeds a u16: cursor should be within the viewport"
    );
    (content_x as u16, screen_row as u16)
}

// Screen-to-buffer reverse mapping

/// The buffer cluster under a click at pane-relative `(content_x,
/// content_y)`, where `(0, 0)` is the pane's top-left cell. Callers translate
/// a terminal-absolute `MouseEvent` through `EngineView::pane_hit` first.
///
/// `gutter_w` is the gutter width in cells. A click in the gutter returns
/// `None`; any other click resolves, clamped to the last display line.
pub(in crate::editor) fn screen_to_cluster(
    content_x: u16,
    content_y: u16,
    gutter_w: u16,
    viewport: &mut Viewport,
    dlm: &mut DisplayLineMap<'_>,
) -> Option<hume_rope::cluster::ClusterStart> {
    if content_x < gutter_w {
        return None;
    }
    let display_col = viewport
        .horizontal_offset()
        .advance_saturating((content_x - gutter_w) as u32);

    let top = viewport.top_at(dlm);
    let clicked = dlm.advance_saturating(top, content_y as isize);
    // A column past the text resolves to the display line's last cell, its
    // `\n`, which is a cursor position in the inclusive selection model.
    Some(dlm.char_at(clicked, display_col, DisplayColTarget::Cell))
}

/// The buffer cluster under a click on the scope header row that draws
/// `line`, at pane-relative `content_x`. `dlm` is the map the header rows are
/// laid out with ([`header_format_key`](hume_engine::scope_headers::header_format_key)),
/// and columns count from the line's column 0 whatever the pane's horizontal
/// scroll. A click in the `gutter_w`-wide gutter returns `None`.
pub(in crate::editor) fn header_to_cluster(
    content_x: u16,
    gutter_w: u16,
    dlm: &mut DisplayLineMap<'_>,
    line: ContentLine,
) -> Option<ClusterStart> {
    if content_x < gutter_w {
        return None;
    }
    let col = DisplayLineCol::new(0).advance_saturating(u32::from(content_x - gutter_w));
    let row = header_display_line(dlm, line);
    Some(dlm.char_at(row, col, DisplayColTarget::Cell))
}

#[cfg(test)]
mod tests;
