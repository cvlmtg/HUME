//! Shared test infrastructure for `hume-editor` and `hume-treesitter`.
//!
//! [`testing`] holds the marker-annotated buffer/selection DSL
//! (`parse_state`/`serialize_state`/`assert_state!`) used by editing-command
//! tests. Everything below is grammar-fixture paths and fixture-presence
//! preconditions shared by both crates' test suites. Fixtures are installed
//! by `scripts/fetch-test-grammars.sh` into `tests/fixtures/grammars/<name>/`,
//! with `queries/` normalized to the fixture root regardless of whether the
//! upstream grammar repo is a monorepo — callers never need to parse
//! `grammar-sources.scm` for subpaths.

use std::path::{Path, PathBuf};

pub mod testing;

fn fixtures_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("hume-test-fixtures sits directly under the workspace root")
        .join("tests/fixtures/grammars")
}

/// Shared-library extension for a compiled tree-sitter grammar on this
/// platform. Exposed (not just used internally by [`grammar_parser_path`])
/// for callers that stage a grammar fixture at a runtime path of their own
/// (e.g. a fake `<data>/grammars/`) rather than reading the fixture root.
pub fn grammar_platform_ext() -> &'static str {
    if cfg!(target_os = "macos") {
        "dylib"
    } else if cfg!(windows) {
        "dll"
    } else {
        "so"
    }
}

/// Absolute path to the pre-built grammar shared library for `name`.
///
/// Callers that require the file to exist should check or load it immediately
/// after calling this — the helper does not verify presence.
pub fn grammar_parser_path(name: &str) -> PathBuf {
    fixtures_root()
        .join(name)
        .join(format!("parser.{}", grammar_platform_ext()))
}

/// Absolute path to the highlights query file for `name`.
pub fn grammar_query_path(name: &str) -> PathBuf {
    fixtures_root().join(name).join("queries/highlights.scm")
}

/// Absolute path to the *Helix-maintained* injections query for `name`,
/// fetched by `scripts/fetch-test-grammars.sh` from the pinned Helix commit —
/// distinct from (and can differ from!) the grammar's own bundled
/// `queries/injections.scm`. PLUM installs the Helix version, so tests
/// validating what PLUM actually ships should use this. `None` if the fetch
/// script found no Helix injections query for `name` (most grammars have
/// none).
pub fn helix_injections_path(name: &str) -> Option<PathBuf> {
    let path = fixtures_root().join(name).join("helix-injections.scm");
    path.exists().then_some(path)
}

/// Absolute path to the *Helix-maintained* textobjects query for `name`,
/// fetched by `scripts/fetch-test-grammars.sh` from the pinned Helix commit.
/// `None` if the fetch script found no Helix textobjects query for `name`.
pub fn helix_textobjects_path(name: &str) -> Option<PathBuf> {
    let path = fixtures_root().join(name).join("helix-textobjects.scm");
    path.exists().then_some(path)
}

/// Require `names`' compiled grammar fixtures to be present.
///
/// Panics naming every missing fixture (not just the first) when one or more
/// is absent, pointing at `scripts/fetch-test-grammars.sh`.
pub fn require_grammars(names: &[&str]) {
    let missing: Vec<&str> = names
        .iter()
        .copied()
        .filter(|&name| !grammar_parser_path(name).exists())
        .collect();
    if !missing.is_empty() {
        panic!(
            "grammar fixture(s) missing: {missing:?} — run \
             scripts/fetch-test-grammars.sh from the repo root"
        );
    }
}

/// Same contract as [`require_grammars`], for a single fixture file that
/// isn't a compiled grammar (e.g. a Helix-maintained or bundled injections
/// query).
pub fn require_fixture_file(path: &Path, what: &str) {
    if !path.exists() {
        panic!(
            "fixture missing: {what} ({}) — run scripts/fetch-test-grammars.sh \
             from the repo root",
            path.display()
        );
    }
}
