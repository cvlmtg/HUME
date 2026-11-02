use hume_engine::types::ResolvedStyle;
use std::borrow::Cow;

use super::StatuslineElement;
use crate::statusline::HumeStatusline;
use crate::statusline::colors::EditorColors;

pub(in crate::statusline) struct LanguageElement;

impl StatuslineElement for LanguageElement {
    type Data<'a> = Option<&'a str>;

    fn read<'a>(editor: &'a HumeStatusline<'_>) -> Self::Data<'a> {
        editor
            .doc()
            .language
            .map(|id| editor.state.config.languages.name_of(id))
    }

    fn format<'a>(lang: Self::Data<'a>, colors: &EditorColors) -> (Cow<'a, str>, ResolvedStyle) {
        match lang {
            Some(lang) => (Cow::Owned(format!("[{lang}]")), colors.statusline),
            None => (Cow::Borrowed(""), colors.statusline),
        }
    }
}
