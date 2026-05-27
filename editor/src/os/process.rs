//! Process-spawning helpers.
//!
//! All `std::process::Command` usage in the editor crate lives here so that
//! `editor/src/os/` is the sole audit surface for process spawning.
//!
//! Sandbox enforcement (path prefix checks) is the caller's responsibility;
//! these functions only perform the spawn.
//!
//! ## Captured vs inherited stdio
//!
//! - **Captured** (`git_clone`, `git_pull_in`): returns `Output` so callers can
//!   surface stderr in error messages.
//! - **Inherited** (`git_clone_rev`, `git_checkout`, `curl_fetch`,
//!   `tree_sitter_build`): subprocess output flows directly to the terminal so
//!   the user sees live progress; returns `ExitStatus` only.

use std::io;
use std::path::Path;
use std::process::{Command, ExitStatus, Output};

/// Run `git clone -- <url> <dest>` and return captured output.
///
/// The caller is responsible for validating that `dest` resolves inside the
/// write sandbox before calling this.
pub(crate) fn git_clone(url: &str, dest: &Path) -> io::Result<Output> {
    Command::new("git")
        .args(["clone", "--", url])
        .arg(dest)
        .output()
}

/// Run `git pull` inside `dir` and return captured output.
///
/// `dir` must already be canonicalized and sandbox-checked by the caller.
pub(crate) fn git_pull_in(dir: &Path) -> io::Result<Output> {
    Command::new("git").arg("pull").current_dir(dir).output()
}

/// Clone `url` at the specific `rev` into `dest` using inherited stdio
/// (progress shown live in the terminal).
///
/// Uses `--filter=blob:none` (blobless partial clone) to avoid fetching all
/// file history.  `git_checkout` is called afterward to pin the exact revision.
pub(crate) fn git_clone_rev(url: &str, dest: &Path, rev: &str) -> io::Result<ExitStatus> {
    let status = Command::new("git")
        .args(["clone", "--filter=blob:none", "--", url])
        .arg(dest)
        .status()?;
    if !status.success() {
        return Ok(status);
    }
    git_checkout(dest, rev)
}

/// Run `git checkout --force <rev>` inside `dir` with inherited stdio.
pub(crate) fn git_checkout(dir: &Path, rev: &str) -> io::Result<ExitStatus> {
    Command::new("git")
        .args(["-C"])
        .arg(dir)
        .args(["checkout", "--force", "--end-of-options", rev, "--"])
        .status()
}

/// Fetch `url` to `dest` via `curl -fsSL` with inherited stdio.
///
/// `dest`'s parent directory must already exist before calling this.
pub(crate) fn curl_fetch(url: &str, dest: &Path) -> io::Result<ExitStatus> {
    Command::new("curl")
        .args(["-fsSL", "-o"])
        .arg(dest)
        .args(["--", url])
        .status()
}

/// Compile a tree-sitter grammar source at `src` to a shared library at `out`
/// using `tree-sitter build`, with inherited stdio.
pub(crate) fn tree_sitter_build(src: &Path, out: &Path) -> io::Result<ExitStatus> {
    Command::new("tree-sitter")
        .args(["build", "-o"])
        .arg(out)
        .arg(src)
        .status()
}

/// Convert a non-successful `ExitStatus` to a human-readable string for error
/// messages.
pub(crate) fn exit_code_str(status: ExitStatus) -> String {
    match status.code() {
        Some(c) => format!("exit code {c}"),
        None => "killed by signal".to_string(),
    }
}
