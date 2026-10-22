//! Server process management: spawns a language server over piped stdio and
//! bridges its stdout/stdin/stderr to `mpsc` channels via three threads.
//!
//! Mirrors `hume-treesitter`'s `ThreadedParseBackend`: thread + channel
//! ownership, the `Option<Sender>` close-to-signal pattern, `Drop` ordering.

use std::io::{BufRead, BufReader, Read, Write};
use std::sync::Arc;
use std::sync::mpsc;
use std::thread;

use hume_platform::process::line_source::read_lines;
use hume_platform::process::tracked::TrackedChild;
use hume_platform::process::{Launch, spawn_bridge, spawn_bridge_waking, spawn_piped};
use hume_platform::wake::WakeCallback;

use crate::codec::{self, Message};

/// One event surfaced by the reader or stderr thread.
#[derive(Debug)]
pub enum InboundEvent {
    Message(Message),
    /// One line of stderr output, decoded with invalid UTF-8 replaced by
    /// U+FFFD.
    Stderr(String),
    /// The reader hit EOF or a codec error; the connection is dead.
    Eof {
        error: Option<String>,
    },
}

/// Bound on the protocol-events channel. A server producing events faster
/// than the editor drains them blocks the reader thread's `send`, which
/// stops it reading stdout, which back-pressures the server on its own
/// stdout pipe: a flooding server slows itself rather than growing memory
/// unboundedly on the editor side.
const EVENTS_CHANNEL_BOUND: usize = 1024;

/// Bound on the stderr channel. stderr is Trace-level logging only. When
/// full, `stderr_loop` drops the line (`try_send`) rather than blocking, so
/// a chatty server can never stall the thread waiting for the editor to
/// drain logs it may never read.
const STDERR_CHANNEL_BOUND: usize = 256;

/// A running server process plus its bridging threads.
///
/// `child` is a [`TrackedChild`], its own process-group leader, so a
/// force-exit that skips this `Drop` entirely still reaps it (and any
/// process it spawned in turn, e.g. rust-analyzer's `proc-macro-srv`) via
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
    /// Disconnects when the writer thread ends: it holds the only sender.
    /// `Drop` waits on it for a bounded time, so a message already queued
    /// (e.g. a `begin_shutdown`'s `shutdown`/`exit` pair) is flushed before
    /// the process is killed out from under it.
    writer_done: mpsc::Receiver<()>,
    /// The reader, writer and stderr threads, joined by `Drop`.
    threads: Vec<thread::JoinHandle<()>>,
}

impl ServerHandle {
    /// Spawns the process from `launch` and its three bridging threads.
    /// `env` is applied additively to the inherited environment (no
    /// `env_clear`). `wake` is called after the reader/stderr threads post
    /// an event, so the editor's main loop wakes instead of polling for
    /// completion.
    pub fn spawn(
        cmd: &str,
        args: &[String],
        launch: Launch<'_>,
        env: &[(String, String)],
        wake: WakeCallback,
    ) -> std::io::Result<ServerHandle> {
        let (child, stdin, stdout, stderr) = spawn_piped(cmd, args, launch, env)?;

        let (tx_events, rx_events) = mpsc::sync_channel::<InboundEvent>(EVENTS_CHANNEL_BOUND);
        let (tx_stderr, rx_stderr) = mpsc::sync_channel::<String>(STDERR_CHANNEL_BOUND);
        let (tx_out, rx_out) = mpsc::channel::<Message>();

        // `child` kills and reaps itself (`ReapOnDrop`) on an early `?`
        // return below, so a bridging thread that fails to spawn leaves the
        // process behind for no one to leak. Threads that already started
        // are not joined here. They wind down on their own once that
        // happens: killing the child closes stdout/stderr, ending the
        // reader/stderr loops, and the writer loop ends once `tx_out` (and
        // every other sender into `rx_out`) drops at the same `?` return.
        let reader = spawn_bridge_waking(cmd, "hume-lsp-reader", Arc::clone(&wake), move |wake| {
            reader_loop(BufReader::new(stdout), &tx_events, wake)
        })?;

        let (writer_done_tx, writer_done) = mpsc::channel::<()>();
        let writer = spawn_bridge(cmd, "hume-lsp-writer", move || {
            let _done = writer_done_tx;
            writer_loop(stdin, rx_out)
        })?;

        let stderr_thread = spawn_bridge_waking(cmd, "hume-lsp-stderr", wake, move |wake| {
            stderr_loop(stderr, &tx_stderr, wake)
        })?;

        Ok(ServerHandle {
            tx: Some(tx_out),
            rx_events: Some(rx_events),
            rx_stderr: Some(rx_stderr),
            child: TrackedChild::new(child.into_inner()),
            writer_done,
            threads: vec![reader, writer, stderr_thread],
        })
    }

