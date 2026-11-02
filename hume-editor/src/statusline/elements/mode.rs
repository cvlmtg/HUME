use hume_engine::types::ResolvedStyle;
use std::borrow::Cow;

use hume_engine::types::EditorMode;

use super::StatuslineElement;
use crate::statusline::HumeStatusline;
use crate::statusline::colors::EditorColors;

pub(in crate::statusline) struct ModeElement;

impl StatuslineElement for ModeElement {
    type Data<'a> = EditorMode;

    fn read<'a>(editor: &'a HumeStatusline<'_>) -> Self::Data<'a> {
        editor.state.mode()
    }

    fn format<'a>(mode: Self::Data<'a>, colors: &EditorColors) -> (Cow<'a, str>, ResolvedStyle) {
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
