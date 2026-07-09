use std::borrow::Cow;

use ratatui::style::Style;

use super::StatuslineElement;
use crate::editor::Editor;
use crate::ui::theme::EditorColors;

pub(in crate::ui::statusline) struct ReadOnlyElement;

impl StatuslineElement for ReadOnlyElement {
    type Data = bool;

    fn read(editor: &Editor) -> Self::Data {
        editor.doc().is_read_only()
    }

    fn format(read_only: Self::Data, colors: &EditorColors) -> (Cow<'static, str>, Style) {
        let label = if read_only { "[RO]" } else { "" };
        (Cow::Borrowed(label), colors.statusline)
    }
}
