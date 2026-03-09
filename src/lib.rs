pub(crate) mod buffer;
pub(crate) mod selection;

// The test DSL is compiled only when running tests. It lives in its own
// module so every other module can `use crate::testing::*;` inside
// `#[cfg(test)]` blocks without any runtime cost in release builds.
#[cfg(test)]
pub(crate) mod testing;
