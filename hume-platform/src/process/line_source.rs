//! Streaming a child process's stdout into complete lines.
//!
//! [`read_lines`](crate::process::line_source::read_lines) is the reader
//! thread of the picker's external-command source (`picker-source-spawn!`,
//! `hume-editor`'s `picker_source.rs`) and of a language server's stderr
//! forwarding (`hume-lsp`'s `transport.rs`): it feeds `LineSplitter`
//! arbitrary-sized chunks off a pipe, and `LineSplitter` yields only
//! complete lines, carrying any trailing partial line across chunk
//! boundaries. `LineSplitter` is kept separate from the spawn/thread
//! machinery so the boundary-carry logic is unit-testable without ever
//! touching a real process.
//!
//! [`spawn_line_source`](crate::process::line_source::spawn_line_source) is
//! the other half: spawns `cmd` with piped stdio, closes stdin immediately
//! (same non-inherited-stdin contract as [`crate::process::run_capture`]),
//! and bridges stdout/stderr to `mpsc` channels via two reader threads. Mirrors
//! `hume-lsp`'s `transport.rs` (thread/channel ownership, the bounded-channel
//! backpressure, `Drop` = kill+wait). No writer thread: this is a one-shot
//! streaming source, not a bidirectional protocol.

use std::io::{self, Read};
use std::path::Path;
use std::process::ExitStatus;
use std::sync::mpsc;
use std::time::Duration;

use crate::process::child::{
    STDERR_CAPTURE_CAP, read_capped, spawn_bridge, spawn_bridge_waking, spawn_piped,
};
use crate::process::decode_lossy;
use crate::process::tracked::TrackedChild;
use crate::wake::WakeCallback;

/// The longest line a [`LineSplitter`] keeps. Bytes past it, up to the next
/// delimiter, are dropped: a stream with no delimiters (binary output, a
/// progress bar redrawn with `\r`) would otherwise grow the carried line
/// for as long as the child runs.
const MAX_LINE_BYTES: usize = 1024 * 1024;

/// Splits a byte stream into complete lines on `delim`, carrying a trailing
/// partial line across `push_chunk` calls.
///
/// `\n`-delimited streams also get `\r` stripped from the line's end
/// (Windows CRLF output); NUL-delimited streams (`#:nul #t`, e.g.
/// `git ls-files -z`) do not, since NUL-separated records have no CRLF
/// convention to strip.
struct LineSplitter {
    delim: u8,
    strip_cr: bool,
    carry: Vec<u8>,
}

impl LineSplitter {
    fn new(delim: u8) -> Self {
        Self {
            delim,
            strip_cr: delim == b'\n',
            carry: Vec::new(),
        }
    }

    /// Split `chunk` into complete lines. A line that doesn't end in `delim`
    /// by the end of `chunk` is carried and prefixed onto the next call's
    /// (or [`finish`](Self::finish)'s) output; it never appears here.
    fn push_chunk(&mut self, chunk: &[u8]) -> Vec<String> {
        let mut lines = Vec::new();
        let mut rest = chunk;
        while let Some(i) = rest.iter().position(|&byte| byte == self.delim) {
            self.carry_capped(&rest[..i]);
            let line = std::mem::take(&mut self.carry);
            lines.push(self.finish_line(line));
            rest = &rest[i + 1..];
        }
        self.carry_capped(rest);
        lines
    }

    /// Appends `bytes` to the carried line, up to [`MAX_LINE_BYTES`].
    fn carry_capped(&mut self, bytes: &[u8]) {
        let room = MAX_LINE_BYTES.saturating_sub(self.carry.len());
        self.carry
            .extend_from_slice(&bytes[..bytes.len().min(room)]);
    }

    /// The trailing unterminated line at end-of-stream, if any bytes are
    /// still carried; `None` if the stream ended cleanly on `delim`.
    fn finish(&mut self) -> Option<String> {
        if self.carry.is_empty() {
            None
        } else {
            let line = std::mem::take(&mut self.carry);
            Some(self.finish_line(line))
        }
    }

    fn finish_line(&self, mut line: Vec<u8>) -> String {
        if self.strip_cr && line.last() == Some(&b'\r') {
            line.pop();
        }
        decode_lossy(line)
    }
}

/// Bound on the line-batch channel. A child producing lines faster than the
/// editor drains them blocks the reader thread's `send`, which stops it
/// reading stdout, which back-pressures the child on its own stdout pipe:
/// a flooding child slows itself rather than growing editor memory
/// unboundedly.
const BATCH_CHANNEL_BOUND: usize = 128;

/// Bound on how long [`SpawnedLineSource::finish`] waits for the captured
/// stderr to arrive once the child has been reaped or killed. The stderr
/// thread's `send` follows right behind, so this only guards against a
/// wedged thread, not normal timing.
const REAP_GRACE: Duration = Duration::from_millis(250);

/// A running external command streaming its stdout as complete lines.
///
/// Owns the child and its bridging threads: dropping it kills the child
/// (`Drop` = kill + wait, matching `hume-lsp::transport::ServerHandle`),
/// which is what makes the picker's kill-on-close/replace automatic: the
/// session that owns this handle needs no explicit cleanup call. The child
/// is a [`TrackedChild`], its own process-group leader, so it's also reaped
/// on a force-exit that skips this `Drop` entirely (see `tracked`'s module
/// doc).
pub struct SpawnedLineSource {
    cmd: String,
    child: TrackedChild,
    rx: Option<mpsc::Receiver<Vec<String>>>,
    stderr_rx: Option<mpsc::Receiver<String>>,
}

