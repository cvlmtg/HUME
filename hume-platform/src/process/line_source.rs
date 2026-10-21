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
mod tests {
    use super::*;
    use crate::wake::no_op_wake;

    #[test]
    fn splits_multiple_lines_in_one_chunk() {
        let mut s = LineSplitter::new(b'\n');
        assert_eq!(s.push_chunk(b"a\nb\nc\n"), vec!["a", "b", "c"]);
        assert_eq!(s.finish(), None);
    }

    #[test]
    fn carries_a_partial_line_across_chunk_boundary() {
        let mut s = LineSplitter::new(b'\n');
        assert_eq!(s.push_chunk(b"ab"), Vec::<String>::new());
        assert_eq!(s.push_chunk(b"c\nd\n"), vec!["abc", "d"]);
        assert_eq!(s.finish(), None);
    }

    #[test]
    fn carries_across_a_chunk_boundary_that_lands_exactly_on_a_delimiter() {
        let mut s = LineSplitter::new(b'\n');
        assert_eq!(s.push_chunk(b"abc\n"), vec!["abc"]);
        assert_eq!(s.push_chunk(b"def\n"), vec!["def"]);
    }

    #[test]
    fn delimiter_as_first_byte_of_a_later_chunk_still_closes_the_carry() {
        let mut s = LineSplitter::new(b'\n');
        assert_eq!(s.push_chunk(b"abc"), Vec::<String>::new());
        assert_eq!(s.push_chunk(b"\ndef\n"), vec!["abc", "def"]);
    }

    #[test]
    fn nul_mode_preserves_interior_newlines_and_carriage_returns() {
        let mut s = LineSplitter::new(b'\0');
        assert_eq!(s.push_chunk(b"a\r\n\0b\0"), vec!["a\r\n", "b"]);
    }

    #[test]
    fn newline_mode_strips_trailing_carriage_return() {
        let mut s = LineSplitter::new(b'\n');
        assert_eq!(s.push_chunk(b"a\r\nb\r\n"), vec!["a", "b"]);
    }

    #[test]
    fn finish_emits_trailing_unterminated_line() {
        let mut s = LineSplitter::new(b'\n');
        assert_eq!(s.push_chunk(b"a\nb"), vec!["a"]);
        assert_eq!(s.finish(), Some("b".to_string()));
    }

    #[test]
    fn finish_after_a_cleanly_terminated_stream_is_none() {
        let mut s = LineSplitter::new(b'\n');
        assert_eq!(s.push_chunk(b"a\n"), vec!["a"]);
        assert_eq!(s.finish(), None);
    }

    #[test]
    fn interior_empty_lines_are_emitted() {
        let mut s = LineSplitter::new(b'\n');
        assert_eq!(s.push_chunk(b"a\n\nb\n"), vec!["a", "", "b"]);
    }

