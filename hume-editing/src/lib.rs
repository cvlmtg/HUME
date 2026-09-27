//! Core text-editing model for HUME: the document representation and the
//! stateless primitives over it. No knowledge of the editor, keymaps,
//! rendering, or scripting.
//!
//! - [`text::BufferText`]: the document rope. Positions are char offsets.
//! - [`selection::SelectionSet`]: sorted, non-overlapping, non-empty set of
//!   inclusive [`selection::Selection`]s.
//! - [`changeset::ChangeSet`]: an invertible, composable document transform.
//! - [`transaction::Transaction`]: a `ChangeSet` plus resulting selections,
//!   the unit of undo.
//! - [`history::History`]: tree-structured undo/redo.
//!
//! Motion and selection code steps positions with
//! [`grapheme::next_grapheme_boundary`]/[`grapheme::prev_grapheme_boundary`],
//! never by raw chars.

#![deny(rustdoc::broken_intra_doc_links)]

pub mod changeset;
pub mod diff;
pub mod error;
pub mod grapheme;
pub mod history;
pub mod lines;
pub mod selection;
pub mod tab_style;
pub mod text;
pub mod transaction;
pub mod word;
