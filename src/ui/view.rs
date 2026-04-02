use std::borrow::Cow;

use unicode_segmentation::UnicodeSegmentation;

use crate::core::buffer::Buffer;
use crate::core::grapheme::{display_col_in_line, grapheme_advance};
use crate::ui::display_line::DisplayLine;
use crate::ui::whitespace::WhitespaceConfig;
use crate::helpers::line_end_exclusive;
use crate::core::selection::SelectionSet;

/// How many lines to keep between the cursor and the top/bottom edge of the
/// viewport before scrolling. 3 lines gives a comfortable look-ahead without
/// being overly aggressive.
const SCROLL_MARGIN: usize = 3;

/// Horizontal scroll margin — columns of look-ahead kept between the cursor
/// and the left/right edges of the content area before scrolling kicks in.
const SCROLL_MARGIN_H: usize = 5;

/// How line numbers are displayed in the gutter.
///
/// - `Absolute` — every line shows its 1-based buffer line number.
/// - `Relative` — every line shows its distance from the cursor line; the
///   cursor line shows `0`.
/// - `Hybrid` *(default)* — the cursor line shows its absolute number; all
///   other lines show their relative distance. This gives the best of both
///   worlds: you can navigate by exact line number and jump by relative offset.
#[allow(dead_code)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) enum LineNumberStyle {
    Absolute,
    Relative,
    #[default]
    Hybrid,
}

/// The viewport state for a single editor pane.
///
/// Tracks which portion of the buffer is visible and how much space is
/// available for content. There is currently one `ViewState`; future split
/// panes will each own their own.
///
/// `height` and `width` are updated from the terminal size at the start of
/// every event-loop iteration, so they always reflect the current terminal
/// dimensions. `gutter_width` is recomputed whenever the buffer's line count
/// changes (which happens on every edit).
pub(crate) struct ViewState {
    /// Index of the first buffer line visible at the top of the viewport (0-based).
    pub scroll_offset: usize,

    /// Number of rows available for document content.
    ///
    /// This is the terminal height minus the statusline (1 row). The renderer
    /// only draws document lines into this many rows.
    pub height: usize,

    /// Total terminal width in columns.
    pub width: usize,

    /// Width of the line-number gutter in display columns.
    ///
    /// Computed by [`compute_gutter_width`] and cached here so the renderer
    /// and the viewport both use the same value without recomputing.
    pub gutter_width: usize,

    /// How line numbers are rendered in the gutter.
    pub line_number_style: LineNumberStyle,

    /// Number of display columns scrolled horizontally (0 = no horizontal scroll).
    ///
    /// Measured in display columns (terminal cells), not grapheme clusters, so
    /// that CJK double-width characters are accounted for correctly. Updated by
    /// [`ensure_cursor_visible_horizontal`](Self::ensure_cursor_visible_horizontal).
    pub col_offset: usize,

    /// Tab stop width in display columns. A tab at display column `c` expands
    /// to `tab_width - (c % tab_width)` columns. Default: 4.
    pub tab_width: usize,

    /// Whitespace rendering configuration — which whitespace characters get
    /// visual indicators and what replacement characters to use.
    pub whitespace: WhitespaceConfig,

    /// When `true`, long lines wrap to the next display row instead of
    /// scrolling horizontally. `col_offset` is forced to 0 while active.
    pub soft_wrap: bool,

    /// Number of wrapped sub-rows to skip within the buffer line at
    /// `scroll_offset`. Only meaningful when `soft_wrap` is `true`.
    ///
    /// Handles the edge case where a single buffer line wraps to more rows
    /// than the viewport height — the cursor might be on sub-row 50 of a
    /// 100-row wrapped line, so the viewport needs to start partway through.
    pub scroll_sub_offset: usize,
}

/// Compute the line-number gutter width for a buffer with `total_lines` lines.
///
/// The gutter renders line numbers as `"{number:>w$} "` where `w = gutter_width - 1`.
/// That is: the number right-aligned in all-but-one columns, followed by one
/// trailing space separator. Left padding fills the remaining space automatically.
///
/// - digits  = decimal digits in `total_lines` (minimum 1)
/// - width   = digits + 2 (one trailing space + at least one leading space), minimum 4
///
/// Minimum 4 keeps the gutter from becoming uselessly narrow on tiny files.
pub(crate) fn compute_gutter_width(total_lines: usize) -> usize {
    // ilog10(0) is undefined; treat 0-line buffers the same as 1-line.
    let digits = if total_lines <= 1 {
        1
    } else {
        total_lines.ilog10() as usize + 1
    };
    (1 + digits + 1).max(4)
}

// ── Soft-wrap helpers ────────────────────────────────────────────────────────

