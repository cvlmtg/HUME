//! Thin editor-side scroll helpers.
//!
//! The scroll verbs themselves (`heal`/`scroll_by`/`reveal`/`align`/
//! `reveal_horizontal`) live on `hume_engine::pane::Viewport` — see
//! `hume_engine::display_lines::scroll`. This module holds only what stays
//! genuinely editor-side: resolving a cursor's `CharOffset` into the
//! `DisplayLinePos` a verb wants, for the one caller (`z z`/`z k`/`z j`) that
//! starts from a char position rather than one a scroll pass already
//! resolved.

use hume_engine::display_lines::DisplayLineMap;
use hume_engine::pane::{ViewGeometry, Viewport};

/// Scroll so the cursor's display line lands `target_display_line` display
/// lines below the top of the viewport, clamped into scrolloff range by
/// [`Viewport::align`]. Used by the `z z`/`z k`/`z j` view commands.
pub(super) fn scroll_cursor_to_display_line(
    viewport: &mut Viewport,
    dlm: &mut DisplayLineMap<'_>,
    geo: ViewGeometry,
    cursor_char: hume_rope::offset::CharOffset,
    target_display_line: usize,
) {
    let cursor_pos = dlm.locate_display_line(cursor_char);
    viewport.align(dlm, geo, cursor_pos, target_display_line);
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests;
