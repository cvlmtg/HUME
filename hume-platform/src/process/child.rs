//! Shared subprocess transport: spawning with piped stdio under a fixed
//! process-group/reap discipline, plus the capped readers both consumption
//! shapes build on: [`crate::process::line_source`]'s line-batch streaming
//! and [`crate::process::job`]'s whole-output capture.

use std::io::{self, Read};
use std::process::{ChildStderr, ChildStdin, ChildStdout, Stdio};
use std::thread::{self, JoinHandle};

use crate::process::{Launch, ReapOnDrop, base_command, spawn_in_own_group, start_failed};
use crate::wake::{WakeCallback, spawn_waking};

/// Cap on captured stderr bytes: enough for a useful error message. Bytes
/// past the cap are still drained from the pipe (so a chatty child never
/// blocks writing to it) but discarded.
pub(crate) const STDERR_CAPTURE_CAP: usize = 8 * 1024;

/// Cap on captured job stdout ([`crate::process::job`]): large enough for any
/// realistic command's legitimate output, small enough that a runaway
/// child (`cat /dev/zero`, a multi-GB blob) can't grow the editor's memory
/// without bound. Unlike [`STDERR_CAPTURE_CAP`], exceeding this fails the
/// job outright ([`read_bounded`]) rather than silently keeping a prefix:
/// stdout is the caller's data, and a truncated batch would be
/// indistinguishable from an untruncated short one.
pub(crate) const JOB_STDOUT_CAP: usize = 64 * 1024 * 1024;

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
/// rest of the pipe so a chatty child never blocks on write. A read error
/// past `Interrupted` just stops the read early rather than failing it.
/// This is for diagnostic-only captures ([`STDERR_CAPTURE_CAP`]) where a
/// shorter-than-expected result is harmless; [`read_bounded`] is the
/// error-propagating counterpart for output a caller actually depends on.
pub(crate) fn read_capped(mut r: impl Read, cap: usize) -> Vec<u8> {
    let mut buf = [0u8; 4096];
    let mut captured = Vec::new();
    loop {
        match r.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => {
                if captured.len() < cap {
                    let remaining = cap - captured.len();
                    captured.extend_from_slice(&buf[..n.min(remaining)]);
                }
            }
            Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
            Err(_) => break,
        }
    }
    captured
}

/// Reads `r` to EOF, erroring instead of truncating: a real read error
/// propagates (`read_capped` would silently stop and keep the prefix), and
/// exceeding `limit` bytes fails the whole read rather than returning a
/// prefix indistinguishable from untruncated short output. Keeps draining
/// past `limit` (discarding the bytes) so the child's write doesn't block
/// on a full pipe while it finishes. The point is to bound this reader's
/// memory, not to stop the child from writing what it's going to write.
pub(crate) fn read_bounded(mut r: impl Read, limit: usize) -> io::Result<Vec<u8>> {
    let mut buf = [0u8; 4096];
    let mut captured = Vec::new();
    let mut overflowed = false;
    loop {
        match r.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => {
                if !overflowed && captured.len() + n > limit {
                    overflowed = true;
                }
                if !overflowed {
                    captured.extend_from_slice(&buf[..n]);
                }
            }
            Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
            Err(e) => return Err(e),
        }
    }
    if overflowed {
        return Err(io::Error::other(format!("output exceeded {limit} bytes")));
    }
    Ok(captured)
}

#[cfg(test)]
mod tests;
