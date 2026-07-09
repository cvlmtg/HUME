use std::borrow::Cow;

use ratatui::style::Style;

use super::StatuslineElement;
use crate::editor::Editor;
use crate::ui::theme::EditorColors;

pub(in crate::ui::statusline) struct SelectionsElement;

impl StatuslineElement for SelectionsElement {
    type Data = ();

    fn read(_editor: &Editor) -> Self::Data {}

    fn format(_data: Self::Data, colors: &EditorColors) -> (Cow<'static, str>, Style) {
        (Cow::Borrowed(""), colors.statusline)
    }
}
