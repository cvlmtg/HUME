// Editor-level tests for core:plum's theme install pipeline (themes.scm):
// slug validation, install/update sync, list, and remove.
//
// `:plum-install-theme`'s own `git clone` step hardcodes a
// `https://github.com/...` URL, so it is not reachable offline — only its
// validation-failure path is covered here (`install_theme_rejects_unsafe_slugs`).
// Everything after a clone (`plum/sync-theme-files!`, the update/list/remove
// commands) is covered against a real *local* git origin: `git clone`/`git
// pull` work identically against a local filesystem path, so these tests
// exercise the real subprocess pipeline with no network involved.

use std::path::{Path, PathBuf};

use super::*;
use crate::editor::Severity;

fn canonical_data_dir(root: &Path) -> PathBuf {
    root.canonicalize().unwrap().join("hume")
}

fn lock() -> ClaimGuard {
    TEST_GLOBALS.claim(Global::Env)
}

/// Mirrors `scripting_lsp_install.rs`'s helper of the same name.
fn load_with_init(ed: &mut Editor, data_dir: &Path, init_src: &str) {
    let repo_runtime_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .join("runtime");
    let config_tmp = safe_tempdir();
    let hume_config = config_tmp.path().join("hume");
    std::fs::create_dir_all(&hume_config).unwrap();
    std::fs::write(hume_config.join("init.scm"), init_src).unwrap();

    unsafe {
        std::env::set_var("XDG_CONFIG_HOME", config_tmp.path());
        std::env::set_var("HUME_RUNTIME", &repo_runtime_dir);
        std::env::set_var("XDG_DATA_HOME", data_dir);
    }
    ed.init_scripting(&mut Default::default());
    unsafe {
        std::env::remove_var("XDG_CONFIG_HOME");
        std::env::remove_var("HUME_RUNTIME");
        std::env::remove_var("XDG_DATA_HOME");
    }
}

/// Load the real `core:plum` plugin (plus its `core:stdlib` dependency).
fn load_plum(ed: &mut Editor, data_dir: &Path) {
    load_with_init(
        ed,
        data_dir,
        "(load-plugin \"core:stdlib\")\n(load-plugin \"core:plum\")",
    );
}

// ── Local git fixture helpers ─────────────────────────────────────────────────

/// `git -C dir <args>`, asserting success — test fixture plumbing only.
fn git(dir: &Path, args: &[&str]) {
    let status = std::process::Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .status()
        .expect("git must be on PATH for this test");
    assert!(status.success(), "git {args:?} failed in {dir:?}");
}

/// Write `files` (path relative to `dir` -> content) to disk, creating
/// parent directories as needed.
fn write_files(dir: &Path, files: &[(&str, &str)]) {
    for (rel, content) in files {
        let path = dir.join(rel);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, content).unwrap();
    }
}

/// `git add -A && git commit` with an explicit identity, so the commit
/// succeeds with no global git config in the test environment.
fn commit_all(dir: &Path, message: &str) {
    git(dir, &["add", "-A"]);
    git(
        dir,
        &[
            "-c",
            "user.email=test@hume.test",
            "-c",
            "user.name=HUME Test",
            "commit",
            "--quiet",
            "-m",
            message,
        ],
    );
}

/// `git init` at `dir`, write `files`, and commit them — a local origin a
/// test can `git clone`/`git pull` from with no network access.
fn init_theme_origin(dir: &Path, files: &[(&str, &str)]) {
    std::fs::create_dir_all(dir).unwrap();
    git(dir, &["init", "--quiet"]);
    write_files(dir, files);
    commit_all(dir, "themes");
}

/// Clone `origin` into `<data_dir>/themes/sources/<slug>` and copy its
/// `themes/*.toml` flat into `<data_dir>/themes/` — the on-disk state
/// `:plum-install-theme` itself would have produced, fabricated directly so
/// these tests don't depend on a live `github.com`.
fn fabricate_installed_theme_repo(data_dir_root: &Path, slug: &str, origin: &Path) {
    let data_dir = canonical_data_dir(data_dir_root);
    let src_dir = data_dir.join("themes/sources").join(slug);
    std::fs::create_dir_all(src_dir.parent().unwrap()).unwrap();
    let status = std::process::Command::new("git")
        .args(["clone", "--quiet"])
        .arg(origin)
        .arg(&src_dir)
        .status()
        .unwrap();
    assert!(status.success(), "fixture clone of {origin:?} failed");

    let themes_dir = data_dir.join("themes");
    std::fs::create_dir_all(&themes_dir).unwrap();
    for entry in std::fs::read_dir(src_dir.join("themes")).unwrap() {
        let entry = entry.unwrap();
        std::fs::copy(entry.path(), themes_dir.join(entry.file_name())).unwrap();
    }
}

// ── Slug validation ────────────────────────────────────────────────────────────

