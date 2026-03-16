use std::fmt;

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
