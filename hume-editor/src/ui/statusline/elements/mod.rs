use hume_engine::types::ResolvedStyle;
use std::borrow::Cow;

use crate::editor::Editor;
use crate::ui::theme::EditorColors;

/// Shared shape for a statusline element: gather data from the editor, then
/// turn that data into display text + style. Splitting `read` from `format`
/// lets `format` be unit-tested against a synthetic `Data` value with no
/// `Editor` fixture. `render` is the single call dispatch sites use.
///
/// `FilePath` and `Custom` don't implement this trait — `FilePath`'s content
/// isn't read from `Editor` at all, but injected by `render_statusline`'s
/// two-pass sizing pass (see `file_path.rs`); `Custom` needs its own `name`
/// alongside `Editor`, which `read`'s single-argument signature has no room
/// for (see `custom.rs`).
pub(super) trait StatuslineElement {
    type Data;

    fn read(editor: &Editor) -> Self::Data;

    fn format(data: Self::Data, colors: &EditorColors) -> (Cow<'static, str>, ResolvedStyle);

    fn render(editor: &Editor, colors: &EditorColors) -> (Cow<'static, str>, ResolvedStyle) {
        Self::format(Self::read(editor), colors)
    }
}

pub(super) mod custom;
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
mod separator;

pub(super) use cwd::CwdElement;
pub(super) use diagnostics::DiagnosticsElement;
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
pub(super) use separator::SeparatorElement;
