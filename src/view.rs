use crate::buffer::Buffer;
use crate::display_line::{DisplayLine, DisplayLineSource};
use crate::helpers::line_end_exclusive;
use crate::selection::SelectionSet;

/// How many lines to keep between the cursor and the top/bottom edge of the
/// viewport before scrolling. 3 lines gives a comfortable look-ahead without
/// being overly aggressive.
const SCROLL_MARGIN: usize = 3;

/// How line numbers are displayed in the gutter.
///
/// - `Absolute` — every line shows its 1-based buffer line number.
/// - `Relative` — every line shows its distance from the cursor line; the
///   cursor line shows `0`.
/// - `Hybrid` *(default)* — the cursor line shows its absolute number; all
///   other lines show their relative distance. This gives the best of both
///   worlds: you can navigate by exact line number and jump by relative offset.
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
    /// This is the terminal height minus the status bar (1 row). The renderer
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

impl ViewState {
    /// Produce the display lines that are currently visible in the viewport.
    ///
    /// Iterates buffer lines in `[scroll_offset, scroll_offset + height)`
    /// and wraps each in a [`DisplayLine`]. Currently every display line maps
    /// 1:1 to a buffer line (no soft-wrap, no virtual lines).
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
        // Clamp: don't ask for lines that don't exist.
        let first = self.scroll_offset.min(total.saturating_sub(1));
        let last = (first + self.height).min(total);

        (first..last)
            .map(|line_idx| {
                let start = buf.line_to_char(line_idx);
                let end_excl = line_end_exclusive(buf, line_idx);

                // Strip the trailing '\n' from displayed content.
                // The renderer draws each line into a row and advances —
                // the newline is implicit in the row change, never drawn.
                let content_end = if end_excl > start
                    && buf.char_at(end_excl - 1) == Some('\n')
                {
                    end_excl - 1
                } else {
                    end_excl
                };

                DisplayLine {
                    source: DisplayLineSource::BufferLine { line_idx },
                    content: buf.slice(start..content_end),
                    line_number: Some(line_idx + 1), // 1-based for display
                    char_offset: Some(start),
                }
            })
            .collect()
    }

    /// Adjust `scroll_offset` so the primary cursor's line stays visible.
    ///
    /// Maintains a margin of [`SCROLL_MARGIN`] lines between the cursor and
    /// the top/bottom edges of the viewport. If the viewport is very short
    /// the margin is halved to avoid thrashing.
    pub(crate) fn ensure_cursor_visible(&mut self, buf: &Buffer, sels: &SelectionSet) {
        let cursor_line = buf.char_to_line(sels.primary().head);
        let margin = SCROLL_MARGIN.min(self.height / 2);

        if cursor_line < self.scroll_offset + margin {
            // Cursor is above (or near) the top edge — scroll up.
            self.scroll_offset = cursor_line.saturating_sub(margin);
        } else if self.height > 0 && cursor_line >= self.scroll_offset + self.height - margin {
            // Cursor is below (or near) the bottom edge — scroll down.
            self.scroll_offset = cursor_line.saturating_sub(self.height - margin - 1);
        }
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::buffer::Buffer;
    use crate::selection::{Selection, SelectionSet};

    fn view(scroll_offset: usize, height: usize, buf: &Buffer) -> ViewState {
        ViewState {
            scroll_offset,
            height,
            width: 80,
            gutter_width: compute_gutter_width(buf.len_lines()),
            line_number_style: LineNumberStyle::Absolute,
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

    fn cursor_at(line: usize, buf: &Buffer) -> SelectionSet {
        let pos = buf.line_to_char(line);
        SelectionSet::single(Selection::cursor(pos))
    }

    #[test]
    fn cursor_visible_no_scroll_needed() {
        let buf = Buffer::from("a\nb\nc\nd\ne\n");
        let mut v = view(0, 10, &buf);
        let sels = cursor_at(2, &buf);
        v.ensure_cursor_visible(&buf, &sels);
        assert_eq!(v.scroll_offset, 0); // cursor is well within viewport
    }

    #[test]
    fn cursor_below_viewport_scrolls_down() {
        let buf = Buffer::from("a\nb\nc\nd\ne\nf\ng\nh\n");
        // Viewport shows lines 0..5, cursor is on line 7 (below).
        let mut v = view(0, 5, &buf);
        let sels = cursor_at(7, &buf);
        v.ensure_cursor_visible(&buf, &sels);
        // After scroll the cursor should be within viewport with margin.
        let cursor_line = 7;
        assert!(cursor_line >= v.scroll_offset);
        assert!(cursor_line < v.scroll_offset + v.height);
    }

    #[test]
    fn cursor_above_viewport_scrolls_up() {
        let buf = Buffer::from("a\nb\nc\nd\ne\nf\ng\nh\n");
        // Viewport starts at line 5, cursor is on line 1 (above).
        let mut v = view(5, 5, &buf);
        let sels = cursor_at(1, &buf);
        v.ensure_cursor_visible(&buf, &sels);
        let cursor_line = 1;
        assert!(cursor_line >= v.scroll_offset);
        assert!(cursor_line < v.scroll_offset + v.height);
    }

    #[test]
    fn cursor_at_top_of_buffer_scroll_offset_is_zero() {
        let buf = Buffer::from("a\nb\nc\n");
        let mut v = view(2, 5, &buf); // scrolled down
        let sels = cursor_at(0, &buf);
        v.ensure_cursor_visible(&buf, &sels);
        assert_eq!(v.scroll_offset, 0);
    }
}
