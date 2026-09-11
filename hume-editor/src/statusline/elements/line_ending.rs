use hume_engine::types::ResolvedStyle;
use std::borrow::Cow;

use hume_editing::text::LineEnding as TextLineEnding;

use super::StatuslineElement;
use crate::statusline::HumeStatusline;
use crate::statusline::colors::EditorColors;

pub(in crate::statusline) struct LineEndingElement;

impl StatuslineElement for LineEndingElement {
    type Data = TextLineEnding;

    fn read(editor: &HumeStatusline<'_>) -> Self::Data {
        editor.doc().text().line_ending()
    }

    fn format(ending: Self::Data, colors: &EditorColors) -> (Cow<'static, str>, ResolvedStyle) {
        let label = match ending {
            TextLineEnding::Lf => "LF",
            TextLineEnding::CrLf => "CRLF",
        };
        (Cow::Borrowed(label), colors.statusline)
    }
}
