//! Unix test helpers for processes, shared with consumer crates' tests
//! through the `test-util` feature.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use nix::sys::signal::kill;
use nix::unistd::Pid;

/// Whether a process with this id exists. A zombie still counts.
pub fn is_alive(pid: u32) -> bool {
    let pid = i32::try_from(pid).expect("pid fits i32");
    kill(Pid::from_raw(pid), None).is_ok()
}

/// A file a `sh` process waits for, so the test decides when it goes on
/// instead of a sleep racing the machine's load.
pub struct ShellGate(PathBuf);

impl ShellGate {
    /// A closed gate at `path`, a file that does not exist yet.
    pub fn new(path: impl Into<PathBuf>) -> Self {
        let path = path.into();
        assert!(!path.exists(), "a gate starts closed: {}", path.display());
        Self(path)
    }

    /// `sh` that returns once the gate is open.
    pub fn wait_sh(&self) -> String {
        format!("while [ ! -e '{}' ]; do sleep 0.01; done", self.0.display())
    }

    /// Lets every process waiting on the gate go on.
    pub fn open(&self) {
        std::fs::write(&self.0, "").expect("the gate's directory is writable");
    }
}

/// A `sh` command that logs `+` as it starts to `log`, waits for `gate`,
/// and logs `-` as it ends, for a test of how many processes run at once.
pub fn logging_command(log: &Path, gate: &ShellGate) -> String {
    format!(
        "echo + >> '{log}'; {wait}; echo - >> '{log}'",
        log = log.display(),
        wait = gate.wait_sh()
    )
}

/// Waits until `n` processes of [`logging_command`] have logged their start.
///
/// # Panics
/// Panics when they have not after 5 s, a process that never started.
pub fn wait_for_starts(log: &Path, n: usize) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while starts(log) < n {
        assert!(Instant::now() < deadline, "{n} processes never started");
        std::thread::sleep(Duration::from_millis(5));
    }
}

fn starts(log: &Path) -> usize {
    std::fs::read_to_string(log).map_or(0, |events| {
        events.lines().filter(|&line| line == "+").count()
    })
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

/// What a stderr capture keeps of `stream`, a stream longer than its 8 KiB
/// cap: the first and last 4 KiB around a line naming the bytes cut.
#[cfg(test)]
pub(crate) fn kept_stderr(stream: &str) -> String {
    const HALF: usize = 4 * 1024;
    let cut = stream.len() - 2 * HALF;
    format!(
        "{}\n[… {cut} bytes cut …]\n{}",
        &stream[..HALF],
        &stream[stream.len() - HALF..]
    )
}