/// An unsafe or malformed "user/repo" slug is rejected before anything is
/// created on disk — no clone, no sources directory.
#[test]
fn install_theme_rejects_unsafe_slugs() {
    let _lock = lock();
    let data_tmp = safe_tempdir();

    for bad_slug in ["../evil", "a/b/c", "nope", "a/.."] {
        let mut ed = editor_from("-[x]>\n");
        load_plum(&mut ed, data_tmp.path());
        type_cmd(&mut ed, &format!(":plum-install-theme {bad_slug}"));

        // An uncaught Scheme `error` from a typed-command body is reported at
        // Error severity (message_log, not status_msg) — unlike plum's own
        // `log! 'info` confirmations, which are status_msg-only (Severity::Info
        // routing, see `message_log.rs`'s `EditorState::report`).
        let log = ed.state.message_log.format_for_display();
        assert!(
            log.contains("is not a valid"),
            "slug {bad_slug:?} must be rejected: {log}"
        );
    }

    let sources_dir = canonical_data_dir(data_tmp.path()).join("themes/sources");
    assert!(
        !sources_dir.exists(),
        "no clone should ever start for a rejected slug"
    );
}

// ── Update ──────────────────────────────────────────────────────────────────────

/// `:plum-update-themes` pulls each installed repo and re-syncs its copies:
/// a theme upstream drops is pruned, one it gains is copied in.
#[test]
fn update_themes_pulls_and_syncs_copies() {
    let _lock = lock();
    let data_tmp = safe_tempdir();
    let origin_tmp = safe_tempdir();
    let origin = origin_tmp.path().join("acme-theme.hume");
    init_theme_origin(
        &origin,
        &[(
            "themes/old.toml",
            "\"ui.cursor.primary\" = { fg = \"#111111\" }",
        )],
    );

    fabricate_installed_theme_repo(data_tmp.path(), "acme/theme.hume", &origin);

    // Upstream drops old.toml, adds new.toml.
    std::fs::remove_file(origin.join("themes/old.toml")).unwrap();
    write_files(
        &origin,
        &[(
            "themes/new.toml",
            "\"ui.cursor.primary\" = { fg = \"#222222\" }",
        )],
    );
    commit_all(&origin, "swap theme");

    let mut ed = editor_from("-[x]>\n");
    load_plum(&mut ed, data_tmp.path());
    type_cmd(&mut ed, ":plum-update-themes");

    let themes_dir = canonical_data_dir(data_tmp.path()).join("themes");
    assert!(
        !themes_dir.join("old.toml").exists(),
        "a theme dropped upstream must be pruned from the data-dir copy"
    );
    assert!(
        themes_dir.join("new.toml").exists(),
        "a theme added upstream must be copied into the data-dir"
    );

    // plum/batch-run's summary is `log! 'info` — status_msg only, never
    // message_log (Severity::Info routing).
    let status = ed.state.status_msg.as_deref().unwrap_or_default();
    assert!(
        status.contains("1 updated theme repo"),
        "expected an update summary in status_msg: {status:?}"
    );
    let errors: Vec<&str> = ed
        .state
        .message_log
        .entries()
        .filter(|e| e.severity == Severity::Error)
        .map(|e| e.text.as_str())
        .collect();
    assert!(
        errors.is_empty(),
        "update must not report an error: {errors:?}"
    );
}

/// A repo whose `themes/` directory disappears upstream fails `git pull`'s
/// sync step cleanly, and leaves the existing copies untouched rather than
/// half-pruning them.
#[test]
fn sync_errors_when_repo_has_no_themes_dir() {
    let _lock = lock();
    let data_tmp = safe_tempdir();
    let origin_tmp = safe_tempdir();
    let origin = origin_tmp.path().join("acme-theme.hume");
    init_theme_origin(
        &origin,
        &[(
            "themes/kept.toml",
            "\"ui.cursor.primary\" = { fg = \"#111111\" }",
        )],
    );

    fabricate_installed_theme_repo(data_tmp.path(), "acme/theme.hume", &origin);

    // Upstream removes the themes/ directory entirely.
    std::fs::remove_dir_all(origin.join("themes")).unwrap();
    write_files(&origin, &[("README.md", "no themes here anymore")]);
    commit_all(&origin, "drop themes");

    let mut ed = editor_from("-[x]>\n");
    load_plum(&mut ed, data_tmp.path());
    type_cmd(&mut ed, ":plum-update-themes");

    // The per-item error is `log! 'error` (message_log). plum/batch-run's
    // "N updated — M failed" summary is `log! 'info`, logged *before* the
    // per-item errors — Error severity also overwrites status_msg (see
    // `EditorState::report`), so the summary text is overwritten by the
    // error that follows it and isn't independently observable here; the
    // untouched `kept.toml` below is the behavioral proof the failed repo
    // didn't count as updated.
    let log = ed.state.message_log.format_for_display();
    assert!(
        log.contains("has no themes/*.toml"),
        "expected the sync error in the log: {log}"
    );

    let themes_dir = canonical_data_dir(data_tmp.path()).join("themes");
    assert!(
        themes_dir.join("kept.toml").exists(),
        "existing copies must survive a failed sync, not be half-pruned"
    );
}

