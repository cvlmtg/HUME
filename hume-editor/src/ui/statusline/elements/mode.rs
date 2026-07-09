use std::borrow::Cow;

use ratatui::style::Style;

use hume_engine::types::EditorMode;

use super::StatuslineElement;
use crate::editor::Editor;
use crate::ui::theme::EditorColors;

pub(in crate::ui::statusline) struct ModeElement;

impl StatuslineElement for ModeElement {
    type Data = EditorMode;

    fn read(editor: &Editor) -> Self::Data {
        editor.state.mode()
    }

    fn format(mode: Self::Data, colors: &EditorColors) -> (Cow<'static, str>, Style) {
        let (label, style) = match mode {
            EditorMode::Normal => ("NOR", colors.status_normal),
            EditorMode::Extend => ("EXT", colors.status_extend),
            EditorMode::Insert => ("INS", colors.status_insert),
            EditorMode::Search => ("SRC", colors.status_search),
            EditorMode::Command => ("CMD", colors.status_command),
            EditorMode::Select => ("SEL", colors.status_select),
        };
        (Cow::Borrowed(label), style)
    }
}
