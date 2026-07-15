use std::borrow::Cow;

use ratatui::style::Style;

use super::StatuslineElement;
use crate::editor::Editor;
use crate::editor::lsp::introspect::LspActivity;
use crate::ui::theme::EditorColors;

pub(crate) const DIAGNOSTICS_ERROR_GLYPH: &str = "✘";
pub(crate) const DIAGNOSTICS_WARNING_GLYPH: &str = "⚠";

/// Braille spinner frames for the loading state, indexed by
/// `frame % SPINNER.len()`.
const SPINNER: [char; 10] = ['⠋', '⠙', '⠹', '⠸', '⠼', '⠴', '⠦', '⠧', '⠇', '⠏'];

pub(in crate::ui::statusline) struct DiagnosticsElement;

impl StatuslineElement for DiagnosticsElement {
    /// The focused buffer's LSP loading state (takes priority when the
    /// server isn't ready yet), its `(errors, warnings)` counts, and the
    /// current spinner animation frame.
    ///
    /// Diagnostic counts are read directly from the diagnostics store in
    /// Rust — the statusline renders every frame, so this never goes
    /// through Steel's `(diagnostic-counts …)` builtin (that one is for
    /// plugins, not the render path).
    type Data = (LspActivity, usize, usize, usize);

    fn read(editor: &Editor) -> Self::Data {
        let bid = editor.focused_buffer_id();
        let (errors, warnings) = editor.diagnostic_counts(bid);
        (
            editor.lsp_activity(bid),
            errors,
            warnings,
            editor.lsp_spinner_frame(),
        )
    }

    fn format(
        (activity, errors, warnings, frame): Self::Data,
        colors: &EditorColors,
    ) -> (Cow<'static, str>, Style) {
        let spinner = SPINNER[frame % SPINNER.len()];
        let label = match activity {
            LspActivity::Starting => format!("{spinner} starting…"),
            LspActivity::Progress { percentage, .. } => match percentage {
                Some(p) => format!("{spinner} {p}%"),
                None => format!("{spinner} lsp"),
            },
            // Idle: error/warning counts, empty when there are none.
            LspActivity::Idle => match (errors, warnings) {
                (0, 0) => String::new(),
                (e, 0) => format!("{DIAGNOSTICS_ERROR_GLYPH} {e}"),
                (0, w) => format!("{DIAGNOSTICS_WARNING_GLYPH} {w}"),
                (e, w) => format!("{DIAGNOSTICS_ERROR_GLYPH} {e} {DIAGNOSTICS_WARNING_GLYPH} {w}"),
            },
        };
        (Cow::Owned(label), colors.statusline)
    }
}
