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
//
// plum reports through `log!`: `'info` reaches `status_msg` only, and each
// call overwrites the last, so a test asserts on whichever line is
// guaranteed to run last; `'error` reaches `message_log` instead (see
// `EditorState::report` in `message_log.rs`).

use std::path::{Path, PathBuf};

use super::*;
use crate::editor::Severity;

/// A theme file whose content no test asserts on — only its presence,
/// absence, or filename matters.
const THEME_TOML: &str = "\"ui.cursor.primary\" = { fg = \"#111111\" }";

/// Error-severity entries in `ed`'s log. plum's own confirmations are all
/// `log! 'info` (status_msg only), so anything here is a real failure.
fn error_log(ed: &Editor) -> Vec<&str> {
    ed.state
        .message_log
        .entries()
        .filter(|e| e.severity == Severity::Error)
        .map(|e| e.text.as_str())
        .collect()
}

/// Warning-severity entries in `ed`'s log.
fn warning_log(ed: &Editor) -> Vec<&str> {
    ed.state
        .message_log
        .entries()
        .filter(|e| e.severity == Severity::Warning)
        .map(|e| e.text.as_str())
        .collect()
}

fn plum_editor(data_dir: &Path) -> Editor {
    let mut ed = editor_from("-[x]>\n");
    load_plum(&mut ed, data_dir);
    ed
}

// ── Local git fixture helpers ─────────────────────────────────────────────────

/// Write `files` (path relative to `dir` -> content) to disk, creating
/// parent directories as needed.
fn write_files(dir: &Path, files: &[(&str, &str)]) {
    for (rel, content) in files {
        let path = dir.join(rel);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, content).unwrap();
    }
}

/// `git add -A && git commit`, relying on `init_theme_origin`'s `git_init`
/// for the commit identity.
fn commit_all(dir: &Path, message: &str) {
    git(dir, &["add", "-A"]);
    git(dir, &["commit", "--quiet", "-m", message]);
}

/// `git init` at `dir`, write `files`, and commit them — a local origin a
/// test can `git clone`/`git pull` from with no network access.
fn init_theme_origin(dir: &Path, files: &[(&str, &str)]) {
    std::fs::create_dir_all(dir).unwrap();
    git_init(dir);
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
    git(
        data_dir_root,
        &[
            "clone",
            "--quiet",
            origin.to_str().unwrap(),
            src_dir.to_str().unwrap(),
        ],
    );

    let themes_dir = data_dir.join("themes");
    std::fs::create_dir_all(&themes_dir).unwrap();
    for entry in std::fs::read_dir(src_dir.join("themes")).unwrap() {
        let entry = entry.unwrap();
        std::fs::copy(entry.path(), themes_dir.join(entry.file_name())).unwrap();
    }
}

/// A local origin holding `files`, cloned into the data dir as the installed
/// repo `acme/theme.hume` — the on-disk state every post-clone test starts
/// from. Both tempdirs must outlive the test (the clone in `data_tmp` points
/// back at `origin_tmp` for `git pull`).
fn installed_fixture(files: &[(&str, &str)]) -> (tempfile::TempDir, tempfile::TempDir, PathBuf) {
    let data_tmp = safe_tempdir();
    let origin_tmp = safe_tempdir();
    let origin = origin_tmp.path().join("acme-theme.hume");
    init_theme_origin(&origin, files);
    fabricate_installed_theme_repo(data_tmp.path(), "acme/theme.hume", &origin);
    (data_tmp, origin_tmp, origin)
}

// ── Slug validation ────────────────────────────────────────────────────────────

