//! Server process management: spawns a language server over piped stdio and
//! bridges its stdout/stdin/stderr to `mpsc` channels via three threads.
//!
//! Mirrors `hume-treesitter`'s `ThreadedParseBackend` — thread + channel
//! ownership, the `Option<Sender>` close-to-signal pattern, `Drop` ordering.

use std::io::{BufRead, BufReader, Write};
use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::Arc;
use std::sync::mpsc;
use std::thread;

use hume_platform::process::tracked::TrackedChild;
use hume_platform::process::{ReapOnDrop, spawn_in_own_group};

use crate::codec::{self, Message};

/// Called by the reader/stderr threads after posting an event, so the
/// editor's main loop wakes and drains it instead of rechecking on a poll
/// cadence. Type-erased to keep `termina` types out of this
/// crate's API even though it depends on `hume-platform` for
/// [`TrackedChild`] —
/// production wraps `termina::PlatformWaker::wake`.
pub type WakeCallback = Arc<dyn Fn() + Send + Sync>;

/// Invokes a [`WakeCallback`] on drop — fires whether a thread exits
/// normally or unwinds from a panic, so a dead transport thread still wakes
/// the main loop once (the subsequent drain observes the disconnect via the
/// existing channel). A normal exit firing one extra, spurious wake is
/// harmless — callers already tolerate spurious wakes by design.
struct WakeOnDrop(WakeCallback);

impl Drop for WakeOnDrop {
    fn drop(&mut self) {
        (self.0)();
    }
}

/// One event surfaced by the reader or stderr thread.
#[derive(Debug)]
pub enum InboundEvent {
    Message(Message),
    /// One line of stderr output, already utf8-lossy decoded.
    Stderr(String),
    /// The reader hit EOF or a codec error — the connection is dead.
    Eof {
        error: Option<String>,
    },
}

/// Bound on the protocol-events channel. A server producing events faster
/// than the editor drains them blocks the reader thread's `send`, which
/// stops it reading stdout, which back-pressures the server on its own
/// stdout pipe — a flooding server slows itself rather than growing memory
/// unboundedly on the editor side.
const EVENTS_CHANNEL_BOUND: usize = 1024;

/// Bound on the stderr channel. stderr is Trace-level logging only — when
/// full, `stderr_loop` drops the line (`try_send`) rather than blocking, so
/// a chatty server can never stall the thread waiting for the editor to
/// drain logs it may never read.
const STDERR_CHANNEL_BOUND: usize = 256;

/// A running server process plus its bridging threads.
///
/// `child` is a [`TrackedChild`], its own process-group leader, so a
/// force-exit that skips this `Drop` entirely still reaps it — and any
/// process it spawned in turn (e.g. rust-analyzer's `proc-macro-srv`) — via
/// `hume-platform`'s `process::tracked`.
pub(crate) struct ServerHandle {
    /// Writer thread input; `None` after `Drop` closes it to signal the
    /// writer thread to exit.
    tx: Option<mpsc::Sender<Message>>,
    /// Reader thread output (protocol messages + EOF); `None` after `Drop`
    /// closes it to unblock a thread possibly stuck mid-`send`.
    rx_events: Option<mpsc::Receiver<InboundEvent>>,
    /// Stderr thread output; `None` after `Drop`, same reason as `rx_events`.
    rx_stderr: Option<mpsc::Receiver<String>>,
    child: TrackedChild,
    /// Tracked separately (not lumped into `other_threads`) so `Drop` can
    /// give it a bounded window to flush any already-queued message (e.g. a
    /// `begin_shutdown`'s `shutdown`/`exit` pair) before the process is
    /// killed out from under it.
    writer: Option<thread::JoinHandle<()>>,
    other_threads: Vec<thread::JoinHandle<()>>,
}

