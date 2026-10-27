//! Names of the theme scopes HUME's LSP diagnostics resolve by fixed name:
//! same purpose as `super::ui_scopes`, split into its own file since these
//! aren't `"ui.*"`-namespaced.
//!
//! [`ERROR_INLINE`] and its siblings have no Rust call site: Steel's own
//! `string-append` builds them (`lsp/severity-scope` in
//! `runtime/plugins/core/lsp/lib/diagnostics.scm`), and they are declared
//! here so the generator can offer them to the theme editor. The bare
//! severity names closing [`ALL`] are [`DiagSeverity::NAMES`], which a
//! diagnostic's `'severity` symbol carries to Steel as its gutter-sign scope.
//!
//! `hume-engine/src/theme/loader/vocabulary.rs` renders [`ALL`] into
//! `tools/theme-editor/src/lib/vocabulary.generated.js`.

use hume_core::{DiagSeverity, Severity};

pub const ERROR: &str = "diagnostic.error";
pub const WARNING: &str = "diagnostic.warning";
pub const INFO: &str = "diagnostic.info";
pub const HINT: &str = "diagnostic.hint";

pub const ERROR_MESSAGE: &str = "diagnostic.error.message";
pub const WARNING_MESSAGE: &str = "diagnostic.warning.message";
pub const INFO_MESSAGE: &str = "diagnostic.info.message";
pub const HINT_MESSAGE: &str = "diagnostic.hint.message";

pub const ERROR_MESSAGE_TEXT: &str = "diagnostic.error.message-text";
pub const WARNING_MESSAGE_TEXT: &str = "diagnostic.warning.message-text";
pub const INFO_MESSAGE_TEXT: &str = "diagnostic.info.message-text";
pub const HINT_MESSAGE_TEXT: &str = "diagnostic.hint.message-text";

/// End-of-line diagnostic summary. See this module's own doc for why
/// nothing in Rust constructs this string.
pub const ERROR_INLINE: &str = "error.diagnostic.inline";
pub const WARNING_INLINE: &str = "warning.diagnostic.inline";
pub const INFO_INLINE: &str = "info.diagnostic.inline";
pub const HINT_INLINE: &str = "hint.diagnostic.inline";

/// Every name above, in the theme editor catalog's "Diagnostic" display
/// order: the four `diagnostic.<sev>` scopes, then their `.message`
/// siblings, then `.message-text`, then the end-of-line summary, then the
/// bare gutter-sign names, least-to-most-lenient ([`DiagSeverity::NAMES`]).
pub const ALL: &[&str] = &{
    let [error, warning, info, hint] = DiagSeverity::NAMES;
    [
        ERROR,
        WARNING,
        INFO,
        HINT,
        ERROR_MESSAGE,
        WARNING_MESSAGE,
        INFO_MESSAGE,
        HINT_MESSAGE,
        ERROR_MESSAGE_TEXT,
        WARNING_MESSAGE_TEXT,
        INFO_MESSAGE_TEXT,
        HINT_MESSAGE_TEXT,
        ERROR_INLINE,
        WARNING_INLINE,
        INFO_INLINE,
        HINT_INLINE,
        error,
        warning,
        info,
        hint,
    ]
};

/// Editing-area scope of a diagnostic's highlight.
pub fn highlight(severity: DiagSeverity) -> &'static str {
    match severity {
        DiagSeverity::Error => ERROR,
        DiagSeverity::Warning => WARNING,
        DiagSeverity::Info => INFO,
        DiagSeverity::Hint => HINT,
    }
}

/// Theme scope for a message severity's `[label]` badge in `:messages`:
/// carries a background fill, no underline. `Trace` reuses `hint`, the
/// lowest-noise diagnostic severity, since it has no diagnostic counterpart.
///
/// Sub-keys of `diagnostic.<severity>` rather than edits to it:
/// `Theme::resolve_raw`'s dot-notation fallback *picks* the first match
/// rather than layering, so writing directly to `diagnostic.error` here
/// would strip the `underlined` modifier LSP diagnostic squiggles in code
/// buffers rely on. Themes that don't define the sub-keys fall back to
/// plain `diagnostic.<severity>` styling.
pub fn message_badge(severity: Severity) -> &'static str {
    match severity {
        Severity::Info => INFO_MESSAGE,
        Severity::Warning => WARNING_MESSAGE,
        Severity::Error => ERROR_MESSAGE,
        Severity::Trace => HINT_MESSAGE,
    }
}

/// Theme scope for a message severity's text in `:messages`: a foreground
/// tint, distinct from [`message_badge`] so the body doesn't inherit the
/// badge's background via dot-notation fallback.
pub fn message_text(severity: Severity) -> &'static str {
    match severity {
        Severity::Info => INFO_MESSAGE_TEXT,
        Severity::Warning => WARNING_MESSAGE_TEXT,
        Severity::Error => ERROR_MESSAGE_TEXT,
        Severity::Trace => HINT_MESSAGE_TEXT,
    }
}
