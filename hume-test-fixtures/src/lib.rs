//! Grammar-fixture paths and require-fixtures gating shared by
//! `hume-treesitter`'s and `hume-editor`'s test suites. Fixtures are
//! installed by `scripts/fetch-test-grammars.sh` into
//! `tests/fixtures/grammars/<name>/`, with `queries/` normalized to the
//! fixture root regardless of whether the upstream grammar repo is a
//! monorepo — callers never need to parse `grammar-sources.scm` for subpaths.

use std::path::{Path, PathBuf};

/// Set to `"1"` (by CI and `scripts/test-all.sh`) to turn a missing grammar
/// fixture into a hard test failure instead of a skip — mirrors
/// `HUME_REQUIRE_LIVE_GRAMMAR_E2E`.
pub const REQUIRE_FIXTURES_ENV: &str = "HUME_REQUIRE_GRAMMAR_FIXTURES";

fn fixtures_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("hume-test-fixtures sits directly under the workspace root")
        .join("tests/fixtures/grammars")
}

/// Absolute path to the pre-built grammar shared library for `name`.
///
/// Callers that require the file to exist should check or load it immediately
/// after calling this — the helper does not verify presence.
pub fn grammar_parser_path(name: &str) -> PathBuf {
    let suffix = if cfg!(target_os = "macos") {
        "dylib"
    } else if cfg!(windows) {
        "dll"
    } else {
        "so"
    };
    fixtures_root().join(name).join(format!("parser.{suffix}"))
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

fn fixtures_required() -> bool {
    std::env::var(REQUIRE_FIXTURES_ENV).is_ok_and(|v| v == "1")
}

/// Gate a test on the presence of `names`' compiled grammar fixtures.
///
/// Returns `true` when the caller should skip — the caller should
/// `eprintln!` its own skip note and `return` early. Returns `false` when
/// every fixture is present. Panics instead of returning `true` when a
/// fixture is missing and `HUME_REQUIRE_GRAMMAR_FIXTURES=1` (set by CI and
/// `scripts/test-all.sh`), so CI can never pass vacuously.
pub fn skip_unless_grammars(names: &[&str]) -> bool {
    let mut missing: Vec<&str> = Vec::new();
    for &name in names {
        if !grammar_parser_path(name).exists() {
            missing.push(name);
        }
    }
    if missing.is_empty() {
        return false;
    }
    if fixtures_required() {
        panic!(
            "grammar fixture(s) missing: {missing:?} (required by \
             {REQUIRE_FIXTURES_ENV}=1) — run scripts/fetch-test-grammars.sh \
             from the repo root"
        );
    }
    eprintln!(
        "skipping: grammar fixture(s) missing: {missing:?} (run \
         scripts/fetch-test-grammars.sh, or set {REQUIRE_FIXTURES_ENV}=1 to \
         make this a hard failure)"
    );
    true
}

/// Same contract as [`skip_unless_grammars`], for a single fixture file that
/// isn't a compiled grammar (e.g. a Helix-maintained or bundled injections
/// query).
pub fn skip_unless_file(path: &Path, what: &str) -> bool {
    if path.exists() {
        return false;
    }
    if fixtures_required() {
        panic!(
            "fixture missing: {what} ({}) — required by {REQUIRE_FIXTURES_ENV}=1 \
             — run scripts/fetch-test-grammars.sh from the repo root",
            path.display()
        );
    }
    eprintln!(
        "skipping: fixture missing: {what} ({}) (run scripts/fetch-test-grammars.sh, \
         or set {REQUIRE_FIXTURES_ENV}=1 to make this a hard failure)",
        path.display()
    );
    true
}

/// Set to `"1"` to opt into e2e tests that hit the network (git clone + curl
/// fetch + a real `tree-sitter` CLI build) instead of a pre-built fixture.
/// Distinct from [`REQUIRE_FIXTURES_ENV`]: that one gates *presence of
/// pre-compiled fixtures*, this one gates *live installs* run through PLUM.
pub const REQUIRE_LIVE_E2E_ENV: &str = "HUME_REQUIRE_LIVE_GRAMMAR_E2E";

/// Gate a live (network + `tree-sitter` CLI) grammar-install e2e test.
///
/// Returns `true` when the caller should skip — the caller should `return`
/// early; this function already `eprintln!`s the skip note, tagged with
/// `test_name`. Returns `false` (never skip) once `git`, `curl`, and
/// `tree-sitter` are all confirmed present. Panics — naming whichever tools
/// are missing — when `HUME_REQUIRE_LIVE_GRAMMAR_E2E=1` but a prerequisite
/// tool isn't on `PATH`, so CI (which sets the var) can never pass vacuously.
pub fn skip_unless_live_grammar_e2e(test_name: &str) -> bool {
    use std::process::Command;

    let require_live = std::env::var(REQUIRE_LIVE_E2E_ENV).is_ok_and(|v| v == "1");
    if !require_live {
        eprintln!("{test_name}: skipping (set {REQUIRE_LIVE_E2E_ENV}=1 to run live e2e)");
        return true;
    }

    let has_tool = |name: &str| {
        Command::new(name)
            .arg("--version")
            .output()
            .is_ok_and(|o| o.status.success())
    };
    let has_git = has_tool("git");
    let has_curl = has_tool("curl");
    let has_ts = has_tool("tree-sitter");
    if !has_git || !has_curl || !has_ts {
        panic!("{REQUIRE_LIVE_E2E_ENV}=1 but git={has_git} curl={has_curl} tree-sitter={has_ts}");
    }
    false
}
