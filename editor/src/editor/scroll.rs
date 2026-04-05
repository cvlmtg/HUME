//! Scroll logic for the engine-based viewport.
//!
//! Operates on `engine::pane::ViewportState` and `ropey::Rope` — no
//! dependency on `Editor` or the old `ViewState`. Called from `Editor::run()`
//! via `scroll::ensure_cursor_visible(...)`.
//!
//! Field mapping from old `ViewState` to engine `ViewportState`:
//! - `scroll_offset`     → `top_line`
//! - `scroll_sub_offset` → `top_row_offset` (u16)
//! - `col_offset`        → `horizontal_offset` (u16)
//! - `height`/`width`    → same names

use engine::format::{FormatScratch, count_visual_rows};
use engine::pane::{ViewportState, WrapMode, WhitespaceConfig};

/// Rows of look-ahead kept between the cursor and the top/bottom edge.
const SCROLL_MARGIN: usize = 3;

/// Columns of look-ahead kept between the cursor and the left/right edge.
const SCROLL_MARGIN_H: usize = 5;

// ---------------------------------------------------------------------------
// Public entry points
// ---------------------------------------------------------------------------

/// Adjust `viewport.top_line` (and `top_row_offset` when wrapping) so the
/// cursor's display row is visible with margin.
pub(super) fn ensure_cursor_visible(
    viewport: &mut ViewportState,
    rope: &ropey::Rope,
    cursor_char: usize,
    wrap_mode: &WrapMode,
    tab_width: u8,
    whitespace: &WhitespaceConfig,
) {
    if wrap_mode.is_wrapping() {
        ensure_cursor_visible_wrapped(viewport, rope, cursor_char, wrap_mode, tab_width, whitespace);
    } else {
        let cursor_line = rope.char_to_line(cursor_char);
        ensure_cursor_visible_unwrapped(viewport, cursor_line);
    }
}

/// Adjust `viewport.horizontal_offset` so the cursor's display column is
/// visible with margin. When wrapping is active, horizontal offset is forced
/// to 0 (wrapping handles long lines).
pub(super) fn ensure_cursor_visible_horizontal(
    viewport: &mut ViewportState,
    rope: &ropey::Rope,
    cursor_char: usize,
    wrap_mode: &WrapMode,
    tab_width: usize,
) {
    if wrap_mode.is_wrapping() {
        viewport.horizontal_offset = 0;
        return;
    }

    let cursor_line = rope.char_to_line(cursor_char);
    let cursor_col = display_col_in_line(rope, cursor_line, cursor_char, tab_width);
    let content_width = viewport.width as usize;
    if content_width == 0 {
        return;
    }

    let margin = SCROLL_MARGIN_H.min(content_width / 2);
    let offset = viewport.horizontal_offset as usize;

    if cursor_col < offset + margin {
        viewport.horizontal_offset = cursor_col.saturating_sub(margin) as u16;
    } else if cursor_col >= offset + content_width - margin {
        viewport.horizontal_offset =
            cursor_col.saturating_sub(content_width - margin - 1) as u16;
    }
}

// ---------------------------------------------------------------------------
// Private helpers
// ---------------------------------------------------------------------------

fn ensure_cursor_visible_unwrapped(viewport: &mut ViewportState, cursor_line: usize) {
    let height = viewport.height as usize;
    let margin = SCROLL_MARGIN.min(height / 2);

    let top = viewport.top_line;
    if cursor_line < top + margin {
        viewport.top_line = cursor_line.saturating_sub(margin);
    } else if height > 0 && cursor_line >= top + height - margin {
        viewport.top_line = cursor_line.saturating_sub(height - margin - 1);
    }
}

