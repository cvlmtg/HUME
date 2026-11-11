//! Tree-sitter integration: language registry, grammar loading, the
//! background parse worker, embedded-language injection resolution,
//! structural text-object/navigation queries, foldable regions, and
//! indentation.
//!
//! Editor-domain glue (hooks, lazy-plugin activation, message logging, and
//! the per-frame orchestration that ties this crate's `ParseBackend` to a
//! live `Editor`) stays in `hume-editor`; this crate only knows about
//! buffers, ropes, and grammars.

#![deny(rustdoc::broken_intra_doc_links)]
#![deny(rustdoc::private_intra_doc_links)]

pub mod context;
mod edited_bytes;
mod edits;
pub mod fold;
pub mod grammar;
pub mod highlight;
pub mod indent;
pub mod injections;
pub mod layers;
pub mod parse_worker;
pub mod registry;
pub mod syntax;
pub mod textobjects;

#[cfg(test)]
mod test_support;
