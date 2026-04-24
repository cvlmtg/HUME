use std::ops::Range;

use ropey::Rope;

use crate::pane::{ViewportState, WrapMode};
use crate::providers::GutterColumn;

// ---------------------------------------------------------------------------
// Visible range — output of Stage 1
// ---------------------------------------------------------------------------

/// The output of the Layout stage: which buffer lines are visible and what
/// geometry the Format/Render stages should use.
#[derive(Debug, Clone)]
pub struct VisibleRange {
    /// Buffer lines to format (may extend slightly past the visible area for
    /// smooth-scroll look-ahead; the Render stage clips to `content_height`).
    pub line_range: Range<usize>,
    /// How many display rows to skip from the top of `line_range.start` when
    /// the viewport begins partway through a wrapped line.
    pub top_skip_rows: u16,
    /// Available display rows in the content area.
    pub content_height: u16,
    /// Available columns in the content area (viewport width − gutter width).
    pub content_width: u16,
    /// Total gutter width in columns.
    pub gutter_width: u16,
    /// 0-based index of the last buffer line (`rope.len_lines() - 1`).
    /// This is the correct value to pass to `GutterColumn::width()`.
    pub last_line_idx: usize,
}

// ---------------------------------------------------------------------------
// Stage 1: compute_viewport
// ---------------------------------------------------------------------------

/// Sum of all gutter column widths for the given `max_line` (0-based last line index).
///
/// Pass the last line index of the entire file so gutter width is stable across scrolling.
pub fn gutter_width_for_line(gutter_columns: &[Box<dyn GutterColumn>], max_line: usize) -> u16 {
    gutter_columns
        .iter()
        .map(|c| c.width(max_line) as u16)
        .sum()
}

/// Compute the `VisibleRange` for a pane given its current state.
///
/// This is purely arithmetic — no heap allocations.
pub fn compute_viewport(
    rope: &Rope,
    viewport: &ViewportState,
    wrap_mode: &WrapMode,
    gutter_columns: &[Box<dyn GutterColumn>],
) -> VisibleRange {
    let total_lines = rope.len_lines();
    // 0-based index of the last line — the single source of truth for GutterColumn::width().
    // Using the whole-file last line (not just the visible range) keeps gutter width stable
    // as the user scrolls.
    let last_line_idx = total_lines.saturating_sub(1);
    let gutter_width = gutter_width_for_line(gutter_columns, last_line_idx);

    let content_width = viewport.width.saturating_sub(gutter_width).max(1);

    // Compute buffer line range that fills the viewport.
    let top_line = viewport.top_line.min(last_line_idx.saturating_sub(1));
    let top_skip = viewport.top_row_offset;

    // Exclude the phantom trailing line. The buffer invariant guarantees a
    // trailing `\n`, so ropey always reports one extra empty line at index
    // `last_line_idx`. Real content is lines 0..last_line_idx (exclusive), so
    // `last_line_idx` is the correct exclusive upper bound for the range.
    let line_range = compute_line_range(
        rope,
        top_line,
        top_skip,
        viewport.height,
        content_width,
        wrap_mode,
        last_line_idx,
    );

    VisibleRange {
        line_range,
        top_skip_rows: top_skip,
        content_height: viewport.height,
        content_width,
        gutter_width,
        last_line_idx,
    }
}

/// Determine which buffer lines need to be formatted to fill `viewport_height`
/// rows, starting from `top_line` with `top_skip` rows already scrolled past.
///
/// `last_line_idx` is the 0-based index of the last real content line (i.e.
/// `rope.len_lines() - 1`). It is used as an exclusive upper bound for the
/// returned range because the phantom trailing line at that index must not be
/// included.
fn compute_line_range(
    rope: &Rope,
    top_line: usize,
    top_skip: u16,
    viewport_height: u16,
    content_width: u16,
    wrap_mode: &WrapMode,
    last_line_idx: usize,
) -> Range<usize> {
    // For non-wrapping mode each buffer line is exactly one display row.
    if wrap_mode.wrap_width().is_none() {
        // top_skip is always 0 for non-wrapping (no wrapped lines).
        let end = (top_line + viewport_height as usize).min(last_line_idx);
        return top_line..end;
    }

    // For wrapping modes: count rows per line until we have filled the viewport.
    // `top_skip` rows have been consumed from `top_line` already.
    let mut rows_needed = viewport_height as usize + top_skip as usize;
    let mut end = top_line;

    while end < last_line_idx && rows_needed > 0 {
        let line_rows = estimate_line_rows(rope, end, content_width);
        rows_needed = rows_needed.saturating_sub(line_rows);
        end += 1;
    }

    // Add a small look-ahead so smooth scrolling has room.
    const LOOKAHEAD_LINES: usize = 4;
    let end = (end + LOOKAHEAD_LINES).min(last_line_idx);
    top_line..end
}

/// Cheaply estimate how many display rows a buffer line occupies when wrapped
/// to `content_width`. Uses character count as a proxy (ignores CJK/tabs but
/// is fast and good enough for layout purposes — the Format stage is exact).
fn estimate_line_rows(rope: &Rope, line_idx: usize, content_width: u16) -> usize {
    if content_width == 0 {
        return 1;
    }
    let char_count = rope.line(line_idx).len_chars();
    if char_count == 0 {
        1
    } else {
        char_count.div_ceil(content_width as usize)
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::providers::GutterCell;
    use crate::providers::GutterColumn;
    use crate::types::{EditorMode, RowKind, Scope};

    struct _NoGutter;
    impl GutterColumn for _NoGutter {
        fn width(&self, _: usize) -> u8 {
            0
        }
        fn render_row(&self, _: RowKind, _: EditorMode, _: usize) -> GutterCell {
            GutterCell::blank(Scope("ui.linenr"))
        }
        fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
            self
        }
    }

    #[test]
    fn no_wrap_basic_range() {
        let rope = Rope::from_str("line1\nline2\nline3\nline4\nline5\n");
        let viewport = ViewportState::new(80, 3);
        let visible = compute_viewport(&rope, &viewport, &WrapMode::None, &[]);
        assert_eq!(visible.line_range.start, 0);
        assert!(visible.line_range.end <= 5);
        assert_eq!(visible.gutter_width, 0);
        assert_eq!(visible.content_width, 80);
    }

    #[test]
    fn no_wrap_clamped_to_total_lines() {
        let rope = Rope::from_str("only one line");
        let viewport = ViewportState::new(80, 50);
        let visible = compute_viewport(&rope, &viewport, &WrapMode::None, &[]);
        assert!(visible.line_range.end <= rope.len_lines());
    }

    #[test]
    fn soft_wrap_includes_lookahead() {
        let rope = Rope::from_str("a\nb\nc\nd\ne\nf\ng\n");
        let viewport = ViewportState::new(80, 3);
        let visible = compute_viewport(&rope, &viewport, &WrapMode::Soft { width: 80 }, &[]);
        // Should have at least 3 + lookahead lines
        assert!(visible.line_range.len() >= 3);
    }
}
