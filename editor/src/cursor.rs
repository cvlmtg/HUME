//! Terminal cursor placement logic.
//!
//! The terminal cursor (the blinking bar or block emitted via escape sequences)
//! is an editor-level concern. The engine knows nothing about it — it only
//! styles the grapheme at each selection head.
//!
//! This module computes:
//! - [`screen_pos`] — the `(col, row)` of the primary selection head in the
//!   pane content area, for [`crossterm::cursor::SetCursorStyle`] placement.
//! - [`gutter_width`] — the gutter offset to add so the terminal cursor lands
//!   at the correct absolute screen column.
//! - [`sub_row`] — which wrapped display row the primary selection head is on
//!   (used by scroll to keep the head visible).
//! - [`shape`] — the [`crossterm::cursor::SetCursorStyle`] for the current mode.

use crossterm::cursor::SetCursorStyle;
use engine::format::{FormatScratch, count_visual_rows};
use engine::pane::{ViewportState, WrapMode, WhitespaceConfig};
use engine::providers::GutterColumn;
use engine::types::EditorMode;

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Compute the on-screen `(col, row)` of `cursor_char` within the pane content
/// area (i.e., after the gutter).
///
/// Returns `None` if the position is outside the visible viewport (defensive;
/// should not happen after `scroll::ensure_cursor_visible`).
///
/// In no-wrap mode, `col` accounts for `viewport.horizontal_offset`.
/// In wrap mode, `col` is the column within the display row (offset 0 = left edge).
pub(crate) fn screen_pos(
    viewport: &ViewportState,
    rope: &ropey::Rope,
    cursor_char: usize,
    wrap_mode: &WrapMode,
    tab_width: u8,
    whitespace: &WhitespaceConfig,
) -> Option<(u16, u16)> {
    let cursor_line = rope.char_to_line(cursor_char);
    let height = viewport.height as usize;
    if height == 0 { return None; }

    let mut scratch = FormatScratch::new();
    let (cursor_sub, cursor_col) =
        format_row_col(rope, cursor_line, cursor_char, wrap_mode, tab_width, whitespace, &mut scratch);

    if wrap_mode.is_wrapping() {
        let top_row = viewport.top_row_offset as usize;
        let mut screen_row = 0usize;

        for line_idx in viewport.top_line..=cursor_line {
            let skip = if line_idx == viewport.top_line { top_row } else { 0 };
            if line_idx == cursor_line {
                screen_row += cursor_sub.saturating_sub(skip);
                break;
            }
            let rows = count_visual_rows(rope, line_idx, tab_width, whitespace, wrap_mode, &mut scratch);
            screen_row += rows.saturating_sub(skip);
            if screen_row >= height { return None; }
        }

        if screen_row >= height { return None; }
        Some((cursor_col as u16, screen_row as u16))
    } else {
        if cursor_line < viewport.top_line { return None; }
        let screen_row = cursor_line - viewport.top_line;
        if screen_row >= height { return None; }

        let col = cursor_col.saturating_sub(viewport.horizontal_offset as usize);
        Some((col as u16, screen_row as u16))
    }
}

/// Gutter width in terminal columns for the current frame.
///
/// Mirrors the engine's `compute_viewport` formula — the sum of all registered
/// gutter column widths for the widest visible line. Used to offset the terminal
/// cursor column past line numbers and other gutter providers.
pub(crate) fn gutter_width(
    viewport: &ViewportState,
    gutter_columns: &[Box<dyn GutterColumn>],
    total_lines: usize,
) -> u16 {
    let approx_end = viewport.top_line + viewport.height as usize;
    let max_visible_line = approx_end.min(total_lines.saturating_sub(1));
    gutter_columns.iter().map(|c| c.width(max_visible_line) as u16).sum()
}

/// Which wrapped display sub-row of buffer `line_idx` contains `cursor_char`.
///
/// Used by `scroll::ensure_cursor_visible` to keep the selection head visible.
pub(crate) fn sub_row(
    rope: &ropey::Rope,
    line_idx: usize,
    cursor_char: usize,
    wrap_mode: &WrapMode,
    tab_width: u8,
    whitespace: &WhitespaceConfig,
    scratch: &mut FormatScratch,
) -> usize {
    format_row_col(rope, line_idx, cursor_char, wrap_mode, tab_width, whitespace, scratch).0
}

/// The terminal cursor shape for `mode`.
///
/// Bar modes (Insert, Command, Search, Select) get `SteadyBar`; all others
/// get `SteadyBlock`.
pub(crate) fn shape(mode: EditorMode) -> SetCursorStyle {
    if mode.cursor_is_bar() { SetCursorStyle::SteadyBar } else { SetCursorStyle::SteadyBlock }
}

// ---------------------------------------------------------------------------
// Private helpers
// ---------------------------------------------------------------------------

/// Format `line_idx` and locate `cursor_char` within the resulting display rows.
///
/// Returns `(sub_row, col)` where `sub_row` is the 0-based display row index
/// within the line, and `col` is the display column within that row (the
/// grapheme's `col` field from the engine format output).
fn format_row_col(
    rope: &ropey::Rope,
    line_idx: usize,
    cursor_char: usize,
    wrap_mode: &WrapMode,
    tab_width: u8,
    whitespace: &WhitespaceConfig,
    scratch: &mut FormatScratch,
) -> (usize, usize) {
    let line_start_char = rope.line_to_char(line_idx);
    let line_start_byte = rope.char_to_byte(line_start_char);
    let cursor_byte_abs = rope.char_to_byte(cursor_char);
    let cursor_byte_in_line = cursor_byte_abs.saturating_sub(line_start_byte);

    scratch.display_rows.clear();
    scratch.graphemes.clear();
    scratch.line_texts.clear();
    engine::format::format_buffer_line(rope, line_idx, tab_width, whitespace, wrap_mode, &[], scratch);

    for (i, row) in scratch.display_rows.iter().enumerate() {
        if row.graphemes.is_empty() {
            continue;
        }
        let first = &scratch.graphemes[row.graphemes.start];
        let last  = &scratch.graphemes[row.graphemes.end - 1];
        let row_byte_start = first.byte_range.start;
        let row_byte_end   = last.byte_range.end;
        let is_last = i + 1 == scratch.display_rows.len();

        if cursor_byte_in_line >= row_byte_start
            && (cursor_byte_in_line < row_byte_end || is_last)
        {
            let col = scratch.graphemes[row.graphemes.clone()]
                .iter()
                .find(|g| g.byte_range.start == cursor_byte_in_line)
                .map_or_else(
                    || {
                        // Selection head is past all graphemes in this row (e.g., at eol).
                        let lg = &scratch.graphemes[row.graphemes.end - 1];
                        (lg.col + lg.width as u16) as usize
                    },
                    |g| g.col as usize,
                );
            return (i, col);
        }
    }

    // Fallback: last sub-row, column past last grapheme.
    let last = scratch.display_rows.len().saturating_sub(1);
    let col = scratch.display_rows.get(last)
        .filter(|r| !r.graphemes.is_empty())
        .map(|r| {
            let lg = &scratch.graphemes[r.graphemes.end - 1];
            (lg.col + lg.width as u16) as usize
        })
        .unwrap_or(0);
    (last, col)
}