    /// Send a message to the server. Silently dropped if the connection is
    /// already dead; the crash is reported via the `Eof` event instead.
    pub fn send(&self, msg: Message) {
        if let Some(tx) = &self.tx {
            let _ = tx.send(msg);
        }
    }

    /// Drains all events that have arrived since the last call: every
    /// protocol message/EOF first, then every stderr line. Not strict
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
/// before killing the process: long enough for a normal write+flush to a
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

/// Waits up to `timeout` for the thread that holds the sender of `done` to
/// end, which disconnects the channel; returns whether it ended in time.
/// Extracted from `Drop` so the bounded-wait mechanism is unit-testable
/// without spawning a real child process.
fn wait_for_finish(done: &mpsc::Receiver<()>, timeout: std::time::Duration) -> bool {
    matches!(
        done.recv_timeout(timeout),
        Err(mpsc::RecvTimeoutError::Disconnected)
    )
}

impl Drop for ServerHandle {
    fn drop(&mut self) {
        // Closing tx signals the writer thread's `for msg in rx` to end,
        // but only once it drains every message already queued. Killing
        // the process immediately after would race that flush purely on
        // scheduling luck, silently downgrading every "graceful" shutdown
        // (shutdown request + exit notification) to a plain kill. Give the
        // writer a bounded window to actually finish first.
        self.tx = None;
        wait_for_finish(&self.writer_done, WRITER_FLUSH_GRACE);
        // Killing the child closes its stdout/stderr, which ends the reader
        // and stderr threads' blocking reads.
        self.child.reap();
        // Bounded channels: a reader/stderr thread can be blocked mid-`send`
        // on a full channel. Closing the child's pipes only unblocks a
        // thread stuck in a blocking *read*, not one already past that and
        // stuck pushing the result into a full channel. Dropping the
        // receivers here makes any such blocked `send` return `Err`,
        // letting the loop self-exit before the joins below; otherwise a
        // flooded channel could hang `Drop` forever.
        self.rx_events = None;
        self.rx_stderr = None;
        for t in self.threads.drain(..) {
            let _ = t.join();
        }
    }
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
            // server exit, not a crash. Report no error so the editor
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
/// every message: servers block on partial frames.
fn writer_loop(mut w: impl Write, rx: mpsc::Receiver<Message>) {
    for msg in rx {
        if codec::write_message(&mut w, &msg).is_err() {
            return;
        }
    }
}

/// Forwards each stderr line. Unstructured text, never parsed, just
/// relayed (the editor glue logs it). Uses `try_send`: a full channel drops
/// the line rather than blocking (see `STDERR_CHANNEL_BOUND`): stderr is
/// Trace-level logging, not protocol traffic, so losing a line under a
/// flood is an acceptable trade against ever stalling this thread. Wakes
/// the main loop once per read that forwarded a line. A dropped (`Full`)
/// line adds no observable data, and the send that filled the channel
/// already woke it. Stops once the receiver is gone.
/// Known risk: a server that crashes right after a burst fills the channel
/// can have its most useful line (the one explaining *why*) dropped
/// along with the flood, right when the log is most needed.
fn stderr_loop(r: impl Read, tx: &mpsc::SyncSender<String>, wake: &WakeCallback) {
    read_lines(r, b'\n', |lines| {
        let mut forwarded = false;
        for line in lines {
            match tx.try_send(line) {
                Ok(()) => forwarded = true,
                Err(mpsc::TrySendError::Full(_)) => {}
                Err(mpsc::TrySendError::Disconnected(_)) => return false,
            }
        }
        if forwarded {
            wake();
        }
        true
    });
}

#[cfg(test)]
mod tests;
