//! Streaming a child process's stdout into complete lines.
//!
//! [`LineSplitter`] is the pure, allocation-light half of the picker's
//! external-command source (`picker-source-spawn!`, `hume-editor`'s
//! `picker_source.rs`): a reader thread
//! feeds it arbitrary-sized chunks off a pipe, and it yields only complete
//! lines, carrying any trailing partial line across chunk boundaries. Kept
//! separate from the spawn/thread machinery so the boundary-carry logic is
//! unit-testable without ever touching a real process.
//!
//! [`spawn_line_source`] is the other half: spawns `cmd` with piped stdio,
//! closes stdin immediately (same non-inherited-stdin contract as PLUM's
//! `plum/run!`), and bridges stdout/stderr to `mpsc` channels via two reader
//! threads — mirrors `hume-lsp`'s `transport.rs` (thread/channel ownership,
//! the bounded-channel backpressure, `Drop` = kill+wait). No writer thread:
//! this is a one-shot streaming source, not a bidirectional protocol.

use std::io::{self, Read};
use std::path::Path;
use std::process::ExitStatus;
use std::sync::Arc;
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

use crate::process::child::{STDERR_CAPTURE_CAP, WakeOnDrop, read_capped, spawn_piped};
use crate::process::tracked::TrackedChild;

/// Re-exported so existing callers (`hume-editor`'s `host_impl.rs`) keep
/// naming this as `line_source::WakeCallback` — the canonical definition
/// now lives in `child.rs`, shared with `job.rs`.
pub use crate::process::child::WakeCallback;

/// Splits a byte stream into complete lines on `delim`, carrying a trailing
/// partial line across `push_chunk` calls.
///
/// `\n`-delimited streams also get `\r` stripped from the line's end
/// (Windows CRLF output); NUL-delimited streams (`#:nul #t`, e.g.
/// `git ls-files -z`) do not, since NUL-separated records have no CRLF
/// convention to strip.
pub struct LineSplitter {
    delim: u8,
    strip_cr: bool,
    carry: Vec<u8>,
}

impl LineSplitter {
    pub fn new(delim: u8) -> Self {
        Self {
            delim,
            strip_cr: delim == b'\n',
            carry: Vec::new(),
        }
    }

    /// Split `chunk` into complete lines. A line that doesn't end in `delim`
    /// by the end of `chunk` is carried and prefixed onto the next call's
    /// (or [`finish`](Self::finish)'s) output — it never appears here.
    pub fn push_chunk(&mut self, chunk: &[u8]) -> Vec<String> {
        let mut lines = Vec::new();
        let mut start = 0;
        for (i, &byte) in chunk.iter().enumerate() {
            if byte == self.delim {
                let mut line = std::mem::take(&mut self.carry);
                line.extend_from_slice(&chunk[start..i]);
                lines.push(self.finish_line(line));
                start = i + 1;
            }
        }
        self.carry.extend_from_slice(&chunk[start..]);
        lines
    }

    /// The trailing unterminated line at end-of-stream, if any bytes are
    /// still carried; `None` if the stream ended cleanly on `delim`.
    pub fn finish(&mut self) -> Option<String> {
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
        String::from_utf8_lossy(&line).into_owned()
    }
}

/// Bound on the line-batch channel. A child producing lines faster than the
/// editor drains them blocks the reader thread's `send`, which stops it
/// reading stdout, which back-pressures the child on its own stdout pipe —
/// a flooding child slows itself rather than growing editor memory
/// unboundedly.
const BATCH_CHANNEL_BOUND: usize = 128;

/// Bound on how long [`SpawnedLineSource::finish`] waits for the captured
/// stderr to arrive once the child has been reaped or killed — the stderr
/// thread's `send` follows right behind, so this only guards against a
/// wedged thread, not normal timing.
const REAP_GRACE: Duration = Duration::from_millis(250);

/// A running external command streaming its stdout as complete lines.
///
/// Owns the child and its bridging threads: dropping it kills the child
/// (`Drop` = kill + wait, matching `hume-lsp::transport::ServerHandle`),
/// which is what makes the picker's kill-on-close/replace automatic — the
/// session that owns this handle needs no explicit cleanup call. The child
/// is a [`TrackedChild`], its own process-group leader, so it's also reaped
/// on a force-exit that skips this `Drop` entirely — see `tracked`'s module
/// doc.
pub struct SpawnedLineSource {
    cmd: String,
    child: TrackedChild,
    rx: Option<mpsc::Receiver<Vec<String>>>,
    stderr_rx: Option<mpsc::Receiver<String>>,
    /// Detached (not joined) on drop — unlike the LSP writer thread, nothing
    /// here needs to flush before exit, so paying a join's latency on the
    /// user's Esc keypress would buy nothing.
    threads: Vec<thread::JoinHandle<()>>,
}