fn ensure_cursor_visible_wrapped(
    viewport: &mut ViewportState,
    rope: &ropey::Rope,
    cursor_char: usize,
    wrap_mode: &WrapMode,
    tab_width: u8,
    whitespace: &WhitespaceConfig,
) {
    let cursor_line = rope.char_to_line(cursor_char);
    let height = viewport.height as usize;
    if height == 0 {
        return;
    }

    let margin = SCROLL_MARGIN.min(height / 2);
    let mut scratch = FormatScratch::new();

    let cursor_sub = cursor_sub_row(rope, cursor_line, cursor_char, wrap_mode, tab_width, whitespace, &mut scratch);

    // ── Cursor above the viewport ────────────────────────────────────────────
    let top_row = viewport.top_row_offset as usize;
    if cursor_line < viewport.top_line
        || (cursor_line == viewport.top_line && cursor_sub < top_row)
    {
        scroll_backward_from_cursor(viewport, rope, cursor_line, cursor_sub, margin, wrap_mode, tab_width, whitespace, &mut scratch);
        return;
    }

    // ── Count display rows from scroll position to cursor ────────────────────
    let mut display_row: usize = 0;
    for line_idx in viewport.top_line..=cursor_line {
        let rows = count_visual_rows(rope, line_idx, tab_width, whitespace, wrap_mode, &mut scratch);
        let skip = if line_idx == viewport.top_line { top_row } else { 0 };
        if line_idx == cursor_line {
            display_row += cursor_sub.saturating_sub(skip);
            break;
        }
        display_row += rows.saturating_sub(skip);
        if display_row >= height {
            break;
        }
    }

    // ── Cursor below the viewport ────────────────────────────────────────────
    if display_row >= height.saturating_sub(margin) {
        let target_row = height.saturating_sub(margin).saturating_sub(1);
        scroll_backward_from_cursor(viewport, rope, cursor_line, cursor_sub, target_row, wrap_mode, tab_width, whitespace, &mut scratch);
    }
}

fn scroll_backward_from_cursor(
    viewport: &mut ViewportState,
    rope: &ropey::Rope,
    cursor_line: usize,
    cursor_sub: usize,
    target_rows: usize,
    wrap_mode: &WrapMode,
    tab_width: u8,
    whitespace: &WhitespaceConfig,
    scratch: &mut FormatScratch,
) {
    viewport.top_line = cursor_line;
    viewport.top_row_offset = cursor_sub as u16;
    let mut rows_above = 0;
    while rows_above < target_rows {
        if viewport.top_row_offset > 0 {
            viewport.top_row_offset -= 1;
            rows_above += 1;
        } else if viewport.top_line > 0 {
            viewport.top_line -= 1;
            let rows = count_visual_rows(rope, viewport.top_line, tab_width, whitespace, wrap_mode, scratch);
            if rows_above + rows > target_rows {
                viewport.top_row_offset = (rows - (target_rows - rows_above)) as u16;
                break;
            }
            rows_above += rows;
        } else {
            break;
        }
    }
}

/// Which wrapped sub-row of buffer `line_idx` contains `cursor_char`.
///
/// Uses the engine's `format_buffer_line` to get display rows, then finds
/// which row's grapheme range contains the cursor's byte offset within the line.
pub(super) fn cursor_sub_row(
    rope: &ropey::Rope,
    line_idx: usize,
    cursor_char: usize,
    wrap_mode: &WrapMode,
    tab_width: u8,
    whitespace: &WhitespaceConfig,
    scratch: &mut FormatScratch,
) -> usize {
    format_cursor_row_col(rope, line_idx, cursor_char, wrap_mode, tab_width, whitespace, scratch).0
}

