//! Shell builtins for HUME's Steel scripting engine.
//!
//! Exposes a narrow, auditable surface — only git, curl, and npm wrappers.
//! No generic process runner is provided.
//!
//! All operations are sandboxed: destinations must resolve inside
//! `<data>/plugins/`, `<data>/grammars/`, or `<data>/servers/` depending on
//! the operation. Canonicalize failures are treated as hard errors (never
//! fallbacks).

use std::path::{Path, PathBuf};

use steel::rerrs::SteelErr;
use steel::rvals::SteelVal;

use crate::SteelCtx;
use crate::log::LogLevel;

use super::list_to_strings;

// ── Sandbox kind ─────────────────────────────────────────────────────────────

pub(crate) enum SandboxKind {
    Plugins,
    Grammars,
    /// `<data>/servers/` — LSP server install operations (npm installs,
    /// archive unpacking).
    Servers,
    /// `<data>/grammars/` ∪ `<data>/servers/` — operations shared by both
    /// artifact classes (`curl-fetch`, `write-file`, `delete-file`).
    Install,
}

// ── validate_new_path ────────────────────────────────────────────────────────

/// Validate that `dest` is a safe, new path inside the requested sandbox.
///
/// Checks (in order):
/// 1. No `..` components.
/// 2. Has a parent that exists and canonicalizes.
/// 3. Has a file-name component (`file_name()` is None for paths ending in `.`).
/// 4. `<canonical-parent>/<file-name>` is inside the requested sandbox root.
/// 5. `dest` is not a symlink (symlinks rejected to prevent TOCTOU escapes).
///
/// Returns the canonical destination path so callers can pass it directly to
/// subprocesses, closing the TOCTOU window between the sandbox check and the
/// spawn.
pub(crate) fn validate_new_path(
    dest: &Path,
    fn_name: &str,
    kind: SandboxKind,
) -> Result<PathBuf, SteelErr> {
    if super::sandbox::has_dotdot(dest) {
        steel::stop!(Generic =>
            "{fn_name}: dest must not contain '..' components: {}", dest.display());
    }
    let parent = dest.parent().ok_or_else(|| {
        SteelErr::new(
            steel::rerrs::ErrorKind::Generic,
            format!(
                "{fn_name}: dest has no parent directory: {}",
                dest.display()
            ),
        )
    })?;
    let canonical_parent = hume_platform::fs::canonicalize(parent).map_err(|e| {
        SteelErr::new(
            steel::rerrs::ErrorKind::Generic,
            format!(
                "{fn_name}: cannot resolve parent of '{}': {e}",
                dest.display()
            ),
        )
    })?;
    // file_name() is None for paths ending in "." (CurDir); has_dotdot() only
    // rejects ".." (ParentDir).  Hard-error rather than silently joining "".
    let file_name = dest.file_name().ok_or_else(|| {
        SteelErr::new(
            steel::rerrs::ErrorKind::Generic,
            format!(
                "{fn_name}: dest has no file name component (path ends with '.'?): {}",
                dest.display()
            ),
        )
    })?;
    let canonical_dest = canonical_parent.join(file_name);
    match hume_platform::fs::symlink_metadata(&canonical_dest) {
        Ok(meta) if meta.file_type().is_symlink() => {
            steel::stop!(Generic =>
                "{fn_name}: dest is a symlink (refusing to follow): {}", dest.display());
        }
        // Path does not yet exist — that is the expected case for a new dest.
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        // Any other error (EACCES, EIO, …) means we cannot verify safety.
        Err(e) => {
            steel::stop!(Generic =>
                "{fn_name}: cannot stat dest '{}': {e}", dest.display());
        }
        Ok(_) => {} // exists and is not a symlink — fine
    }
    sandbox_write_check(&canonical_dest, &dest.to_string_lossy(), kind)?;
    Ok(canonical_dest)
}

// ── git-clone ─────────────────────────────────────────────────────────────────