/// The outcome of a finished [`SpawnedLineSource`]: exit status (`None` only
/// if the OS gave none back even after a kill+wait fallback — vanishingly
/// rare) and whatever stderr was captured, capped at [`STDERR_CAPTURE_CAP`].
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
    cwd: Option<&Path>,
    delimiter: u8,
    wake: WakeCallback,
) -> io::Result<SpawnedLineSource> {
    let (child, stdout, stderr) = spawn_piped(cmd, args, cwd)?;

    let (tx, rx) = mpsc::sync_channel::<Vec<String>>(BATCH_CHANNEL_BOUND);
    let (tx_err, rx_err) = mpsc::sync_channel::<String>(1);

    // `child` kills and reaps itself (`ReapOnDrop`) on an early `?` return
    // below — a bridging thread failing to spawn leaves nothing for the
    // process to leak. A thread that already started is not joined here:
    // killing the child closes stdout/stderr, which ends its blocking read.
    let reader_wake = Arc::clone(&wake);
    let reader = thread::Builder::new()
        .name("hume-line-source-reader".into())
        .spawn(move || {
            let _wake_on_drop = WakeOnDrop(Arc::clone(&reader_wake));
            reader_loop(stdout, delimiter, &tx, &reader_wake);
        })?;

    let stderr_thread = thread::Builder::new()
        .name("hume-line-source-stderr".into())
        .spawn(move || {
            let captured = read_capped(stderr, STDERR_CAPTURE_CAP);
            let _ = tx_err.send(String::from_utf8_lossy(&captured).into_owned());
        })?;

    Ok(SpawnedLineSource {
        cmd: cmd.to_string(),
        child: TrackedChild::new(child.into_inner()),
        rx: Some(rx),
        stderr_rx: Some(rx_err),
        threads: vec![reader, stderr_thread],
    })
}

impl SpawnedLineSource {
    pub fn cmd(&self) -> &str {
        &self.cmd
    }

    /// The child's OS process id, for a signal-0 liveness probe independent
    /// of this handle's own state — not for signalling it directly (that's
    /// `Drop`'s job). Test-support only: every caller is cross-crate test
    /// code (`hume-editor`'s `tests/unix/picker_source.rs`), so this can't be
    /// `#[cfg(test)]`-gated the way a same-crate test-only method would be.
    #[doc(hidden)]
    pub fn pid(&self) -> u32 {
        self.child.id()
    }

    /// Drains every batch of lines queued since the last call. The returned
    /// bool is whether the reader thread has disconnected (stdout EOF or a
    /// read error) — once true, call [`finish`](Self::finish) to reap the
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
    /// a background thread — so this never blocks waiting for the child.
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
        // full channel — dropping the receiver makes that `send` return
        // `Err`, letting the thread self-exit even though it's detached
        // rather than joined below.
        self.rx = None;
        self.stderr_rx = None;
        self.threads.clear();
    }
}

