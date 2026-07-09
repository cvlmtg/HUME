use std::borrow::Cow;

use ratatui::style::Style;

use super::StatuslineElement;
use crate::editor::Editor;
use crate::ui::theme::EditorColors;

pub(in crate::ui::statusline) struct SearchMatchesElement;

impl StatuslineElement for SearchMatchesElement {
    /// `(current, total)` 1-based match position, and whether search wrapped.
    type Data = (Option<(usize, usize)>, bool);

    fn read(editor: &Editor) -> Self::Data {
        let cursor = editor.current_search_cursor();
        (cursor.match_count, cursor.wrapped)
    }

    fn format(
        (match_count, wrapped): Self::Data,
        colors: &EditorColors,
    ) -> (Cow<'static, str>, Style) {
        match match_count {
            Some((current, total)) if total > 0 => {
                let w = if wrapped { "W " } else { "" };
                (
                    Cow::Owned(format!("{w}[{current}/{total}]")),
                    colors.statusline,
                )
            }
            _ => (Cow::Borrowed(""), colors.statusline),
        }
    }
}
