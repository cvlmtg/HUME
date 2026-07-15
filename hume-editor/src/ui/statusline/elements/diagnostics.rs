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
    /// The focused buffer's LSP loading state, its `(errors, warnings)`
    /// counts, and the current spinner animation frame — rendered together
    /// (spinner prefix + counts suffix) so a background progress task never
    /// hides the counts the user already has.
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
        // `Starting` and a percentage-less `Progress` render identically —
        // neither has anything more specific to show than "a server task is
        // running".
        let spinner_prefix = match activity {
            LspActivity::Idle => None,
            LspActivity::Starting | LspActivity::Progress { percentage: None } => {
                Some(format!("{spinner} lsp"))
            }
            LspActivity::Progress {
                percentage: Some(p),
            } => Some(format!("{spinner} {p}%")),
        };
        let counts = match (errors, warnings) {
            (0, 0) => None,
            (e, 0) => Some(format!("{DIAGNOSTICS_ERROR_GLYPH} {e}")),
            (0, w) => Some(format!("{DIAGNOSTICS_WARNING_GLYPH} {w}")),
            (e, w) => Some(format!("{DIAGNOSTICS_ERROR_GLYPH} {e} {DIAGNOSTICS_WARNING_GLYPH} {w}")),
        };
        let label = match (spinner_prefix, counts) {
            (Some(s), Some(c)) => format!("{s} {c}"),
            (Some(s), None) => s,
            (None, Some(c)) => c,
            (None, None) => String::new(),
        };
        (Cow::Owned(label), colors.statusline)
    }
}
