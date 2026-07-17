use std::borrow::Cow;

use ratatui::style::Style;

use super::StatuslineElement;
use crate::editor::Editor;
use crate::ui::theme::EditorColors;

pub(in crate::ui::statusline) struct LanguageElement;

impl StatuslineElement for LanguageElement {
    type Data = Option<String>;

    fn read(editor: &Editor) -> Self::Data {
        editor
            .doc()
            .language
            .map(|id| editor.state.languages.name_of(id).to_owned())
    }

    fn format(lang: Self::Data, colors: &EditorColors) -> (Cow<'static, str>, Style) {
        match lang {
            Some(lang) => (Cow::Owned(format!("[{lang}]")), colors.statusline),
            None => (Cow::Borrowed(""), colors.statusline),
        }
    }
}
