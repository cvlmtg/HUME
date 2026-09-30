use std::fmt;

/// Errors returned by [`crate::changeset::ChangeSet::apply`] when the
/// changeset cannot be applied to the given buffer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ApplyError {
    /// The buffer's length doesn't match the changeset's `len_before`.
    ///
    /// Every changeset is built for a specific document length. Applying it
    /// to a buffer of a different length is a programming error, likely a
    /// mismatched buffer/changeset pair.
    LengthMismatch {
        /// Actual length of the buffer.
        buf_len: usize,
        /// Length the changeset was built for.
        expected: usize,
    },
    /// After applying all operations, the result rope doesn't end with `\n`.
    ///
    /// Every buffer must end with a structural trailing newline. A changeset
    /// that deletes it is invalid.
    TrailingNewlineMissing,
}

impl fmt::Display for ApplyError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ApplyError::LengthMismatch { buf_len, expected } => write!(
                f,
                "changeset expects a buffer of {expected} chars but got {buf_len}"
            ),
            ApplyError::TrailingNewlineMissing => write!(
                f,
                "changeset deleted the structural trailing '\\n': every buffer must end with '\\n'"
            ),
        }
    }
}

/// Errors returned by [`crate::transaction::Transaction::apply`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TransactionError {
    Apply(ApplyError),
    /// The recorded selections do not fit the text the changes produced.
    Selections(InvariantViolation),
}

impl fmt::Display for TransactionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            TransactionError::Apply(e) => write!(f, "changeset error: {e}"),
            TransactionError::Selections(e) => write!(f, "recorded selections: {e}"),
        }
    }
}

impl std::error::Error for ApplyError {}

impl From<ApplyError> for TransactionError {
    fn from(e: ApplyError) -> Self {
        TransactionError::Apply(e)
    }
}

// `source()` exposes the underlying cause so callers using `?` or
// `Box<dyn Error>` chains can inspect the root error rather than only the
// wrapper's Display message.
impl std::error::Error for TransactionError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            TransactionError::Apply(e) => Some(e),
            TransactionError::Selections(e) => Some(e),
        }
    }
}

/// A broken invariant of a selection set, as reported by
/// [`crate::selection::EditView::check`]. `index` names the offending
/// selection in document order.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InvariantViolation {
    /// The set holds no selection.
    Empty,
    /// The primary index is past the last selection.
    PrimaryOutOfRange { primary: usize, len: usize },
    /// The set is tagged with a different text than the one it is read with.
    VersionMismatch,
    /// An anchor or head is at or past the text's end.
    OutOfBounds { index: usize },
    /// An anchor or head is not the start of a grapheme cluster.
    SplitsCluster { index: usize },
    /// The selection shares a cluster with, or starts before, the one before it.
    Overlapping { index: usize },
}

impl std::error::Error for InvariantViolation {}

impl fmt::Display for InvariantViolation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Empty => write!(f, "the selection set is empty"),
            Self::PrimaryOutOfRange { primary, len } => {
                write!(
                    f,
                    "primary index {primary} is out of range for {len} selections"
                )
            }
            Self::VersionMismatch => {
                write!(f, "the selection set belongs to a different text")
            }
            Self::OutOfBounds { index } => {
                write!(f, "selection {index} reaches past the end of the text")
            }
            Self::SplitsCluster { index } => {
                write!(f, "selection {index} splits a grapheme cluster")
            }
            Self::Overlapping { index } => {
                write!(f, "selection {index} overlaps the one before it")
            }
        }
    }
}
