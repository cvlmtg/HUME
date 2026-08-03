use std::fmt;

/// A user-facing error produced when a command fails to execute.
///
/// Carries a human-readable message suitable for display in the status bar.
/// Distinct from [`hume_editing::error::ApplyError`] / [`hume_editing::error::TransactionError`]
/// (internal buffer integrity errors) — `CommandError` represents a user-level
/// failure such as "no match", "unsaved changes", or an I/O error during a
/// file write.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct CommandError(String);

impl CommandError {
    /// Construct a `CommandError` from any string-like value.
    pub(crate) fn new(msg: impl Into<String>) -> Self {
        Self(msg.into())
    }

    /// The human-readable error message.
    pub(crate) fn message(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for CommandError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for CommandError {}
