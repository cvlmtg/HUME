use hume_engine::types::ResolvedStyle;
use hume_grid::box_glyphs::VERTICAL;
use std::borrow::Cow;

use super::StatuslineElement;
use crate::statusline::HumeStatusline;
use crate::statusline::colors::EditorColors;

pub(in crate::statusline) struct SeparatorElement;

impl StatuslineElement for SeparatorElement {
    type Data<'a> = ();

    fn read<'a>(_editor: &'a HumeStatusline<'_>) -> Self::Data<'a> {}

    fn format<'a>(_data: Self::Data<'a>, colors: &EditorColors) -> (Cow<'a, str>, ResolvedStyle) {
        (Cow::Borrowed(VERTICAL), colors.statusline_separator)
    }
}
