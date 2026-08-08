use std::borrow::Cow;

use ratatui::style::Style;

use hume_editing::grapheme::grapheme_col_in_line;

use super::StatuslineElement;
use crate::editor::Editor;
use crate::ui::theme::EditorColors;

pub(in crate::ui::statusline) struct PositionElement;

impl StatuslineElement for PositionElement {
    /// 1-based (line, col, max_row). `max_row` is the highest row index the
    /// cursor can reach in the buffer, used to size the padding field.
    type Data = (usize, usize, usize);

    fn read(editor: &Editor) -> Self::Data {
        let buf = editor.doc().text();
        let head = editor.current_selections().primary().head();
        let head_line = buf.char_to_line(head);
        let col_0 = grapheme_col_in_line(buf, head_line, head);
        // Largest 1-based line number this buffer can display.
        let max_row = buf.content_line_count();
        (head_line + 1, col_0 + 1, max_row)
    }

    fn format(
        (line, col, max_row): Self::Data,
        colors: &EditorColors,
    ) -> (Cow<'static, str>, Style) {
        // Right-align into a field sized for the largest row this buffer can
        // show (min 3 digits) and a fixed 3-digit column budget, so the
        // element's right edge stays put as the cursor moves and elements
        // after it (e.g. FilePath) don't jitter left-right. A column past
        // the 3-digit budget just overflows the field rather than shifting it.
        let row_digits = digit_count(max_row).max(3);
        let width = row_digits + 1 + 3;
        (
            Cow::Owned(format!("{:>width$}", format!("{line}:{col}"))),
            colors.statusline,
        )
    }
}

/// Number of base-10 digits needed to represent `n` (treats 0 as 1 digit).
fn digit_count(n: usize) -> usize {
    if n == 0 { 1 } else { n.ilog10() as usize + 1 }
}
