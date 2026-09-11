use hume_engine::types::ResolvedStyle;
use std::borrow::Cow;

use hume_editing::grapheme::grapheme_col_in_line;

use super::StatuslineElement;
use crate::statusline::HumeStatusline;
use crate::statusline::colors::EditorColors;

pub(in crate::statusline) struct PositionElement;

impl StatuslineElement for PositionElement {
    /// 1-based (line, grapheme_col, max_line). `max_line` is the highest
    /// line number the cursor can reach in the buffer, used to size the
    /// padding field.
    type Data = (usize, usize, usize);

    fn read(editor: &HumeStatusline<'_>) -> Self::Data {
        let text = editor.doc().text();
        let head = editor.current_selections().primary().head();
        let head_line = text.char_to_line(head);
        let grapheme_col = grapheme_col_in_line(text, head_line, head);
        // Largest 1-based line number this buffer can display.
        let max_line = text.content_line_count().get();
        (head_line.number(), grapheme_col.number(), max_line)
    }

    fn format(
        (line, grapheme_col, max_line): Self::Data,
        colors: &EditorColors,
    ) -> (Cow<'static, str>, ResolvedStyle) {
        // Right-align into a field sized for the largest line number this
        // buffer can show (min 3 digits) and a fixed 3-digit column budget,
        // so the element's right edge stays put as the cursor moves and
        // elements after it (e.g. FilePath) don't jitter left-right. A
        // column past the 3-digit budget just overflows the field rather
        // than shifting it.
        let line_digits = hume_engine::builtins::line_number::digit_count(max_line).max(3) as usize;
        let width = line_digits + 1 + 3;
        (
            Cow::Owned(format!("{:>width$}", format!("{line}:{grapheme_col}"))),
            colors.statusline,
        )
    }
}
