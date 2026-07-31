//! The marker-annotated buffer/selection test DSL (`parse_state` /
//! `serialize_state` / `assert_state!` / `IntoTestResult`) is portable — it
//! depends only on `hume_editing` — and lives in `hume-test-fixtures` so
//! `hume-ops` can use it too without depending on `hume-editor`. Re-exported
//! here so every existing `crate::testing::*` reference stays unchanged.
//!
//! [`MockHost`] depends on `hume_engine` + `hume_scripting` and stays here.

mod mock_host;
pub(crate) use mock_host::MockHost;

pub(crate) use hume_test_fixtures::testing::{parse_state, serialize_state};

#[cfg(test)]
mod tests;
