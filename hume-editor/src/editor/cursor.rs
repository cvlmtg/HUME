//! Terminal cursor placement logic.
//!
//! The terminal cursor (the blinking bar or block emitted via escape sequences)
//! is an editor-level concern. The engine knows nothing about it — it only
//! styles the grapheme at each selection head.
//!
//! Both directions of the screen ↔ buffer mapping are thin consumers of
//! `hume_engine::display_lines::DisplayLineMap`, so neither can disagree with the renderer
//! about which display line a position is on.

use hume_engine::display_lines::{DisplayColTarget, DisplayLineMap};
use hume_engine::layout::gutter_width_for_line;
use hume_engine::pane::ViewportState;
use hume_engine::providers::GutterColumn;
use hume_rope::column::DisplayLineCol;

use super::scroll::top_pos;

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Compute the pane-content-relative `(x, row)` of `cursor_char` within the
/// pane content area (i.e., after the gutter — not a terminal-absolute
/// screen cell; callers add the gutter width and pane origin for that).
///
/// Returns `None` if the position is outside the visible viewport (defensive;
/// should not happen after `scroll::ensure_cursor_visible`).
///
/// The returned `x` accounts for `viewport.horizontal_offset` (0 while
/// wrapping, since wrap mode has no horizontal scroll — see
/// `scroll::ensure_cursor_visible_horizontal`).
pub(in crate::editor) fn content_pos(
    viewport: &ViewportState,
    dlm: &mut DisplayLineMap<'_>,
    cursor_char: hume_rope::offset::CharOffset,
) -> Option<(u16, u16)> {
    let height = viewport.height;
    if height == 0 {
        return None;
    }
    let (cursor_pos, cursor_display_col) = dlm.locate(cursor_char);
    if cursor_display_col < viewport.horizontal_offset {
        // Off the visible viewport on the horizontal axis — same contract as
        // a row below the bottom (checked below via `distance`). Every
        // caller but one is the live cursor, which `ensure_cursor_visible_horizontal`
        // keeps `>= horizontal_offset`; the exception is the completion-menu
        // anchor (`overlay_sync.rs`'s `session.anchor()`), fixed at the
        // token's start while the cursor — and the scroll it drives — moves
        // on. `place`'s own subtraction saturates rather than relying on this
        // check alone, since `scroll.rs` calls it directly without going
        // through `content_pos` first.
        return None;
    }
    // Clamp the top the same way `pane_render.rs` does before its
    // display-line walk: a write site that doesn't validate `top_slot`
    // against the block it addresses (`recall_scroll`, an LSP jump — see
    // `clamp_viewport_top`'s doc) can leave it stale for a frame, and
    // walking from the raw address would disagree with the renderer about
    // which display line is on screen.
    let top = dlm.clamp(top_pos(viewport));
    // Capping the walk one row short of the viewport's height makes an
    // off-screen cursor a `None` rather than a row past the last one; a cursor
    // scrolled off the *top* is likewise unreachable walking forward.
    let screen_row = dlm.distance(top, cursor_pos, height as usize - 1)?;
    Some(place(viewport, cursor_display_col, screen_row))
}

/// Turn a resolved document display column and screen row into a
/// pane-content-relative cell.
///
/// Split out from [`content_pos`] so the scroll step — which necessarily resolves
/// both while deciding where to scroll — can produce the same answer without
/// re-walking the display-line list. The two must not drift: this is the only place the
/// horizontal-offset subtraction and the `u16` narrowing happen.
pub(in crate::editor) fn place(
    viewport: &ViewportState,
    cursor_display_col: DisplayLineCol,
    screen_row: usize,
) -> (u16, u16) {
    // Saturating, not `cells_since`: `frame.rs::scroll_into_view` — the only
    // caller that bypasses `content_pos` — always satisfies `cursor_display_col
    // >= horizontal_offset` (it calls `ensure_cursor_visible_horizontal` just
    // above), but `content_pos`'s own caller can pass a completion session's
    // fixed anchor column, which falls behind as the cursor — and the
    // horizontal scroll it drives — moves on. `content_pos` already screens
    // that case to `None`; this saturates too so a direct caller can't panic
    // either, without adding a second precondition this function would have
    // to document and enforce itself.
    let content_x = cursor_display_col.cells_since_saturating(viewport.horizontal_offset);
    // `ensure_cursor_visible_horizontal` keeps the cursor's document column
    // within one viewport width of `horizontal_offset`, so once past that
    // subtraction it's a small on-screen offset — safe to narrow to the
    // terminal-cell (`u16`) domain this function returns.
    debug_assert!(
        u16::try_from(content_x).is_ok(),
        "on-screen cursor column {content_x} exceeds a u16 — cursor should be within the viewport"
    );
    (content_x as u16, screen_row as u16)
}

/// Gutter width in terminal columns for the current frame.
///
/// Used to offset the terminal cursor column past line numbers and other
/// gutter providers. `last_line_idx` is the buffer's last ropey line index
/// (`hume_rope::lines::last_ropey_line`) — deliberately the phantom trailing line,
/// not the last content line, so the gutter is sized one digit wider than
/// content strictly requires.
pub(in crate::editor) fn gutter_width<'a>(
    gutter_columns: impl Iterator<Item = &'a dyn GutterColumn> + 'a,
    last_line_idx: hume_rope::line::RopeyLine,
) -> u16 {
    gutter_width_for_line(gutter_columns, last_line_idx)
}

// ---------------------------------------------------------------------------
// Screen-to-buffer reverse mapping
// ---------------------------------------------------------------------------

/// Convert a pane-content-relative `(content_x, content_y)` click position to
/// a buffer char offset.
///
/// `gutter_w` is the width of the gutter in terminal columns (from
/// [`gutter_width`]). Clicks in the gutter return `None`; every other click
/// resolves, clamped to the document's last display line if it lands past the end.
///
/// The coordinate space is pane-relative, not terminal-absolute: `(0, 0)` is
/// the top-left cell of the pane. `MouseEvent.column`/`.row` are
/// terminal-absolute — callers translate through `Editor::pane_at_screen_pos`
/// (`hume-editor/src/editor/mouse.rs`) first, which also decides which pane a
/// click landed in when more than one is on screen (a `:split`/`:vsplit`).
pub(in crate::editor) fn screen_to_char_offset(
    content_x: u16,
    content_y: u16,
    gutter_w: u16,
    viewport: &ViewportState,
    dlm: &mut DisplayLineMap<'_>,
) -> Option<hume_rope::offset::CharOffset> {
    // Clicks inside the gutter (line numbers etc.) do not map to text.
    if content_x < gutter_w {
        return None;
    }
    // Pane-content column past the gutter, plus horizontal scroll (0 while
    // wrapping — see `scroll::ensure_cursor_visible_horizontal`),
    // reconstructed back into a document display column.
    let display_col = viewport
        .horizontal_offset
        .advance_saturating((content_x - gutter_w) as u32);

    let top = dlm.clamp(top_pos(viewport));
    let clicked = dlm.advance_saturating(top, content_y as isize);
    // A click asks which cell it hit, so a column past the text resolves to
    // the display line's last cell rather than its last *content* cell —
    // landing on the line's `\n`, a real cursor position in HUME's inclusive model.
    Some(dlm.char_at(clicked, display_col, DisplayColTarget::Cell))
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests;
