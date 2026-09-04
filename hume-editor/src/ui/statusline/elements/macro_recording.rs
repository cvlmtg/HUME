use hume_engine::types::ResolvedStyle;
use std::borrow::Cow;

use super::StatuslineElement;
use crate::ui::statusline::HumeStatusline;
use crate::ui::theme::EditorColors;

pub(in crate::ui::statusline) struct MacroRecordingElement;

impl StatuslineElement for MacroRecordingElement {
    /// The register being recorded into, if any.
    type Data = Option<char>;

    fn read(editor: &HumeStatusline<'_>) -> Self::Data {
        editor.state.macro_recording.as_ref().map(|(reg, _)| *reg)
    }

    fn format(reg: Self::Data, colors: &EditorColors) -> (Cow<'static, str>, ResolvedStyle) {
        match reg {
            Some(reg) => (Cow::Owned(format!("[recording @{reg}]")), colors.statusline),
            None => (Cow::Borrowed(""), colors.statusline),
        }
    }
}