/// Split a buffer line into wrapped segments that each fit within `content_width`
/// display columns.
///
/// Returns `Vec<(char_start, char_end)>` — one pair per wrapped row. Char
/// positions are absolute buffer offsets (not relative to `line_start`).
///
/// Rules:
/// - Never splits a grapheme cluster across segments.
/// - A CJK double-width char that would straddle the right edge starts a new
///   segment (the trailing cell of the previous row stays empty — standard
///   terminal behavior).
/// - Tab expansion uses `tab_width - (col % tab_width)` where `col` is the
///   absolute display column within the buffer line (not the segment), so tab
///   stops remain visually consistent across wrapped rows.
/// - An empty line produces one segment `(line_start, line_start)`.
/// - If a single grapheme is wider than `content_width`, it gets its own
///   segment (the renderer will clip).
fn wrap_line(
    buf: &Buffer,
    line_start: usize,
    content_end: usize,
    content_width: usize,
    tab_width: usize,
) -> Vec<(usize, usize)> {
    if line_start == content_end {
        return vec![(line_start, line_start)];
    }

    let slice = buf.slice(line_start..content_end);
    let cow: Cow<str> = slice.into();
    let tab_width = tab_width.max(1);
    let content_width = content_width.max(1);

    let mut segments: Vec<(usize, usize)> = Vec::new();
    let mut seg_start = line_start;
    // Display column within the current segment (resets to 0 on each wrap).
    let mut seg_col: usize = 0;
    // Absolute display column from the buffer line start (for tab-stop math).
    let mut abs_col: usize = 0;
    let mut char_pos = line_start;

    for grapheme in cow.graphemes(true) {
        let advance = grapheme_advance(grapheme, abs_col, tab_width);

        // Would this grapheme exceed the segment width?
        if seg_col + advance > content_width && seg_col > 0 {
            // Finish the current segment before this grapheme.
            segments.push((seg_start, char_pos));
            seg_start = char_pos;
            seg_col = 0;
        }

        seg_col += advance;
        abs_col += advance;
        char_pos += grapheme.chars().count();
    }

    if seg_start <= content_end {
        segments.push((seg_start, char_pos));
    }

    segments
}

/// Which wrapped sub-row of buffer line `line_idx` contains `cursor_char`.
///
/// Returns 0 for the first row, 1 for the first continuation, etc.
fn cursor_sub_row(
    buf: &Buffer,
    line_idx: usize,
    cursor_char: usize,
    content_width: usize,
    tab_width: usize,
) -> usize {
    let (line_start, content_end) = line_content_range(buf, line_idx);
    let segments = wrap_line(buf, line_start, content_end, content_width, tab_width);
    for (i, &(seg_start, seg_end)) in segments.iter().enumerate() {
        // The cursor belongs to this segment if it's within [seg_start, seg_end).
        // For the last segment, the cursor can be at seg_end (the newline pos).
        let is_last = i + 1 == segments.len();
        if cursor_char >= seg_start && (cursor_char < seg_end || (is_last && cursor_char <= seg_end)) {
            return i;
        }
    }
    // Fallback: last sub-row (cursor is at or past end of line content).
    segments.len().saturating_sub(1)
}

/// How many display rows buffer line `line_idx` occupies when soft-wrapped.
fn count_wrapped_rows(
    buf: &Buffer,
    line_idx: usize,
    content_width: usize,
    tab_width: usize,
) -> usize {
    let (line_start, content_end) = line_content_range(buf, line_idx);
    wrap_line(buf, line_start, content_end, content_width, tab_width).len()
}

/// Return the char range of visible content for buffer line `line_idx`,
/// stripping the trailing `\n` (which is implicit in the row advance).
fn line_content_range(buf: &Buffer, line_idx: usize) -> (usize, usize) {
    let start = buf.line_to_char(line_idx);
    let end_excl = line_end_exclusive(buf, line_idx);
    let content_end = if end_excl > start && buf.char_at(end_excl - 1) == Some('\n') {
        end_excl - 1
    } else {
        end_excl
    };
    (start, content_end)
}

impl ViewState {
    /// Width of the content area in display columns (total width minus gutter).
    pub(crate) fn content_width(&self) -> usize {
        self.width.saturating_sub(self.gutter_width)
    }

