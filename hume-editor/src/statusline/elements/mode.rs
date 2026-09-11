use hume_engine::types::ResolvedStyle;
use std::borrow::Cow;

use hume_engine::types::EditorMode;

use super::StatuslineElement;
use crate::statusline::HumeStatusline;
use crate::statusline::colors::EditorColors;

pub(in crate::statusline) struct ModeElement;

impl StatuslineElement for ModeElement {
    type Data = EditorMode;

    fn read(editor: &HumeStatusline<'_>) -> Self::Data {
        editor.state.mode()
    }

    fn format(mode: Self::Data, colors: &EditorColors) -> (Cow<'static, str>, ResolvedStyle) {
        let label = match mode {
            EditorMode::Normal => "NOR",
            EditorMode::Extend => "EXT",
            EditorMode::Insert => "INS",
            EditorMode::Search => "SRC",
            EditorMode::Command => "CMD",
            EditorMode::Sift => "SIF",
        };
        (Cow::Borrowed(label), colors.statusline)
    }
}