    #[test]
    fn a_line_past_the_cap_is_truncated_and_the_next_line_is_whole() {
        let mut s = LineSplitter::new(b'\n');
        let half = vec![b'x'; MAX_LINE_BYTES / 2 + 1];
        assert_eq!(s.push_chunk(&half), Vec::<String>::new());
        assert_eq!(s.push_chunk(&half), Vec::<String>::new());
        let lines = s.push_chunk(b"tail\nnext\n");
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0].len(), MAX_LINE_BYTES);
        assert!(lines[0].bytes().all(|b| b == b'x'));
        assert_eq!(lines[1], "next");
    }

    #[test]
    fn invalid_utf8_is_lossy_replaced_not_dropped() {
        let mut s = LineSplitter::new(b'\n');
        let lines = s.push_chunk(b"\xffbad\n");
        assert_eq!(lines.len(), 1);
        assert!(lines[0].contains('\u{FFFD}'), "got: {:?}", lines[0]);
    }

    // ── spawn_line_source ─────────────────────────────────────────────────

    #[test]
    fn spawn_missing_binary_is_io_error() {
        assert!(
            spawn_line_source(
                "definitely-not-a-real-binary-xyz",
                &[],
                Path::new("."),
                b'\n',
                no_op_wake()
            )
            .is_err()
        );
    }

    #[cfg(unix)]
    mod unix {
        use super::*;
        use crate::process::test_support::is_alive;
        use crate::wake::counting_wake;
        use std::time::Instant;

        /// Drains `source` until the reader disconnects, with a generous
        /// bound so a slow CI box can't flake this.
        fn drain_until_disconnected(source: &mut SpawnedLineSource) -> Vec<String> {
            let deadline = Instant::now() + Duration::from_secs(2);
            let mut lines = Vec::new();
            loop {
                let (batch, disconnected) = source.try_recv_batches();
                lines.extend(batch);
                if disconnected {
                    return lines;
                }
                assert!(Instant::now() < deadline, "reader never disconnected");
                std::thread::sleep(Duration::from_millis(10));
            }
        }

        #[test]
        fn happy_path_streams_lines_and_reports_success() {
            let args = vec!["-c".to_string(), "printf 'a\\nb\\nc'".to_string()];
            let mut source = spawn_line_source("sh", &args, Path::new("."), b'\n', no_op_wake())
                .expect("spawn sh");
            let lines = drain_until_disconnected(&mut source);
            assert_eq!(lines, vec!["a", "b", "c"]);
            let exit = source.finish();
            assert_eq!(exit.status.map(|s| s.success()), Some(true));
            assert_eq!(exit.stderr, "");
        }

        #[test]
        fn nul_delimited_output_splits_on_nul() {
            let args = vec!["-c".to_string(), "printf 'x\\0y\\0z'".to_string()];
            let mut source = spawn_line_source("sh", &args, Path::new("."), b'\0', no_op_wake())
                .expect("spawn sh");
            let lines = drain_until_disconnected(&mut source);
            assert_eq!(lines, vec!["x", "y", "z"]);
        }

        #[test]
        fn wake_is_called_after_a_batch_arrives() {
            let (wake, woken) = counting_wake();
            let args = vec!["-c".to_string(), "printf 'a\\n'".to_string()];
            let mut source =
                spawn_line_source("sh", &args, Path::new("."), b'\n', wake).expect("spawn sh");
            drain_until_disconnected(&mut source);
            assert!(woken.try_iter().count() > 0, "wake must fire at least once");
        }

        #[test]
        fn nonzero_exit_and_stderr_are_captured() {
            let args = vec!["-c".to_string(), "echo oops >&2; exit 3".to_string()];
            let mut source = spawn_line_source("sh", &args, Path::new("."), b'\n', no_op_wake())
                .expect("spawn sh");
            drain_until_disconnected(&mut source);
            let exit = source.finish();
            assert_eq!(exit.status.and_then(|s| s.code()), Some(3));
            assert!(exit.stderr.contains("oops"), "got: {:?}", exit.stderr);
        }

        #[test]
        fn stderr_capture_is_capped() {
            // Emit well over the cap in stderr; the thread must still drain
            // to EOF promptly (not block on the child) and the capture must
            // not exceed the cap.
            let args = vec![
                "-c".to_string(),
                "yes x | head -c 200000 1>&2; exit 0".to_string(),
            ];
            let mut source = spawn_line_source("sh", &args, Path::new("."), b'\n', no_op_wake())
                .expect("spawn sh");
            drain_until_disconnected(&mut source);
            let exit = source.finish();
            assert!(exit.stderr.len() <= STDERR_CAPTURE_CAP);
        }

        #[test]
        fn finish_kills_a_child_that_lingers_after_closing_stdout_without_polling() {
            // Closes stdout immediately (triggering `disconnected`) but keeps
            // running: the exact shape `finish()` must not busy-wait on: it
            // should kill the child right away rather than polling for a
            // grace period before falling back to `kill`. The second `exec`
            // replaces the shell's own process image instead of forking a
            // `sleep` child, so killing the tracked pid actually terminates
            // the process holding stderr open (a plain trailing `sleep 30`
            // would fork, leaving an orphaned `sleep` that `kill` can't
            // reach and that would hold the stderr pipe open for real).
            let args = vec!["-c".to_string(), "exec 1>&-; exec sleep 30".to_string()];
            let mut source = spawn_line_source("sh", &args, Path::new("."), b'\n', no_op_wake())
                .expect("spawn sh");
            drain_until_disconnected(&mut source);
            let pid = source.child.id();

            // 5s, not a tight bound on the kill/reap/stderr-EOF handoff
            // itself (which spans two thread wakeups and is scheduler-
            // sensitive). It only needs to rule out `finish()` waiting the
            // child out instead of killing it, which would take the full 30s
            // the child sleeps for. Matches the margin `drop_kills_the_child_
            // promptly` below uses against the same kind of child.
            let started = Instant::now();
            let exit = source.finish();
            assert!(
                started.elapsed() < Duration::from_secs(5),
                "finish() must kill a lingering child immediately, not wait it out, took {:?}",
                started.elapsed()
            );
            assert!(
                exit.status.is_some(),
                "the killed child must still be reaped, not left as unknown"
            );
            assert!(
                !is_alive(pid),
                "child must already be dead once finish() has returned"
            );
        }

        #[test]
        fn drop_kills_the_child_promptly() {
            // `sleep 30` makes a missing `kill()` observable two ways: the
            // signal-liveness check below (a `wait()`-only Drop still reaps
            // it, just 30s later) AND (the check that actually catches
            // that case) `drop()` itself must return promptly, not block
            // for the child's remaining lifetime.
            let args = vec!["30".to_string()];
            let source = spawn_line_source("sleep", &args, Path::new("."), b'\n', no_op_wake())
                .expect("spawn sleep");
            let pid = source.child.id();
            let started = Instant::now();
            drop(source);
            assert!(
                started.elapsed() < Duration::from_secs(5),
                "Drop must not block for the child's remaining lifetime, took {:?}",
                started.elapsed()
            );
            assert!(
                !is_alive(pid),
                "child must already be dead once Drop has returned"
            );
        }

        #[test]
        fn nonexistent_cwd_is_error() {
            let dir = tempfile::tempdir().expect("tempdir");
            let bogus = dir.path().join("does-not-exist");
            assert!(spawn_line_source("sh", &[], &bogus, b'\n', no_op_wake()).is_err());
        }
    }
}
