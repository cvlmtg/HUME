use hume_engine::types::ResolvedStyle;
use std::borrow::Cow;

use super::StatuslineElement;
use crate::editor::PipeProgress;
use crate::statusline::HumeStatusline;
use crate::statusline::colors::EditorColors;

pub(in crate::statusline) struct PipeElement;

impl StatuslineElement for PipeElement {
    /// The pipe being run, if any.
    type Data = Option<PipeProgress>;

    fn read(editor: &HumeStatusline<'_>) -> Self::Data {
        editor.state.running_pipe.clone()
    }

    fn format(progress: Self::Data, colors: &EditorColors) -> (Cow<'static, str>, ResolvedStyle) {
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