/// Reads raw bytes until EOF or a read error, splitting them into lines and
/// forwarding each non-empty batch, waking the main loop after every send.
/// Factored over `impl Read` so it's testable without a real pipe.
fn reader_loop(
    mut r: impl Read,
    delimiter: u8,
    tx: &mpsc::SyncSender<Vec<String>>,
    wake: &WakeCallback,
) {
    let mut splitter = LineSplitter::new(delimiter);
    let mut buf = [0u8; 64 * 1024];
    loop {
        match r.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => {
                let batch = splitter.push_chunk(&buf[..n]);
                if !batch.is_empty() {
                    if tx.send(batch).is_err() {
                        return;
                    }
                    wake();
                }
            }
            Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
            Err(_) => break,
        }
    }
    if let Some(last) = splitter.finish() {
        let _ = tx.send(vec![last]);
        wake();
    }
    // `tx` (moved into the caller's closure) drops when this returns,
    // disconnecting the channel — the drain side observes that as EOF.
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Instant;

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
    fn invalid_utf8_is_lossy_replaced_not_dropped() {
        let mut s = LineSplitter::new(b'\n');
        let lines = s.push_chunk(b"\xffbad\n");
        assert_eq!(lines.len(), 1);
        assert!(lines[0].contains('\u{FFFD}'), "got: {:?}", lines[0]);
    }

    // ── spawn_line_source ─────────────────────────────────────────────────

    #[test]
    fn spawn_missing_binary_is_io_error() {
        let wake: WakeCallback = Arc::new(|| {});
        assert!(
            spawn_line_source("definitely-not-a-real-binary-xyz", &[], None, b'\n', wake).is_err()
        );
    }

    #[cfg(unix)]
    mod unix {
        use super::*;
        use std::sync::atomic::{AtomicUsize, Ordering};

        fn no_op_wake() -> WakeCallback {
            Arc::new(|| {})
        }

        fn counting_wake() -> (WakeCallback, Arc<AtomicUsize>) {
            let count = Arc::new(AtomicUsize::new(0));
            let counted = Arc::clone(&count);
            let wake: WakeCallback = Arc::new(move || {
                counted.fetch_add(1, Ordering::SeqCst);
            });
            (wake, count)
        }

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
                thread::sleep(Duration::from_millis(10));
            }
        }

        #[test]
        fn happy_path_streams_lines_and_reports_success() {
            let args = vec!["-c".to_string(), "printf 'a\\nb\\nc'".to_string()];
            let mut source =
                spawn_line_source("sh", &args, None, b'\n', no_op_wake()).expect("spawn sh");
            let lines = drain_until_disconnected(&mut source);
            assert_eq!(lines, vec!["a", "b", "c"]);
            let exit = source.finish();
            assert_eq!(exit.status.map(|s| s.success()), Some(true));
            assert_eq!(exit.stderr, "");
        }

        #[test]
        fn nul_delimited_output_splits_on_nul() {
            let args = vec!["-c".to_string(), "printf 'x\\0y\\0z'".to_string()];
            let mut source =
                spawn_line_source("sh", &args, None, b'\0', no_op_wake()).expect("spawn sh");
            let lines = drain_until_disconnected(&mut source);
            assert_eq!(lines, vec!["x", "y", "z"]);
        }

        #[test]
        fn wake_is_called_after_a_batch_arrives() {
            let (wake, count) = counting_wake();
            let args = vec!["-c".to_string(), "printf 'a\\n'".to_string()];
            let mut source = spawn_line_source("sh", &args, None, b'\n', wake).expect("spawn sh");
            drain_until_disconnected(&mut source);
            assert!(
                count.load(Ordering::SeqCst) > 0,
                "wake must fire at least once"
            );
        }

        #[test]
        fn nonzero_exit_and_stderr_are_captured() {
            let args = vec!["-c".to_string(), "echo oops >&2; exit 3".to_string()];
            let mut source =
                spawn_line_source("sh", &args, None, b'\n', no_op_wake()).expect("spawn sh");
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
            let mut source =
                spawn_line_source("sh", &args, None, b'\n', no_op_wake()).expect("spawn sh");
            drain_until_disconnected(&mut source);
            let exit = source.finish();
            assert!(exit.stderr.len() <= STDERR_CAPTURE_CAP);
        }

        #[test]
        fn finish_kills_a_child_that_lingers_after_closing_stdout_without_polling() {
            // Closes stdout immediately (triggering `disconnected`) but keeps
            // running — the exact shape `finish()` must not busy-wait on: it
            // should kill the child right away rather than polling for a
            // grace period before falling back to `kill`. The second `exec`
            // replaces the shell's own process image instead of forking a
            // `sleep` child, so killing the tracked pid actually terminates
            // the process holding stderr open (a plain trailing `sleep 30`
            // would fork, leaving an orphaned `sleep` that `kill` can't
            // reach and that would hold the stderr pipe open for real).
            let args = vec!["-c".to_string(), "exec 1>&-; exec sleep 30".to_string()];
            let mut source =
                spawn_line_source("sh", &args, None, b'\n', no_op_wake()).expect("spawn sh");
            drain_until_disconnected(&mut source);
            let pid =
                nix::unistd::Pid::from_raw(i32::try_from(source.child.id()).expect("pid fits i32"));

            let started = Instant::now();
            let exit = source.finish();
            assert!(
                started.elapsed() < Duration::from_millis(100),
                "finish() must kill a lingering child immediately, not poll for a grace \
                 period, took {:?}",
                started.elapsed()
            );
            assert!(
                exit.status.is_some(),
                "the killed child must still be reaped, not left as unknown"
            );
            assert!(
                nix::sys::signal::kill(pid, None).is_err(),
                "child must already be dead once finish() has returned"
            );
        }

        #[test]
        fn drop_kills_the_child_promptly() {
            // `sleep 30` makes a missing `kill()` observable two ways: the
            // signal-liveness check below (a `wait()`-only Drop still reaps
            // it, just 30s later) AND — the check that actually catches
            // that case — `drop()` itself must return promptly, not block
            // for the child's remaining lifetime.
            let args = vec!["30".to_string()];
            let source =
                spawn_line_source("sleep", &args, None, b'\n', no_op_wake()).expect("spawn sleep");
            let pid =
                nix::unistd::Pid::from_raw(i32::try_from(source.child.id()).expect("pid fits i32"));
            let started = Instant::now();
            drop(source);
            assert!(
                started.elapsed() < Duration::from_secs(5),
                "Drop must not block for the child's remaining lifetime, took {:?}",
                started.elapsed()
            );
            assert!(
                nix::sys::signal::kill(pid, None).is_err(),
                "child must already be dead once Drop has returned"
            );
        }

        #[test]
        fn nonexistent_cwd_is_error() {
            let dir = tempfile::tempdir().expect("tempdir");
            let bogus = dir.path().join("does-not-exist");
            assert!(spawn_line_source("sh", &[], Some(&bogus), b'\n', no_op_wake()).is_err());
        }
    }
}
