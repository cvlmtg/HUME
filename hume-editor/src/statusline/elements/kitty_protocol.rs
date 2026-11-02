use hume_engine::types::ResolvedStyle;
use std::borrow::Cow;

use super::StatuslineElement;
use crate::statusline::HumeStatusline;
use crate::statusline::colors::EditorColors;

pub(in crate::statusline) struct KittyProtocolElement;

impl StatuslineElement for KittyProtocolElement {
    type Data<'a> = bool;

    fn read<'a>(editor: &'a HumeStatusline<'_>) -> Self::Data<'a> {
        editor.kitty_enabled
    }

    fn format<'a>(enabled: Self::Data<'a>, colors: &EditorColors) -> (Cow<'a, str>, ResolvedStyle) {
        let label = if enabled { "ᓚᘏᗢ" } else { "" };
        (Cow::Borrowed(label), colors.statusline)
    }
}
