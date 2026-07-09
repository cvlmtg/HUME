use std::borrow::Cow;

use ratatui::style::Style;

use super::StatuslineElement;
use crate::editor::Editor;
use crate::ui::theme::EditorColors;

pub(in crate::ui::statusline) struct DiagnosticsElement;

impl StatuslineElement for DiagnosticsElement {
    /// `(errors, warnings)` for the focused buffer.
    ///
    /// Reads the diagnostics store directly in Rust — the statusline renders
    /// every frame, so this never goes through Steel's `(diagnostic-counts …)`
    /// builtin (that one is for plugins, not the render path).
    type Data = (usize, usize);

    fn read(editor: &Editor) -> Self::Data {
        editor.diagnostic_counts(editor.focused_buffer_id())
    }

    fn format((errors, warnings): Self::Data, colors: &EditorColors) -> (Cow<'static, str>, Style) {
        let label = match (errors, warnings) {
            (0, 0) => String::new(),
            (e, 0) => format!("✗ {e}"),
            (0, w) => format!("⚠ {w}"),
            (e, w) => format!("✗ {e} ⚠ {w}"),
        };
        (Cow::Owned(label), colors.statusline)
    }
}