impl ServerHandle {
    /// Spawns the process (cwd = `root`) and its three bridging threads.
    /// `env` is applied additively to the inherited environment (no
    /// `env_clear`). `wake` is called after the reader/stderr threads post
    /// an event, so the editor's main loop wakes instead of polling for
    /// completion.
    pub fn spawn(
        cmd: &str,
        args: &[String],
        root: &Path,
        env: &[(String, String)],
        wake: WakeCallback,
    ) -> std::io::Result<ServerHandle> {
        #[cfg(windows)]
        let mut command = if needs_cmd_shim(cmd) {
            // npm-kind servers register a `.cmd` shim (e.g.
            // `node_modules/.bin/typescript-language-server.cmd`), which
            // `CreateProcess` cannot spawn directly. Args with cmd.exe
            // metacharacters are unsupported here — registered npm-kind args
            // are trivial (e.g. `--stdio`).
            let mut c = Command::new("cmd");
            c.arg("/C").arg(cmd);
            c
        } else {
            Command::new(cmd)
        };
        #[cfg(not(windows))]
        let mut command = Command::new(cmd);

        command
            .args(args)
            .envs(env.iter().map(|(k, v)| (k.as_str(), v.as_str())))
            .current_dir(root)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        let mut child = ReapOnDrop::new(spawn_in_own_group(&mut command)?);

        let stdin = child.get_mut().stdin.take().expect("piped stdin");
        let stdout = child.get_mut().stdout.take().expect("piped stdout");
        let stderr = child.get_mut().stderr.take().expect("piped stderr");

        let (tx_events, rx_events) = mpsc::sync_channel::<InboundEvent>(EVENTS_CHANNEL_BOUND);
        let (tx_stderr, rx_stderr) = mpsc::sync_channel::<String>(STDERR_CHANNEL_BOUND);
        let (tx_out, rx_out) = mpsc::channel::<Message>();

        // `child` kills and reaps itself (`ReapOnDrop`) on an early `?`
        // return below, so a bridging thread that fails to spawn leaves the
        // process behind for no one to leak. Threads that already started
        // are not joined here — they wind down on their own once that
        // happens: killing the child closes stdout/stderr, ending the
        // reader/stderr loops, and the writer loop ends once `tx_out` (and
        // every other sender into `rx_out`) drops at the same `?` return.
        let reader_wake = Arc::clone(&wake);
        let reader = thread::Builder::new()
            .name("hume-lsp-reader".into())
            .spawn(move || {
                let _wake_on_drop = WakeOnDrop(Arc::clone(&reader_wake));
                reader_loop(BufReader::new(stdout), &tx_events, &reader_wake)
            })?;

        let writer = thread::Builder::new()
            .name("hume-lsp-writer".into())
            .spawn(move || writer_loop(stdin, rx_out))?;

        let stderr_wake = Arc::clone(&wake);
        let stderr_thread = thread::Builder::new()
            .name("hume-lsp-stderr".into())
            .spawn(move || {
                let _wake_on_drop = WakeOnDrop(Arc::clone(&stderr_wake));
                stderr_loop(BufReader::new(stderr), &tx_stderr, &stderr_wake)
            })?;

        Ok(ServerHandle {
            tx: Some(tx_out),
            rx_events: Some(rx_events),
            rx_stderr: Some(rx_stderr),
            child: TrackedChild::new(child.into_inner()),
            writer: Some(writer),
            other_threads: vec![reader, stderr_thread],
        })
    }

    /// Send a message to the server. Silently dropped if the connection is
    /// already dead — the crash is reported via the `Eof` event instead.
    pub fn send(&self, msg: Message) {
        if let Some(tx) = &self.tx {
            let _ = tx.send(msg);
        }
    }

    /// Drains all events that have arrived since the last call: every
    /// protocol message/EOF first, then every stderr line — not strict
    /// arrival order across the two source threads (they're on separate
    /// channels), but stderr is log-only, so its ordering relative to
    /// protocol traffic is cosmetic.
    pub fn try_recv_all(&self) -> Vec<InboundEvent> {
        let mut out = Vec::new();
        if let Some(rx) = &self.rx_events {
            while let Ok(ev) = rx.try_recv() {
                out.push(ev);
            }
        }
        if let Some(rx) = &self.rx_stderr {
            while let Ok(line) = rx.try_recv() {
                out.push(InboundEvent::Stderr(line));
            }
        }
        out
    }
}

/// Bound on how long `Drop` waits for the writer thread to flush any
/// already-queued message (e.g. `begin_shutdown`'s `shutdown`/`exit` pair)
/// before killing the process — long enough for a normal write+flush to a
/// live pipe, short enough that a server ignoring stdin doesn't hang exit.
const WRITER_FLUSH_GRACE: std::time::Duration = std::time::Duration::from_millis(200);

/// [`WRITER_FLUSH_GRACE`], exposed to consumer crates so
/// `hume_platform::QUIT_GRACE`'s "sized against this, per live server" budget
/// can be checked against the real value instead of just a comment promising
/// they're kept in step (see the invariant test in `hume-editor`).
#[cfg(any(test, feature = "test-util"))]
pub fn writer_flush_grace() -> std::time::Duration {
    WRITER_FLUSH_GRACE
}

