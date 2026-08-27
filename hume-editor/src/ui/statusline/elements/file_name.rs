use hume_engine::types::ResolvedStyle;
use std::borrow::Cow;

use super::StatuslineElement;
use crate::editor::Editor;
use crate::ui::theme::EditorColors;

pub(in crate::ui::statusline) struct FileNameElement;

impl StatuslineElement for FileNameElement {
    type Data = String;

    fn read(editor: &Editor) -> Self::Data {
        editor.doc().display_name()
    }

    fn format(name: Self::Data, colors: &EditorColors) -> (Cow<'static, str>, ResolvedStyle) {
        (Cow::Owned(name), colors.statusline)
    }
}
