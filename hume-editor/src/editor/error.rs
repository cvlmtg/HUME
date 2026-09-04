use std::fmt;

use super::message_log::Severity;

/// A user-facing error produced when a command fails to execute.
///
/// Carries a human-readable message suitable for display in the status bar,
/// plus the [`Severity`] its report site should use — see [`Self::new`] vs
/// [`Self::transient`]. Distinct from [`hume_editing::error::ApplyError`] /
/// [`hume_editing::error::TransactionError`] (internal buffer integrity
/// errors) — `CommandError` represents a user-level failure such as an I/O
/// error during a file write, or a boundary condition like "no match".
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct CommandError {
    message: String,
    severity: Severity,
}

impl CommandError {
    /// Construct a `CommandError` reported at `Severity::Error` — logged to
    /// `:messages` and raises the unread-message statusline nudge.
    ///
    /// Use this when the editor attempted an operation and it went wrong
    /// (an I/O failure, a broken invariant). `Error` is the default for any
    /// site that hasn't been triaged, so an unclassified failure keeps its
    /// permanent record rather than silently disappearing — see
    /// [`Self::transient`] for the other case.
    pub(crate) fn new(msg: impl Into<String>) -> Self {
        Self {
            message: msg.into(),
            severity: Severity::Error,
        }
    }

    /// Construct a `CommandError` reported at `Severity::Info` — shown in
    /// the statusline and forgotten, never written to `:messages`.
    ///
    /// Use this when the user asked for something not currently possible
    /// (search found nothing, `:b` named a buffer that isn't open, `:set`
    /// got a typo'd key) rather than when an attempted operation failed.
    pub(crate) fn transient(msg: impl Into<String>) -> Self {
        Self {
            message: msg.into(),
            severity: Severity::Info,
        }
    }

    /// The human-readable error message.
    pub(crate) fn message(&self) -> &str {
        &self.message
    }

    /// The severity this error should be reported at.
    pub(crate) fn severity(&self) -> Severity {
        self.severity
    }
}

impl fmt::Display for CommandError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for CommandError {}
