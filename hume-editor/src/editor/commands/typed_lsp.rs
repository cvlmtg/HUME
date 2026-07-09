//! `:lsp-status` / `:lsp-stop` / `:lsp-restart` — observability and
//! lifecycle commands.

use super::super::Editor;
use super::super::Severity;
use crate::editor::error::CommandError;

/// `:lsp-status` — read-only view listing every registered server
/// (language, root, state, in-flight count, encoding) and every attached
/// buffer's diagnostic counts.
pub fn typed_lsp_status(
    ed: &mut Editor,
    _arg: Option<&str>,
    _force: bool,
) -> Result<(), CommandError> {
    let content = ed.lsp_status_text();
    ed.open_read_only_view("[lsp-status]", &content, 0);
    Ok(())
}

/// `:lsp-stop [language]` — graceful shutdown. No argument stops the
/// focused buffer's server; an argument stops every server registered for
/// that language (there can be more than one root/instance).
pub fn typed_lsp_stop(
    ed: &mut Editor,
    arg: Option<&str>,
    _force: bool,
) -> Result<(), CommandError> {
    let n = ed.lsp_stop(arg);
    if n == 0 {
        ed.report(
            Severity::Info,
            "lsp: no matching server to stop".to_string(),
        );
    } else {
        ed.report(
            Severity::Info,
            format!("lsp: stopped {n} server(s)"),
        );
    }
    Ok(())
}

/// `:lsp-restart [language]` — stop then respawn through the same
/// attach path, re-attaching every buffer that was on the old instance.
pub fn typed_lsp_restart(
    ed: &mut Editor,
    arg: Option<&str>,
    _force: bool,
) -> Result<(), CommandError> {
    let n = ed.lsp_restart(arg);
    if n == 0 {
        ed.report(
            Severity::Info,
            "lsp: no matching server to restart".to_string(),
        );
    } else {
        ed.report(
            Severity::Info,
            format!("lsp: restarted {n} server(s)"),
        );
    }
    Ok(())
}
