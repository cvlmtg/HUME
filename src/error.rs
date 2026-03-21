use std::fmt;

/// Errors returned by [`crate::changeset::ChangeSet::apply`] when the
/// changeset cannot be applied to the given buffer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ApplyError {
    /// The buffer's length doesn't match the changeset's `len_before`.
    ///
    /// Every changeset is built for a specific document length. Applying it
    /// to a buffer of a different length is a programming error — likely a
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
                "changeset deleted the structural trailing '\\n' — every buffer must end with '\\n'"
            ),
        }
    }
}

/// Errors returned by [`crate::transaction::Transaction::apply`], covering
/// both changeset application and selection validation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum TransactionError {
    Apply(ApplyError),
    Validation(ValidationError),
}

impl fmt::Display for TransactionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            TransactionError::Apply(e) => write!(f, "changeset error: {e}"),
            TransactionError::Validation(e) => write!(f, "selection error: {e}"),
        }
    }
}

impl std::error::Error for ApplyError {}

impl From<ApplyError> for TransactionError {
    fn from(e: ApplyError) -> Self {
        TransactionError::Apply(e)
    }
}

// `TransactionError` wraps one of two inner errors; `source()` exposes the
// underlying cause so callers using `?` or `Box<dyn Error>` chains can
// inspect the root error rather than only the wrapper's Display message.
impl std::error::Error for TransactionError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            TransactionError::Apply(e) => Some(e),
            TransactionError::Validation(e) => Some(e),
        }
    }
}

impl From<ValidationError> for TransactionError {
    fn from(e: ValidationError) -> Self {
        TransactionError::Validation(e)
    }
}

/// Errors that arise when validating plugin-constructed state before it
/// touches the buffer.
///
/// These are returned from [`crate::selection::SelectionSet::validate`] and
/// propagated through [`crate::transaction::Transaction::apply`] so that a
/// plugin layer can surface a meaningful message instead of silently
/// corrupting the editor state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ValidationError {
    /// A selection's `head` or `anchor` is >= `buf_len`.
    ///
    /// Cursor positions are zero-indexed and must be strictly less than
    /// `buf_len` (the last valid index is the structural trailing `\n`).
    SelectionOutOfBounds {
        /// Index of the offending selection in the set.
        index: usize,
        /// Which field was out of bounds: `"head"` or `"anchor"`.
        field: &'static str,
        /// The out-of-bounds value.
        value: usize,
        /// The buffer length the value was checked against.
        buf_len: usize,
    },
    /// `buf_len` was 0, which violates the buffer invariant (every buffer has
    /// at least one char: the structural `\n`).
    EmptyBuffer,
}

impl std::error::Error for ValidationError {}

impl fmt::Display for ValidationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ValidationError::SelectionOutOfBounds { index, field, value, buf_len } => write!(
                f,
                "selection {index}: {field} {value} is out of bounds for buffer of length {buf_len}"
            ),
            ValidationError::EmptyBuffer => {
                write!(f, "buffer length is 0 — buffer must always have at least one char (the structural \\n)")
            }
        }
    }
}
