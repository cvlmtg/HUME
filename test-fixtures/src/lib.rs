//! Shared test infrastructure.
//!
//! [`testing`] holds the marker-annotated buffer/selection DSL
//! (`parse_state`/`serialize_state`/`assert_state!`) used by editing-command
//! tests. Everything below is grammar-fixture paths and fixture-presence
//! preconditions. `scripts/fetch-test-grammars.sh` installs each fixture
//! into `tests/fixtures/grammars/<name>/` the way PLUM installs a grammar:
//! `<name>.<ext>`, `queries/`, `manifest.scm` and `licenses/`, from the pinned
//! treesitter.hume release. It keeps each fetched archive in
//! [`fixture_archive_dir`], and packs them into a release-format bundle,
//! [`fixture_bundle_dir`].

#![deny(rustdoc::broken_intra_doc_links)]
#![deny(rustdoc::private_intra_doc_links)]

use std::path::{Path, PathBuf};

pub mod slow_delete;
pub mod testing;
pub mod unicode;

fn fixtures_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("test-fixtures sits directly under the workspace root")
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
        .join(format!("{name}.{}", grammar_platform_ext()))
}

/// Absolute path to the highlights query file for `name`.
pub fn grammar_query_path(name: &str) -> PathBuf {
    fixtures_root().join(name).join("queries/highlights.scm")
}

/// Where the query file `file` (e.g. `textobjects.scm`) for `name` lives,
/// whether or not the fixture has one: `queries/<file>`, Helix's query at
/// the release's Helix commit with its inherits resolved, as PLUM installs
/// it.
fn helix_query_path_unchecked(name: &str, file: &str) -> PathBuf {
    fixtures_root().join(name).join("queries").join(file)
}

/// Where the query file `file` (e.g. `textobjects.scm`) for `name` lives, or
/// `None` if the fixture has no such query.
pub fn helix_query_path(name: &str, file: &str) -> Option<PathBuf> {
    let path = helix_query_path_unchecked(name, file);
    path.exists().then_some(path)
}

/// Where the query file `file` for `name` lives, present.
///
/// # Panics
/// With [`require_fixture_file`]'s message when the fixture has no such
/// query for `name`.
pub fn require_helix_query(name: &str, file: &str) -> PathBuf {
    let path = helix_query_path_unchecked(name, file);
    require_fixture_file(&path, &format!("{name} queries/{file}"));
    path
}

/// Where Helix's indent test corpus file `file` (e.g. `rust.rs`, from
/// Helix's `tests/indent/`) for `name` lives: `helix-indent-corpus.<ext>`
/// beside the grammar, `<ext>` being `file`'s extension. Helix checks its
/// `indents.scm` against these files, so they hold the indentation the
/// Helix-maintained indents query expects.
///
/// # Panics
/// With [`require_fixture_file`]'s message when the fetch script found no
/// such corpus file for `name`.
pub fn require_helix_indent_corpus(name: &str, file: &str) -> PathBuf {
    let (_, ext) = file
        .rsplit_once('.')
        .expect("a corpus file name has an extension");
    let path = fixtures_root()
        .join(name)
        .join(format!("helix-indent-corpus.{ext}"));
    require_fixture_file(&path, &format!("{name} helix indent corpus for {file}"));
    path
}

/// The grammar bundle the fetch script packs from the fixtures:
/// `grammars-<platform>.tar` and its `.index`, in the release format, for a
/// test to serve as PLUM's release.
///
/// # Panics
/// With [`require_fixture_file`]'s message when the fetch script has not
/// built it.
pub fn fixture_bundle_dir() -> PathBuf {
    let dir = fixtures_root().join("bundle");
    require_fixture_file(&dir, "the local grammar bundle");
    dir
}

/// The folder of `<name>.tar.gz` archives the fetch script fetched, laid out
/// as `hume-grammar-build build` writes one, for a test to install from as
/// PLUM's local grammar source.
///
/// # Panics
/// With [`require_fixture_file`]'s message when the fetch script has not
/// fetched them.
pub fn fixture_archive_dir() -> PathBuf {
    let dir = fixtures_root().join("members");
    require_fixture_file(&dir, "the fixture grammar archives");
    dir
}

/// Copy `name`'s fixture, as the fetch script unpacked it, to `to`: the
/// contents of one release archive.
pub fn copy_fixture_grammar(to: &Path, name: &str) {
    fn copy_tree(from: &Path, to: &Path) {
        std::fs::create_dir_all(to).expect("create grammar dir");
        for entry in std::fs::read_dir(from).expect("read fixture dir") {
            let entry = entry.expect("read fixture entry");
            let target = to.join(entry.file_name());
            if entry.file_type().expect("fixture entry type").is_dir() {
                copy_tree(&entry.path(), &target);
            } else {
                std::fs::copy(entry.path(), &target).expect("copy fixture file");
            }
        }
    }
    copy_tree(&fixtures_root().join(name), to);
}

/// Install `name`'s fixture under `grammars_dir` as PLUM installs a grammar
/// under `<data>/grammars/`, as the one install `<name>/1/`, and return that
/// folder. Callers call [`require_grammars`] first.
pub fn install_fixture_grammar(grammars_dir: &Path, name: &str) -> PathBuf {
    let dir = grammars_dir.join(name).join("1");
    copy_fixture_grammar(&dir, name);
    dir
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
/// isn't a compiled grammar (e.g. an injections query).
pub fn require_fixture_file(path: &Path, what: &str) {
    if !path.exists() {
        panic!(
            "fixture missing: {what} ({}) — run scripts/fetch-test-grammars.sh \
             from the repo root",
            path.display()
        );
    }
}
