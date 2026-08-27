use hume_engine::types::ResolvedStyle;
use std::borrow::Cow;
use std::path::PathBuf;

use hume_platform::path::display_form;

use super::StatuslineElement;
use crate::editor::Editor;
use crate::ui::theme::EditorColors;

pub(in crate::ui::statusline) struct CwdElement;

impl StatuslineElement for CwdElement {
    type Data = PathBuf;

    fn read(editor: &Editor) -> Self::Data {
        editor.state.cwd.clone()
    }

    fn format(cwd: Self::Data, colors: &EditorColors) -> (Cow<'static, str>, ResolvedStyle) {
        (Cow::Owned(display_form(&cwd)), colors.statusline)
    }
}
