//! Shared subprocess transport: spawning with piped stdio under a fixed
//! process-group/reap discipline, plus the capped readers both consumption
//! shapes build on: [`crate::process::line_source`]'s line-batch streaming
//! and [`crate::process::job`]'s whole-output capture.

use std::collections::VecDeque;
use std::io::{self, Read};
use std::process::{ChildStderr, ChildStdin, ChildStdout, Stdio};
use std::thread::{self, JoinHandle};

use crate::process::{Launch, ReapOnDrop, base_command, spawn_in_own_group, start_failed};
use crate::wake::{WakeCallback, spawn_waking};

/// Cap on captured stderr bytes: enough for a useful error message. Past
/// it, [`read_capped`] keeps the first and last halves, so both a
/// program's first error and its closing summary survive. The bytes
/// between are still drained from the pipe (so a chatty child never blocks
/// writing to it) but discarded.
pub(crate) const STDERR_CAPTURE_CAP: usize = 8 * 1024;

/// Cap on the stdout of every whole-output capture, a job's
/// ([`crate::process::job`]) and `run-capture!`'s
/// ([`crate::process::run_capture`]) alike: large enough for any realistic
/// command's legitimate output, small enough that a runaway child
/// (`cat /dev/zero`, a multi-GB blob) can't grow the editor's memory without
/// bound, or keep the capture waiting on it forever. Unlike
/// [`STDERR_CAPTURE_CAP`], exceeding this fails the capture outright and
/// stops the child ([`read_bounded`]) rather than keeping a prefix: stdout
/// is the caller's data, and a truncated batch would be indistinguishable
/// from an untruncated short one.
pub(crate) const STDOUT_CAPTURE_CAP: usize = 64 * 1024 * 1024;

/// Spawns `cmd` with `args` (direct argv, no shell) in its own process
/// group, all three stdio streams piped, with `env` added to the inherited
/// environment. The child never inherits the
/// terminal as stdin: the caller feeds the returned stdin and closes it, or
/// closes it at once, so a read sees EOF rather than racing the editor's own
/// key reads (same contract as `hume_platform::process::run_capture`).
/// See `super::base_command`'s own doc for why `GIT_TERMINAL_PROMPT=0` is
/// set. Returns the
/// kill-on-early-return guard plus the piped stdio handles; the
/// caller starts its bridging threads before converting the guard into a
/// [`crate::process::tracked::TrackedChild`], so a thread failing to spawn
/// leaves nothing for the process to leak.
pub fn spawn_piped(
    cmd: &str,
    args: &[String],
    launch: Launch<'_>,
    env: &[(String, String)],
) -> io::Result<(ReapOnDrop, ChildStdin, ChildStdout, ChildStderr)> {
    let mut command = base_command(cmd, args, launch);
    command
        .envs(env.iter().map(|(k, v)| (k, v)))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = ReapOnDrop::new(spawn_in_own_group(&mut command)?);
    let stdin = child.get_mut().stdin.take().expect("piped stdin");
    let stdout = child.get_mut().stdout.take().expect("piped stdout");
    let stderr = child.get_mut().stderr.take().expect("piped stderr");
    Ok((child, stdin, stdout, stderr))
}

/// Starts a thread named `name` that bridges `program`'s stdio. A failure
/// to start it names `program`, as a failed spawn does.
pub fn spawn_bridge<T, F>(program: &str, name: &str, body: F) -> io::Result<JoinHandle<T>>
where
    F: FnOnce() -> T + Send + 'static,
    T: Send + 'static,
{
    thread::Builder::new()
        .name(name.into())
        .spawn(body)
        .map_err(|e| start_failed(program, e))
}

/// [`spawn_bridge`] for a thread that wakes the main loop, started through
/// [`spawn_waking`].
pub fn spawn_bridge_waking<T, F>(
    program: &str,
    name: &str,
    wake: WakeCallback,
    body: F,
) -> io::Result<JoinHandle<T>>
where
    F: FnOnce(&WakeCallback) -> T + Send + 'static,
    T: Send + 'static,
{
    spawn_waking(name, wake, body).map_err(|e| start_failed(program, e))
}

