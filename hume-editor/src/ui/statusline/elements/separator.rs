use hume_engine::types::ResolvedStyle;
use hume_grid::box_glyphs::VERTICAL;
use std::borrow::Cow;

use super::StatuslineElement;
use crate::editor::Editor;
use crate::ui::theme::EditorColors;

pub(in crate::ui::statusline) struct SeparatorElement;

impl StatuslineElement for SeparatorElement {
    type Data = ();

    fn read(_editor: &Editor) -> Self::Data {}

    fn format(_data: Self::Data, colors: &EditorColors) -> (Cow<'static, str>, ResolvedStyle) {
        (Cow::Borrowed(VERTICAL), colors.statusline_separator)
    }
}
