//! The marker-annotated buffer/selection test DSL is `hume_editing::marked`,
//! wrapped for tests by `test_fixtures::testing` (`parse_state` /
//! `serialize_state` / `assert_state!` / `IntoTestResult`) so `hume-ops` can
//! use it too without depending on `hume-editor`.
//!
//! [`MockHost`] depends on `hume_engine` + `hume_scripting` and stays here.

mod mock_host;
pub use mock_host::MockHost;

// Not needed by the `test-util` external test crates `mock_host`'s own doc
// describes, narrower than this module's own `cfg(any(test, feature =
// "test-util"))` gate.
#[cfg(test)]
mod snapshot_theme;
#[cfg(test)]
pub(crate) use snapshot_theme::build_snapshot_theme;

#[cfg(test)]
mod tests;
