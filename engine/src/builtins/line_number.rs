use crate::providers::{GutterCell, GutterCellContent, GutterColumn};
use crate::types::{EditorMode, RowKind, Scope};

// ---------------------------------------------------------------------------
// Line number style
// ---------------------------------------------------------------------------

/// How line numbers are displayed in the gutter.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum LineNumberStyle {
    /// 1-based absolute line numbers.
    Absolute,
    /// Distance from the cursor line (0 at the cursor, counting outward).
    Relative,
    /// Absolute number on the cursor line, relative everywhere else.
    #[default]
    Hybrid,
}

// ---------------------------------------------------------------------------
// LineNumberColumn
// ---------------------------------------------------------------------------

/// Built-in gutter column that renders line numbers.
///
/// Width is computed dynamically: `floor(log10(total_lines)) + 1` digits plus
/// one space of padding on the right.
pub struct LineNumberColumn {
    pub style: LineNumberStyle,
}

impl Default for LineNumberColumn {
    fn default() -> Self {
        Self { style: LineNumberStyle::Hybrid }
    }
}

impl LineNumberColumn {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_style(style: LineNumberStyle) -> Self {
        Self { style }
    }

    /// Number of digits needed to represent `total_lines`.
    fn digit_count(total_lines: usize) -> u8 {
        if total_lines == 0 { 1 } else { total_lines.ilog10() as u8 + 1 }
    }
}

impl GutterColumn for LineNumberColumn {
    fn width(&self, last_line_idx: usize) -> u8 {
        // Digits needed to display the 1-based line number, plus 1 space of right-padding.
        Self::digit_count(last_line_idx + 1).saturating_add(1)
    }

    fn render_row(&self, kind: RowKind, _: EditorMode, primary_head_line: usize) -> GutterCell {
        match kind {
            RowKind::Filler | RowKind::Virtual { .. } | RowKind::Wrap { .. } => {
                GutterCell::blank(Scope("ui.linenr"))
            }
            RowKind::LineStart { line_idx } => {
                let scope = if line_idx == primary_head_line {
                    Scope("ui.linenr.selected")
                } else {
                    Scope("ui.linenr")
                };

                let display_num = match self.style {
                    LineNumberStyle::Absolute => line_idx + 1,
                    LineNumberStyle::Relative => {
                        (line_idx as isize - primary_head_line as isize).unsigned_abs()
                    }
                    LineNumberStyle::Hybrid => {
                        if line_idx == primary_head_line {
                            line_idx + 1 // absolute on the primary selection head line
                        } else {
                            (line_idx as isize - primary_head_line as isize).unsigned_abs()
                        }
                    }
                };

                GutterCell {
                    content: GutterCellContent::from_number(display_num),
                    scope,
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{EditorMode, RowKind};

    #[test]
    fn width_grows_with_line_count() {
        // width(max_line) must fit the 1-based line number max_line+1.
        // digit_count(n+1) + 1 pad.
        let col = LineNumberColumn::new();
        assert_eq!(col.width(0), 2);   // max line "1" → 1 digit + 1 pad
        assert_eq!(col.width(8), 2);   // max line "9" → 1 digit + 1 pad
        assert_eq!(col.width(9), 3);   // max line "10" → 2 digits + 1 pad
        assert_eq!(col.width(10), 3);  // max line "11" → 2 digits + 1 pad
        assert_eq!(col.width(98), 3);  // max line "99" → 2 digits + 1 pad
        assert_eq!(col.width(99), 4);  // max line "100" → 3 digits + 1 pad
        assert_eq!(col.width(100), 4); // max line "101" → 3 digits + 1 pad
    }

    #[test]
    fn absolute_line_numbers() {
        let col = LineNumberColumn::with_style(LineNumberStyle::Absolute);
        let cell = col.render_row(RowKind::LineStart { line_idx: 4 }, EditorMode::Normal, 0);
        assert_eq!(cell.as_str(), "5"); // 1-based
    }

    #[test]
    fn hybrid_head_line_shows_absolute() {
        let col = LineNumberColumn::with_style(LineNumberStyle::Hybrid);
        // Cursor is on line 2 (0-based).
        let cell = col.render_row(RowKind::LineStart { line_idx: 2 }, EditorMode::Normal, 2);
        assert_eq!(cell.as_str(), "3"); // absolute
        assert_eq!(cell.scope, Scope("ui.linenr.selected"));
    }

    #[test]
    fn hybrid_non_head_line_shows_relative() {
        let col = LineNumberColumn::with_style(LineNumberStyle::Hybrid);
        let cell = col.render_row(RowKind::LineStart { line_idx: 5 }, EditorMode::Normal, 2);
        assert_eq!(cell.as_str(), "3"); // |5-2| = 3
    }

    #[test]
    fn wrap_rows_are_blank() {
        let col = LineNumberColumn::new();
        let cell = col.render_row(RowKind::Wrap { line_idx: 3, wrap_row: 1 }, EditorMode::Normal, 0);
        assert_eq!(cell.as_str(), " "); // blank
    }

    #[test]
    fn virtual_rows_are_blank() {
        let col = LineNumberColumn::new();
        let cell = col.render_row(RowKind::Virtual { provider_id: 0, anchor_line: 0 }, EditorMode::Normal, 0);
        assert_eq!(cell.as_str(), " ");
    }

    #[test]
    fn relative_line_numbers() {
        let col = LineNumberColumn::with_style(LineNumberStyle::Relative);
        // Cursor at line 5 (0-based). Line 3 is distance 2, line 8 is distance 3.
        let cell = col.render_row(RowKind::LineStart { line_idx: 3 }, EditorMode::Normal, 5);
        assert_eq!(cell.as_str(), "2");
        let cell = col.render_row(RowKind::LineStart { line_idx: 8 }, EditorMode::Normal, 5);
        assert_eq!(cell.as_str(), "3");
    }

    #[test]
    fn relative_head_line_shows_zero() {
        let col = LineNumberColumn::with_style(LineNumberStyle::Relative);
        let cell = col.render_row(RowKind::LineStart { line_idx: 5 }, EditorMode::Normal, 5);
        assert_eq!(cell.as_str(), "0");
    }

    #[test]
    fn hybrid_line_below_head_shows_relative() {
        // Cursor at line 5, render line 2 (below in the file, higher index than cursor).
        let col = LineNumberColumn::with_style(LineNumberStyle::Hybrid);
        let cell = col.render_row(RowKind::LineStart { line_idx: 2 }, EditorMode::Normal, 5);
        assert_eq!(cell.as_str(), "3"); // |2-5| = 3
    }

    #[test]
    fn digit_count_zero_is_one() {
        assert_eq!(LineNumberColumn::digit_count(0), 1);
    }

    #[test]
    fn large_line_number_renders_correctly() {
        let col = LineNumberColumn::with_style(LineNumberStyle::Absolute);
        // line_idx = 9_999_998 → display = 9_999_999 (1-based)
        let cell = col.render_row(RowKind::LineStart { line_idx: 9_999_998 }, EditorMode::Normal, 0);
        assert_eq!(cell.as_str(), "9999999");
    }
}
