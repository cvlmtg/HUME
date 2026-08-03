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
pub mod diff;
pub mod error;
pub mod grapheme;
pub mod history;
pub mod lines;
pub mod position_encoding;
pub mod selection;
pub mod tab_style;
pub mod text;
pub mod transaction;
pub mod word;
