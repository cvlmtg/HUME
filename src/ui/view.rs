use crate::core::buffer::Buffer;
use crate::core::grapheme::display_col_in_line;
use crate::ui::formatter::{count_visual_rows, cursor_sub_row};
use crate::ui::gutter::GutterConfig;
use crate::ui::whitespace::WhitespaceConfig;
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

    /// Gutter column configuration.
    ///
    /// Describes which columns appear in the gutter (left of the content area)
    /// and in what order. The total gutter width is derived from this via
    /// [`gutter_width`](Self::gutter_width).
    pub gutter: GutterConfig,

    /// Cached real buffer line count: `buf.len_lines().saturating_sub(1)`.
    ///
    /// Updated at the top of every event-loop iteration (alongside the old
    /// `gutter_width` field it replaces). Used by `gutter_width()` and
    /// `content_width()` so those methods stay parameter-free.
    pub cached_total_lines: usize,

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


impl ViewState {
    /// Total gutter width in display columns (sum of column widths + separator).
    pub(crate) fn gutter_width(&self) -> usize {
        self.gutter.total_width(self.cached_total_lines)
    }

    /// Width of the content area in display columns (total width minus gutter).
    pub(crate) fn content_width(&self) -> usize {
        self.width.saturating_sub(self.gutter_width())
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
                    let rows = count_visual_rows(buf, self.scroll_offset, content_width, self.tab_width);
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
        // Capped at `height` rows: if the cursor is far below the viewport we
        // don't need the exact count — just enough to know it exceeds the
        // bottom margin. This keeps the scan O(height) instead of O(N).
        let mut display_row: usize = 0;
        for line_idx in self.scroll_offset..=cursor_line {
            let rows = count_visual_rows(buf, line_idx, content_width, self.tab_width);
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
            // Early exit: cursor is definitely below the viewport. Recomputing
            // from the cursor end (below) is O(height), not O(distance).
            if display_row >= self.height {
                break;
            }
        }

        // ── Cursor below the viewport ────────────────────────────────────────
        if display_row >= self.height.saturating_sub(margin) {
            // Walk backward from the cursor `target_row` rows to find the new
            // scroll position. Symmetric to the above-viewport path, and always
            // O(height) regardless of how far the cursor jumped (e.g. `ge`).
            let target_row = self.height.saturating_sub(margin).saturating_sub(1);
            self.scroll_offset = cursor_line;
            self.scroll_sub_offset = cursor_sub;
            let mut rows_above = 0;
            while rows_above < target_row {
                if self.scroll_sub_offset > 0 {
                    self.scroll_sub_offset -= 1;
                    rows_above += 1;
                } else if self.scroll_offset > 0 {
                    self.scroll_offset -= 1;
                    let rows = count_visual_rows(buf, self.scroll_offset, content_width, self.tab_width);
                    if rows_above + rows > target_row {
                        // Don't overshoot — start partway through this line.
                        self.scroll_sub_offset = rows - (target_row - rows_above);
                        break;
                    }
                    rows_above += rows;
                } else {
                    break; // top of file
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
        let content_width = self.content_width();
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
    use crate::ui::gutter::GutterConfig;

    fn view(scroll_offset: usize, height: usize, buf: &Buffer) -> ViewState {
        let cached_total_lines = buf.len_lines().saturating_sub(1);
        ViewState {
            scroll_offset,
            height,
            width: 80,
            gutter: GutterConfig::default(),
            cached_total_lines,
            line_number_style: LineNumberStyle::Absolute,
            col_offset: 0,
            tab_width: 4,
            whitespace: WhitespaceConfig::default(),
            soft_wrap: false,
            scroll_sub_offset: 0,
        }
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

    /// Build a ViewState with an explicit total width for horizontal scroll tests.
    ///
    /// `desired_gutter_width` is the gutter width the test expects. We derive
    /// `cached_total_lines` so that `GutterConfig::default().total_width(n)`
    /// equals `desired_gutter_width`.
    fn hview(width: usize, desired_gutter_width: usize) -> ViewState {
        // total_width(n) = col.width(n) + 1 separator.
        // col.width(n) = max(1 + digits(n), 3).
        // Solve for n such that total_width(n) == desired:
        //   col.width(n) = desired - 1
        //   => 1 + digits(n) = desired - 1  (for desired >= 4)
        //   => digits(n) = desired - 2
        // For desired=4: digits=2 → n=10 works (10 has 2 digits).
        // For desired=5: digits=3 → n=100. Etc.
        // For desired<4: use 0 (total_width returns 0 if desired=0, else min is 4).
        let gutter = GutterConfig::default();
        let cached_total_lines = (0usize..=99_999)
            .find(|&n| gutter.total_width(n) == desired_gutter_width)
            .unwrap_or(1);
        ViewState {
            scroll_offset: 0,
            height: 10,
            width,
            gutter,
            cached_total_lines,
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

    // ── wrap_view helper ─────────────────────────────────────────────────────

    /// Build a ViewState with soft wrap enabled.
    fn wrap_view(scroll_offset: usize, height: usize, width: usize, buf: &Buffer) -> ViewState {
        let cached_total_lines = buf.len_lines().saturating_sub(1);
        ViewState {
            scroll_offset,
            height,
            width,
            gutter: GutterConfig::default(),
            cached_total_lines,
            line_number_style: LineNumberStyle::Absolute,
            col_offset: 0,
            tab_width: 4,
            whitespace: WhitespaceConfig::default(),
            soft_wrap: true,
            scroll_sub_offset: 0,
        }
    }

    // ── cursor_sub_row (formatter) ───────────────────────────────────────────

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

    // ── count_visual_rows ───────────────────────────────────────────────────

    #[test]
    fn count_visual_rows_short_line() {
        let buf = Buffer::from("hello\n");
        assert_eq!(count_visual_rows(&buf, 0, 80, 4), 1);
    }

    #[test]
    fn count_visual_rows_wrapped() {
        let buf = Buffer::from("abcdefghijklmno\n");
        // 15 chars, width 5 → 3 rows.
        assert_eq!(count_visual_rows(&buf, 0, 5, 4), 3);
    }

    // ── ensure_cursor_visible_wrapped ─────────────────────────────────────────

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
        // margin = min(SCROLL_MARGIN=3, height/2=2) = 2
        // target_row = 4 - 2 - 1 = 1
        // Backward walk from (line 3, sub 0): step into line 2 (2 rows),
        // 0 + 2 > 1, so scroll_sub_offset = 2 - (1 - 0) = 1.
        assert_eq!(v.scroll_offset, 2);
        assert_eq!(v.scroll_sub_offset, 1);
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
