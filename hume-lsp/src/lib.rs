//! LSP transport, JSON-RPC codec, and client state for HUME.
//!
//! Follows the `hume-treesitter` precedent: this crate has zero dependency
//! on `Editor`, `Buffer`, or anything in `hume-editor`/`hume-engine`. It
//! speaks `BufferId`-free protocol types (`lsp_types`) plus opaque metadata
//! the editor glue in `hume-editor/src/editor/lsp/` attaches. Depends only
//! on `hume-editing` (for `ChangeSet` → `didChange` conversion, P6) and
//! `lsp-types`; acyclic (see the LSP hub's *Crate boundary* decision).
//!
//! ## Modules
//! - `uri`: path ↔ `file://` URI conversion (P5).
//! - `codec`: JSON-RPC framing, message enum, id allocation (C2).
//! - `transport`: server process management — reader/writer/stderr threads (C3).
//! - `client`: the `LspBackend` trait, lifecycle, request bookkeeping (C4–C6).
//! - `sync`: `ChangeSet` → `TextDocumentContentChangeEvent[]` (P6).

pub mod client;
pub mod codec;
pub mod sync;
pub mod transport;
pub mod uri;
