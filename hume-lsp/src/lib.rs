//! LSP transport, JSON-RPC codec, and client state for HUME.
//!
//! Follows the `hume-treesitter` precedent: this crate has zero dependency
//! on `Editor`, `Buffer`, or anything in `hume-editor`/`hume-engine`. It
//! speaks `BufferId`-free protocol types (`lsp_types`) plus opaque metadata
//! the editor glue in `hume-editor/src/editor/lsp/` attaches, so the crate
//! graph stays acyclic. Its dependency edges are listed in `docs/CRATES.md`.
//!
//! ## Modules
//! - `uri`: path ↔ `file://` URI conversion.
//! - `location`: `Location`/`LocationLink` wire-object decoding.
//! - `position`: `hume_rope::position_encoding::WirePos` ↔ `lsp_types::Range`
//!   conversion, the one place `hume-rope`'s wire-position vocabulary
//!   crosses into `lsp_types`.
//! - `completion_item`: snippet stripping + lenient `TextEdit` decoding for
//!   `textDocument/completion` items.
//! - `codec`: JSON-RPC framing, message enum, id allocation.
//! - `transport`: server process management: reader/writer/stderr threads.
//! - `backend`: the `LspBackend` trait + `ThreadedLspBackend`.
//! - `inline`: `InlineLspBackend`, the scripted test double.
//! - `client`: lifecycle, request bookkeeping, notification classification.
//! - `initialize`: the `initialize` params and advertised client capabilities.
//! - `diagnostic`: pushed and pulled diagnostic decoding.
//! - `server_request`: answers to server-initiated requests.
//! - `sync`: `ChangeSet` → `TextDocumentContentChangeEvent[]`.
//! - `test_util` (behind the `test-util` feature): cross-crate test doubles.

#![deny(rustdoc::broken_intra_doc_links)]
#![deny(rustdoc::private_intra_doc_links)]

pub mod backend;
pub mod client;
pub mod codec;
pub mod completion_item;
pub mod diagnostic;
mod initialize;
#[cfg(any(test, feature = "test-util"))]
pub mod inline;
pub mod location;
pub mod position;
pub mod server_request;
pub mod sync;
#[cfg(any(test, feature = "test-util"))]
pub mod test_util;
pub mod transport;
pub mod uri;
