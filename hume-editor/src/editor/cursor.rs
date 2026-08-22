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
use hume_engine::rows::{DisplayColTarget, RowMap};

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
    Some(place(viewport, cursor_col, screen_row))
}

/// Turn a resolved document column and screen row into a pane-relative cell.
///
/// Split out from [`screen_pos`] so the scroll step — which necessarily resolves
/// both while deciding where to scroll — can produce the same answer without
/// re-walking the row list. The two must not drift: this is the only place the
/// horizontal-offset subtraction and the `u16` narrowing happen.
pub(crate) fn place(viewport: &ViewportState, cursor_col: u32, screen_row: usize) -> (u16, u16) {
    let col = cursor_col.saturating_sub(viewport.horizontal_offset);
    // `ensure_cursor_visible_horizontal` keeps the cursor's document column
    // within one viewport width of `horizontal_offset`, so once past that
    // subtraction it's a small on-screen offset — safe to narrow to the
    // terminal-cell (`u16`) domain this function returns.
    debug_assert!(
        u16::try_from(col).is_ok(),
        "on-screen cursor column {col} exceeds a u16 — cursor should be within the viewport"
    );
    (col as u16, screen_row as u16)
}

/// Gutter width in terminal columns for the current frame.
///
/// Used to offset the terminal cursor column past line numbers and other
/// gutter providers. `last_line_idx` is the buffer's last ropey line index
/// (`hume_rope::last_ropey_line`) — deliberately the phantom trailing line,
/// not the last content line, so the gutter is sized one digit wider than
/// content strictly requires.
pub(crate) fn gutter_width<'a>(
    gutter_columns: impl Iterator<Item = &'a dyn GutterColumn>,
    last_line_idx: usize,
) -> u16 {
    gutter_width_for_line(gutter_columns, last_line_idx)
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
/// The coordinate space is pane-relative: `(0, 0)` is the top-left cell of
/// the pane. `MouseEvent.column`/`.row` are terminal-absolute — callers
/// translate through `Editor::pane_at_screen_pos` (`editor/src/editor/mouse.rs`)
/// first, which also decides which pane a click landed in when more than one
/// is on screen (a `:split`/`:vsplit`).
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
    let content_col = ((screen_x - gutter_w) as u32).saturating_add(viewport.horizontal_offset);

    let top = rm.clamp(top_pos(viewport));
    let clicked = rm.advance(top, screen_y as isize);
    // A click asks which cell it hit, so a column past the text resolves to
    // the row's last cell rather than its last *content* cell — landing on the
    // line's `\n`, a real cursor position in HUME's inclusive model.
    Some(rm.char_at(clicked, content_col, DisplayColTarget::Cell))
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests;
