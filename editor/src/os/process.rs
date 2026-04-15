//! Process-spawning helpers.
//!
//! The only `std::process::Command` usage in the editor crate is launching
//! `git` for plugin management. Both entry points live here so that
//! `editor/src/os/` is the sole audit surface for process spawning.
//!
//! Sandbox enforcement (path prefix checks) is the caller's responsibility;
//! these functions only perform the spawn.

use std::io;
use std::path::Path;
use std::process::{Command, ExitStatus};

/// Run `git clone -- <url> <dest>` and return its exit status.
///
/// `url` is an untrusted string from Steel; the caller is responsible for
/// validating that `dest` resolves inside the write sandbox before calling this.
pub(crate) fn git_clone(url: &str, dest: &str) -> io::Result<ExitStatus> {
    Command::new("git")
        .args(["clone", "--", url, dest])
        .status()
}

/// Run `git pull` inside `dir` and return its exit status.
///
/// `dir` must already be canonicalized and sandbox-checked by the caller.
pub(crate) fn git_pull_in(dir: &Path) -> io::Result<ExitStatus> {
    Command::new("git")
        .arg("pull")
        .current_dir(dir)
        .status()
}
