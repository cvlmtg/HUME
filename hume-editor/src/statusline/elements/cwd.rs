use hume_engine::types::ResolvedStyle;
use hume_platform::path::display_form;
use std::borrow::Cow;

use super::StatuslineElement;
use crate::statusline::HumeStatusline;
use crate::statusline::colors::EditorColors;

pub(in crate::statusline) struct CwdElement;

impl StatuslineElement for CwdElement {
    type Data = String;

    fn read(editor: &HumeStatusline<'_>) -> Self::Data {
        display_form(&editor.state.cwd)
    }

    fn format(cwd: Self::Data, colors: &EditorColors) -> (Cow<'static, str>, ResolvedStyle) {
        (Cow::Owned(cwd), colors.statusline)
    }
}
