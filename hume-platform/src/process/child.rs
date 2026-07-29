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
/// rest of the pipe so a chatty child never blocks on write. Callers that
/// want an uncapped read (job.rs's stdout capture) pass `usize::MAX`.
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
