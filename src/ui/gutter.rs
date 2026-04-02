use ratatui::buffer::Buffer as ScreenBuf;

use crate::ui::formatter::VisualRow;
use crate::ui::theme::EditorColors;
use crate::ui::view::LineNumberStyle;

// ── GutterColumn ──────────────────────────────────────────────────────────────

/// One vertical column in the gutter area.
///
/// Columns are stacked left-to-right; the total gutter width is the sum of all
/// column widths plus one trailing separator space. Extending the gutter with a
/// new column — diagnostics, git signs, fold markers — means adding a variant
/// here and a `render` arm. No other code changes required.
///
/// This enum-based design matches Hume's broader pattern of concrete types over
/// trait objects: `MappableCommand` and `StatusElement` use the same idiom.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum GutterColumn {
    /// Line numbers: absolute, relative, or hybrid (see [`LineNumberStyle`]).
    LineNumber,
    // Future variants:
    // Diagnostic,  // severity dot (error/warn/info/hint)
    // GitSigns,    // diff sign (+/-/~), implemented via Steel plugin
    // FoldMarker,  // tree-sitter powered fold/collapse
}

impl GutterColumn {
    /// Width in display columns for this column, given the total buffer line count.
    ///
    /// Line numbers need enough columns for the widest possible label.
    pub(crate) fn width(&self, total_lines: usize) -> usize {
        match self {
            // One leading space + digits, minimum 3 (so that total_width adds up
            // to at least 4 — avoids an uncomfortably narrow gutter on small files).
            Self::LineNumber => {
                let digits = if total_lines <= 1 {
                    1
                } else {
                    total_lines.ilog10() as usize + 1
                };
                (1 + digits).max(3)
            }
        }
    }

    /// Render this column's cell for one visual row into `screen_buf`.
    ///
    /// `x` is the left edge of this column's area; `y` is the screen row.
    /// `wrap_indicator` is shown for continuation rows when `Some`.
    pub(crate) fn render(
        &self,
        screen_buf: &mut ScreenBuf,
        vrow: &VisualRow,
        line_number_style: LineNumberStyle,
        cursor_line: usize,
        colors: &EditorColors,
        wrap_indicator: Option<char>,
        x: u16,
        y: u16,
        total_lines: usize,
    ) {
        match self {
            Self::LineNumber => render_line_number(
                screen_buf,
                vrow,
                line_number_style,
                cursor_line,
                colors,
                wrap_indicator,
                x,
                y,
                self.width(total_lines),
            ),
        }
    }
}

// ── GutterConfig ─────────────────────────────────────────────────────────────

/// The ordered list of gutter columns for one editor pane.
///
/// Stored on [`ViewState`](crate::ui::view::ViewState). The default
/// configuration has a single `LineNumber` column; future Steel scripting will
/// allow users to add or reorder columns.
#[derive(Debug, Clone)]
pub(crate) struct GutterConfig {
    pub columns: Vec<GutterColumn>,
    /// Character drawn in the gutter for soft-wrap continuation rows.
    ///
    /// `None` (the default) leaves continuation gutter cells blank. Set to a
    /// character such as `'↪'` to show a wrap indicator — useful when
    /// `indent_wrap` is disabled and there is no other visual cue.
    pub wrap_indicator: Option<char>,
}

impl Default for GutterConfig {
    fn default() -> Self {
        Self { columns: vec![GutterColumn::LineNumber], wrap_indicator: None }
    }
}

impl GutterConfig {
    /// Total gutter width in display columns.
    ///
    /// Equals the sum of all column widths plus one trailing separator space
    /// (the gap between the gutter and the content area). Returns 0 when there
    /// are no columns (rare but valid — callers handle zero gracefully).
    ///
    /// `total_lines` must reflect the current buffer line count so that the
    /// `LineNumber` column allocates enough digits.
    pub(crate) fn total_width(&self, total_lines: usize) -> usize {
        let sum: usize = self.columns.iter().map(|c| c.width(total_lines)).sum();
        if sum == 0 {
            0
        } else {
            sum + 1 // +1 trailing separator space
        }
    }

    /// Render the full gutter for one visual row by iterating columns left to right.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn render_row(
        &self,
        screen_buf: &mut ScreenBuf,
        vrow: &VisualRow,
        line_number_style: LineNumberStyle,
        cursor_line: usize,
        colors: &EditorColors,
        base_x: u16,
        y: u16,
        total_lines: usize,
    ) {
        let mut x = base_x;
        for col in &self.columns {
            col.render(screen_buf, vrow, line_number_style, cursor_line, colors, self.wrap_indicator, x, y, total_lines);
            x += col.width(total_lines) as u16;
        }
        // The trailing separator space is implicit: the content area Rect
        // starts at base_x + total_width(), which already includes the +1.
    }
}

