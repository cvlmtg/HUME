//! Tree-sitter integration: language registry, grammar attachment, the
//! background parse worker, embedded-language injection resolution, and
//! structural text-object/navigation queries.
//!
//! Editor-domain glue (hooks, lazy-plugin activation, message logging, and
//! the per-frame orchestration that ties this crate's `ParseBackend` to a
//! live `Editor`) stays in `hume-editor`; this crate only knows about
//! buffers, ropes, and grammars.

mod edits;
pub mod grammar;
pub mod highlight;
pub mod injections;
pub mod layers;
pub mod parse_worker;
pub mod registry;
pub mod syntax;
pub mod textobjects;

#[cfg(test)]
mod test_support;
