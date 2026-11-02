use hume_engine::types::ResolvedStyle;
use std::borrow::Cow;

use super::StatuslineElement;
use crate::statusline::HumeStatusline;
use crate::statusline::colors::EditorColors;

pub(in crate::statusline) struct DirtyIndicatorElement;

impl StatuslineElement for DirtyIndicatorElement {
    type Data<'a> = bool;

    fn read<'a>(editor: &'a HumeStatusline<'_>) -> Self::Data<'a> {
        editor.state.has_unsaved_changes(editor.focused_bid)
    }

    fn format<'a>(dirty: Self::Data<'a>, colors: &EditorColors) -> (Cow<'a, str>, ResolvedStyle) {
        let label = if dirty { "[+]" } else { "" };
        (Cow::Borrowed(label), colors.statusline)
    }
}