    /// Produce the display lines that are currently visible in the viewport.
    ///
    /// When `soft_wrap` is `false`, every display line maps 1:1 to a buffer
    /// line. When `true`, long buffer lines are split into multiple display
    /// rows via [`wrap_line`], with continuation rows marked by
    /// `is_continuation: true` and `line_number: None`.
    ///
    /// The returned `Vec` borrows content from `buf` — it cannot outlive the
    /// borrow. Using a `Vec` (rather than a lazy iterator) keeps the call
    /// sites simple and the allocation is tiny (at most `height` elements,
    /// typically 20–50).
    pub(crate) fn display_lines<'buf>(&self, buf: &'buf Buffer) -> Vec<DisplayLine<'buf>> {
        // Ropey's len_lines() counts the phantom empty "line" that follows a
        // trailing '\n'. For a buffer with content "hello\nworld\n" it returns
        // 3, not 2. Since every buffer ends with '\n' by invariant, the real
        // visible line count is always `len_lines() - 1`.
        let total = buf.len_lines().saturating_sub(1);
        let first = self.scroll_offset.min(total.saturating_sub(1));
        let content_width = self.content_width();

        if self.soft_wrap && content_width > 0 {
            return self.display_lines_wrapped(buf, total, first);
        }

        // Non-wrapped path: one display line per buffer line.
        let last = (first + self.height).min(total);
        (first..last)
            .map(|line_idx| {
                let (start, content_end) = line_content_range(buf, line_idx);
                DisplayLine {
                    content: buf.slice(start..content_end),
                    line_number: Some(line_idx + 1),
                    char_offset: Some(start),
                    is_continuation: false,
                }
            })
            .collect()
    }

    /// Wrapped variant of [`display_lines`]: splits long buffer lines into
    /// multiple display rows. `scroll_sub_offset` controls how many wrapped
    /// sub-rows to skip within the first visible buffer line (handles the
    /// edge case where a single line wraps to more rows than the viewport).
    fn display_lines_wrapped<'buf>(
        &self,
        buf: &'buf Buffer,
        total: usize,
        first: usize,
    ) -> Vec<DisplayLine<'buf>> {
        let content_width = self.content_width();
        let mut result = Vec::with_capacity(self.height);
        let mut line_idx = first;

        while result.len() < self.height && line_idx < total {
            let (line_start, content_end) = line_content_range(buf, line_idx);
            let segments = wrap_line(buf, line_start, content_end, content_width, self.tab_width);

            // For the first visible buffer line, skip `scroll_sub_offset`
            // segments so the viewport starts partway through a long line.
            let skip = if line_idx == first { self.scroll_sub_offset } else { 0 };

            for (seg_idx, &(seg_start, seg_end)) in segments.iter().enumerate() {
                if seg_idx < skip {
                    continue;
                }
                if result.len() >= self.height {
                    break;
                }
                result.push(DisplayLine {
                    content: buf.slice(seg_start..seg_end),
                    line_number: if seg_idx == 0 { Some(line_idx + 1) } else { None },
                    char_offset: Some(seg_start),
                    is_continuation: seg_idx > 0,
                });
            }
            line_idx += 1;
        }

        result
    }

    /// Adjust `scroll_offset` (and `scroll_sub_offset` when soft-wrapping)
    /// so the primary cursor stays visible in the viewport.
    ///
    /// Dispatches to a simple buffer-line check when wrapping is off, or to
    /// a display-row-counting algorithm when wrapping is on.
    pub(crate) fn ensure_cursor_visible(&mut self, buf: &Buffer, cursor_char: usize) {
        if self.soft_wrap {
            self.ensure_cursor_visible_wrapped(buf, cursor_char);
        } else {
            let cursor_line = buf.char_to_line(cursor_char);
            self.ensure_cursor_visible_unwrapped(cursor_line);
        }
    }

    /// Non-wrapped scroll adjustment: simple buffer-line arithmetic with margin.
    fn ensure_cursor_visible_unwrapped(&mut self, cursor_line: usize) {
        let margin = SCROLL_MARGIN.min(self.height / 2);

        if cursor_line < self.scroll_offset + margin {
            self.scroll_offset = cursor_line.saturating_sub(margin);
        } else if self.height > 0 && cursor_line >= self.scroll_offset + self.height - margin {
            self.scroll_offset = cursor_line.saturating_sub(self.height - margin - 1);
        }
    }

    /// Wrapped scroll adjustment: accounts for buffer lines spanning multiple
    /// display rows. Counts rows from the scroll position to the cursor's
    /// sub-row and adjusts so the cursor sits within the viewport with margin.
    fn ensure_cursor_visible_wrapped(&mut self, buf: &Buffer, cursor_char: usize) {
        let cursor_line = buf.char_to_line(cursor_char);
        let content_width = self.content_width();
        if content_width == 0 || self.height == 0 {
            return;
        }

        let margin = SCROLL_MARGIN.min(self.height / 2);
        let cursor_sub = cursor_sub_row(buf, cursor_line, cursor_char, content_width, self.tab_width);

        // ── Cursor above the viewport ────────────────────────────────────────
        if cursor_line < self.scroll_offset
            || (cursor_line == self.scroll_offset && cursor_sub < self.scroll_sub_offset)
        {
            // Place cursor at `margin` rows from the top. Walk backward from
            // the cursor's sub-row to find the right scroll position.
            self.scroll_offset = cursor_line;
            self.scroll_sub_offset = cursor_sub;
            // Try to give `margin` rows above the cursor.
            let mut rows_above = 0;
            while rows_above < margin {
                if self.scroll_sub_offset > 0 {
                    self.scroll_sub_offset -= 1;
                    rows_above += 1;
                } else if self.scroll_offset > 0 {
                    self.scroll_offset -= 1;
                    let rows = count_wrapped_rows(buf, self.scroll_offset, content_width, self.tab_width);
                    if rows_above + rows > margin {
                        // Don't overshoot — start partway through this line.
                        self.scroll_sub_offset = rows - (margin - rows_above);
                        break;
                    }
                    rows_above += rows;
                } else {
                    break; // top of file
                }
            }
            return;
        }

        // ── Count display rows from scroll position to cursor ────────────────
        let mut display_row: usize = 0;
        for line_idx in self.scroll_offset..=cursor_line {
            let rows = count_wrapped_rows(buf, line_idx, content_width, self.tab_width);
            let skip = if line_idx == self.scroll_offset { self.scroll_sub_offset } else { 0 };
            if line_idx == cursor_line {
                // cursor_sub < skip would mean the cursor is in a row that
                // the viewport has scrolled past — the above-viewport guard
                // should prevent this.
                debug_assert!(cursor_sub >= skip, "cursor scrolled past by scroll_sub_offset");
                display_row += cursor_sub.saturating_sub(skip);
                break;
            }
            display_row += rows.saturating_sub(skip);
        }

        // ── Cursor below the viewport ────────────────────────────────────────
        if display_row >= self.height.saturating_sub(margin) {
            // Scroll down until cursor is at `height - margin - 1`.
            let target_row = self.height.saturating_sub(margin).saturating_sub(1);
            let overshoot = display_row - target_row;
            let mut remaining = overshoot;
            while remaining > 0 {
                let rows = count_wrapped_rows(buf, self.scroll_offset, content_width, self.tab_width);
                // scroll_sub_offset is always < rows (reset to 0 on every line
                // advance), so this subtraction never wraps. saturating_sub
                // guards against any future invariant violation.
                let available = rows.saturating_sub(self.scroll_sub_offset);
                if remaining < available {
                    self.scroll_sub_offset += remaining;
                    remaining = 0;
                } else {
                    remaining -= available;
                    self.scroll_offset += 1;
                    self.scroll_sub_offset = 0;
                }
            }
        }
    }

    /// Adjust `col_offset` so the primary cursor's display column stays visible.
    ///
    /// Mirrors [`ensure_cursor_visible`] for the horizontal axis. The cursor's
    /// display column (in terminal cells) is kept at least [`SCROLL_MARGIN_H`]
    /// columns from the left and right edges of the content area.
    ///
    /// When soft wrap is active, horizontal scrolling is disabled — wrapping
    /// handles long lines, so `col_offset` is forced to 0.
    pub(crate) fn ensure_cursor_visible_horizontal(&mut self, buf: &Buffer, sels: &SelectionSet, cursor_line: usize) {
        if self.soft_wrap {
            self.col_offset = 0;
            return;
        }

        let head = sels.primary().head;
        let cursor_col = display_col_in_line(buf, cursor_line, head, self.tab_width);
        let content_width = self.width.saturating_sub(self.gutter_width);
        if content_width == 0 {
            return;
        }

        let margin = SCROLL_MARGIN_H.min(content_width / 2);

        if cursor_col < self.col_offset + margin {
            // Cursor is near (or past) the left edge — scroll left.
            self.col_offset = cursor_col.saturating_sub(margin);
        } else if cursor_col >= self.col_offset + content_width - margin {
            // Cursor is near (or past) the right edge — scroll right.
            self.col_offset = cursor_col.saturating_sub(content_width - margin - 1);
        }
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::buffer::Buffer;
    use crate::core::selection::{Selection, SelectionSet};

    fn view(scroll_offset: usize, height: usize, buf: &Buffer) -> ViewState {
        ViewState {
            scroll_offset,
            height,
            width: 80,
            gutter_width: compute_gutter_width(buf.len_lines()),
            line_number_style: LineNumberStyle::Absolute,
            col_offset: 0,
            tab_width: 4,
            whitespace: WhitespaceConfig::default(),
            soft_wrap: false,
            scroll_sub_offset: 0,
        }
    }

    // ── compute_gutter_width ──────────────────────────────────────────────────

    #[test]
    fn gutter_width_minimum_is_4() {
        assert_eq!(compute_gutter_width(0), 4);
        assert_eq!(compute_gutter_width(1), 4);
        assert_eq!(compute_gutter_width(9), 4);  // " 9 " = 3, but min is 4
    }

    #[test]
    fn gutter_width_two_digit_lines() {
        assert_eq!(compute_gutter_width(10), 4);  // " 10 " = 4
        assert_eq!(compute_gutter_width(99), 4);  // " 99 " = 4
    }

    #[test]
    fn gutter_width_three_digit_lines() {
        assert_eq!(compute_gutter_width(100), 5); // " 100 " = 5
        assert_eq!(compute_gutter_width(999), 5);
    }

    #[test]
    fn gutter_width_four_digit_lines() {
        assert_eq!(compute_gutter_width(1000), 6); // " 1000 " = 6
        assert_eq!(compute_gutter_width(9999), 6);
    }

    // ── display_lines ─────────────────────────────────────────────────────────

    #[test]
    fn display_lines_simple_file() {
        let buf = Buffer::from("hello\nworld\n");
        let v = view(0, 10, &buf);
        let lines = v.display_lines(&buf);

        // Two real lines in the buffer.
        assert_eq!(lines.len(), 2);

        assert_eq!(lines[0].content.to_string(), "hello");
        assert_eq!(lines[0].line_number, Some(1));
        assert_eq!(lines[0].char_offset, Some(0));

        assert_eq!(lines[1].content.to_string(), "world");
        assert_eq!(lines[1].line_number, Some(2));
        assert_eq!(lines[1].char_offset, Some(6));
    }

    #[test]
    fn display_lines_strips_trailing_newline() {
        let buf = Buffer::from("abc\n");
        let v = view(0, 10, &buf);
        let lines = v.display_lines(&buf);
        assert_eq!(lines.len(), 1);
        // '\n' must not appear in displayed content.
        assert_eq!(lines[0].content.to_string(), "abc");
    }

    #[test]
    fn display_lines_empty_buffer() {
        // An empty buffer contains only the structural '\n'.
        let buf = Buffer::empty();
        let v = view(0, 10, &buf);
        let lines = v.display_lines(&buf);
        // One display line for the structural newline, content is empty string.
        assert_eq!(lines.len(), 1);
        assert_eq!(lines[0].content.to_string(), "");
        assert_eq!(lines[0].line_number, Some(1));
    }

    #[test]
    fn display_lines_viewport_clips_to_height() {
        let buf = Buffer::from("a\nb\nc\nd\ne\n");
        let v = view(0, 3, &buf);
        let lines = v.display_lines(&buf);
        // Only the first 3 lines visible.
        assert_eq!(lines.len(), 3);
        assert_eq!(lines[0].content.to_string(), "a");
        assert_eq!(lines[2].content.to_string(), "c");
    }

    #[test]
    fn display_lines_scrolled() {
        let buf = Buffer::from("a\nb\nc\nd\ne\n");
        let v = view(2, 3, &buf);
        let lines = v.display_lines(&buf);
        // Lines 2..5 (0-based): "c", "d", "e".
        assert_eq!(lines.len(), 3);
        assert_eq!(lines[0].content.to_string(), "c");
        assert_eq!(lines[0].line_number, Some(3)); // 1-based
        assert_eq!(lines[2].content.to_string(), "e");
    }

    #[test]
    fn display_lines_partial_last_page() {
        // Scroll past midpoint — fewer lines than height.
        let buf = Buffer::from("a\nb\nc\n");
        let v = view(2, 10, &buf);
        let lines = v.display_lines(&buf);
        // Only line index 2 ("c") is visible.
        assert_eq!(lines.len(), 1);
        assert_eq!(lines[0].content.to_string(), "c");
    }

    #[test]
    fn display_lines_line_numbers_are_one_based() {
        let buf = Buffer::from("x\ny\nz\n");
        let v = view(0, 10, &buf);
        let lines = v.display_lines(&buf);
        assert_eq!(lines[0].line_number, Some(1));
        assert_eq!(lines[1].line_number, Some(2));
        assert_eq!(lines[2].line_number, Some(3));
    }

    // ── ensure_cursor_visible ─────────────────────────────────────────────────

    #[test]
    fn cursor_visible_no_scroll_needed() {
        let buf = Buffer::from("a\nb\nc\nd\ne\n");
        let mut v = view(0, 10, &buf);
        // Cursor on line 2 (char offset = 4: "a\nb\n" = 4 chars).
        v.ensure_cursor_visible(&buf, buf.line_to_char(2));
        assert_eq!(v.scroll_offset, 0); // cursor is well within viewport
    }

    #[test]
    fn cursor_below_viewport_scrolls_down() {
        let buf = Buffer::from("a\nb\nc\nd\ne\nf\ng\nh\n");
        // Viewport shows lines 0..5, cursor is on line 7 (below).
        let mut v = view(0, 5, &buf);
        v.ensure_cursor_visible(&buf, buf.line_to_char(7));
        // After scroll the cursor should be within viewport with margin.
        assert!(7 >= v.scroll_offset);
        assert!(7 < v.scroll_offset + v.height);
    }

    #[test]
    fn cursor_above_viewport_scrolls_up() {
        let buf = Buffer::from("a\nb\nc\nd\ne\nf\ng\nh\n");
        // Viewport starts at line 5, cursor is on line 1 (above).
        let mut v = view(5, 5, &buf);
        v.ensure_cursor_visible(&buf, buf.line_to_char(1));
        assert!(1 >= v.scroll_offset);
        assert!(1 < v.scroll_offset + v.height);
    }

    #[test]
    fn cursor_at_top_of_buffer_scroll_offset_is_zero() {
        let buf = Buffer::from("a\nb\nc\n");
        let mut v = view(2, 5, &buf); // scrolled down
        v.ensure_cursor_visible(&buf, 0);
        assert_eq!(v.scroll_offset, 0);
    }

    // ── ensure_cursor_visible_horizontal ──────────────────────────────────────

    /// Build a ViewState with explicit width and gutter for horizontal tests.
    fn hview(width: usize, gutter_width: usize) -> ViewState {
        ViewState {
            scroll_offset: 0,
            height: 10,
            width,
            gutter_width,
            line_number_style: LineNumberStyle::Absolute,
            col_offset: 0,
            tab_width: 4,
            whitespace: WhitespaceConfig::default(),
            soft_wrap: false,
            scroll_sub_offset: 0,
        }
    }

    /// Place cursor at a specific char position on line 0.
    fn cursor_at_char(pos: usize) -> SelectionSet {
        SelectionSet::single(Selection::cursor(pos))
    }

    #[test]
    fn horizontal_no_scroll_needed() {
        // 20-char line, width 80, gutter 4 → content_width 76.
        // Cursor at col 10 — well within viewport.
        let buf = Buffer::from("abcdefghijklmnopqrst\n");
        let mut v = hview(80, 4);
        let sels = cursor_at_char(10);
        v.ensure_cursor_visible_horizontal(&buf, &sels, 0);
        assert_eq!(v.col_offset, 0);
    }

    #[test]
    fn horizontal_scroll_right() {
        // Content width = 20 - 4 = 16, margin = 5.
        // Cursor at char 18 → display col 18.
        // 18 >= 0 + 16 - 5 = 11, so scroll right.
        // col_offset = 18 - (16 - 5 - 1) = 18 - 10 = 8.
        let buf = Buffer::from("abcdefghijklmnopqrstuvwxyz\n");
        let mut v = hview(20, 4);
        let sels = cursor_at_char(18);
        v.ensure_cursor_visible_horizontal(&buf, &sels, 0);
        assert_eq!(v.col_offset, 8);
    }

    #[test]
    fn horizontal_scroll_left() {
        // Content width = 20 - 4 = 16, margin = 5.
        // Start scrolled right (col_offset = 10), cursor at char 12 → col 12.
        // 12 < 10 + 5 = 15, so scroll left.
        // col_offset = 12 - 5 = 7.
        let buf = Buffer::from("abcdefghijklmnopqrstuvwxyz\n");
        let mut v = hview(20, 4);
        v.col_offset = 10;
        let sels = cursor_at_char(12);
        v.ensure_cursor_visible_horizontal(&buf, &sels, 0);
        assert_eq!(v.col_offset, 7);
    }

    #[test]
    fn horizontal_scroll_resets_near_start() {
        // Cursor at char 2 → col 2. col_offset was 5.
        // 2 < 5 + 5 = 10, so scroll left.
        // col_offset = 2 - 5 = saturating_sub → 0.
        let buf = Buffer::from("abcdefghijklmnopqrstuvwxyz\n");
        let mut v = hview(20, 4);
        v.col_offset = 5;
        let sels = cursor_at_char(2);
        v.ensure_cursor_visible_horizontal(&buf, &sels, 0);
        assert_eq!(v.col_offset, 0);
    }

    #[test]
    fn horizontal_scroll_with_cjk() {
        // "世界世界世界世界世界" = 10 CJK chars, 20 display columns.
        // Content width = 20 - 4 = 16, margin = 5.
        // Cursor at char 8 → display col = 8 * 2 = 16.
        // 16 >= 0 + 16 - 5 = 11, so scroll right.
        // col_offset = 16 - (16 - 5 - 1) = 16 - 10 = 6.
        let buf = Buffer::from("世界世界世界世界世界\n");
        let mut v = hview(20, 4);
        let sels = cursor_at_char(8);
        v.ensure_cursor_visible_horizontal(&buf, &sels, 0);
        assert_eq!(v.col_offset, 6);
    }

    // ── display_lines (soft wrap) ───────────────────────────────────────────

    /// Build a ViewState with soft wrap enabled.
    fn wrap_view(scroll_offset: usize, height: usize, width: usize, buf: &Buffer) -> ViewState {
        ViewState {
            scroll_offset,
            height,
            width,
            gutter_width: compute_gutter_width(buf.len_lines()),
            line_number_style: LineNumberStyle::Absolute,
            col_offset: 0,
            tab_width: 4,
            whitespace: WhitespaceConfig::default(),
            soft_wrap: true,
            scroll_sub_offset: 0,
        }
    }

    #[test]
    fn display_lines_wrap_short_lines_unchanged() {
        let buf = Buffer::from("hi\nbye\n");
        let v = wrap_view(0, 10, 80, &buf);
        let lines = v.display_lines(&buf);
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0].content.to_string(), "hi");
        assert!(!lines[0].is_continuation);
        assert_eq!(lines[1].content.to_string(), "bye");
        assert!(!lines[1].is_continuation);
    }

    #[test]
    fn display_lines_wrap_splits_long_line() {
        // "abcdefgh" (8 chars), width 8, gutter 4 → content_width 4.
        // Segments: "abcd" (0..4), "efgh" (4..8).
        let buf = Buffer::from("abcdefgh\n");
        let v = wrap_view(0, 10, 8, &buf);
        let lines = v.display_lines(&buf);
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0].content.to_string(), "abcd");
        assert_eq!(lines[0].line_number, Some(1));
        assert!(!lines[0].is_continuation);
        assert_eq!(lines[1].content.to_string(), "efgh");
        assert_eq!(lines[1].line_number, None);
        assert!(lines[1].is_continuation);
    }

    #[test]
    fn display_lines_wrap_clips_to_height() {
        // Long line wraps to 4 rows but viewport is only 2.
        let buf = Buffer::from("abcdefghijklmnop\n");
        let gw = compute_gutter_width(buf.len_lines());
        let content_width = 4;
        let v = wrap_view(0, 2, gw + content_width, &buf);
        let lines = v.display_lines(&buf);
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0].content.to_string(), "abcd");
        assert_eq!(lines[1].content.to_string(), "efgh");
    }

    #[test]
    fn display_lines_wrap_scroll_sub_offset() {
        // "abcdefghijklmnop" wraps to 4 rows at width 4.
        // scroll_sub_offset=2 skips first 2 sub-rows.
        let buf = Buffer::from("abcdefghijklmnop\n");
        let gw = compute_gutter_width(buf.len_lines());
        let mut v = wrap_view(0, 10, gw + 4, &buf);
        v.scroll_sub_offset = 2;
        let lines = v.display_lines(&buf);
        assert_eq!(lines.len(), 2);
        // Third segment: "ijkl", fourth: "mnop".
        assert_eq!(lines[0].content.to_string(), "ijkl");
        assert!(lines[0].is_continuation);
        assert_eq!(lines[1].content.to_string(), "mnop");
        assert!(lines[1].is_continuation);
    }

    #[test]
    fn display_lines_wrap_mixed_lines() {
        // Short line "ab" + long line "cdefghij" (wraps to 2 rows at content_width 4).
        let buf = Buffer::from("ab\ncdefghij\n");
        let gw = compute_gutter_width(buf.len_lines());
        let v = wrap_view(0, 10, gw + 4, &buf);
        let lines = v.display_lines(&buf);
        assert_eq!(lines.len(), 3);
        assert_eq!(lines[0].content.to_string(), "ab");
        assert_eq!(lines[0].line_number, Some(1));
        assert_eq!(lines[1].content.to_string(), "cdef");
        assert_eq!(lines[1].line_number, Some(2));
        assert!(!lines[1].is_continuation);
        assert_eq!(lines[2].content.to_string(), "ghij");
        assert_eq!(lines[2].line_number, None);
        assert!(lines[2].is_continuation);
    }

    // ── wrap_line ────────────────────────────────────────────────────────────

    #[test]
    fn wrap_line_short_line_no_wrap() {
        let buf = Buffer::from("hello\n");
        // "hello" = 5 display cols, content_width = 10 → fits in one segment.
        let segs = wrap_line(&buf, 0, 5, 10, 4);
        assert_eq!(segs, vec![(0, 5)]);
    }

    #[test]
    fn wrap_line_exact_fit_no_wrap() {
        let buf = Buffer::from("abcde\n");
        // Exactly 5 chars in width 5 → one segment, no wrap.
        let segs = wrap_line(&buf, 0, 5, 5, 4);
        assert_eq!(segs, vec![(0, 5)]);
    }

    #[test]
    fn wrap_line_one_char_overflow() {
        let buf = Buffer::from("abcdef\n");
        // 6 chars in width 5 → wraps: "abcde" + "f".
        let segs = wrap_line(&buf, 0, 6, 5, 4);
        assert_eq!(segs, vec![(0, 5), (5, 6)]);
    }

    #[test]
    fn wrap_line_multiple_wraps() {
        let buf = Buffer::from("abcdefghijklmno\n");
        // 15 chars in width 5 → 3 segments of 5 each.
        let segs = wrap_line(&buf, 0, 15, 5, 4);
        assert_eq!(segs, vec![(0, 5), (5, 10), (10, 15)]);
    }

    #[test]
    fn wrap_line_empty_line() {
        let buf = Buffer::from("\n");
        let segs = wrap_line(&buf, 0, 0, 10, 4);
        assert_eq!(segs, vec![(0, 0)]);
    }

    #[test]
    fn wrap_line_cjk_at_boundary() {
        // "abcd世" = 4 + 2 = 6 display cols. Width 5.
        // "abcd" fits (4 cols). "世" needs 2 cols, 4+2=6 > 5 → wrap before it.
        let buf = Buffer::from("abcd世\n");
        let segs = wrap_line(&buf, 0, 5, 5, 4);
        assert_eq!(segs, vec![(0, 4), (4, 5)]);
    }

    #[test]
    fn wrap_line_cjk_fits() {
        // "ab世" = 2 + 2 = 4 display cols. Width 4 → fits.
        let buf = Buffer::from("ab世\n");
        let segs = wrap_line(&buf, 0, 3, 4, 4);
        assert_eq!(segs, vec![(0, 3)]);
    }

    #[test]
    fn wrap_line_cjk_sequence() {
        // "世界世界世" = 5 CJK chars = 10 display cols. Width 4.
        // Row 1: "世界" (4 cols), Row 2: "世界" (4 cols), Row 3: "世" (2 cols).
        let buf = Buffer::from("世界世界世\n");
        let segs = wrap_line(&buf, 0, 5, 4, 4);
        assert_eq!(segs, vec![(0, 2), (2, 4), (4, 5)]);
    }

    #[test]
    fn wrap_line_tab_expansion() {
        // "\tabc" with tab_width=4: tab at col 0 → 4 cols, then "abc" → 3 cols = 7 total.
        // Width 5: tab takes 4 cols, then 'a' at col 4 → 5 cols ≤ 5, OK.
        // 'b' at col 5 → 6 > 5 → wrap. Second segment: "bc" = 2 cols.
        let buf = Buffer::from("\tabc\n");
        let segs = wrap_line(&buf, 0, 4, 5, 4);
        assert_eq!(segs, vec![(0, 2), (2, 4)]);
    }

    #[test]
    fn wrap_line_tab_at_boundary() {
        // "ab\t" with tab_width=4: 'a'=1, 'b'=2, tab at col 2 → 4-2=2 cols → col 4.
        // Width 4 → fits. Total = 4.
        let buf = Buffer::from("ab\t\n");
        let segs = wrap_line(&buf, 0, 3, 4, 4);
        assert_eq!(segs, vec![(0, 3)]);
    }

    #[test]
    fn wrap_line_tab_exceeds_boundary() {
        // "abc\t" with tab_width=4: 'a'=1,'b'=2,'c'=3, tab at col 3 → 4-3=1 col → col 4.
        // Width 3: 'a','b','c' fill 3 cols, tab at col 3 → advance 1 → 4 > 3 → wrap.
        // Second segment: tab at abs_col 3 → advance 1 → fits in width 3.
        let buf = Buffer::from("abc\t\n");
        let segs = wrap_line(&buf, 0, 4, 3, 4);
        assert_eq!(segs, vec![(0, 3), (3, 4)]);
    }

    #[test]
    fn wrap_line_single_wide_char_wider_than_content() {
        // Pathological: CJK char (2 cols) in content_width=1.
        // It gets its own segment even though it exceeds width.
        let buf = Buffer::from("世\n");
        let segs = wrap_line(&buf, 0, 1, 1, 4);
        assert_eq!(segs, vec![(0, 1)]);
    }

    // ── cursor_sub_row ───────────────────────────────────────────────────────

    #[test]
    fn cursor_sub_row_no_wrap() {
        let buf = Buffer::from("hello\n");
        assert_eq!(cursor_sub_row(&buf, 0, 0, 80, 4), 0);
        assert_eq!(cursor_sub_row(&buf, 0, 4, 80, 4), 0);
    }

    #[test]
    fn cursor_sub_row_wrapped() {
        let buf = Buffer::from("abcdefghij\n");
        // Width 5 → segs: (0,5), (5,10).
        assert_eq!(cursor_sub_row(&buf, 0, 0, 5, 4), 0); // 'a'
        assert_eq!(cursor_sub_row(&buf, 0, 4, 5, 4), 0); // 'e'
        assert_eq!(cursor_sub_row(&buf, 0, 5, 5, 4), 1); // 'f' (first of second row)
        assert_eq!(cursor_sub_row(&buf, 0, 9, 5, 4), 1); // 'j'
    }

    // ── count_wrapped_rows ───────────────────────────────────────────────────

    #[test]
    fn count_wrapped_rows_short_line() {
        let buf = Buffer::from("hello\n");
        assert_eq!(count_wrapped_rows(&buf, 0, 80, 4), 1);
    }

    #[test]
    fn count_wrapped_rows_wrapped() {
        let buf = Buffer::from("abcdefghijklmno\n");
        // 15 chars, width 5 → 3 rows.
        assert_eq!(count_wrapped_rows(&buf, 0, 5, 4), 3);
    }

    // ── ensure_cursor_visible_wrapped ─────────────────────────────────────────

    /// Build a wrap_view with an explicit scroll_sub_offset.
    fn wrap_view_sub(
        scroll_offset: usize,
        scroll_sub_offset: usize,
        height: usize,
        width: usize,
        buf: &Buffer,
    ) -> ViewState {
        ViewState { scroll_sub_offset, ..wrap_view(scroll_offset, height, width, buf) }
    }

    #[test]
    fn wrapped_cursor_visible_no_scroll_needed() {
        // Cursor on line 0, already at top — no scroll.
        let buf = Buffer::from("abcdefgh\n");
        // content_width = 8 - 4 = 4, line wraps to 2 rows.
        let mut v = wrap_view(0, 10, 8, &buf);
        v.ensure_cursor_visible(&buf, 0);
        assert_eq!(v.scroll_offset, 0);
        assert_eq!(v.scroll_sub_offset, 0);
    }

    #[test]
    fn wrapped_cursor_below_viewport_scrolls_down() {
        // 4 lines × 2 rows each = 8 display rows. Viewport height 4 → can
        // only show 2 lines. Cursor on line 3 (last line) should scroll.
        // Each line is "abcdefgh" (8 chars), content_width = 8 - 4 = 4 → 2 rows/line.
        let buf = Buffer::from("abcdefgh\nabcdefgh\nabcdefgh\nabcdefgh\n");
        let mut v = wrap_view(0, 4, 8, &buf);
        let cursor_char = buf.line_to_char(3); // start of line 3
        v.ensure_cursor_visible(&buf, cursor_char);
        // Cursor (line 3, sub-row 0) should be near the bottom with margin.
        // margin = min(SCROLL_MARGIN=3, height/2=2) = 2.
        // Target row = height - margin - 1 = 4 - 2 - 1 = 1.
        // Cursor display row from new scroll position should be ≤ 1.
        assert!(v.scroll_offset > 0 || v.scroll_sub_offset > 0, "should have scrolled");
        // After scrolling, cursor must be in view.
        let cursor_line = buf.char_to_line(cursor_char);
        let content_width = v.content_width();
        let cursor_sub = cursor_sub_row(&buf, cursor_line, cursor_char, content_width, v.tab_width);
        let mut display_row = 0usize;
        for line_idx in v.scroll_offset..=cursor_line {
            let rows = count_wrapped_rows(&buf, line_idx, content_width, v.tab_width);
            let skip = if line_idx == v.scroll_offset { v.scroll_sub_offset } else { 0 };
            if line_idx == cursor_line {
                display_row += cursor_sub.saturating_sub(skip);
                break;
            }
            display_row += rows.saturating_sub(skip);
        }
        assert!(display_row < v.height, "cursor should be within viewport after scroll");
    }

    #[test]
    fn wrapped_cursor_above_viewport_scrolls_up() {
        // Start scrolled to line 3, cursor moves to line 0.
        let buf = Buffer::from("abcdefgh\nabcdefgh\nabcdefgh\nabcdefgh\n");
        // content_width = 8 - 4 = 4 → each line wraps to 2 rows.
        let mut v = wrap_view(3, 4, 8, &buf);
        v.ensure_cursor_visible(&buf, 0); // cursor at char 0, line 0
        assert_eq!(v.scroll_offset, 0);
        assert_eq!(v.scroll_sub_offset, 0);
    }

    #[test]
    fn wrapped_cursor_on_continuation_row_scrolls_into_view() {
        // Cursor is on the second wrapped segment of line 0 (sub-row 1).
        // Viewport shows only 1 row. Cursor should cause a scroll so sub-row 1
        // becomes the top of the viewport.
        // "abcdefgh" with content_width 4 → 2 rows: "abcd" / "efgh".
        let buf = Buffer::from("abcdefgh\n");
        let mut v = wrap_view(0, 1, 8, &buf);
        let cursor_char = 4; // start of "efgh" segment, sub-row 1
        v.ensure_cursor_visible(&buf, cursor_char);
        assert_eq!(v.scroll_offset, 0);
        assert_eq!(v.scroll_sub_offset, 1, "should scroll within the line to show sub-row 1");
    }

    #[test]
    fn wrapped_single_line_taller_than_viewport() {
        // "abcdefghijklmnop" (16 chars), content_width 4 → 4 rows.
        // Viewport height 2. Cursor at char 12 (sub-row 3).
        let buf = Buffer::from("abcdefghijklmnop\n");
        let mut v = wrap_view(0, 2, 8, &buf);
        let cursor_char = 12;
        v.ensure_cursor_visible(&buf, cursor_char);
        // All scrolling is within line 0 via scroll_sub_offset.
        assert_eq!(v.scroll_offset, 0);
        assert!(v.scroll_sub_offset > 0, "sub-offset should advance within the long line");
        // Cursor must be in view.
        let content_width = v.content_width();
        let cursor_line = buf.char_to_line(cursor_char);
        let cursor_sub = cursor_sub_row(&buf, cursor_line, cursor_char, content_width, v.tab_width);
        assert!(cursor_sub >= v.scroll_sub_offset);
        assert!(cursor_sub - v.scroll_sub_offset < v.height);
    }

    #[test]
    fn wrapped_cursor_at_margin_boundary_no_scroll() {
        // Cursor already within margin — no scroll should happen.
        // 5 short lines (no wrap). Height 10, margin = min(3, 5) = 3.
        let buf = Buffer::from("a\nb\nc\nd\ne\n");
        let mut v = wrap_view(0, 10, 80, &buf);
        let cursor_char = buf.line_to_char(2); // line 2, well within viewport
        v.ensure_cursor_visible(&buf, cursor_char);
        assert_eq!(v.scroll_offset, 0);
        assert_eq!(v.scroll_sub_offset, 0);
    }
}
