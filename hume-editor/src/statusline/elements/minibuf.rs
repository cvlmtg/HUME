use hume_engine::types::ResolvedStyle;
use std::borrow::Cow;

use super::StatuslineElement;
use crate::statusline::HumeStatusline;
use crate::statusline::colors::EditorColors;

pub(in crate::statusline) struct MiniBufElement;

impl StatuslineElement for MiniBufElement {
    /// `(prompt, input)`, present only while the minibuffer is active.
    type Data = Option<(String, String)>;

    fn read(editor: &HumeStatusline<'_>) -> Self::Data {
        editor
            .state
            .minibuf()
            .map(|mb| (mb.prompt.clone(), mb.input.clone()))
    }

    fn format(data: Self::Data, colors: &EditorColors) -> (Cow<'static, str>, ResolvedStyle) {
        match data {
            Some((prompt, input)) => (Cow::Owned(prompt + &input), colors.statusline),
            None => (Cow::Borrowed(""), colors.statusline),
        }
    }
}
