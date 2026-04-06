//! Scroll logic for the engine-based viewport.
//!
//! Operates on `engine::pane::ViewportState` and `ropey::Rope`.
//! Called from `Editor::run()` via `scroll::ensure_cursor_visible(...)`.

use engine::format::{FormatScratch, count_visual_rows};
use engine::pane::{ViewportState, WrapMode, WhitespaceConfig};

use crate::core::grapheme::display_col_in_line;
use crate::cursor;

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
    scratch: &mut FormatScratch,
) {
    if wrap_mode.is_wrapping() {
        ensure_cursor_visible_wrapped(viewport, rope, cursor_char, wrap_mode, tab_width, whitespace, scratch);
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
    scratch: &mut FormatScratch,
) {
    let cursor_line = rope.char_to_line(cursor_char);
    let height = viewport.height as usize;
    if height == 0 {
        return;
    }

    let margin = SCROLL_MARGIN.min(height / 2);

    let cursor_sub = cursor::sub_row(rope, cursor_line, cursor_char, wrap_mode, tab_width, whitespace, scratch);

    // ── Cursor above the viewport ────────────────────────────────────────────
    let top_row = viewport.top_row_offset as usize;
    if cursor_line < viewport.top_line
        || (cursor_line == viewport.top_line && cursor_sub < top_row)
    {
        scroll_backward_from_cursor(viewport, rope, cursor_line, cursor_sub, margin, wrap_mode, tab_width, whitespace, scratch);
        return;
    }

    // ── Count display rows from scroll position to cursor ────────────────────
    let mut display_row: usize = 0;
    for line_idx in viewport.top_line..=cursor_line {
        let rows = count_visual_rows(rope, line_idx, tab_width, whitespace, wrap_mode, scratch);
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
        scroll_backward_from_cursor(viewport, rope, cursor_line, cursor_sub, target_row, wrap_mode, tab_width, whitespace, scratch);
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
        ensure_cursor_visible(&mut v, &r, r.line_to_char(2), &WrapMode::None, 4, &WhitespaceConfig::default(), &mut FormatScratch::new());
        assert_eq!(v.top_line, 0);
    }

    #[test]
    fn no_wrap_cursor_below_viewport_scrolls_down() {
        let r = rope("a\nb\nc\nd\ne\nf\ng\nh\n");
        let mut v = viewport(0, 5, 80);
        ensure_cursor_visible(&mut v, &r, r.line_to_char(7), &WrapMode::None, 4, &WhitespaceConfig::default(), &mut FormatScratch::new());
        let cursor_line = 7usize;
        assert!(cursor_line >= v.top_line);
        assert!(cursor_line < v.top_line + v.height as usize);
    }

    #[test]
    fn no_wrap_cursor_above_viewport_scrolls_up() {
        let r = rope("a\nb\nc\nd\ne\nf\ng\nh\n");
        let mut v = viewport(5, 5, 80);
        ensure_cursor_visible(&mut v, &r, r.line_to_char(1), &WrapMode::None, 4, &WhitespaceConfig::default(), &mut FormatScratch::new());
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
    fn display_col_cjk() {
        // CJK ideographs are double-width: "世界abc\n", cursor at char 3 ("b").
        // Width of "世界a" = 2 + 2 + 1 = 5 display columns.
        let r = rope("世界abc\n");
        assert_eq!(display_col_in_line(&r, 0, 3, 4), 5);
    }

    #[test]
    fn display_col_combining_mark() {
        // "é" = e + combining acute (2 chars, 1 grapheme cluster).
        // Combining marks have display width 0 but the cluster is clamped to
        // min 1 by grapheme_advance, so the total is 1 display column.
        let r = rope("e\u{0301}\n");
        assert_eq!(display_col_in_line(&r, 0, 2, 4), 1);
    }

    #[test]
    fn display_col_tab_expansion() {
        let r = rope("\thello\n");
        // Tab at col 0 with tab_width 4 → expands to 4 cells. Cursor at char 1 → col 4.
        assert_eq!(display_col_in_line(&r, 0, 1, 4), 4);
    }

    #[test]
    fn display_col_tab_after_content() {
        // "ab\t" — 'a'(1) + 'b'(1) = col 2, tab expands to 2 (next stop at 4).
        let r = rope("ab\t\n");
        assert_eq!(display_col_in_line(&r, 0, 3, 4), 4);
    }

    #[test]
    fn display_col_multiple_tabs() {
        // "\t\t" with tab_width 4 → 4 + 4 = 8 columns.
        let r = rope("\t\t\n");
        assert_eq!(display_col_in_line(&r, 0, 2, 4), 8);
    }

    #[test]
    fn display_col_multiline() {
        // "hello\nworld\n" — cursor at char 8 ("r") on line 1.
        // Line 1 starts at char 6 ("world"), so column = width of "wo" = 2.
        let r = rope("hello\nworld\n");
        assert_eq!(display_col_in_line(&r, 1, 8, 4), 2);
    }

    // ── cursor_sub_row ───────────────────────────────────────────────────────

    #[test]
    fn cursor_sub_row_no_wrap() {
        // With a WrapMode::None, the whole line is one row, sub-row 0.
        let r = rope("hello world\n");
        let mut scratch = FormatScratch::new();
        let sub = crate::cursor::sub_row(&r, 0, 5, &WrapMode::None, 4, &WhitespaceConfig::default(), &mut scratch);
        assert_eq!(sub, 0);
    }

    #[test]
    fn cursor_sub_row_wrapped() {
        // "abcdefgh" with Soft { width: 4 } → 2 rows: "abcd" / "efgh".
        let r = rope("abcdefgh\n");
        let mut scratch = FormatScratch::new();
        // Cursor at char 0 → sub-row 0.
        let sub0 = crate::cursor::sub_row(&r, 0, 0, &WrapMode::Soft { width: 4 }, 4, &WhitespaceConfig::default(), &mut scratch);
        assert_eq!(sub0, 0);
        // Cursor at char 4 → sub-row 1.
        let sub1 = crate::cursor::sub_row(&r, 0, 4, &WrapMode::Soft { width: 4 }, 4, &WhitespaceConfig::default(), &mut scratch);
        assert_eq!(sub1, 1);
    }
}
