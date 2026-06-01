//! Core text-editing model for HUME.
//!
//! This crate owns the document representation and all stateless primitives
//! that operate on it. It has no knowledge of the editor, keymaps, rendering,
//! or scripting — it is a pure data-and-algorithm layer.
//!
//! ## Central types
//!
//! - [`text::Text`] — the document: a rope of Unicode scalar values with a
//!   recorded line-ending style. All positions are **char offsets**.
//! - [`selection::Selection`] / [`selection::SelectionSet`] — the cursor model.
//!   Every selection is *inclusive* (`anchor == head` is a 1-char selection).
//!   A `SelectionSet` is a sorted, non-overlapping, non-empty collection.
//! - [`changeset::ChangeSet`] — a document transform described as a sequence
//!   of `Retain`/`Delete`/`Insert` operations. Supports inversion (undo) and
//!   composition (squash).
//! - [`transaction::Transaction`] — a `ChangeSet` paired with the resulting
//!   `SelectionSet`. The unit of undo history.
//! - [`history::History`] — tree-structured undo/redo using a revision arena.
//!
//! ## Grapheme-cluster safety
//!
//! All motions and edits must advance by grapheme clusters, never by raw chars.
//! Use [`grapheme::next_grapheme_boundary`] and
//! [`grapheme::prev_grapheme_boundary`] for all position arithmetic in motion
//! or selection code.

pub mod changeset;
pub mod error;
pub mod grapheme;
pub mod history;
pub mod lines;
pub mod selection;
pub mod text;
pub mod transaction;
pub mod word;

// ── Facade re-exports ─────────────────────────────────────────────────────────
// Convenience: import the most-used types without spelling the module path.

pub use changeset::{ChangeSet, ChangeSetBuilder, Operation};
pub use error::{ApplyError, TransactionError, ValidationError};
pub use grapheme::{
    grapheme_col_in_line, grapheme_count, next_grapheme_boundary, prev_grapheme_boundary,
};
pub use history::{History, RevisionId};
pub use lines::{line_content_end, line_end_exclusive, snap_to_grapheme_boundary};
pub use selection::{Selection, SelectionSet};
pub use text::{LineEnding, Text};
pub use transaction::Transaction;
pub use word::{classify_char, is_long_word_boundary, is_word_boundary, CharClass};
