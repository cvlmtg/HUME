//! The marker-annotated buffer/selection test DSL (`parse_state` /
//! `serialize_state` / `assert_state!` / `IntoTestResult`) is portable — it
//! depends only on `hume_editing` — and lives in `test-fixtures` so
//! `hume-ops` can use it too without depending on `hume-editor`.
//!
//! [`MockHost`] depends on `hume_engine` + `hume_scripting` and stays here.

mod mock_host;
pub use mock_host::MockHost;

// Not needed by the `test-util` external test crates `mock_host`'s own doc
// describes — narrower than this module's own `cfg(any(test, feature =
// "test-util"))` gate.
#[cfg(test)]
mod snapshot_theme;
#[cfg(test)]
pub(crate) use snapshot_theme::build_snapshot_theme;

#[cfg(test)]
mod tests;
