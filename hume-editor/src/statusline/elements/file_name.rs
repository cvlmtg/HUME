use hume_engine::types::ResolvedStyle;
use std::borrow::Cow;

use super::StatuslineElement;
use crate::statusline::HumeStatusline;
use crate::statusline::colors::EditorColors;

pub(in crate::statusline) struct FileNameElement;

impl StatuslineElement for FileNameElement {
    type Data = String;

    fn read(editor: &HumeStatusline<'_>) -> Self::Data {
        editor.doc().display_name()
    }

    fn format(name: Self::Data, colors: &EditorColors) -> (Cow<'static, str>, ResolvedStyle) {
        (Cow::Owned(name), colors.statusline)
    }
}