/// The outcome of a finished [`SpawnedLineSource`]: exit status (`None` only
/// if the OS gave none back even after a kill+wait fallback, vanishingly
/// rare) and whatever stderr was captured, capped at `STDERR_CAPTURE_CAP`.
pub struct SourceExit {
    pub status: Option<ExitStatus>,
    pub stderr: String,
}

/// Spawns `cmd` with `args` (direct argv, no shell), piped stdio, stdin
/// closed immediately, and bridges stdout/stderr to bounded channels via two
/// reader threads. `delimiter` is `b'\n'` or `b'\0'` (`#:nul`). `wake` is
/// called after the stdout thread posts a batch.
pub fn spawn_line_source(
    cmd: &str,
    args: &[String],
    cwd: &Path,
    delimiter: u8,
    wake: WakeCallback,
) -> io::Result<SpawnedLineSource> {
    let (child, stdin, stdout, stderr) = spawn_piped(cmd, args, cwd, &[])?;
    drop(stdin);

    let (tx, rx) = mpsc::sync_channel::<Vec<String>>(BATCH_CHANNEL_BOUND);
    let (tx_err, rx_err) = mpsc::sync_channel::<String>(1);

    // `child` kills and reaps itself (`ReapOnDrop`) on an early `?` return
    // below, so a bridging thread failing to spawn leaves nothing for the
    // process to leak. A thread that already started is not joined here:
    // killing the child closes stdout/stderr, which ends its blocking read.
    spawn_bridge_waking(cmd, "hume-line-source-reader", wake, move |wake| {
        read_lines(stdout, delimiter, |batch| {
            let sent = tx.send(batch).is_ok();
            if sent {
                wake();
            }
            sent
        });
    })?;

    spawn_bridge(cmd, "hume-line-source-stderr", move || {
        let captured = read_capped(stderr, STDERR_CAPTURE_CAP);
        let _ = tx_err.send(decode_lossy(captured));
    })?;

    Ok(SpawnedLineSource {
        cmd: cmd.to_string(),
        child: TrackedChild::new(child.into_inner()),
        rx: Some(rx),
        stderr_rx: Some(rx_err),
    })
}

impl SpawnedLineSource {
    pub fn cmd(&self) -> &str {
        &self.cmd
    }

    /// The child's OS process id, for a signal-0 liveness probe independent
    /// of this handle's own state, not for signalling it directly (that's
    /// `Drop`'s job). Test-support only.
    #[cfg(any(test, feature = "test-util"))]
    pub fn pid(&self) -> u32 {
        self.child.id()
    }

    /// Whether the child has already exited, without reaping it: a
    /// non-blocking peek used to decide whether a source being superseded or
    /// explicitly stopped (as opposed to draining to a natural EOF) has an
    /// exit worth reporting. An `Err` from the underlying `try_wait` (rare)
    /// reads as "not exited", safer than reporting an exit that may not
    /// have happened.
    pub fn has_exited(&self) -> bool {
        matches!(self.child.try_wait(), Ok(Some(_)))
    }

    /// Drains every batch of lines queued since the last call. The returned
    /// bool is whether the reader thread has disconnected (stdout EOF or a
    /// read error). Once true, call [`finish`](Self::finish) to reap the
    /// exit status and captured stderr.
    pub fn try_recv_batches(&mut self) -> (Vec<String>, bool) {
        let mut lines = Vec::new();
        let mut disconnected = false;
        if let Some(rx) = &self.rx {
            loop {
                match rx.try_recv() {
                    Ok(batch) => lines.extend(batch),
                    Err(mpsc::TryRecvError::Empty) => break,
                    Err(mpsc::TryRecvError::Disconnected) => {
                        disconnected = true;
                        break;
                    }
                }
            }
        }
        (lines, disconnected)
    }

    /// Consumes the source once the reader has disconnected: reaps the exit
    /// status and whatever stderr was captured.
    ///
    /// Runs on the editor's per-frame drain path (`drain_picker_source`), not
    /// a background thread, so this never blocks waiting for the child.
    /// Stdout EOF (the caller's precondition for calling `finish` at all)
    /// almost always means the child has already exited, in which case
    /// `try_wait` returns immediately with the real status; the rare child
    /// that lingers after closing stdout is killed right away rather than
    /// polled for, trading its exact exit status for a bounded frame.
    pub fn finish(mut self) -> SourceExit {
        let status = match self.child.try_wait() {
            Ok(Some(status)) => Some(status),
            Ok(None) => self.child.reap(),
            Err(_) => None,
        };
        let stderr = self
            .stderr_rx
            .take()
            .and_then(|rx| rx.recv_timeout(REAP_GRACE).ok())
            .unwrap_or_default();
        SourceExit { status, stderr }
    }
}

impl Drop for SpawnedLineSource {
    fn drop(&mut self) {
        self.child.reap();
        // Bounded channel: a reader thread can be blocked mid-`send` on a
        // full channel; dropping the receiver makes that `send` return
        // `Err`, letting the thread self-exit even though it's detached
        // rather than joined below.
        self.rx = None;
        self.stderr_rx = None;
    }
}

/// Reads `r` until EOF or a read error, handing `sink` each read's complete
/// lines split on `delimiter` (never an empty batch), then the trailing
/// unterminated line, if any. Stops reading once `sink` returns `false`.
pub fn read_lines(mut r: impl Read, delimiter: u8, mut sink: impl FnMut(Vec<String>) -> bool) {
    let mut splitter = LineSplitter::new(delimiter);
    let mut buf = [0u8; 64 * 1024];
    loop {
        match r.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => {
                let batch = splitter.push_chunk(&buf[..n]);
                if !batch.is_empty() && !sink(batch) {
                    return;
                }
            }
            Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
            Err(_) => break,
        }
    }
    if let Some(last) = splitter.finish() {
        sink(vec![last]);
    }
}

#[cfg(test)]
mod tests;