/// `(git-clone url dest)` — clone `url` into the directory `dest`.
///
/// `dest` must be inside `<data>/plugins/`.  The parent of `dest` must exist;
/// `git` will create `dest` itself (mirroring normal `git clone` behaviour).
///
/// On success, returns `#<void>`.  On failure (git not found, non-zero exit,
/// sandbox violation), raises a Steel error.
pub(crate) fn git_clone(
    ctx: &mut SteelCtx,
    url: String,
    dest: String,
) -> Result<SteelVal, SteelErr> {
    let dest_path = validate_new_path(&PathBuf::from(&dest), "git-clone", SandboxKind::Plugins)?;

    ctx.log(
        LogLevel::Trace,
        format!("git-clone: running `git clone {url} {dest}`"),
    );

    let output = hume_platform::process::git_clone(&url, &dest_path).map_err(|e| {
        SteelErr::new(
            steel::rerrs::ErrorKind::Generic,
            format!("git-clone: cannot run git: {e}"),
        )
    })?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        steel::stop!(Generic =>
            "git-clone: `git clone {url}` failed ({}): {}",
            hume_platform::process::exit_code_str(output.status),
            stderr.trim());
    }
    Ok(SteelVal::Void)
}

// ── git-pull ──────────────────────────────────────────────────────────────────

/// `(git-pull dir)` — run `git pull` inside the existing directory `dir`.
///
/// `dir` must be inside `<data>/plugins/` and must exist.  Canonicalize
/// failure is a hard error.
///
/// On success, returns `#<void>`.  On failure raises a Steel error.
pub(crate) fn git_pull(ctx: &mut SteelCtx, dir: String) -> Result<SteelVal, SteelErr> {
    let dir_path = PathBuf::from(&dir);

    let canonical = hume_platform::fs::canonicalize(&dir_path).map_err(|e| {
        SteelErr::new(
            steel::rerrs::ErrorKind::Generic,
            format!("git-pull: cannot resolve '{dir}': {e}"),
        )
    })?;

    sandbox_write_check(&canonical, &dir, SandboxKind::Plugins)?;

    ctx.log(
        LogLevel::Trace,
        format!("git-pull: running `git pull` in {dir}"),
    );

    let output = hume_platform::process::git_pull_in(&canonical).map_err(|e| {
        SteelErr::new(
            steel::rerrs::ErrorKind::Generic,
            format!("git-pull: cannot run git: {e}"),
        )
    })?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        steel::stop!(Generic =>
            "git-pull: `git pull` in '{dir}' failed ({}): {}",
            hume_platform::process::exit_code_str(output.status),
            stderr.trim());
    }
    Ok(SteelVal::Void)
}

// ── git-clone-rev ─────────────────────────────────────────────────────────────

/// `(git-clone-rev url dest rev)` — blobless-clone `url` at `rev` into `dest`.
///
/// `dest` must be inside `<data>/grammars/`.  Subprocess output is inherited
/// so the user sees live progress (inline-output bracket).  On failure the
/// partial clone is removed before raising a Steel error.
pub(crate) fn git_clone_rev(
    ctx: &mut SteelCtx,
    url: String,
    dest: String,
    rev: String,
) -> Result<SteelVal, SteelErr> {
    let dest_path = validate_new_path(
        &PathBuf::from(&dest),
        "git-clone-rev",
        SandboxKind::Grammars,
    )?;

    ctx.log(
        LogLevel::Trace,
        format!("git-clone-rev: cloning {url} @ {rev} → {dest}"),
    );

    let status = hume_platform::process::git_clone_rev(&url, &dest_path, &rev).map_err(|e| {
        SteelErr::new(
            steel::rerrs::ErrorKind::Generic,
            format!("git-clone-rev: cannot run git: {e}"),
        )
    })?;

    if !status.success() {
        // Remove partial clone so a retry starts clean.
        let _ = hume_platform::fs::remove_dir_all(&dest_path);
        steel::stop!(Generic =>
            "git-clone-rev: clone of {url} @ {rev} failed ({})",
            hume_platform::process::exit_code_str(status));
    }
    Ok(SteelVal::Void)
}

// ── curl-fetch ────────────────────────────────────────────────────────────────

