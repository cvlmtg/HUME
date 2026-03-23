// The library is currently only exercised through tests — main.rs does not
// use it yet. Suppress the wave of dead-code warnings until M2 wires up the
// editor layer.
#![allow(dead_code)]

pub(crate) mod display_line;
pub(crate) mod view;
pub(crate) mod renderer;
pub(crate) mod terminal;
pub(crate) mod buffer;
pub(crate) mod document;
pub(crate) mod changeset;
pub(crate) mod edit;
pub(crate) mod error;
pub(crate) mod grapheme;
pub(crate) mod helpers;
pub(crate) mod history;
pub(crate) mod motion;
pub(crate) mod register;
pub(crate) mod selection;
pub(crate) mod selection_cmd;
pub(crate) mod text_object;
pub(crate) mod transaction;

// The test DSL is compiled only when running tests. It lives in its own
// module so every other module can `use crate::testing::*;` inside
// `#[cfg(test)]` blocks without any runtime cost in release builds.
#[cfg(test)]
pub(crate) mod testing;
#[cfg(test)]
mod proptest_doc;