// ── List ──────────────────────────────────────────────────────────────────────
//
// `:plum-list-themes` reports (via `log! 'info`, so only the *last* line
// survives as `status_msg` — see `EditorState::report`) one line per
// installed repo, then a separate line naming any unmanaged `.toml`. Split
// into two tests so each checks the one line guaranteed to be the final
// `status_msg`, rather than a middle line an overwrite would hide.

/// One installed repo, no unmanaged files: the per-repo line is the last
/// (and only) `log!` call, so it's exactly what `status_msg` holds after.
#[test]
fn list_themes_reports_installed_repo() {
    let _lock = lock();
    let data_tmp = safe_tempdir();
    let origin_tmp = safe_tempdir();
    let origin = origin_tmp.path().join("acme-theme.hume");
    init_theme_origin(
        &origin,
        &[
            (
                "themes/acme_dark.toml",
                "\"ui.cursor.primary\" = { fg = \"#111111\" }",
            ),
            (
                "themes/acme_light.toml",
                "\"ui.cursor.primary\" = { fg = \"#eeeeee\" }",
            ),
        ],
    );
    fabricate_installed_theme_repo(data_tmp.path(), "acme/theme.hume", &origin);

    let mut ed = editor_from("-[x]>\n");
    load_plum(&mut ed, data_tmp.path());
    type_cmd(&mut ed, ":plum-list-themes");

    let status = ed.state.status_msg.as_deref().unwrap_or_default();
    assert!(
        status.contains("acme/theme.hume: acme_dark, acme_light"),
        "expected the installed repo and its themes in status_msg: {status:?}"
    );
}

/// An unmanaged `.toml` with no installed repos at all: the unmanaged line
/// is logged last regardless, so it's what `status_msg` holds after.
#[test]
fn list_themes_reports_unmanaged_file() {
    let _lock = lock();
    let data_tmp = safe_tempdir();
    let themes_dir = canonical_data_dir(data_tmp.path()).join("themes");
    std::fs::create_dir_all(&themes_dir).unwrap();
    std::fs::write(
        themes_dir.join("hand_dropped.toml"),
        "\"ui.cursor.primary\" = { fg = \"#000000\" }",
    )
    .unwrap();

    let mut ed = editor_from("-[x]>\n");
    load_plum(&mut ed, data_tmp.path());
    type_cmd(&mut ed, ":plum-list-themes");

    let status = ed.state.status_msg.as_deref().unwrap_or_default();
    assert!(
        status.contains("PLUM unmanaged: hand_dropped"),
        "expected the unmanaged file called out in status_msg: {status:?}"
    );
}

// ── Remove ────────────────────────────────────────────────────────────────────

/// `:plum-remove-theme` deletes both the data-dir copies and the clone.
#[test]
fn remove_theme_deletes_copies_and_clone() {
    let _lock = lock();
    let data_tmp = safe_tempdir();
    let origin_tmp = safe_tempdir();
    let origin = origin_tmp.path().join("acme-theme.hume");
    init_theme_origin(
        &origin,
        &[(
            "themes/acme_dark.toml",
            "\"ui.cursor.primary\" = { fg = \"#111111\" }",
        )],
    );
    fabricate_installed_theme_repo(data_tmp.path(), "acme/theme.hume", &origin);

    let data_dir = canonical_data_dir(data_tmp.path());
    let themes_dir = data_dir.join("themes");
    let src_dir = data_dir.join("themes/sources/acme/theme.hume");
    assert!(themes_dir.join("acme_dark.toml").exists());
    assert!(src_dir.exists());

    let mut ed = editor_from("-[x]>\n");
    load_plum(&mut ed, data_tmp.path());
    type_cmd(&mut ed, ":plum-remove-theme acme/theme.hume");

    assert!(
        !themes_dir.join("acme_dark.toml").exists(),
        "removed theme's data-dir copy must be deleted"
    );
    assert!(!src_dir.exists(), "removed theme's clone must be deleted");

    // Single `log! 'info` call — status_msg only, see `EditorState::report`.
    let status = ed.state.status_msg.as_deref().unwrap_or_default();
    assert!(
        status.contains("removed acme/theme.hume: acme_dark"),
        "expected a removal confirmation in status_msg: {status:?}"
    );
}

/// Removing a repo that was never installed is a no-op, not an error.
#[test]
fn remove_theme_not_installed_is_a_noop() {
    let _lock = lock();
    let data_tmp = safe_tempdir();

    let mut ed = editor_from("-[x]>\n");
    load_plum(&mut ed, data_tmp.path());
    type_cmd(&mut ed, ":plum-remove-theme acme/theme.hume");

    let status = ed.state.status_msg.as_deref().unwrap_or_default();
    assert!(
        status.contains("acme/theme.hume is not installed"),
        "expected a not-installed notice in status_msg: {status:?}"
    );
    let errors: Vec<&str> = ed
        .state
        .message_log
        .entries()
        .filter(|e| e.severity == Severity::Error)
        .map(|e| e.text.as_str())
        .collect();
    assert!(
        errors.is_empty(),
        "removing an uninstalled repo must not error: {errors:?}"
    );
}