/// A malformed "user/repo" slug is a typo — reported at Info severity
/// (statusline only), matching `plum/resolve-grammar-arg`'s routing for a
/// bad grammar argument. A slug whose segments aren't safe path components
/// reaches `path-join`/`git clone`, so it stays loud in `:messages`. Neither
/// case starts a clone.
#[test]
fn install_theme_rejects_unsafe_slugs() {
    let _lock = lock();
    let data_tmp = safe_tempdir();
    let mut ed = plum_editor(data_tmp.path());

    let cases = [
        ("../evil", true),
        ("a/b/c", false),
        ("nope", false),
        ("a/..", true),
        ("c:evil/repo", true),
        ("a\"b/repo", true),
        ("a\\b/repo", true),
    ];
    for (bad_slug, loud) in cases {
        type_cmd(&mut ed, &format!(":plum-install-theme {bad_slug}"));
        if !loud {
            let status = ed.state.status_msg.as_deref().unwrap_or_default();
            assert!(
                status.contains("expected a \"user/repo\" slug"),
                "slug {bad_slug:?} must be rejected: {status:?}"
            );
        }
    }

    // An uncaught Scheme `error`'s message has each `"`/`\` backslash-escaped
    // on the way into the log (Rust's `char::escape_debug`), so a slug
    // containing one of those characters needs the same escaping applied
    // before it's searched for.
    let log = ed.state.message_log.format_for_display();
    for (bad_slug, _) in cases.into_iter().filter(|(_, loud)| *loud) {
        let escaped: String = bad_slug.chars().flat_map(char::escape_debug).collect();
        assert!(
            log.contains(&format!("\\\"{escaped}\\\" is not a valid")),
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
    let (data_tmp, _origin_tmp, origin) = installed_fixture(&[("themes/old.toml", THEME_TOML)]);

    // Upstream drops old.toml, adds new.toml.
    std::fs::remove_file(origin.join("themes/old.toml")).unwrap();
    write_files(&origin, &[("themes/new.toml", THEME_TOML)]);
    commit_all(&origin, "swap theme");

    let mut ed = plum_editor(data_tmp.path());
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

    let status = ed.state.status_msg.as_deref().unwrap_or_default();
    assert!(
        status.contains("1 updated theme repo"),
        "expected an update summary in status_msg: {status:?}"
    );
    assert!(
        error_log(&ed).is_empty(),
        "update must not report an error: {:?}",
        error_log(&ed)
    );
}

/// A repo whose `themes/` directory disappears upstream fails `git pull`'s
/// sync step cleanly, and leaves the existing copies untouched rather than
/// half-pruning them.
#[test]
fn sync_errors_when_repo_has_no_themes_dir() {
    let _lock = lock();
    let (data_tmp, _origin_tmp, origin) = installed_fixture(&[("themes/kept.toml", THEME_TOML)]);

    // Upstream removes the themes/ directory entirely.
    std::fs::remove_dir_all(origin.join("themes")).unwrap();
    write_files(&origin, &[("README.md", "no themes here anymore")]);
    commit_all(&origin, "drop themes");

    let mut ed = plum_editor(data_tmp.path());
    type_cmd(&mut ed, ":plum-update-themes");

    // The per-item error is `log! 'error` (message_log). plum/batch-run's
    // "N updated — M failed" summary is `log! 'info`, logged *before* the
    // per-item errors — Error severity also overwrites status_msg, so the
    // summary text is overwritten by the error that follows it and isn't
    // independently observable here; the untouched `kept.toml` below is the
    // behavioral proof the failed repo didn't count as updated.
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

/// Two repos shipping the same theme stem shadow each other in
/// `<data>/themes/` with no separate state file to notice — `plum/
/// sync-theme-files!` must at least warn instead of overwriting silently.
#[test]
fn update_themes_warns_on_shadowed_theme_name() {
    let _lock = lock();
    let data_tmp = safe_tempdir();

    let origin_a_tmp = safe_tempdir();
    let origin_a = origin_a_tmp.path().join("acme-dark.hume");
    init_theme_origin(&origin_a, &[("themes/dark.toml", THEME_TOML)]);
    fabricate_installed_theme_repo(data_tmp.path(), "acme/dark.hume", &origin_a);

    let origin_b_tmp = safe_tempdir();
    let origin_b = origin_b_tmp.path().join("zed-dark.hume");
    init_theme_origin(&origin_b, &[("themes/dark.toml", THEME_TOML)]);
    fabricate_installed_theme_repo(data_tmp.path(), "zed/dark.hume", &origin_b);

    let mut ed = plum_editor(data_tmp.path());
    type_cmd(&mut ed, ":plum-update-themes");

    let warnings = warning_log(&ed);
    assert!(
        warnings
            .iter()
            .any(|w| w.contains("dark")
                && (w.contains("acme/dark.hume") || w.contains("zed/dark.hume"))),
        "expected a shadowed-name warning naming \"dark\" and the other repo: {warnings:?}"
    );
}

// ── List ──────────────────────────────────────────────────────────────────────
//
// Split into two tests so each checks the one line guaranteed to be the
// final `status_msg`, rather than a middle line an overwrite would hide.

/// One installed repo, no unmanaged files: the per-repo line is the last
/// (and only) `log!` call, so it's exactly what `status_msg` holds after.
#[test]
fn list_themes_reports_installed_repo() {
    let _lock = lock();
    let (data_tmp, _origin_tmp, _origin) = installed_fixture(&[
        ("themes/acme_dark.toml", THEME_TOML),
        ("themes/acme_light.toml", THEME_TOML),
    ]);

    let mut ed = plum_editor(data_tmp.path());
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
    std::fs::write(themes_dir.join("hand_dropped.toml"), THEME_TOML).unwrap();

    let mut ed = plum_editor(data_tmp.path());
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
    let (data_tmp, _origin_tmp, _origin) =
        installed_fixture(&[("themes/acme_dark.toml", THEME_TOML)]);

    let data_dir = canonical_data_dir(data_tmp.path());
    let themes_dir = data_dir.join("themes");
    let src_dir = data_dir.join("themes/sources/acme/theme.hume");
    assert!(themes_dir.join("acme_dark.toml").exists());
    assert!(src_dir.exists());

    let mut ed = plum_editor(data_tmp.path());
    type_cmd(&mut ed, ":plum-remove-theme acme/theme.hume");

    assert!(
        !themes_dir.join("acme_dark.toml").exists(),
        "removed theme's data-dir copy must be deleted"
    );
    assert!(!src_dir.exists(), "removed theme's clone must be deleted");

    let status = ed.state.status_msg.as_deref().unwrap_or_default();
    assert!(
        status.contains("removed acme/theme.hume: acme_dark"),
        "expected a removal confirmation in status_msg: {status:?}"
    );
}

/// A repo that loses its `themes/` directory upstream (a failed
/// `:plum-update-themes` sync, per `sync_errors_when_repo_has_no_themes_dir`
/// above) must still be removable — discovery keys on the clone existing,
/// not on it still holding a `themes/` directory.
#[test]
fn remove_theme_survives_a_failed_sync() {
    let _lock = lock();
    let (data_tmp, _origin_tmp, origin) = installed_fixture(&[("themes/kept.toml", THEME_TOML)]);

    // Upstream drops themes/ entirely; the update fails but the clone stays.
    std::fs::remove_dir_all(origin.join("themes")).unwrap();
    write_files(&origin, &[("README.md", "no themes here anymore")]);
    commit_all(&origin, "drop themes");

    let mut ed = plum_editor(data_tmp.path());
    type_cmd(&mut ed, ":plum-update-themes");

    let src_dir = canonical_data_dir(data_tmp.path()).join("themes/sources/acme/theme.hume");
    assert!(src_dir.exists(), "the clone must survive a failed sync");

    type_cmd(&mut ed, ":plum-remove-theme acme/theme.hume");

    assert!(
        !src_dir.exists(),
        "remove must delete a repo whose sync failed, not report it as never installed"
    );
    let status = ed.state.status_msg.as_deref().unwrap_or_default();
    assert!(
        status.contains("removed acme/theme.hume"),
        "expected a removal confirmation in status_msg: {status:?}"
    );
}

/// Removing a repo that was never installed is a no-op, not an error.
#[test]
fn remove_theme_not_installed_is_a_noop() {
    let _lock = lock();
    let data_tmp = safe_tempdir();
    let mut ed = plum_editor(data_tmp.path());
    type_cmd(&mut ed, ":plum-remove-theme acme/theme.hume");

    let status = ed.state.status_msg.as_deref().unwrap_or_default();
    assert!(
        status.contains("acme/theme.hume is not installed"),
        "expected a not-installed notice in status_msg: {status:?}"
    );
    assert!(
        error_log(&ed).is_empty(),
        "removing an uninstalled repo must not error: {:?}",
        error_log(&ed)
    );
}
