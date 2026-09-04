//! # Test-suite process-global hygiene
//!
//! `editor/tests/mod.rs`'s `TestGlobals` (a reentrant lock) and its two
//! constructors, `safe_tempdir()`/`safe_named_tempfile()`, exist because a
//! bare `tempfile::tempdir()`/`NamedTempFile::new()` called while a
//! `HumeRuntimeGuard` has `TMPDIR` redirected can land inside — and later be
//! deleted along with — that guard's tree (see `safe_tempdir`'s own doc).
//! Likewise, `HUME_RUNTIME`/`TMPDIR`/`XDG_*`/`HOME`/`PATH` are process-global
//! env vars: mutating one outside the small set of guard structs that
//! already claim `TEST_GLOBALS` for their lifetime races every other test
//! reading or writing the same var concurrently.
//!
//! Two lints enforce routing through that shared infrastructure instead of a
//! new one-off bypass:
//! - [`no_bare_tempdir_outside_the_safe_constructors`] — bare
//!   `tempfile::tempdir()`/`tempfile::NamedTempFile::new(` anywhere in
//!   `editor/tests/` except `tests/mod.rs` itself (where the two sanctioned
//!   constructors live).
//! - [`env_var_mutation_confined_to_guard_files`] — `std::env::set_var(`/
//!   `std::env::remove_var(` anywhere in `editor/tests/` except the handful
//!   of files that already own a guard struct or loader helper documented to
//!   require a caller-held `TEST_GLOBALS` claim.
//!
//! **Opt-out**: annotate the violation line (or the line above it, so
//! `cargo fmt` doesn't hoist a trailing comment) with
//! `// test-global-safe: <reason>` — for a genuinely new site that legitimately
//! needs a raw call (e.g. a fresh guard struct); update the file's own
//! allowlist below too, since the marker alone would leave the new file
//! unexamined for every *other* raw call it might add later.

use super::{collect_all_rs, scan_forbidden};

fn test_tree_rs_files(root: &std::path::Path) -> Vec<std::path::PathBuf> {
    let mut paths = Vec::new();
    collect_all_rs(&root.join("src/editor/tests"), &mut paths);
    paths
}

/// Fail oracle: add `let dir = tempfile::tempdir().unwrap();` to any test
/// file other than `tests/mod.rs` — this test must fail naming that line.
#[test]
fn no_bare_tempdir_outside_the_safe_constructors() {
    let manifest = std::env::var("CARGO_MANIFEST_DIR")
        .expect("CARGO_MANIFEST_DIR not set — run via `cargo test`");
    let root = std::path::Path::new(&manifest);

    let forbidden: &[&str] = &["tempfile::tempdir()", "tempfile::NamedTempFile::new("];

    // `tests/mod.rs` owns `safe_tempdir()`/`safe_named_tempfile()`, the only
    // sanctioned callers of these raw constructors.
    let allowed_file = "src/editor/tests/mod.rs";

    let mut paths = test_tree_rs_files(root);
    paths.retain(|p| {
        p.strip_prefix(root)
            .unwrap_or(p)
            .to_string_lossy()
            .replace('\\', "/")
            != allowed_file
    });

    let violations: Vec<String> = scan_forbidden(&paths, root, forbidden, "// test-global-safe:")
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

/// Fail oracle: add `unsafe { std::env::set_var("HUME_RUNTIME", ...) }` to
/// any test file not in `ALLOWED_ENV_MUTATORS` — this test must fail naming
/// that line.
#[test]
fn env_var_mutation_confined_to_guard_files() {
    let manifest = std::env::var("CARGO_MANIFEST_DIR")
        .expect("CARGO_MANIFEST_DIR not set — run via `cargo test`");
    let root = std::path::Path::new(&manifest);

    let forbidden: &[&str] = &["std::env::set_var(", "std::env::remove_var("];

    // Every file that owns a guard struct (or a loader helper documented to
    // require a caller-held `TEST_GLOBALS` claim) mutating `HUME_RUNTIME`,
    // `TMPDIR`, `XDG_CONFIG_HOME`, `XDG_DATA_HOME`, `HOME`, or `PATH`.
    const ALLOWED_ENV_MUTATORS: &[&str] = &[
        "src/editor/tests/mod.rs",
        "src/editor/tests/scripting_host_globals.rs",
        "src/editor/tests/settings_effects.rs",
        "src/editor/tests/unix/completion.rs",
        "src/editor/tests/unix/injections_editor.rs",
        "src/editor/tests/unix/mod.rs",
        "src/editor/tests/unix/plugins.rs",
        "src/editor/tests/unix/reload_config.rs",
        "src/editor/tests/unix/scripting_grammar.rs",
        "src/editor/tests/unix/scripting_lsp_install.rs",
        "src/editor/tests/unix/theme_dirs.rs",
    ];

    let mut paths = test_tree_rs_files(root);
    paths.retain(|p| {
        let rel = p
            .strip_prefix(root)
            .unwrap_or(p)
            .to_string_lossy()
            .replace('\\', "/");
        !ALLOWED_ENV_MUTATORS.contains(&rel.as_str())
    });

    let violations: Vec<String> = scan_forbidden(&paths, root, forbidden, "// test-global-safe:")
        .into_iter()
        .map(|v| format!("  {}:{} — {}", v.file, v.lineno, v.trimmed))
        .collect();

    assert!(
        violations.is_empty(),
        "\nRaw std::env::set_var/remove_var found outside the files that own a\n\
         TEST_GLOBALS-guarded env mutation. HUME_RUNTIME/TMPDIR/XDG_*/HOME/PATH are\n\
         process-global — route the mutation through an existing guard struct (or\n\
         EnvVarGuard) in one of those files, or add the new file to\n\
         ALLOWED_ENV_MUTATORS in this lint alongside its own TEST_GLOBALS claim.\n\
         Violations:\n{}\n",
        violations.join("\n")
    );
}
