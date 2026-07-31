//! The marker-annotated buffer/selection test DSL (`parse_state` /
//! `serialize_state` / `assert_state!` / `IntoTestResult`) is portable — it
//! depends only on `hume_editing` — and lives in `hume-test-fixtures` so
//! `hume-ops` can use it too without depending on `hume-editor`.
//!
//! [`MockHost`] depends on `hume_engine` + `hume_scripting` and stays here.

mod mock_host;
pub(crate) use mock_host::MockHost;

#[cfg(test)]
mod tests;
