use hume_engine::types::ResolvedStyle;
use std::borrow::Cow;

use super::StatuslineElement;
use crate::statusline::HumeStatusline;
use crate::statusline::colors::EditorColors;

pub(in crate::statusline) struct MiniBufElement;

impl StatuslineElement for MiniBufElement {
    /// `(prompt, input)`, present only while the minibuffer is active.
    type Data<'a> = Option<(&'a str, &'a str)>;

    fn read<'a>(editor: &'a HumeStatusline<'_>) -> Self::Data<'a> {
        editor
            .state
            .minibuf()
            .map(|mb| (mb.prompt.as_str(), mb.input.as_str()))
    }

    fn format<'a>(data: Self::Data<'a>, colors: &EditorColors) -> (Cow<'a, str>, ResolvedStyle) {
        match data {
            Some((prompt, input)) => (Cow::Owned(format!("{prompt}{input}")), colors.statusline),
            None => (Cow::Borrowed(""), colors.statusline),
        }
    }
}
