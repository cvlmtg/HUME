use std::borrow::Cow;
use std::path::PathBuf;

use ratatui::style::Style;

use hume_platform::path::shorten_home;

use super::StatuslineElement;
use crate::editor::Editor;
use crate::ui::theme::EditorColors;

pub(in crate::ui::statusline) struct CwdElement;

impl StatuslineElement for CwdElement {
    type Data = PathBuf;

    fn read(editor: &Editor) -> Self::Data {
        editor.state.cwd.clone()
    }

    fn format(cwd: Self::Data, colors: &EditorColors) -> (Cow<'static, str>, Style) {
        (Cow::Owned(shorten_home(&cwd)), colors.statusline)
    }
}
