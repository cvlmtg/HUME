use hume_engine::types::ResolvedStyle;
use std::borrow::Cow;
use std::path::PathBuf;

use hume_platform::path::display_form;

use super::StatuslineElement;
use crate::statusline::HumeStatusline;
use crate::statusline::colors::EditorColors;

pub(in crate::statusline) struct CwdElement;

impl StatuslineElement for CwdElement {
    type Data = PathBuf;

    fn read(editor: &HumeStatusline<'_>) -> Self::Data {
        editor.state.cwd.clone()
    }

    fn format(cwd: Self::Data, colors: &EditorColors) -> (Cow<'static, str>, ResolvedStyle) {
        (Cow::Owned(display_form(&cwd)), colors.statusline)
    }
}
