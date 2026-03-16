pub(crate) mod buffer;
pub(crate) mod changeset;
pub(crate) mod edit;
pub(crate) mod grapheme;
pub(crate) mod helpers;
pub(crate) mod motion;
pub(crate) mod selection;
pub(crate) mod text_object;
pub(crate) mod transaction;

// The test DSL is compiled only when running tests. It lives in its own
// module so every other module can `use crate::testing::*;` inside
// `#[cfg(test)]` blocks without any runtime cost in release builds.
#[cfg(test)]
pub(crate) mod testing;
