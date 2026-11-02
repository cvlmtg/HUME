use hume_engine::types::ResolvedStyle;
use std::borrow::Cow;

use super::StatuslineElement;
use crate::statusline::HumeStatusline;
use crate::statusline::colors::EditorColors;

pub(in crate::statusline) struct FileNameElement;

impl StatuslineElement for FileNameElement {
    type Data<'a> = String;

    fn read<'a>(editor: &'a HumeStatusline<'_>) -> Self::Data<'a> {
        editor.doc().display_name()
    }

    fn format<'a>(name: Self::Data<'a>, colors: &EditorColors) -> (Cow<'a, str>, ResolvedStyle) {
        (Cow::Owned(name), colors.statusline)
    }
}
