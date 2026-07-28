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
        let label = match mode {
            EditorMode::Normal => "NOR",
            EditorMode::Extend => "EXT",
            EditorMode::Insert => "INS",
            EditorMode::Search => "SRC",
            EditorMode::Command => "CMD",
            EditorMode::Select => "SEL",
        };
        (Cow::Borrowed(label), colors.statusline)
    }
}
