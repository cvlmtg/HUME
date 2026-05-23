fn main() {
    let manifest = std::env::var("CARGO_MANIFEST_DIR")
        .expect("CARGO_MANIFEST_DIR set by cargo");
    // editor/ is one level below the workspace root where .git/ lives
    let workspace = std::path::Path::new(&manifest)
        .parent()
        .expect("manifest dir has a parent");

    let sha = std::process::Command::new("git")
        .args(["rev-parse", "--short", "HEAD"])
        .current_dir(workspace)
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "unknown".into());
    println!("cargo:rustc-env=HUME_GIT_SHA={sha}");

    // Resolve the real git-dir paths via `--git-path` so the rerun-if-changed
    // directives work correctly in git worktrees and submodules (where `.git`
    // is a file not a directory).  Only emit a path when it actually exists on
    // disk; an absent logs/HEAD (e.g. reflog disabled in CI) is fine — HEAD
    // alone is sufficient to detect commit/checkout changes.
    for arg in ["HEAD", "logs/HEAD"] {
        if let Some(resolved) = git_path(workspace, arg) {
            println!("cargo:rerun-if-changed={}", resolved.display());
        }
    }
}

/// Run `git rev-parse --git-path <arg>` in `cwd` and return the resolved path
/// if it exists on disk.  Trims whitespace; makes relative paths absolute by
/// joining against `cwd`.
fn git_path(cwd: &std::path::Path, arg: &str) -> Option<std::path::PathBuf> {
    let out = std::process::Command::new("git")
        .args(["rev-parse", "--git-path", arg])
        .current_dir(cwd)
        .output()
        .ok()
        .filter(|o| o.status.success())?;
    let raw = String::from_utf8_lossy(&out.stdout);
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return None;
    }
    let path = std::path::Path::new(trimmed);
    let abs = if path.is_absolute() {
        path.to_path_buf()
    } else {
        cwd.join(path)
    };
    if abs.exists() { Some(abs) } else { None }
}
