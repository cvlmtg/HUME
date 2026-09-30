//! Core text-editing model for HUME: the document representation and the
//! stateless primitives over it. No knowledge of the editor, keymaps,
//! rendering, or scripting.
//!
//! - [`text::BufferText`]: the document rope. Positions are char offsets.
//! - [`selection::SelectionSet`]: sorted, non-overlapping, non-empty set of
//!   [`selection::Selection`]s, each covering whole grapheme clusters, tagged
//!   with the version of the text it was computed for.
//! - [`state::EditState`]: a text and a selection set that fits it; reads go
//!   through [`selection::EditView`] and [`selection::SelectionView`].
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
pub mod edit;
pub mod error;
pub mod grapheme;
pub mod history;
pub mod lines;
#[cfg(any(test, feature = "test-util"))]
pub mod marked;
pub mod selection;
pub mod state;
pub mod tab_style;
pub mod text;
pub mod tracked;
pub mod transaction;
pub mod word;
