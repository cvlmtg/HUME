use hume_engine::types::ResolvedStyle;
use std::borrow::Cow;

use super::StatuslineElement;
use crate::statusline::HumeStatusline;
use crate::statusline::colors::EditorColors;

pub(in crate::statusline) struct MacroRecordingElement;

impl StatuslineElement for MacroRecordingElement {
    /// The register being recorded into, if any.
    type Data<'a> = Option<char>;

    fn read<'a>(editor: &'a HumeStatusline<'_>) -> Self::Data<'a> {
        editor.state.macros.recording_register()
    }

    fn format<'a>(reg: Self::Data<'a>, colors: &EditorColors) -> (Cow<'a, str>, ResolvedStyle) {
        match reg {
            Some(reg) => (Cow::Owned(format!("[recording @{reg}]")), colors.statusline),
            None => (Cow::Borrowed(""), colors.statusline),
        }
    }
}
