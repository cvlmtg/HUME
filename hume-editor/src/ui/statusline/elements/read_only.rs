use hume_engine::types::ResolvedStyle;
use std::borrow::Cow;

use super::StatuslineElement;
use crate::ui::statusline::HumeStatusline;
use crate::ui::theme::EditorColors;

pub(in crate::ui::statusline) struct ReadOnlyElement;

impl StatuslineElement for ReadOnlyElement {
    type Data = bool;

    fn read(editor: &HumeStatusline<'_>) -> Self::Data {
        editor.doc().is_read_only()
    }

    fn format(read_only: Self::Data, colors: &EditorColors) -> (Cow<'static, str>, ResolvedStyle) {
        let label = if read_only { "[RO]" } else { "" };
        (Cow::Borrowed(label), colors.statusline)
    }
}