/// `(curl-fetch url dest)` — download `url` to the file `dest` via curl.
///
/// `dest` must be inside `<data>/grammars/` or `<data>/servers/`.  The parent
/// directory is created if absent.  On failure the partial output file is
/// removed.
pub(crate) fn curl_fetch(
    ctx: &mut SteelCtx,
    url: String,
    dest: String,
) -> Result<SteelVal, SteelErr> {
    let dest_path = validate_new_path(&PathBuf::from(&dest), "curl-fetch", SandboxKind::Install)?;

    // Create parent after the sandbox check so we don't mkdir outside the sandbox.
    if let Some(parent) = dest_path.parent() {
        hume_platform::fs::create_dir_all(parent).map_err(|e| {
            SteelErr::new(
                steel::rerrs::ErrorKind::Generic,
                format!("curl-fetch: cannot create parent directory for '{dest}': {e}"),
            )
        })?;
    }

    ctx.log(LogLevel::Trace, format!("curl-fetch: {url} → {dest}"));

    let status = hume_platform::process::curl_fetch(&url, &dest_path).map_err(|e| {
        SteelErr::new(
            steel::rerrs::ErrorKind::Generic,
            format!("curl-fetch: cannot run curl: {e}"),
        )
    })?;

    if !status.success() {
        let _ = hume_platform::fs::remove_file(&dest_path);
        steel::stop!(Generic =>
            "curl-fetch: download of {url} failed ({})",
            hume_platform::process::exit_code_str(status));
    }
    Ok(SteelVal::Void)
}

// ── npm-install! ───────────────────────────────────────────────────────────────

/// `(npm-install! dest packages)` — run `npm install --ignore-scripts
/// --prefix <dest> -- <packages…>`.
///
/// `dest` must be inside `<data>/servers/`. `packages` must be a non-empty
/// list of strings, none starting with `-` (defense against argument
/// injection on top of the `--` separator). node/npm preflighting
/// (`exe-on-path?`) is the caller's job — this wrapper stays narrow.
///
/// On failure, raises a Steel error; no cleanup (the receipt mechanism
/// covers interrupted installs).
pub(crate) fn npm_install(
    ctx: &mut SteelCtx,
    dest: String,
    packages_val: SteelVal,
) -> Result<SteelVal, SteelErr> {
    let dest_path = validate_new_path(&PathBuf::from(&dest), "npm-install!", SandboxKind::Servers)?;

    let packages = list_to_strings(packages_val, "npm-install! packages")?;
    if packages.is_empty() {
        steel::stop!(Generic => "npm-install!: packages must be a non-empty list");
    }
    if let Some(bad) = packages.iter().find(|p| p.starts_with('-')) {
        steel::stop!(Generic =>
            "npm-install!: package name must not start with '-': {}", bad);
    }

    ctx.log(
        LogLevel::Trace,
        format!("npm-install!: {} → {dest}", packages.join(" ")),
    );

    let status = hume_platform::process::npm_install(&dest_path, &packages).map_err(|e| {
        SteelErr::new(
            steel::rerrs::ErrorKind::Generic,
            format!("npm-install!: cannot run npm: {e}"),
        )
    })?;

    if !status.success() {
        steel::stop!(Generic =>
            "npm-install!: install of {} failed ({})",
            packages.join(" "),
            hume_platform::process::exit_code_str(status));
    }
    Ok(SteelVal::Void)
}

// ── Internal helpers ──────────────────────────────────────────────────────────

