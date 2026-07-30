//! Terminal cursor placement logic.
//!
//! The terminal cursor (the blinking bar or block emitted via escape sequences)
//! is an editor-level concern. The engine knows nothing about it — it only
//! styles the grapheme at each selection head.
//!
//! Both directions of the screen ↔ buffer mapping are thin consumers of
//! `hume_engine::rows::RowMap`, so neither can disagree with the renderer
//! about which display row a position is on.

use hume_engine::layout::gutter_width_for_line;
use hume_engine::pane::ViewportState;
use hume_engine::providers::GutterColumn;
use hume_engine::rows::{ColTarget, RowMap};

use super::scroll::top_pos;

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Compute the on-screen `(col, row)` of `cursor_char` within the pane content
/// area (i.e., after the gutter).
///
/// Returns `None` if the position is outside the visible viewport (defensive;
/// should not happen after `scroll::ensure_cursor_visible`).
///
/// `col` accounts for `viewport.horizontal_offset` (0 while wrapping, since
/// wrap mode has no horizontal scroll — see
/// `scroll::ensure_cursor_visible_horizontal`).
pub(crate) fn screen_pos(
    viewport: &ViewportState,
    rm: &mut RowMap<'_>,
    cursor_char: usize,
) -> Option<(u16, u16)> {
    let height = viewport.height;
    if height == 0 {
        return None;
    }
    let (cursor_pos, cursor_col) = rm.locate(cursor_char);
    // Clamp the top the same way `pane_render.rs` does before its row walk:
    // a write site that doesn't validate `top_row_offset` against the block
    // it addresses (`recall_scroll`, an LSP jump — see `clamp_viewport_top`'s
    // doc) can leave it stale for a frame, and walking from the raw address
    // would disagree with the renderer about which row is on screen.
    let top = rm.clamp(top_pos(viewport));
    // Capping the walk one row short of the viewport's height makes an
    // off-screen cursor a `None` rather than a row past the last one; a cursor
    // scrolled off the *top* is likewise unreachable walking forward.
    let screen_row = rm.distance(top, cursor_pos, height as usize - 1)?;
    let col = cursor_col.saturating_sub(viewport.horizontal_offset);
    Some((col, screen_row as u16))
}

/// Gutter width in terminal columns for the current frame.
///
/// Used to offset the terminal cursor column past line numbers and other gutter
/// providers.
pub(crate) fn gutter_width<'a>(
    gutter_columns: impl Iterator<Item = &'a dyn GutterColumn>,
    total_lines: usize,
) -> u16 {
    gutter_width_for_line(gutter_columns, total_lines.saturating_sub(1))
}

// ---------------------------------------------------------------------------
// Screen-to-buffer reverse mapping
// ---------------------------------------------------------------------------

/// Convert a pane-relative `(screen_x, screen_y)` click position to a buffer
/// char offset.
///
/// `gutter_w` is the width of the gutter in terminal columns (from
/// [`gutter_width`]). Clicks in the gutter return `None`; every other click
/// resolves, clamped to the document's last row if it lands past the end.
///
/// The coordinate space is pane-relative: `(0, 0)` is the top-left cell of the
/// pane, matching what `MouseEvent.column`/`.row` report when the pane fills
/// the whole terminal (which is currently always true).
pub(crate) fn screen_to_char_offset(
    screen_x: u16,
    screen_y: u16,
    gutter_w: u16,
    viewport: &ViewportState,
    rm: &mut RowMap<'_>,
) -> Option<usize> {
    // Clicks inside the gutter (line numbers etc.) do not map to text.
    if screen_x < gutter_w {
        return None;
    }
    // Screen column past the gutter, plus horizontal scroll (0 while wrapping
    // — see `scroll::ensure_cursor_visible_horizontal`).
    let content_col = (screen_x - gutter_w).saturating_add(viewport.horizontal_offset);

    let top = rm.clamp(top_pos(viewport));
    let clicked = rm.advance(top, screen_y as isize);
    // A click asks which cell it hit, so a column past the text resolves to
    // the row's last cell rather than its last *content* cell — landing on the
    // line's `\n`, a real cursor position in HUME's inclusive model.
    Some(rm.char_at(clicked, content_col, ColTarget::Cell))
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests;
