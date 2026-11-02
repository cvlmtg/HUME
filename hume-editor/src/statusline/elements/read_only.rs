use hume_engine::types::ResolvedStyle;
use std::borrow::Cow;

use super::StatuslineElement;
use crate::statusline::HumeStatusline;
use crate::statusline::colors::EditorColors;

pub(in crate::statusline) struct ReadOnlyElement;

impl StatuslineElement for ReadOnlyElement {
    type Data<'a> = bool;

    fn read<'a>(editor: &'a HumeStatusline<'_>) -> Self::Data<'a> {
        editor.doc().is_read_only()
    }

    fn format<'a>(
        read_only: Self::Data<'a>,
        colors: &EditorColors,
    ) -> (Cow<'a, str>, ResolvedStyle) {
        let label = if read_only { "[RO]" } else { "" };
        (Cow::Borrowed(label), colors.statusline)
    }
}
