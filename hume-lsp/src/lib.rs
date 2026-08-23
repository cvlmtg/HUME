//! LSP transport, JSON-RPC codec, and client state for HUME.
//!
//! Follows the `hume-treesitter` precedent: this crate has zero dependency
//! on `Editor`, `Buffer`, or anything in `hume-editor`/`hume-engine`. It
//! speaks `BufferId`-free protocol types (`lsp_types`) plus opaque metadata
//! the editor glue in `hume-editor/src/editor/lsp/` attaches. Depends only
//! on `hume-editing` (for `ChangeSet` → `didChange` conversion) and
//! `lsp-types`; acyclic.
//!
//! ## Modules
//! - `uri`: path ↔ `file://` URI conversion.
//! - `location`: `Location`/`LocationLink` wire-object decoding.
//! - `codec`: JSON-RPC framing, message enum, id allocation.
//! - `transport`: server process management — reader/writer/stderr threads.
//! - `backend`: the `LspBackend` trait + `ThreadedLspBackend`.
//! - `inline`: `InlineLspBackend`, the scripted test double.
//! - `client`: lifecycle, request bookkeeping.
//! - `sync`: `ChangeSet` → `TextDocumentContentChangeEvent[]`.
//! - `test_util` (behind the `test-util` feature): cross-crate test doubles.

pub mod backend;
pub mod client;
pub mod codec;
pub mod inline;
pub mod location;
pub mod sync;
#[cfg(any(test, feature = "test-util"))]
pub mod test_util;
pub mod transport;
pub mod uri;
