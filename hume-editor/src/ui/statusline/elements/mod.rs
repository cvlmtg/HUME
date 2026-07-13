use std::borrow::Cow;

use ratatui::style::Style;

use crate::editor::Editor;
use crate::ui::theme::EditorColors;

/// Shared shape for a statusline element: gather data from the editor, then
/// turn that data into display text + style. Splitting `read` from `format`
/// lets `format` be unit-tested against a synthetic `Data` value with no
/// `Editor` fixture. `render` is the single call dispatch sites use.
///
/// `FilePath` does not implement this trait — its content isn't read from
/// `Editor` at all, but injected by `render_statusline`'s two-pass sizing
/// pass (see `file_path.rs`).
pub(super) trait StatuslineElement {
    type Data;

    fn read(editor: &Editor) -> Self::Data;

    fn format(data: Self::Data, colors: &EditorColors) -> (Cow<'static, str>, Style);

    fn render(editor: &Editor, colors: &EditorColors) -> (Cow<'static, str>, Style) {
        Self::format(Self::read(editor), colors)
    }
}

mod cwd;
mod diagnostics;
mod dirty_indicator;
mod file_name;
pub(super) mod file_path;
mod kitty_protocol;
mod language;
mod line_ending;
mod macro_recording;
mod minibuf;
mod mode;
mod position;
mod read_only;
mod search_matches;
mod selections;
mod separator;

pub(super) use cwd::CwdElement;
pub(super) use diagnostics::DiagnosticsElement;
#[cfg(test)]
pub(crate) use diagnostics::{DIAGNOSTICS_ERROR_GLYPH, DIAGNOSTICS_WARNING_GLYPH};
pub(super) use dirty_indicator::DirtyIndicatorElement;
pub(super) use file_name::FileNameElement;
pub(super) use kitty_protocol::KittyProtocolElement;
pub(super) use language::LanguageElement;
pub(super) use line_ending::LineEndingElement;
pub(super) use macro_recording::MacroRecordingElement;
pub(super) use minibuf::MiniBufElement;
pub(super) use mode::ModeElement;
pub(super) use position::PositionElement;
pub(super) use read_only::ReadOnlyElement;
pub(super) use search_matches::SearchMatchesElement;
pub(super) use selections::SelectionsElement;
pub(super) use separator::SeparatorElement;