/// Reads `r` to EOF, retaining at most `cap` bytes but always draining the
/// rest of the pipe so a chatty child never blocks on write. A stream past
/// `cap` keeps its first `cap / 2` bytes and its last `cap - cap / 2`,
/// joined by a line naming how many bytes were cut. A read error past
/// `Interrupted` just stops the read early rather than failing it.
/// This is for diagnostic-only captures ([`STDERR_CAPTURE_CAP`]) where a
/// shorter-than-expected result is harmless; [`read_bounded`] is the
/// error-propagating counterpart for output a caller actually depends on.
pub(crate) fn read_capped(mut r: impl Read, cap: usize) -> Vec<u8> {
    let head_cap = cap / 2;
    let tail_cap = cap - head_cap;
    let mut buf = [0u8; 4096];
    let mut head = Vec::new();
    let mut tail = VecDeque::<u8>::new();
    let mut cut = 0;
    loop {
        match r.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => {
                let to_head = n.min(head_cap - head.len());
                head.extend_from_slice(&buf[..to_head]);
                tail.extend(&buf[to_head..n]);
                let excess = tail.len().saturating_sub(tail_cap);
                tail.drain(..excess);
                cut += excess;
            }
            Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
            Err(_) => break,
        }
    }
    if cut > 0 {
        head.extend_from_slice(format!("\n[… {cut} bytes cut …]\n").as_bytes());
    }
    head.extend(tail);
    head
}

/// Reads `r` to EOF, erroring instead of truncating: a real read error
/// propagates (`read_capped` would silently stop and keep the prefix), and
/// exceeding `limit` bytes fails the whole read rather than returning a
/// prefix indistinguishable from untruncated short output.
///
/// Both failures call `stop_writer` and stop reading at once. Nothing
/// reads the pipe after that, so a child left running would block on it,
/// or write forever when its output is endless, and a caller's wait for it
/// would never return.
pub(crate) fn read_bounded(
    r: impl Read,
    limit: usize,
    stop_writer: impl FnOnce(),
) -> io::Result<Vec<u8>> {
    let mut captured = Vec::new();
    let read = r.take(limit as u64 + 1).read_to_end(&mut captured);
    let failure = match read {
        Err(e) => e,
        Ok(_) if captured.len() > limit => {
            io::Error::other(format!("output exceeded {limit} bytes"))
        }
        Ok(_) => return Ok(captured),
    };
    stop_writer();
    Err(failure)
}

/// A whole-output capture of a child in progress: its stderr read on a
/// thread of its own, so a child filling its stderr pipe never blocks the
/// stdout read [`Self::finish`] does.
pub(crate) struct OutputCapture(JoinHandle<Vec<u8>>);

impl OutputCapture {
    /// Starts reading `stderr` through [`read_capped`].
    pub(crate) fn start(program: &str, stderr: ChildStderr) -> io::Result<Self> {
        spawn_bridge(program, "hume-capture-stderr", move || {
            read_capped(stderr, STDERR_CAPTURE_CAP)
        })
        .map(Self)
    }

    /// `stdout` read to EOF through [`read_bounded`], then the stderr read
    /// so far. A panicked stderr thread degrades to empty stderr rather
    /// than wedging the capture: the exit status still carries the failure,
    /// and stdout is what most callers want.
    pub(crate) fn finish(
        self,
        stdout: ChildStdout,
        stop_writer: impl FnOnce(),
    ) -> (io::Result<Vec<u8>>, Vec<u8>) {
        let stdout = read_bounded(stdout, STDOUT_CAPTURE_CAP, stop_writer);
        (stdout, self.0.join().unwrap_or_default())
    }
}

#[cfg(test)]
mod tests;
