use hume_engine::types::ResolvedStyle;
use std::borrow::Cow;

use super::StatuslineElement;
use crate::editor::Editor;
use crate::ui::theme::EditorColors;

pub(in crate::ui::statusline) struct MiniBufElement;

impl StatuslineElement for MiniBufElement {
    /// `(prompt, input)`, present only while the minibuffer is active.
    type Data = Option<(String, String)>;

    fn read(editor: &Editor) -> Self::Data {
        editor
            .state
            .minibuf
            .as_ref()
            .map(|mb| (mb.prompt.clone(), mb.input.clone()))
    }

    fn format(data: Self::Data, colors: &EditorColors) -> (Cow<'static, str>, ResolvedStyle) {
        match data {
            Some((prompt, input)) => {
                let mut text = String::with_capacity(prompt.len() + input.len());
                text.push_str(&prompt);
                text.push_str(&input);
                (Cow::Owned(text), colors.statusline)
            }
            None => (Cow::Borrowed(""), colors.statusline),
        }
    }
}
