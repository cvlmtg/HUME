//! Shared subprocess transport: spawning with piped stdio under a fixed
//! process-group/reap discipline, plus the wake-on-completion primitives
//! both consumption shapes build on — [`super::line_source`]'s line-batch
//! streaming and [`super::job`]'s whole-output capture. Extracted so the
//! two never duplicate the spawn/wake/reap machinery between them.

use std::io::{self, Read};
use std::path::Path;
use std::process::{ChildStderr, ChildStdout, Command, Stdio};
use std::sync::Arc;

use crate::path::strip_unc_prefix;
use crate::process::{ReapOnDrop, spawn_in_own_group};

/// Called by a background thread the moment it has something to hand off —
/// a line batch ([`super::line_source`]), or a finished capture
/// ([`super::job`]) — so the editor's main loop wakes and drains it instead
/// of rechecking on a poll cadence. Type-erased so this crate stays free of
/// a `hume-lsp`/`termina` dependency; production wraps
/// `termina::PlatformWaker::wake`.
pub type WakeCallback = Arc<dyn Fn() + Send + Sync>;

/// Invokes a [`WakeCallback`] on drop — fires whether the owning thread
/// exits normally or unwinds from a panic, so a dead source still wakes the
/// main loop once (the subsequent drain observes the disconnect via the
/// existing channel). Mirrors `hume-lsp::transport::WakeOnDrop`.
pub(crate) struct WakeOnDrop(pub(crate) WakeCallback);

impl Drop for WakeOnDrop {
    fn drop(&mut self) {
        (self.0)();
    }
}

/// Cap on captured stderr bytes: enough for a useful error message. Bytes
/// past the cap are still drained from the pipe (so a chatty child never
/// blocks writing to it) but discarded.
pub(crate) const STDERR_CAPTURE_CAP: usize = 8 * 1024;

/// Cap on captured job stdout ([`super::job`]): large enough for any
/// realistic command's legitimate output, small enough that a runaway
/// child (`cat /dev/zero`, a multi-GB blob) can't grow the editor's memory
/// without bound. Unlike [`STDERR_CAPTURE_CAP`], exceeding this fails the
/// job outright ([`read_bounded`]) rather than silently keeping a prefix —
/// stdout is the caller's data, and a truncated batch would be
/// indistinguishable from a genuinely short one.
pub(crate) const JOB_STDOUT_CAP: usize = 64 * 1024 * 1024;

/// Spawns `cmd` with `args` (direct argv, no shell) in its own process
/// group, all three stdio streams piped, and stdin closed immediately — the
/// child sees EOF on read rather than racing the editor's own key reads on
/// the terminal (same contract as PLUM's `plum/run!`). Returns the
/// kill-on-early-return guard plus the piped stdout/stderr handles; the
/// caller starts its bridging threads before converting the guard into a
/// [`crate::process::tracked::TrackedChild`] — a thread failing to spawn
/// leaves nothing for the process to leak.
pub(crate) fn spawn_piped(
    cmd: &str,
    args: &[String],
    cwd: Option<&Path>,
) -> io::Result<(ReapOnDrop, ChildStdout, ChildStderr)> {
    let mut command = Command::new(cmd);
    command.args(args);
    if let Some(dir) = cwd {
        command.current_dir(strip_unc_prefix(dir.to_path_buf()));
    }

    command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = ReapOnDrop::new(spawn_in_own_group(&mut command)?);

    // Non-inherited stdin: the child sees immediate EOF on read rather than
    // racing the editor's own key reads on the terminal (same contract as
    // PLUM's `plum/run!`).
    drop(child.get_mut().stdin.take());

    let stdout = child.get_mut().stdout.take().expect("piped stdout");
    let stderr = child.get_mut().stderr.take().expect("piped stderr");
    Ok((child, stdout, stderr))
}

/// Reads `r` to EOF, retaining at most `cap` bytes but always draining the
/// rest of the pipe so a chatty child never blocks on write. A read error
/// past `Interrupted` just stops the read early rather than failing it —
/// this is for diagnostic-only captures ([`STDERR_CAPTURE_CAP`]) where a
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
/// prefix indistinguishable from genuinely short output. Keeps draining
/// past `limit` (discarding the bytes) so the child's write doesn't block
/// on a full pipe while it finishes — the point is to bound this reader's
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
        return Err(io::Error::other(format!(
            "output exceeded {limit} bytes"
        )));
    }
    Ok(captured)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn read_capped_discards_bytes_past_the_cap_but_keeps_reading() {
        let data = b"hello world".repeat(10);
        let captured = read_capped(&data[..], 5);
        assert_eq!(captured, b"hello");
    }

    #[test]
    fn read_bounded_returns_everything_under_the_limit() {
        let captured = read_bounded(&b"hello"[..], 5).expect("under limit");
        assert_eq!(captured, b"hello");
    }

    #[test]
    fn read_bounded_errors_on_overflow_instead_of_truncating() {
        let data = [b'x'; 100];
        let err = read_bounded(&data[..], 10).expect_err("must fail, not truncate");
        assert!(
            err.to_string().contains("10"),
            "error should name the limit: {err}"
        );
    }

    /// A `Read` that fails after handing back a prefix — the shape of a
    /// pipe going bad mid-stream (EIO, ENOMEM), independent of any real
    /// syscall.
    struct FlakyReader {
        prefix: &'static [u8],
        served: bool,
    }

    impl Read for FlakyReader {
        fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
            if !self.served {
                self.served = true;
                buf[..self.prefix.len()].copy_from_slice(self.prefix);
                return Ok(self.prefix.len());
            }
            Err(io::Error::other("simulated pipe failure"))
        }
    }

    #[test]
    fn read_bounded_propagates_a_mid_stream_read_error() {
        let reader = FlakyReader {
            prefix: b"partial",
            served: false,
        };
        let err = read_bounded(reader, 1024).expect_err("read error must propagate");
        assert!(err.to_string().contains("simulated pipe failure"));
    }

    #[test]
    fn read_capped_swallows_a_mid_stream_read_error_and_keeps_the_prefix() {
        let reader = FlakyReader {
            prefix: b"partial",
            served: false,
        };
        // Deliberately lenient — stderr is diagnostic-only, so a truncated
        // prefix beats losing the whole capture over a transient read
        // error. `read_bounded` is the counterpart that does not accept
        // this tradeoff for stdout.
        let captured = read_capped(reader, 1024);
        assert_eq!(captured, b"partial");
    }
}