// ── Line number rendering ─────────────────────────────────────────────────────

/// Render a line-number cell for one visual row.
///
/// The label is right-aligned within the column width, with the style switching
/// to `gutter_cursor_line` on the cursor's row for visual emphasis.
///
/// Continuation rows (soft-wrap second/third/... display rows) show a
/// `wrap_indicator` character (e.g. `↪`) when configured, or a blank cell.
fn render_line_number(
    screen_buf: &mut ScreenBuf,
    vrow: &VisualRow,
    style: LineNumberStyle,
    cursor_line: usize,
    colors: &EditorColors,
    wrap_indicator: Option<char>,
    x: u16,
    y: u16,
    col_width: usize,
) {
    // Continuation rows: show a wrap indicator if configured, otherwise blank.
    if vrow.is_continuation {
        if let Some(indicator) = wrap_indicator {
            let gutter_style = colors.gutter;
            // Right-align the indicator in col_width, then the trailing separator space.
            let cell = format!("{indicator:>col_width$} ");
            screen_buf.set_string(x, y, &cell, gutter_style);
        }
        return;
    }

    // Virtual rows have no line number — nothing to render.
    let Some(line_number) = vrow.line_number else {
        return;
    };

    let line_idx = line_number.saturating_sub(1); // 0-based

    let label = match style {
        LineNumberStyle::Absolute => format!("{line_number}"),
        LineNumberStyle::Relative => format!("{}", line_idx.abs_diff(cursor_line)),
        LineNumberStyle::Hybrid => {
            if line_idx == cursor_line {
                format!("{line_number}")
            } else {
                format!("{}", line_idx.abs_diff(cursor_line))
            }
        }
    };

    // Right-align in col_width columns, then append one trailing separator space.
    // Total = col_width + 1 = total_width (matches the old monolithic render_gutter
    // format string `"{:>w$} "` where w = gutter_width - 1 = col_width).
    let cell = format!("{label:>col_width$} ");

    let gutter_style =
        if line_idx == cursor_line { colors.gutter_cursor_line } else { colors.gutter };

    screen_buf.set_string(x, y, &cell, gutter_style);
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    // ── GutterColumn::width ───────────────────────────────────────────────────

    #[test]
    fn line_number_width_minimum() {
        // 0-9 lines → 1 digit → max(1+1, 3) = 3.
        assert_eq!(GutterColumn::LineNumber.width(0), 3);
        assert_eq!(GutterColumn::LineNumber.width(1), 3);
        assert_eq!(GutterColumn::LineNumber.width(9), 3);
    }

    #[test]
    fn line_number_width_two_digits() {
        // 10-99 lines → 2 digits → max(1+2, 3) = 3.
        assert_eq!(GutterColumn::LineNumber.width(10), 3);
        assert_eq!(GutterColumn::LineNumber.width(99), 3);
    }

    #[test]
    fn line_number_width_three_digits() {
        // 100-999 lines → 3 digits → 1+3 = 4.
        assert_eq!(GutterColumn::LineNumber.width(100), 4);
        assert_eq!(GutterColumn::LineNumber.width(999), 4);
    }

    // ── GutterConfig::total_width ─────────────────────────────────────────────

    #[test]
    fn total_width_single_line_number_column() {
        let cfg = GutterConfig::default();
        // 1-9 lines: col_width=3, +1 separator = 4 (matches old minimum of 4).
        assert_eq!(cfg.total_width(9), 4);
        // For 10 lines: 3 + 1 = 4.
        assert_eq!(cfg.total_width(10), 4);
        // For 99 lines: 3 + 1 = 4.
        assert_eq!(cfg.total_width(99), 4);
        // For 100 lines: 4 + 1 = 5.
        assert_eq!(cfg.total_width(100), 5);
        // For 999 lines: 4 + 1 = 5.
        assert_eq!(cfg.total_width(999), 5);
        // For 1000 lines: 5 + 1 = 6.
        assert_eq!(cfg.total_width(1000), 6);
    }

    #[test]
    fn total_width_no_columns() {
        let cfg = GutterConfig { columns: vec![], wrap_indicator: None };
        assert_eq!(cfg.total_width(100), 0);
    }
}