/// Compute the on-screen `(col, row)` of `cursor_char` within the pane content
/// area (i.e., after the gutter).
///
/// Returns `None` if the cursor is outside the visible viewport (should not
/// happen after `ensure_cursor_visible`, but is handled defensively).
///
/// In no-wrap mode, `col` accounts for `viewport.horizontal_offset`.
/// In wrap mode, `col` is the column within the display row (offset 0 = left edge).
pub(super) fn cursor_screen_pos(
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
        format_cursor_row_col(rope, cursor_line, cursor_char, wrap_mode, tab_width, whitespace, &mut scratch);

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

/// Format `line_idx` and locate `cursor_char` within the resulting display rows.
///
/// Returns `(sub_row, col)` where `sub_row` is the 0-based display row index
/// within the line, and `col` is the display column within that row (grapheme's
/// `col` field from the engine format output).
///
/// Populates `scratch` with the line's formatted output for the caller to reuse.
fn format_cursor_row_col(
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
            // Find the grapheme at the cursor byte to get its display column.
            let col = scratch.graphemes[row.graphemes.clone()]
                .iter()
                .find(|g| g.byte_range.start == cursor_byte_in_line)
                .map_or_else(
                    || {
                        // Cursor is past all graphemes in this row (e.g., at eol).
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

// ---------------------------------------------------------------------------
// Column calculation helper
// ---------------------------------------------------------------------------

/// Display column of `cursor_char` within `line_idx`, accounting for tab stops.
fn display_col_in_line(
    rope: &ropey::Rope,
    line_idx: usize,
    cursor_char: usize,
    tab_width: usize,
) -> usize {
    let line_start = rope.line_to_char(line_idx);
    let mut col = 0usize;
    let line = rope.line(line_idx);
    // Walk graphemes up to cursor_char, accumulating display columns.
    use unicode_segmentation::UnicodeSegmentation;
    use unicode_width::UnicodeWidthStr;
    let mut char_pos = line_start;
    for grapheme in line.chunks().flat_map(|c| c.graphemes(true)) {
        if char_pos >= cursor_char {
            break;
        }
        if grapheme == "\t" {
            // Tab expands to the next tab stop.
            let remainder = tab_width - (col % tab_width);
            col += remainder;
        } else {
            col += UnicodeWidthStr::width(grapheme);
        }
        char_pos += grapheme.chars().count();
    }
    col
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use engine::pane::{ViewportState, WrapMode, WhitespaceConfig};
    use ropey::Rope;

    fn viewport(top: usize, height: u16, width: u16) -> ViewportState {
        let mut v = ViewportState::new(width, height);
        v.top_line = top;
        v
    }

    fn rope(text: &str) -> Rope {
        Rope::from_str(text)
    }

    // ── ensure_cursor_visible (no-wrap) ──────────────────────────────────────

    #[test]
    fn no_wrap_cursor_visible_no_scroll_needed() {
        let r = rope("a\nb\nc\nd\ne\n");
        let mut v = viewport(0, 10, 80);
        ensure_cursor_visible(&mut v, &r, r.line_to_char(2), &WrapMode::None, 4, &WhitespaceConfig::default());
        assert_eq!(v.top_line, 0);
    }

    #[test]
    fn no_wrap_cursor_below_viewport_scrolls_down() {
        let r = rope("a\nb\nc\nd\ne\nf\ng\nh\n");
        let mut v = viewport(0, 5, 80);
        ensure_cursor_visible(&mut v, &r, r.line_to_char(7), &WrapMode::None, 4, &WhitespaceConfig::default());
        let cursor_line = 7usize;
        assert!(cursor_line >= v.top_line);
        assert!(cursor_line < v.top_line + v.height as usize);
    }

    #[test]
    fn no_wrap_cursor_above_viewport_scrolls_up() {
        let r = rope("a\nb\nc\nd\ne\nf\ng\nh\n");
        let mut v = viewport(5, 5, 80);
        ensure_cursor_visible(&mut v, &r, r.line_to_char(1), &WrapMode::None, 4, &WhitespaceConfig::default());
        let cursor_line = 1usize;
        assert!(cursor_line >= v.top_line);
        assert!(cursor_line < v.top_line + v.height as usize);
    }

    // ── display_col_in_line ──────────────────────────────────────────────────

    #[test]
    fn display_col_ascii() {
        let r = rope("hello\n");
        // Cursor at char 3 ('l') → col 3.
        assert_eq!(display_col_in_line(&r, 0, 3, 4), 3);
    }

    #[test]
    fn display_col_tab_expansion() {
        let r = rope("\thello\n");
        // Tab at col 0 with tab_width 4 → expands to 4 cells. Cursor at char 1 → col 4.
        assert_eq!(display_col_in_line(&r, 0, 1, 4), 4);
    }

    // ── cursor_sub_row ───────────────────────────────────────────────────────

    #[test]
    fn cursor_sub_row_no_wrap() {
        // With a WrapMode::None, the whole line is one row, sub-row 0.
        let r = rope("hello world\n");
        let mut scratch = FormatScratch::new();
        let sub = cursor_sub_row(&r, 0, 5, &WrapMode::None, 4, &WhitespaceConfig::default(), &mut scratch);
        assert_eq!(sub, 0);
    }

    #[test]
    fn cursor_sub_row_wrapped() {
        // "abcdefgh" with Soft { width: 4 } → 2 rows: "abcd" / "efgh".
        let r = rope("abcdefgh\n");
        let mut scratch = FormatScratch::new();
        // Cursor at char 0 → sub-row 0.
        let sub0 = cursor_sub_row(&r, 0, 0, &WrapMode::Soft { width: 4 }, 4, &WhitespaceConfig::default(), &mut scratch);
        assert_eq!(sub0, 0);
        // Cursor at char 4 → sub-row 1.
        let sub1 = cursor_sub_row(&r, 0, 4, &WrapMode::Soft { width: 4 }, 4, &WhitespaceConfig::default(), &mut scratch);
        assert_eq!(sub1, 1);
    }
}
