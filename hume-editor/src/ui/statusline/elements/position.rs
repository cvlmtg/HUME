use std::borrow::Cow;

use ratatui::style::Style;

use hume_editing::grapheme::grapheme_col_in_line;

use super::StatuslineElement;
use crate::editor::Editor;
use crate::ui::theme::EditorColors;

pub(in crate::ui::statusline) struct PositionElement;

impl StatuslineElement for PositionElement {
    /// 1-based (line, col).
    type Data = (usize, usize);

    fn read(editor: &Editor) -> Self::Data {
        let buf = editor.doc().text();
        let head = editor.current_selections().primary().head();
        let head_line = buf.char_to_line(head);
        let col_0 = grapheme_col_in_line(buf, head_line, head);
        (head_line + 1, col_0 + 1)
    }

    fn format((line, col): Self::Data, colors: &EditorColors) -> (Cow<'static, str>, Style) {
        (Cow::Owned(format!("{line}:{col}")), colors.statusline)
    }
}
