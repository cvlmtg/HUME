use hume_engine::types::ResolvedStyle;
use std::borrow::Cow;

use super::StatuslineElement;
use crate::statusline::HumeStatusline;
use crate::statusline::colors::EditorColors;

pub(in crate::statusline) struct KittyProtocolElement;

impl StatuslineElement for KittyProtocolElement {
    type Data = bool;

    fn read(editor: &HumeStatusline<'_>) -> Self::Data {
        editor.kitty_enabled
    }

    fn format(enabled: Self::Data, colors: &EditorColors) -> (Cow<'static, str>, ResolvedStyle) {
        let label = if enabled { "ᓚᘏᗢ" } else { "" };
        (Cow::Borrowed(label), colors.statusline)
    }
}
