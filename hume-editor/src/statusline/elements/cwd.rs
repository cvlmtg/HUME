use hume_engine::types::ResolvedStyle;
use hume_platform::path::display_form;
use std::borrow::Cow;
use std::path::Path;

use super::StatuslineElement;
use crate::statusline::HumeStatusline;
use crate::statusline::colors::EditorColors;

pub(in crate::statusline) struct CwdElement;

impl StatuslineElement for CwdElement {
    type Data<'a> = &'a Path;

    fn read<'a>(editor: &'a HumeStatusline<'_>) -> Self::Data<'a> {
        &editor.state.cwd
    }

    fn format<'a>(cwd: Self::Data<'a>, colors: &EditorColors) -> (Cow<'a, str>, ResolvedStyle) {
        (Cow::Owned(display_form(cwd)), colors.statusline)
    }
}
