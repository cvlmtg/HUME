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

#[cfg(unix)]
use std::os::unix::process::CommandExt as _;

/// Run `git clone -- <url> <dest>` and return captured output.
///
/// The caller is responsible for validating that `dest` resolves inside the
/// write sandbox before calling this.
pub fn git_clone(url: &str, dest: &Path) -> io::Result<Output> {
    Command::new("git")
        .args(["clone", "--", url])
        .arg(dest)
        .output()
}

/// Run `git pull` inside `dir` and return captured output.
///
/// `dir` must already be canonicalized and sandbox-checked by the caller.
pub fn git_pull_in(dir: &Path) -> io::Result<Output> {
    Command::new("git").arg("pull").current_dir(dir).output()
}

/// Clone `url` at the specific `rev` into `dest` using inherited stdio
/// (progress shown live in the terminal).
///
/// Uses `--filter=blob:none` (blobless partial clone) to avoid fetching all
/// file history.  `git_checkout` is called afterward to pin the exact revision.
pub fn git_clone_rev(url: &str, dest: &Path, rev: &str) -> io::Result<ExitStatus> {
    let status = Command::new("git")
        .args(["clone", "--filter=blob:none", "--", url])
        .arg(dest)
        .new_process_group()
        .status()?;
    if !status.success() {
        return Ok(status);
    }
    git_checkout(dest, rev)
}

/// Run `git checkout --force <rev>` inside `dir` with inherited stdio.
pub fn git_checkout(dir: &Path, rev: &str) -> io::Result<ExitStatus> {
    Command::new("git")
        .args(["-C"])
        .arg(dir)
        .args(["checkout", "--force", "--end-of-options", rev, "--"])
        .new_process_group()
        .status()
}

/// Fetch `url` to `dest` via `curl -fsSL` with inherited stdio.
///
/// `dest`'s parent directory must already exist before calling this.
pub fn curl_fetch(url: &str, dest: &Path) -> io::Result<ExitStatus> {
    Command::new("curl")
        .args(["-fsSL", "-o"])
        .arg(dest)
        .args(["--", url])
        .new_process_group()
        .status()
}

/// Compile a tree-sitter grammar source at `src` to a shared library at `out`
/// using `tree-sitter build`, with inherited stdio.
pub fn tree_sitter_build(src: &Path, out: &Path) -> io::Result<ExitStatus> {
    Command::new("tree-sitter")
        .args(["build", "-o"])
        .arg(out)
        .arg(src)
        .new_process_group()
        .status()
}

/// Convert a non-successful `ExitStatus` to a human-readable string for error
/// messages.
pub fn exit_code_str(status: ExitStatus) -> String {
    match status.code() {
        Some(c) => format!("exit code {c}"),
        None => "killed by signal".to_string(),
    }
}

// ── Internal helpers ──────────────────────────────────────────────────────────

/// Extension trait to set the child as its own process group leader on Unix.
///
/// On Unix: calls `setpgid(0, 0)` via `CommandExt::process_group(0)` so
/// Ctrl+C (SIGINT to the terminal's foreground process group) reaches only
/// the child, not HUME.  On other platforms this is a no-op.
trait NewProcessGroup {
    fn new_process_group(&mut self) -> &mut Self;
}

impl NewProcessGroup for Command {
    fn new_process_group(&mut self) -> &mut Self {
        #[cfg(unix)]
        self.process_group(0);
        self
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    /// Verify that Ctrl+C (SIGINT to the child's process group) kills the
    /// child but not HUME.
    ///
    /// Behavioral guarantee: after `process_group(0)` the child is its own
    /// process group leader, so `killpg(child_pid, SIGINT)` targets only that
    /// group.  If the test process survives past the assert the guarantee holds.
    ///
    /// `nix::killpg` is used instead of spawning `kill -INT -<pgid>` because
    /// BSD `kill` and util-linux `kill` disagree on negative-pgid argument
    /// parsing — the Linux version returned exit 0 without signalling, causing
    /// `sleep` to run to completion and the test to fail.
    #[test]
    #[cfg(unix)]
    fn sigint_to_child_group_does_not_kill_hume() {
        use nix::sys::signal::{Signal, killpg};
        use nix::unistd::{Pid, setpgid};
        use std::process::Command;

        // Spawn a long-lived child so we can signal it before it exits.
        let child = Command::new("sleep")
            .arg("30")
            .new_process_group()
            .spawn()
            .expect("spawn sleep");
        let pid = Pid::from_raw(i32::try_from(child.id()).expect("pid fits i32"));

        // `process_group(0)` calls setpgid(0,0) in the child's pre-exec hook,
        // which races with the parent.  Calling setpgid(child, child) from the
        // parent is idempotent and closes the race: if the child hasn't run its
        // hook yet we set it; if it already exec'd we get EACCES (the child set
        // it first) — either way the group is correct.
        let _ = setpgid(pid, pid);

        killpg(pid, Signal::SIGINT).expect("killpg");

        // Wait for the child — must have been killed by the signal.
        let exit = child.wait_with_output().expect("wait").status;
        assert!(
            !exit.success(),
            "child should have been killed by SIGINT, got: {exit:?}"
        );

        // Reaching here means HUME survived — the guarantee holds.
    }
}
