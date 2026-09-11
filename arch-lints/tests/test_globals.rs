//! # Test-suite process-global hygiene
//!
//! `editor/tests/mod.rs`'s `TestGlobals` (a reentrant lock) and its two
//! constructors, `safe_tempdir()`/`safe_named_tempfile()`, exist because a
//! bare `tempfile::tempdir()`/`NamedTempFile::new()` called while a
//! `HumeRuntimeGuard` has `TMPDIR` redirected can land inside — and later be
//! deleted along with — that guard's tree (see `safe_tempdir`'s own doc).
//! [`no_bare_tempdir_outside_the_safe_constructors`] enforces routing
//! through those constructors instead of a new one-off bypass, scanning for
//! `tempfile::tempdir()`/`tempfile::NamedTempFile::new(` anywhere in
//! `editor/tests/` except `tests/mod.rs` itself.
//!
//! This can't be a `clippy::disallowed_methods` entry the way the sibling
//! `std::env::set_var`/`remove_var` rule (`clippy.toml`) is: `tempfile::
//! tempdir`/`NamedTempFile::new` are legitimately called raw all over this
//! workspace — every other crate's own tests, and `main.rs`'s production
//! code — so a blanket disallow would need `#[allow]`s scattered across code
//! with no connection to this one test harness's `TMPDIR`-redirect hazard,
//! misrepresenting a narrow, one-suite rule as a project-wide ban. The
//! `TMPDIR`-mutating half of the same hazard (`std::env::set_var`/
//! `remove_var`) has no such legitimate use anywhere else in the tree today,
//! which is exactly why that half *did* move to `clippy.toml`.
//!
//! **Opt-out**: annotate the violation line (or the line above it, so
//! `cargo fmt` doesn't hoist a trailing comment) with
//! `// test-global-safe: <reason>` — for a genuinely new site that legitimately
//! needs a raw call (e.g. a fresh guard struct).

use arch_lints::{collect_all_rs, scan_forbidden, workspace_root};

fn test_tree_rs_files(workspace_root: &std::path::Path) -> Vec<std::path::PathBuf> {
    let scan_root = workspace_root.join("hume-editor/src/editor/tests");
    let mut paths = Vec::new();
    collect_all_rs(&scan_root, &mut paths);
    // `collect_all_rs` returns silently on a missing directory — a renamed
    // `tests/` tree (exactly what the hume-ui/hume-decorations split did to
    // `hume-editor/src/ui/`) would otherwise scan zero files and pass this
    // lint green forever.
    assert!(
        !paths.is_empty(),
        "no .rs files found under {} — this lint would silently check nothing",
        scan_root.display()
    );
    paths
}

/// Fail oracle: add `let dir = tempfile::tempdir().unwrap();` to any test
/// file other than `tests/mod.rs` — this test must fail naming that line.
#[test]
fn no_bare_tempdir_outside_the_safe_constructors() {
    let workspace_root = workspace_root();

    let forbidden: &[&str] = &["tempfile::tempdir()", "tempfile::NamedTempFile::new("];

    // `tests/mod.rs` owns `safe_tempdir()`/`safe_named_tempfile()`, the only
    // sanctioned callers of these raw constructors.
    let allowed_file = "hume-editor/src/editor/tests/mod.rs";

    let mut paths = test_tree_rs_files(&workspace_root);
    paths.retain(|p| {
        p.strip_prefix(&workspace_root)
            .unwrap_or(p)
            .to_string_lossy()
            .replace('\\', "/")
            != allowed_file
    });

    let violations: Vec<String> =
        scan_forbidden(&paths, &workspace_root, forbidden, "// test-global-safe:")
            .into_iter()
            .map(|v| format!("  {}:{} — {}", v.file, v.lineno, v.trimmed))
            .collect();

    assert!(
        violations.is_empty(),
        "\nBare tempdir/named-tempfile constructor found outside `tests/mod.rs`.\n\
         A `HumeRuntimeGuard`-redirected `TMPDIR` can engulf (and later delete) a\n\
         tempdir created while it's live — use `safe_tempdir()`/`safe_named_tempfile()`\n\
         instead, which serialize creation against that redirect.\n\
         Violations:\n{}\n",
        violations.join("\n")
    );
}
