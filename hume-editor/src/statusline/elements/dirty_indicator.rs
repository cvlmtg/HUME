use hume_engine::types::ResolvedStyle;
use std::borrow::Cow;

use super::StatuslineElement;
use crate::statusline::HumeStatusline;
use crate::statusline::colors::EditorColors;

pub(in crate::statusline) struct DirtyIndicatorElement;

impl StatuslineElement for DirtyIndicatorElement {
    type Data = bool;

    fn read(editor: &HumeStatusline<'_>) -> Self::Data {
        editor.doc().is_dirty()
    }

    fn format(dirty: Self::Data, colors: &EditorColors) -> (Cow<'static, str>, ResolvedStyle) {
        let label = if dirty { "[+]" } else { "" };
        (Cow::Borrowed(label), colors.statusline)
    }
}
