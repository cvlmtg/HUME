//! Grammar fixture paths shared by this crate's tests.
//!
//! Duplicated from `hume-editor`'s equivalent test helpers rather than
//! shared across the crate boundary — both crates sit directly under the
//! workspace root, so `CARGO_MANIFEST_DIR`'s parent resolves to the same
//! repo root either way. Fixtures are installed by
//! `scripts/fetch-test-grammars.sh`.

use std::path::PathBuf;

/// Absolute path to the pre-built grammar shared library for `name`.
///
/// Callers that require the file to exist should check or load it immediately
/// after calling this — the helper does not verify presence.
pub(crate) fn grammar_parser_path(name: &str) -> PathBuf {
    let suffix = if cfg!(target_os = "macos") {
        "dylib"
    } else if cfg!(windows) {
        "dll"
    } else {
        "so"
    };
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .join("tests/fixtures/grammars")
        .join(name)
        .join(format!("parser.{suffix}"))
}

/// Subpath within the cloned grammar repo holding its `queries/` and `src/`
/// (`None` for single-grammar repos; `Some` for monorepos like
/// tree-sitter-markdown, which holds `tree-sitter-markdown` and
/// `tree-sitter-markdown-inline` as subdirectories of one clone). Mirrors
/// the lookup in `scripts/fetch-test-grammars.sh`.
fn grammar_subpath(name: &str) -> Option<String> {
    let catalog = std::fs::read_to_string(
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .join("runtime/scheme/grammar-sources.scm"),
    )
    .ok()?;
    let needle = format!("(\"{name}\" ");
    let line = catalog
        .lines()
        .find(|l| l.trim_start().starts_with(&needle))?;
    let subpath = line.split('"').nth(9)?;
    (!subpath.is_empty()).then(|| subpath.to_owned())
}

fn grammar_fixture_root(name: &str) -> PathBuf {
    let base = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .join("tests/fixtures/grammars")
        .join(name);
    match grammar_subpath(name) {
        Some(sub) => base.join(sub),
        None => base,
    }
}

/// Absolute path to the highlights query file for `name`.
pub(crate) fn grammar_query_path(name: &str) -> PathBuf {
    grammar_fixture_root(name).join("queries/highlights.scm")
}

/// Absolute path to the injections query file for `name`, if the grammar
/// fixture ships one (`None` for grammars without embedded-language support).
pub(crate) fn grammar_injections_path(name: &str) -> Option<PathBuf> {
    let path = grammar_fixture_root(name).join("queries/injections.scm");
    path.exists().then_some(path)
}
