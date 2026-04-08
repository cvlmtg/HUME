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
use engine::pipeline::RenderContext;
use engine::providers::GutterColumn;
use engine::layout::gutter_width_for_line;
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
    ctx: &mut RenderContext,
) -> Option<(u16, u16)> {
    let scratch = &mut ctx.cursor_format;
    let cursor_line = rope.char_to_line(cursor_char);
    let height = viewport.height as usize;
    if height == 0 { return None; }

    let (cursor_sub, cursor_col) =
        format_row_col(rope, cursor_line, cursor_char, wrap_mode, tab_width, whitespace, scratch);

    if wrap_mode.is_wrapping() {
        let top_row = viewport.top_row_offset as usize;
        let mut screen_row = 0usize;

        for line_idx in viewport.top_line..=cursor_line {
            let skip = if line_idx == viewport.top_line { top_row } else { 0 };
            if line_idx == cursor_line {
                screen_row += cursor_sub.saturating_sub(skip);
                break;
            }
            let rows = count_visual_rows(rope, line_idx, tab_width, whitespace, wrap_mode, scratch);
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
/// Used to offset the terminal cursor column past line numbers and other gutter
/// providers.
pub(crate) fn gutter_width(
    viewport: &ViewportState,
    gutter_columns: &[Box<dyn GutterColumn>],
    total_lines: usize,
) -> u16 {
    let approx_end = viewport.top_line + viewport.height as usize;
    let max_visible_line = approx_end.min(total_lines.saturating_sub(1));
    gutter_width_for_line(gutter_columns, max_visible_line)
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

/// Emit an OSC 12 sequence to set the terminal cursor colour for `mode`.
///
/// Command/Search mode positions the cursor in the statusline, which has a
/// white background — a default (white) cursor would be invisible. We set it
/// to black so it contrasts. All other modes reset to the terminal default.
///
/// OSC 12 (`\x1b]12;COLOR\x07`) is supported by the overwhelming majority of
/// modern terminal emulators. The reset form (`\x1b]112;\x07`) restores the
/// user's configured cursor colour.
pub(crate) fn set_color_for_mode(mode: EditorMode) -> std::io::Result<()> {
    use std::io::Write;
    let seq: &[u8] = match mode {
        EditorMode::Command | EditorMode::Search => b"\x1b]12;black\x07",
        _ => b"\x1b]112;\x07",
    };
    std::io::stdout().write_all(seq)
}

// ---------------------------------------------------------------------------
// Private helpers
// ---------------------------------------------------------------------------

/// Format `line_idx` and locate `cursor_char` within the resulting display rows.
///
/// Returns `(sub_row, col)` where `sub_row` is the 0-based display row index
/// within the line, and `col` is the display column within that row (the
/// grapheme's `col` field from the engine format output).
pub(crate) fn format_row_col(
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

// ---------------------------------------------------------------------------
// Screen-to-buffer reverse mapping
// ---------------------------------------------------------------------------

/// Convert a terminal-absolute `(screen_x, screen_y)` click position to a
/// buffer char offset.
///
/// `gutter_w` is the width of the gutter in terminal columns (from
/// [`gutter_width`]). Clicks that land inside the gutter return `None`.
///
/// The coordinate space is pane-relative: `(0, 0)` is the top-left cell of
/// the pane, matching what crossterm's `MouseEvent.column` / `.row` report
/// when the pane fills the whole terminal (which is currently always true).
///
/// Returns `None` if the click is:
/// - in the gutter,
/// - below the last buffer line, or
/// - the buffer is empty.
pub(crate) fn screen_to_char_offset(
    screen_x: u16,
    screen_y: u16,
    gutter_w: u16,
    viewport: &ViewportState,
    rope: &ropey::Rope,
    wrap_mode: &WrapMode,
    tab_width: u8,
    whitespace: &WhitespaceConfig,
    scratch: &mut FormatScratch,
) -> Option<usize> {
    // Clicks inside the gutter (line numbers etc.) do not map to text.
    if screen_x < gutter_w {
        return None;
    }

    let total_lines = rope.len_lines();
    // A buffer always ends with '\n', so the last "line" in ropey is an empty
    // sentinel. The real last editable line is `total_lines - 2` (or 0 for a
    // one-line buffer that is just "\n").
    let last_real_line = total_lines.saturating_sub(2);

    let target_row = screen_y as usize;

    if wrap_mode.is_wrapping() {
        // Walk from the top of the viewport counting display rows until we
        // reach the target screen row.
        let mut remaining = target_row;
        let top_row = viewport.top_row_offset as usize;

        for line_idx in viewport.top_line..total_lines {
            let rows =
                count_visual_rows(rope, line_idx, tab_width, whitespace, wrap_mode, scratch);
            let skip = if line_idx == viewport.top_line { top_row } else { 0 };
            let visible_rows = rows.saturating_sub(skip);

            if remaining < visible_rows {
                // This buffer line contains our target display row.
                let target_sub = skip + remaining;
                return char_at_display_col(
                    screen_x - gutter_w,
                    target_sub,
                    line_idx,
                    rope,
                    tab_width,
                    whitespace,
                    wrap_mode,
                    scratch,
                );
            }

            remaining = remaining.saturating_sub(visible_rows);
            if line_idx >= last_real_line {
                break;
            }
        }
        // Click is below the last line — clamp to end of last real line.
        char_at_display_col(
            screen_x - gutter_w,
            // sub-row doesn't matter much; last sub will be used anyway
            usize::MAX,
            last_real_line,
            rope,
            tab_width,
            whitespace,
            wrap_mode,
            scratch,
        )
    } else {
        // No-wrap: each buffer line is exactly one display row.
        let line_idx = (viewport.top_line + target_row).min(last_real_line);

        // Content column = screen column past gutter + horizontal scroll offset.
        let content_col =
            (screen_x - gutter_w) as usize + viewport.horizontal_offset as usize;

        // Format the line and find the grapheme at `content_col`.
        scratch.display_rows.clear();
        scratch.graphemes.clear();
        scratch.line_texts.clear();
        engine::format::format_buffer_line(
            rope,
            line_idx,
            tab_width,
            whitespace,
            wrap_mode,
            &[],
            scratch,
        );

        if scratch.display_rows.is_empty() {
            return Some(rope.line_to_char(line_idx));
        }
        let row = &scratch.display_rows[0];
        Some(col_to_char_offset(content_col, row, scratch, rope, line_idx))
    }
}

/// Given a target display column and a `DisplayRow`, return the char offset of
/// the grapheme that best matches (or the last grapheme if past the end).
fn col_to_char_offset(
    target_col: usize,
    row: &engine::types::DisplayRow,
    scratch: &engine::format::FormatScratch,
    rope: &ropey::Rope,
    line_idx: usize,
) -> usize {
    let graphemes = &scratch.graphemes[row.graphemes.clone()];
    if graphemes.is_empty() {
        return rope.line_to_char(line_idx);
    }

    // Find the grapheme whose column range contains `target_col`.
    for g in graphemes {
        let g_end = g.col as usize + g.width as usize;
        if target_col < g_end {
            return g.char_offset;
        }
    }
    // Past the last grapheme — return the last char offset in the row.
    graphemes.last().map(|g| g.char_offset).unwrap_or_else(|| rope.line_to_char(line_idx))
}

/// Find the char offset for `(content_col, target_sub_row)` within
/// `line_idx`, using the engine format pipeline.
fn char_at_display_col(
    content_col: u16,
    target_sub: usize,
    line_idx: usize,
    rope: &ropey::Rope,
    tab_width: u8,
    whitespace: &WhitespaceConfig,
    wrap_mode: &WrapMode,
    scratch: &mut FormatScratch,
) -> Option<usize> {
    scratch.display_rows.clear();
    scratch.graphemes.clear();
    scratch.line_texts.clear();
    engine::format::format_buffer_line(rope, line_idx, tab_width, whitespace, wrap_mode, &[], scratch);

    if scratch.display_rows.is_empty() {
        return Some(rope.line_to_char(line_idx));
    }

    // Clamp target sub-row to the last display row of this line.
    let sub = target_sub.min(scratch.display_rows.len().saturating_sub(1));
    let row = &scratch.display_rows[sub];
    Some(col_to_char_offset(content_col as usize, row, scratch, rope, line_idx))
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use engine::format::FormatScratch;
    use engine::pane::{ViewportState, WrapMode, WhitespaceConfig};
    use ropey::Rope;

    fn vp(top_line: usize, width: u16, height: u16) -> ViewportState {
        let mut v = ViewportState::new(width, height);
        v.top_line = top_line;
        v
    }

    fn ws() -> WhitespaceConfig { WhitespaceConfig::default() }

    // ── screen_to_char_offset (no-wrap) ──────────────────────────────────────

    /// Click on column 0 of line 0, no gutter → char 0.
    #[test]
    fn nowrap_click_first_char() {
        // "abc\ndef\n": chars 0-2 = 'a','b','c', char 3 = '\n', chars 4-6 = 'd','e','f'
        let rope = Rope::from_str("abc\ndef\n");
        let v = vp(0, 80, 10);
        let mut s = FormatScratch::new();
        let got = screen_to_char_offset(0, 0, 0, &v, &rope, &WrapMode::None, 4, &ws(), &mut s);
        assert_eq!(got, Some(0));
    }

    /// Click on column 2 of line 0 → char 2.
    #[test]
    fn nowrap_click_mid_first_line() {
        let rope = Rope::from_str("abc\ndef\n");
        let v = vp(0, 80, 10);
        let mut s = FormatScratch::new();
        let got = screen_to_char_offset(2, 0, 0, &v, &rope, &WrapMode::None, 4, &ws(), &mut s);
        assert_eq!(got, Some(2));
    }

    /// Click on screen row 1, column 0 → start of second line (char 4).
    #[test]
    fn nowrap_click_second_line() {
        let rope = Rope::from_str("abc\ndef\n");
        let v = vp(0, 80, 10);
        let mut s = FormatScratch::new();
        let got = screen_to_char_offset(0, 1, 0, &v, &rope, &WrapMode::None, 4, &ws(), &mut s);
        assert_eq!(got, Some(4)); // 'd' is char 4
    }

    /// Click in the gutter (screen_x < gutter_w) returns None.
    #[test]
    fn nowrap_gutter_click_returns_none() {
        let rope = Rope::from_str("abc\n");
        let v = vp(0, 80, 10);
        let mut s = FormatScratch::new();
        // gutter_w = 4; click at column 2 is inside the gutter.
        let got = screen_to_char_offset(2, 0, 4, &v, &rope, &WrapMode::None, 4, &ws(), &mut s);
        assert_eq!(got, None);
    }

    /// Click past end of line returns the newline char at the end of the line.
    ///
    /// In HUME's inclusive selection model, the newline char is a valid cursor
    /// position (end-of-line). "hi\n" has chars: h=0, i=1, \n=2.
    #[test]
    fn nowrap_click_past_line_end() {
        let rope = Rope::from_str("hi\n");
        let v = vp(0, 80, 10);
        let mut s = FormatScratch::new();
        // Click at column 99, way past "hi" — lands at '\n' (char 2), the eol marker.
        let got = screen_to_char_offset(99, 0, 0, &v, &rope, &WrapMode::None, 4, &ws(), &mut s);
        assert_eq!(got, Some(2));
    }

    /// Viewport scrolled down: screen_y=0 refers to top_line, not line 0.
    #[test]
    fn nowrap_viewport_scrolled() {
        // Lines: 0=a, 1=b, 2=c, 3=d. top_line=2 → screen row 0 is line 2 = 'c'.
        let rope = Rope::from_str("a\nb\nc\nd\n");
        let v = vp(2, 80, 10); // top_line = 2
        let mut s = FormatScratch::new();
        // Line 2 starts at char 4 ('c'). Screen row 0, col 0 → char 4.
        let got = screen_to_char_offset(0, 0, 0, &v, &rope, &WrapMode::None, 4, &ws(), &mut s);
        assert_eq!(got, Some(4));
    }

    /// Horizontal scroll: content_col = screen_x - gutter_w + h_offset.
    #[test]
    fn nowrap_horizontal_scroll() {
        // "abcde\n" with h_offset=2: screen col 0 maps to content col 2 = 'c' (char 2).
        let rope = Rope::from_str("abcde\n");
        let mut v = vp(0, 80, 10);
        v.horizontal_offset = 2;
        let mut s = FormatScratch::new();
        let got = screen_to_char_offset(0, 0, 0, &v, &rope, &WrapMode::None, 4, &ws(), &mut s);
        assert_eq!(got, Some(2));
    }

    // ── screen_to_char_offset (wrap) ─────────────────────────────────────────

    /// With Soft { width: 4 }, "abcdefgh\n" wraps: row 0 = "abcd", row 1 = "efgh".
    /// Click at screen (0, 0) → char 0 ('a').
    /// Click at screen (0, 1) → char 4 ('e').
    #[test]
    fn wrap_click_first_and_second_visual_row() {
        let rope = Rope::from_str("abcdefgh\n");
        let v = vp(0, 10, 10);
        let wrap = WrapMode::Soft { width: 4 };
        let mut s = FormatScratch::new();

        let row0 = screen_to_char_offset(0, 0, 0, &v, &rope, &wrap, 4, &ws(), &mut s);
        assert_eq!(row0, Some(0));

        let row1 = screen_to_char_offset(0, 1, 0, &v, &rope, &wrap, 4, &ws(), &mut s);
        assert_eq!(row1, Some(4));
    }

    /// Click on column 2 in the second wrap row → char 6 ('g').
    #[test]
    fn wrap_click_mid_second_row() {
        let rope = Rope::from_str("abcdefgh\n");
        let v = vp(0, 10, 10);
        let wrap = WrapMode::Soft { width: 4 };
        let mut s = FormatScratch::new();

        let got = screen_to_char_offset(2, 1, 0, &v, &rope, &wrap, 4, &ws(), &mut s);
        assert_eq!(got, Some(6)); // 'g' is char 6
    }

    /// Click below the last line is clamped to the last real line.
    #[test]
    fn wrap_click_below_last_line_clamped() {
        let rope = Rope::from_str("hi\n");
        let v = vp(0, 80, 10);
        let wrap = WrapMode::Soft { width: 40 };
        let mut s = FormatScratch::new();
        // Screen row 99 is past the end — should return something in line 0.
        let got = screen_to_char_offset(0, 99, 0, &v, &rope, &wrap, 4, &ws(), &mut s);
        assert!(got.is_some());
    }
}
