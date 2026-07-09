use std::borrow::Cow;

use ratatui::style::Style;

use super::StatuslineElement;
use crate::editor::Editor;
use crate::ui::theme::EditorColors;

pub(in crate::ui::statusline) struct KittyProtocolElement;

impl StatuslineElement for KittyProtocolElement {
    type Data = bool;

    fn read(editor: &Editor) -> Self::Data {
        editor.kitty_enabled
    }

    fn format(enabled: Self::Data, colors: &EditorColors) -> (Cow<'static, str>, Style) {
        let label = if enabled { "ᓚᘏᗢ" } else { "" };
        (Cow::Borrowed(label), colors.statusline)
    }
}
