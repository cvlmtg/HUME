use hume_engine::types::ResolvedStyle;
use std::borrow::Cow;

use super::StatuslineElement;
use crate::statusline::HumeStatusline;
use crate::statusline::colors::EditorColors;

pub(in crate::statusline) struct SearchMatchesElement;

impl StatuslineElement for SearchMatchesElement {
    /// `(current, total)` 1-based match position, and whether search wrapped.
    type Data<'a> = (Option<(usize, usize)>, bool);

    fn read<'a>(editor: &'a HumeStatusline<'_>) -> Self::Data<'a> {
        let cursor = editor.current_search_cursor();
        (cursor.match_count, cursor.wrapped)
    }

    fn format<'a>(
        (match_count, wrapped): Self::Data<'a>,
        colors: &EditorColors,
    ) -> (Cow<'a, str>, ResolvedStyle) {
        match match_count {
            Some((current, total)) if total > 0 => {
                let w = if wrapped { "W " } else { "" };
                (
                    Cow::Owned(format!("{w}[{current}/{total}]")),
                    colors.statusline,
                )
            }
            _ => (Cow::Borrowed(""), colors.statusline),
        }
    }
}
