use hume_engine::types::ResolvedStyle;
use std::borrow::Cow;

use super::StatuslineElement;
use crate::statusline::HumeStatusline;
use crate::statusline::colors::EditorColors;

pub(in crate::statusline) struct MiniBufElement;

impl StatuslineElement for MiniBufElement {
    /// The text the minibuffer shows this frame, present only while one is
    /// open.
    type Data<'a> = Option<&'a str>;

    fn read<'a>(editor: &'a HumeStatusline<'_>) -> Self::Data<'a> {
        editor.minibuf.map(|view| view.text())
    }

    fn format<'a>(data: Self::Data<'a>, colors: &EditorColors) -> (Cow<'a, str>, ResolvedStyle) {
        (Cow::Borrowed(data.unwrap_or_default()), colors.statusline)
    }
}