/// Check that `canonical_path` is inside the appropriate sandbox root.
///
/// `raw` is the original unresolved path string, used only for error messages.
fn sandbox_write_check(
    canonical_path: &std::path::Path,
    raw: &str,
    kind: SandboxKind,
) -> Result<(), SteelErr> {
    match kind {
        SandboxKind::Plugins => super::sandbox::with_data_plugins(|sandbox| {
            if !canonical_path.starts_with(sandbox) {
                Err(SteelErr::new(
                    steel::rerrs::ErrorKind::Generic,
                    format!(
                        "shell builtin: path '{raw}' is outside the write sandbox (<data>/plugins/)"
                    ),
                ))
            } else {
                Ok(())
            }
        })?,
        SandboxKind::Grammars => super::sandbox::with_data_grammars(|sandbox| {
            if !canonical_path.starts_with(sandbox) {
                Err(SteelErr::new(
                    steel::rerrs::ErrorKind::Generic,
                    format!(
                        "shell builtin: path '{raw}' is outside the write sandbox (<data>/grammars/)"
                    ),
                ))
            } else {
                Ok(())
            }
        })?,
        SandboxKind::Servers => super::sandbox::with_data_servers(|sandbox| {
            if !canonical_path.starts_with(sandbox) {
                Err(SteelErr::new(
                    steel::rerrs::ErrorKind::Generic,
                    format!(
                        "shell builtin: path '{raw}' is outside the write sandbox (<data>/servers/)"
                    ),
                ))
            } else {
                Ok(())
            }
        })?,
        SandboxKind::Install => {
            if super::sandbox::is_under_install_sandbox(canonical_path) {
                Ok(())
            } else {
                Err(SteelErr::new(
                    steel::rerrs::ErrorKind::Generic,
                    format!(
                        "shell builtin: path '{raw}' is outside the write sandbox (<data>/grammars/ or <data>/servers/)"
                    ),
                ))
            }
        }
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::SteelCtxTestHarness;
    use std::fs;
    use steel::rvals::IntoSteelVal;
    use tempfile::TempDir;

    fn setup(tmp: &TempDir) {
        let data_dir = tmp.path().join("hume");
        fs::create_dir_all(data_dir.join("plugins")).unwrap();
        fs::create_dir_all(data_dir.join("grammars/sources")).unwrap();
        // `init_dirs` also creates `<data>/servers/` (step 2: LSP installer).
        super::super::sandbox::init_dirs(Some(data_dir), None);
    }

    #[test]
    fn git_clone_rejects_dest_outside_sandbox() {
        let tmp = TempDir::new().unwrap();
        setup(&tmp);

        let dest = tmp.path().join("evil").to_string_lossy().to_string();
        let mut h = SteelCtxTestHarness::new();
        let mut ctx = h.ctx();
        let err = git_clone(&mut ctx, "https://example.com/repo.git".into(), dest).unwrap_err();
        assert!(
            err.to_string().contains("sandbox"),
            "expected sandbox error, got: {err}"
        );
    }

    #[test]
    fn git_pull_rejects_dir_outside_sandbox() {
        let tmp = TempDir::new().unwrap();
        setup(&tmp);

        let dir = tmp.path().to_string_lossy().to_string();
        let mut h = SteelCtxTestHarness::new();
        let mut ctx = h.ctx();
        let err = git_pull(&mut ctx, dir).unwrap_err();
        assert!(
            err.to_string().contains("sandbox"),
            "expected sandbox error, got: {err}"
        );
    }

    #[test]
    fn git_clone_rejects_dotdot_in_dest() {
        let tmp = TempDir::new().unwrap();
        setup(&tmp);

        let dest = format!("{}/hume/plugins/user/../../../evil", tmp.path().display());
        let mut h = SteelCtxTestHarness::new();
        let mut ctx = h.ctx();
        assert!(git_clone(&mut ctx, "https://example.com/repo.git".into(), dest).is_err());
    }

    #[test]
    fn git_clone_rev_rejects_dest_outside_grammars_sandbox() {
        let tmp = TempDir::new().unwrap();
        setup(&tmp);

        // Try to clone into plugins/ (wrong sandbox)
        let dest = format!("{}/hume/plugins/evil", tmp.path().display());
        let mut h = SteelCtxTestHarness::new();
        let mut ctx = h.ctx();
        let err = git_clone_rev(
            &mut ctx,
            "https://example.com/ts-json.git".into(),
            dest,
            "abc123".into(),
        )
        .unwrap_err();
        assert!(
            err.to_string().contains("sandbox"),
            "expected sandbox error, got: {err}"
        );
    }

    #[test]
    fn git_clone_rev_rejects_dotdot() {
        let tmp = TempDir::new().unwrap();
        setup(&tmp);

        let dest = format!(
            "{}/hume/grammars/sources/json/../../evil",
            tmp.path().display()
        );
        let mut h = SteelCtxTestHarness::new();
        let mut ctx = h.ctx();
        let err = git_clone_rev(
            &mut ctx,
            "https://example.com/ts-json.git".into(),
            dest,
            "abc123".into(),
        )
        .unwrap_err();
        assert!(
            err.to_string().contains(".."),
            "expected .. error, got: {err}"
        );
    }

    #[test]
    fn curl_fetch_rejects_dest_outside_grammars_sandbox() {
        let tmp = TempDir::new().unwrap();
        setup(&tmp);

        let dest = format!("{}/hume/plugins/queries.scm", tmp.path().display());
        let mut h = SteelCtxTestHarness::new();
        let mut ctx = h.ctx();
        let err = curl_fetch(&mut ctx, "https://example.com/hl.scm".into(), dest).unwrap_err();
        assert!(
            err.to_string().contains("sandbox"),
            "expected sandbox error, got: {err}"
        );
    }

    #[test]
    fn curl_fetch_accepts_servers_dest() {
        // curl-fetch's sandbox is the install union (grammars ∪ servers) as
        // of the LSP installer's step 2 — a servers/ dest must pass the
        // sandbox check even though the actual curl invocation will fail
        // (no network access in tests): a "no such host"/"curl" error, never
        // a sandbox rejection.
        let tmp = TempDir::new().unwrap();
        setup(&tmp);

        // Parent (`servers/`) already exists — `init_dirs` created it — so the
        // sandbox check runs against an existing, canonicalizable parent.
        let dest = format!("{}/hume/servers/rust-analyzer.gz", tmp.path().display());
        let mut h = SteelCtxTestHarness::new();
        let mut ctx = h.ctx();
        let err = curl_fetch(
            &mut ctx,
            "https://example.invalid/rust-analyzer.gz".into(),
            dest,
        )
        .unwrap_err();
        assert!(
            !err.to_string().contains("sandbox"),
            "expected a non-sandbox error (network/curl failure), got: {err}"
        );
    }

    #[test]
    #[cfg(not(windows))]
    fn git_clone_rev_rejects_symlink_dest() {
        let tmp = TempDir::new().unwrap();
        setup(&tmp);

        let outside = tmp.path().join("outside");
        fs::create_dir_all(&outside).unwrap();
        let link = tmp.path().join("hume/grammars/sources/evil");
        std::os::unix::fs::symlink(&outside, &link).unwrap();

        let dest = link.to_string_lossy().to_string();
        let mut h = SteelCtxTestHarness::new();
        let mut ctx = h.ctx();
        let err = git_clone_rev(
            &mut ctx,
            "https://example.com/ts-rust.git".into(),
            dest,
            "abc123".into(),
        )
        .unwrap_err();
        assert!(
            err.to_string().contains("symlink"),
            "expected symlink error, got: {err}"
        );
    }

    // ── npm-install! ─────────────────────────────────────────────────────────

    #[test]
    fn npm_install_rejects_dest_outside_servers_sandbox() {
        let tmp = TempDir::new().unwrap();
        setup(&tmp);

        let dest = format!("{}/hume/plugins/typescript-server", tmp.path().display());
        let mut h = SteelCtxTestHarness::new();
        let mut ctx = h.ctx();
        let packages = vec!["typescript-language-server@5.3.0".to_string()]
            .into_steelval()
            .unwrap();
        let err = npm_install(&mut ctx, dest, packages).unwrap_err();
        assert!(
            err.to_string().contains("sandbox"),
            "expected sandbox error, got: {err}"
        );
    }

    #[test]
    fn npm_install_rejects_empty_package_list() {
        let tmp = TempDir::new().unwrap();
        setup(&tmp);

        let dest = format!("{}/hume/servers/ts-server", tmp.path().display());
        let mut h = SteelCtxTestHarness::new();
        let mut ctx = h.ctx();
        let packages: SteelVal = Vec::<String>::new().into_steelval().unwrap();
        let err = npm_install(&mut ctx, dest, packages).unwrap_err();
        assert!(
            err.to_string().contains("non-empty"),
            "expected non-empty-list error, got: {err}"
        );
    }

    #[test]
    fn npm_install_rejects_package_starting_with_dash() {
        let tmp = TempDir::new().unwrap();
        setup(&tmp);

        let dest = format!("{}/hume/servers/ts-server", tmp.path().display());
        let mut h = SteelCtxTestHarness::new();
        let mut ctx = h.ctx();
        let packages = vec!["--ignore-scripts=false".to_string()]
            .into_steelval()
            .unwrap();
        let err = npm_install(&mut ctx, dest, packages).unwrap_err();
        assert!(
            err.to_string().contains("must not start with '-'"),
            "expected arg-injection rejection, got: {err}"
        );
    }

    #[test]
    #[cfg(not(windows))]
    fn curl_fetch_rejects_symlink_dest() {
        let tmp = TempDir::new().unwrap();
        setup(&tmp);

        let outside = tmp.path().join("outside.txt");
        fs::write(&outside, b"").unwrap();
        let link = tmp.path().join("hume/grammars/sources/rust/highlights.scm");
        fs::create_dir_all(link.parent().unwrap()).unwrap();
        std::os::unix::fs::symlink(&outside, &link).unwrap();

        let dest = link.to_string_lossy().to_string();
        let mut h = SteelCtxTestHarness::new();
        let mut ctx = h.ctx();
        let err = curl_fetch(&mut ctx, "https://example.com/hl.scm".into(), dest).unwrap_err();
        assert!(
            err.to_string().contains("symlink"),
            "expected symlink error, got: {err}"
        );
    }
}