/// Polls `handle` up to `timeout`, returning whether it finished in time.
/// Extracted from `Drop` so the bounded-wait mechanism is unit-testable
/// without spawning a real child process.
fn wait_for_finish(handle: &thread::JoinHandle<()>, timeout: std::time::Duration) -> bool {
    let deadline = std::time::Instant::now() + timeout;
    loop {
        if handle.is_finished() {
            return true;
        }
        if std::time::Instant::now() >= deadline {
            return false;
        }
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
}

impl Drop for ServerHandle {
    fn drop(&mut self) {
        // Closing tx signals the writer thread's `for msg in rx` to end —
        // but only once it drains every message already queued. Killing
        // the process immediately after would race that flush purely on
        // scheduling luck, silently downgrading every "graceful" shutdown
        // (shutdown request + exit notification) to a plain kill. Give the
        // writer a bounded window to actually finish first.
        self.tx = None;
        if let Some(writer) = &self.writer {
            wait_for_finish(writer, WRITER_FLUSH_GRACE);
        }
        // Killing the child closes its stdout/stderr, which ends the reader
        // and stderr threads' blocking reads.
        self.child.reap();
        // Bounded channels: a reader/stderr thread can be blocked mid-`send`
        // on a full channel — closing the child's pipes only unblocks a
        // thread stuck in a blocking *read*, not one already past that and
        // stuck pushing the result into a full channel. Dropping the
        // receivers here makes any such blocked `send` return `Err`,
        // letting the loop self-exit before the joins below; otherwise a
        // flooded channel could hang `Drop` forever.
        self.rx_events = None;
        self.rx_stderr = None;
        if let Some(writer) = self.writer.take() {
            let _ = writer.join();
        }
        for t in self.other_threads.drain(..) {
            let _ = t.join();
        }
    }
}

/// Whether `cmd` is a Windows `.cmd`/`.bat` shim, which `CreateProcess`
/// cannot spawn directly and must instead be run via `cmd /C`.
///
/// Cfg-free and unit-testable on every platform even though it's only
/// consulted (in `ServerHandle::spawn`) on Windows — dead on other targets.
#[cfg_attr(not(windows), allow(dead_code))]
fn needs_cmd_shim(cmd: &str) -> bool {
    let lower = cmd.to_ascii_lowercase();
    lower.ends_with(".cmd") || lower.ends_with(".bat")
}

/// Reads frames until EOF or a codec error, forwarding each as an event and
/// waking the main loop after every successful send. Factored over
/// `impl BufRead` so it's testable with in-memory pipes.
fn reader_loop(mut r: impl BufRead, tx: &mpsc::SyncSender<InboundEvent>, wake: &WakeCallback) {
    loop {
        match codec::read_message(&mut r) {
            Ok(msg) => {
                if tx.send(InboundEvent::Message(msg)).is_err() {
                    return;
                }
                wake();
            }
            // A clean end-of-stream at a frame boundary is a voluntary
            // server exit, not a crash — report no error so the editor
            // glue doesn't log a spurious "server crashed".
            Err(codec::CodecError::Eof) => {
                let _ = tx.send(InboundEvent::Eof { error: None });
                wake();
                return;
            }
            Err(e) => {
                let _ = tx.send(InboundEvent::Eof {
                    error: Some(e.to_string()),
                });
                wake();
                return;
            }
        }
    }
}

/// Writes every message received until the channel closes. Flushes after
/// every message — servers block on partial frames.
fn writer_loop(mut w: impl Write, rx: mpsc::Receiver<Message>) {
    for msg in rx {
        if codec::write_message(&mut w, &msg).is_err() {
            return;
        }
    }
}

/// Forwards each stderr line. Unstructured text — never parsed, just
/// relayed (the editor glue logs it). Uses `try_send`: a full channel drops
/// the line rather than blocking (see `STDERR_CHANNEL_BOUND`) — stderr is
/// Trace-level logging, not protocol traffic, so losing a line under a
/// flood is an acceptable trade against ever stalling this thread. Wakes
/// the main loop only on a forwarded line — a dropped (`Full`) line adds no
/// observable data, and the send that filled the channel already woke it.
/// Known risk: a server that crashes right after a burst fills the channel
/// can have its most useful line — the one explaining *why* — dropped
/// along with the flood, right when the log is most needed.
fn stderr_loop(r: impl BufRead, tx: &mpsc::SyncSender<String>, wake: &WakeCallback) {
    for line in r.lines() {
        match line {
            Ok(l) => match tx.try_send(l) {
                Ok(()) => wake(),
                Err(mpsc::TrySendError::Full(_)) => {}
                Err(mpsc::TrySendError::Disconnected(_)) => return,
            },
            Err(_) => return,
        }
    }
}

#[cfg(test)]
mod tests;
