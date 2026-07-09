use std::borrow::Cow;

use ratatui::style::Style;

use super::StatuslineElement;
use crate::editor::Editor;
use crate::ui::theme::EditorColors;

pub(in crate::ui::statusline) struct FileNameElement;

impl StatuslineElement for FileNameElement {
    type Data = String;

    fn read(editor: &Editor) -> Self::Data {
        editor.doc().display_name()
    }

    fn format(name: Self::Data, colors: &EditorColors) -> (Cow<'static, str>, Style) {
        (Cow::Owned(name), colors.statusline)
    }
}
