//! Shell builtins for HUME's Steel scripting engine.
//!
//! Exposes a narrow, auditable surface — only `git clone` and `git pull`.
//! No generic process runner is provided.
//!
//! Both operations are sandboxed: the destination / working directory must
//! resolve to a canonical path inside `<data>/plugins/`.  Canonicalize
//! failures are treated as hard errors (never fallbacks).

use std::path::PathBuf;

use steel::rvals::SteelVal;
use steel::rerrs::SteelErr;

use crate::editor::Severity;
use crate::scripting::SteelCtx;

// ── git-clone ─────────────────────────────────────────────────────────────────

/// `(git-clone url dest)` — clone `url` into the directory `dest`.
///
/// `dest` must be inside `<data>/plugins/`.  The parent of `dest` must exist;
/// `git` will create `dest` itself (mirroring normal `git clone` behaviour).
///
/// On success, returns `#<void>`.  On failure (git not found, non-zero exit,
/// sandbox violation), raises a Steel error.
pub(crate) fn git_clone(ctx: &mut SteelCtx, url: String, dest: String) -> Result<SteelVal, SteelErr> {
    let dest_path = PathBuf::from(&dest);

    // The destination doesn't exist yet (git creates it). Sandbox-check the
    // parent — which must exist — then verify the full path has no `..`.
    if super::fs::has_dotdot(&dest_path) {
        steel::stop!(Generic => "git-clone: dest must not contain '..' components: {dest}");
    }
    let parent = dest_path.parent().ok_or_else(|| SteelErr::new(
        steel::rerrs::ErrorKind::Generic,
        format!("git-clone: dest has no parent directory: {dest}"),
    ))?;

    let canonical_parent = crate::os::fs::canonicalize(parent)
        .map_err(|e| SteelErr::new(steel::rerrs::ErrorKind::Generic,
            format!("git-clone: cannot resolve parent of '{dest}': {e}")))?;

    // file_name() is None for paths ending in "." (CurDir); has_dotdot() only
    // rejects ".." (ParentDir).  Hard-error rather than silently joining "".
    let file_name = dest_path.file_name().ok_or_else(|| SteelErr::new(
        steel::rerrs::ErrorKind::Generic,
        format!("git-clone: dest has no file name component: {dest}"),
    ))?;
    sandbox_write_check(&canonical_parent.join(file_name), &dest)?;

    // Log which git we'll invoke — useful for debugging.
    ctx.pending_messages.push((Severity::Trace, format!("git-clone: running `git clone {url} {dest}`")));

    let status = crate::os::process::git_clone(&url, &dest)
        .map_err(|e| SteelErr::new(steel::rerrs::ErrorKind::Generic,
            format!("git-clone: cannot run git: {e}")))?;

    if !status.success() {
        steel::stop!(Generic =>
            "git-clone: `git clone {url}` failed with exit code {}",
            status.code().map_or_else(|| "unknown".to_string(), |c| c.to_string()));
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

    let canonical = crate::os::fs::canonicalize(&dir_path)
        .map_err(|e| SteelErr::new(steel::rerrs::ErrorKind::Generic,
            format!("git-pull: cannot resolve '{dir}': {e}")))?;

    sandbox_write_check(&canonical, &dir)?;

    ctx.pending_messages.push((Severity::Trace, format!("git-pull: running `git pull` in {dir}")));

    let status = crate::os::process::git_pull_in(&canonical)
        .map_err(|e| SteelErr::new(steel::rerrs::ErrorKind::Generic,
            format!("git-pull: cannot run git: {e}")))?;

    if !status.success() {
        steel::stop!(Generic =>
            "git-pull: `git pull` in '{dir}' failed with exit code {}",
            status.code().map_or_else(|| "unknown".to_string(), |c| c.to_string()));
    }
    Ok(SteelVal::Void)
}

// ── Internal helpers ──────────────────────────────────────────────────────────

/// Check that `canonical_path` is inside `<data>/plugins/`.
///
/// `raw` is the original unresolved path string, used only for error messages.
fn sandbox_write_check(canonical_path: &std::path::Path, raw: &str) -> Result<(), SteelErr> {
    super::fs::with_data_plugins(|sandbox| {
        if !canonical_path.starts_with(sandbox) {
            Err(SteelErr::new(steel::rerrs::ErrorKind::Generic, format!(
                "shell builtin: path '{raw}' is outside the write sandbox (<data>/plugins/)"
            )))
        } else {
            Ok(())
        }
    })?
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    fn setup(tmp: &TempDir) {
        let data_dir = tmp.path().join("hume");
        fs::create_dir_all(data_dir.join("plugins")).unwrap();
        super::super::fs::init_dirs(Some(data_dir), None);
    }

    #[test]
    fn git_clone_rejects_dest_outside_sandbox() {
        let tmp = TempDir::new().unwrap();
        setup(&tmp);

        // Parent is tmp root (outside sandbox), dest is tmp/evil.
        let dest = tmp.path().join("evil").to_string_lossy().to_string();
        let mut ctx = SteelCtx::for_testing();
        let err = git_clone(&mut ctx, "https://example.com/repo.git".into(), dest)
            .unwrap_err();
        assert!(err.to_string().contains("sandbox"), "expected sandbox error, got: {err}");
    }

    #[test]
    fn git_pull_rejects_dir_outside_sandbox() {
        let tmp = TempDir::new().unwrap();
        setup(&tmp);

        // Use the tmp root itself — it exists but is outside the sandbox.
        let dir = tmp.path().to_string_lossy().to_string();
        let mut ctx = SteelCtx::for_testing();
        let err = git_pull(&mut ctx, dir).unwrap_err();
        assert!(err.to_string().contains("sandbox"), "expected sandbox error, got: {err}");
    }

    #[test]
    fn git_clone_rejects_dotdot_in_dest() {
        let tmp = TempDir::new().unwrap();
        setup(&tmp);

        let dest = format!("{}/hume/plugins/user/../../../evil", tmp.path().display());
        let mut ctx = SteelCtx::for_testing();
        assert!(git_clone(&mut ctx, "https://example.com/repo.git".into(), dest).is_err());
    }

    // Note: Tests that actually run `git clone` / `git pull` would require a
    // local bare repository fixture.  These live in the integration test suite
    // (tests/scripting/plum.rs) where we control the git setup.
}
