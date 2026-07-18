//! Tree-sitter integration: language registry, grammar attachment, the
//! background parse worker, and embedded-language injection resolution.
//!
//! Editor-domain glue (hooks, lazy-plugin activation, message logging, and
//! the per-frame orchestration that ties this crate's `ParseBackend` to a
//! live `Editor`) stays in `hume-editor`; this crate only knows about
//! buffers, ropes, and grammars.

pub mod edits;
pub mod grammar;
pub mod highlight;
pub mod injections;
pub mod layers;
pub mod parse_worker;
pub mod registry;
pub mod syntax;

#[cfg(test)]
mod test_support;
