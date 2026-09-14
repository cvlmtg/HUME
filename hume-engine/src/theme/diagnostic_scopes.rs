//! Names of the theme scopes HUME's LSP diagnostics resolve by fixed name —
//! same purpose as `super::ui_scopes`, split into its own file since these
//! aren't `"ui.*"`-namespaced.
//!
//! [`SEVERITIES`] is a fresh, independent declaration of the four severity
//! words, not a re-export of `hume-editor`'s `DiagSeverity`/
//! `message_log::Severity` enums: `hume-engine` cannot depend on
//! `hume-editor` (the dependency runs the other way), so those enums are
//! simply unreachable from here, regardless of visibility.
//!
//! Two of the five scope shapes below have **no Rust call site at all** —
//! [`ERROR_INLINE`] and its siblings are built by Steel's own
//! `string-append` (`lsp/severity-scope` in
//! `runtime/plugins/core/lsp/diagnostics.scm`), and the bare severity names
//! ([`SEVERITIES`] itself, used as a gutter-sign scope) reach Steel via
//! `DiagSeverity::to_string()` passed straight through
//! (`hume-editor/src/editor/lsp/introspect.rs`), never interned by a shared
//! Rust name. Both are declared here anyway, purely so the generator can
//! still offer the theme editor these names — there is nothing to repoint
//! on the Rust side for either.
//!
//! `hume-engine/src/theme/loader/vocabulary.rs` renders [`ALL`] into
//! `tools/theme-editor/src/lib/vocabulary.generated.js`.

/// Ordered least-to-most-lenient, matching `hume-editor`'s own
/// `DiagSeverity` discriminant order — kept in step by hand (see this
/// module's own doc for why a shared const isn't possible), not by the
/// compiler.
pub const SEVERITIES: [&str; 4] = ["error", "warning", "info", "hint"];

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

/// End-of-line diagnostic summary — see this module's own doc for why
/// nothing in Rust constructs this string.
pub const ERROR_INLINE: &str = "error.diagnostic.inline";
pub const WARNING_INLINE: &str = "warning.diagnostic.inline";
pub const INFO_INLINE: &str = "info.diagnostic.inline";
pub const HINT_INLINE: &str = "hint.diagnostic.inline";

/// Every name above, in the theme editor catalog's "Diagnostic" display
/// order: the four `diagnostic.<sev>` scopes, then their `.message`
/// siblings, then `.message-text`, then the end-of-line summary, then the
/// bare gutter-sign names from `SEVERITIES` itself.
pub const ALL: &[&str] = &[
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
    SEVERITIES[0],
    SEVERITIES[1],
    SEVERITIES[2],
    SEVERITIES[3],
];
