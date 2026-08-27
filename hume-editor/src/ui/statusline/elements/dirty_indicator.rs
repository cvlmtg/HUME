use hume_engine::types::ResolvedStyle;
use std::borrow::Cow;

use super::StatuslineElement;
use crate::editor::Editor;
use crate::ui::theme::EditorColors;

pub(in crate::ui::statusline) struct DirtyIndicatorElement;

impl StatuslineElement for DirtyIndicatorElement {
    type Data = bool;

    fn read(editor: &Editor) -> Self::Data {
        editor.doc().is_dirty()
    }

    fn format(dirty: Self::Data, colors: &EditorColors) -> (Cow<'static, str>, ResolvedStyle) {
        let label = if dirty { "[+]" } else { "" };
        (Cow::Borrowed(label), colors.statusline)
    }
}
