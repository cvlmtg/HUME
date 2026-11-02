use hume_engine::types::ResolvedStyle;
use std::borrow::Cow;

use super::StatuslineElement;
use crate::editor::PipeProgress;
use crate::statusline::HumeStatusline;
use crate::statusline::colors::EditorColors;

pub(in crate::statusline) struct PipeElement;

impl StatuslineElement for PipeElement {
    /// The focused buffer's pipe run, if any.
    type Data<'a> = Option<PipeProgress<'a>>;

    fn read<'a>(editor: &'a HumeStatusline<'_>) -> Self::Data<'a> {
        editor.state.pipe_runs.progress(editor.focused_bid)
    }

    fn format<'a>(
        progress: Self::Data<'a>,
        colors: &EditorColors,
    ) -> (Cow<'a, str>, ResolvedStyle) {
        let text = match progress {
            None => return (Cow::Borrowed(""), colors.statusline),
            Some(PipeProgress {
                command, total: 1, ..
            }) => format!("running `{command}`"),
            Some(PipeProgress {
                command,
                done,
                total,
            }) => format!("running `{command}` {done}/{total}"),
        };
        (Cow::Owned(text), colors.statusline)
    }
}
