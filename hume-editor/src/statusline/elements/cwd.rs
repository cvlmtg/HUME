use hume_engine::types::ResolvedStyle;
use hume_platform::path::{AbsolutePath, display_form};
use std::borrow::Cow;

use super::StatuslineElement;
use crate::statusline::HumeStatusline;
use crate::statusline::colors::EditorColors;

pub(in crate::statusline) struct CwdElement;

impl StatuslineElement for CwdElement {
    type Data = AbsolutePath;

    fn read(editor: &HumeStatusline<'_>) -> Self::Data {
        editor.state.cwd.clone()
    }

    fn format(cwd: Self::Data, colors: &EditorColors) -> (Cow<'static, str>, ResolvedStyle) {
        (Cow::Owned(display_form(&cwd)), colors.statusline)
    }
}
