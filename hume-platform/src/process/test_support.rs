//! Unix test helpers for processes, shared with consumer crates' tests
//! through the `test-util` feature.

use std::path::Path;

use nix::sys::signal::kill;
use nix::unistd::Pid;

/// Whether a process with this id exists. A zombie still counts.
pub fn is_alive(pid: u32) -> bool {
    let pid = i32::try_from(pid).expect("pid fits i32");
    kill(Pid::from_raw(pid), None).is_ok()
}

/// A `sh` command that logs `+` as it starts and `-` as it ends to `log`,
/// for a test of how many processes run at once.
pub fn logging_command(log: &Path) -> String {
    format!(
        "echo + >> '{log}'; sleep 0.1; echo - >> '{log}'",
        log = log.display()
    )
}

/// How many processes [`logging_command`] saw start, and the most that ran
/// at once.
pub fn starts_and_peak(log: &Path) -> (usize, i32) {
    let events = std::fs::read_to_string(log).expect("log written");
    let mut running = 0i32;
    let mut peak = 0;
    for line in events.lines() {
        running += if line == "+" { 1 } else { -1 };
        peak = peak.max(running);
    }
    (events.lines().filter(|&line| line == "+").count(), peak)
}

/// Writes a `sh` script named `name` into `dir` with permission bits
/// `mode`, and returns its path.
pub fn write_program(dir: &Path, name: &str, mode: u32) -> std::path::PathBuf {
    use std::os::unix::fs::PermissionsExt;
    let path = dir.join(name);
    std::fs::write(&path, b"#!/bin/sh\n").expect("program written");
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(mode))
        .expect("program mode set");
    path
}
