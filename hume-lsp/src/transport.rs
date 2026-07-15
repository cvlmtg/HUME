//! Server process management: spawns a language server over piped stdio and
//! bridges its stdout/stdin/stderr to `mpsc` channels via three threads.
//!
//! Mirrors `hume-treesitter`'s `ThreadedParseBackend` — thread + channel
//! ownership, the `Option<Sender>` close-to-signal pattern, `Drop` ordering.

use std::io::{BufRead, BufReader, Write};
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::sync::Arc;
use std::sync::mpsc;
use std::thread;

use crate::codec::{self, Message};

/// Called by the reader/stderr threads after posting an event, so the
/// editor's main loop wakes and drains it instead of rechecking on a poll
/// cadence. Type-erased so this crate stays free of a `hume-platform`
/// dependency — production wraps `hume_platform::events::EventWaker::wake`.
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
pub struct ServerHandle {
    /// Writer thread input; `None` after `Drop` closes it to signal the
    /// writer thread to exit.
    tx: Option<mpsc::Sender<Message>>,
    /// Reader thread output (protocol messages + EOF); `None` after `Drop`
    /// closes it to unblock a thread possibly stuck mid-`send`.
    rx_events: Option<mpsc::Receiver<InboundEvent>>,
    /// Stderr thread output; `None` after `Drop`, same reason as `rx_events`.
    rx_stderr: Option<mpsc::Receiver<String>>,
    child: Child,
    /// Tracked separately (not lumped into `other_threads`) so `Drop` can
    /// give it a bounded window to flush any already-queued message (e.g. a
    /// `begin_shutdown`'s `shutdown`/`exit` pair) before the process is
    /// killed out from under it.
    writer: Option<thread::JoinHandle<()>>,
    other_threads: Vec<thread::JoinHandle<()>>,
}

impl ServerHandle {
    /// Spawns the process (cwd = `root`) and its three bridging threads.
    /// `wake` is called after the reader/stderr threads post an event, so
    /// the editor's main loop wakes instead of polling for completion.
    pub fn spawn(
        cmd: &str,
        args: &[String],
        root: &Path,
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

        let mut child = command
            .args(args)
            .current_dir(root)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()?;

        let stdin = child.stdin.take().expect("piped stdin");
        let stdout = child.stdout.take().expect("piped stdout");
        let stderr = child.stderr.take().expect("piped stderr");

        let (tx_events, rx_events) = mpsc::sync_channel::<InboundEvent>(EVENTS_CHANNEL_BOUND);
        let (tx_stderr, rx_stderr) = mpsc::sync_channel::<String>(STDERR_CHANNEL_BOUND);
        let (tx_out, rx_out) = mpsc::channel::<Message>();

        // If a later thread fails to spawn, the child (already running) must
        // not be orphaned: kill+reap it and join whatever threads did start
        // before propagating the error — `Child`'s own `Drop` does not kill,
        // so leaving this to unwind would leak the process.
        let reader_wake = Arc::clone(&wake);
        let reader = match thread::Builder::new()
            .name("hume-lsp-reader".into())
            .spawn(move || {
                let _wake_on_drop = WakeOnDrop(Arc::clone(&reader_wake));
                reader_loop(BufReader::new(stdout), &tx_events, &reader_wake)
            }) {
            Ok(t) => t,
            Err(e) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(e);
            }
        };

        let writer = match thread::Builder::new()
            .name("hume-lsp-writer".into())
            .spawn(move || writer_loop(stdin, rx_out))
        {
            Ok(t) => t,
            Err(e) => {
                let _ = child.kill();
                let _ = child.wait();
                let _ = reader.join();
                return Err(e);
            }
        };

        let stderr_wake = Arc::clone(&wake);
        let stderr_thread =
            match thread::Builder::new()
                .name("hume-lsp-stderr".into())
                .spawn(move || {
                    let _wake_on_drop = WakeOnDrop(Arc::clone(&stderr_wake));
                    stderr_loop(BufReader::new(stderr), &tx_stderr, &stderr_wake)
                }) {
                Ok(t) => t,
                Err(e) => {
                    let _ = child.kill();
                    let _ = child.wait();
                    let _ = reader.join();
                    // The writer thread blocks on `for msg in rx_out` until its
                    // sender is dropped — `tx_out` isn't moved into `ServerHandle`
                    // on this failure path, so drop it explicitly to let the
                    // thread's loop end before joining.
                    drop(tx_out);
                    let _ = writer.join();
                    return Err(e);
                }
            };

        Ok(ServerHandle {
            tx: Some(tx_out),
            rx_events: Some(rx_events),
            rx_stderr: Some(rx_stderr),
            child,
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
    /// protocol message/EOF first, then every stderr line — no longer
    /// strict arrival order across the two source threads (they're on
    /// separate channels now), but stderr is log-only, so its ordering
    /// relative to protocol traffic is cosmetic.
    pub fn try_recv_all(&mut self) -> Vec<InboundEvent> {
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
        let _ = self.child.kill();
        let _ = self.child.wait();
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
mod tests {
    use super::*;
    use crate::codec::RequestId;
    use std::io::Cursor;

    fn no_op_wake() -> WakeCallback {
        Arc::new(|| {})
    }

    /// A [`WakeCallback`] that counts invocations via an `mpsc` channel —
    /// `recv_timeout` gives a deterministic, non-polling way to assert a
    /// wake fired (or didn't, within the timeout) without racing the
    /// background thread that calls it.
    fn counting_wake() -> (WakeCallback, mpsc::Receiver<()>) {
        let (tx, rx) = mpsc::channel::<()>();
        let wake: WakeCallback = Arc::new(move || {
            let _ = tx.send(());
        });
        (wake, rx)
    }

    #[test]
    fn reader_loop_forwards_messages_then_eof() {
        let mut buf = Vec::new();
        codec::write_message(
            &mut buf,
            &Message::Notification {
                method: "one".to_string(),
                params: serde_json::Value::Null,
            },
        )
        .unwrap();
        let cursor = Cursor::new(buf);
        let (tx, rx) = mpsc::sync_channel(EVENTS_CHANNEL_BOUND);
        reader_loop(cursor, &tx, &no_op_wake());

        match rx.recv().unwrap() {
            InboundEvent::Message(Message::Notification { method, .. }) => {
                assert_eq!(method, "one");
            }
            _ => panic!("expected Message"),
        }
        match rx.recv().unwrap() {
            // A clean end-of-stream at a frame boundary (a voluntary server
            // exit) must not be reported as an error — only a genuine
            // mid-frame truncation should carry one.
            InboundEvent::Eof { error } => assert!(error.is_none()),
            _ => panic!("expected Eof after stream end"),
        }
    }

    #[test]
    fn reader_loop_reports_mid_frame_truncation_with_an_error() {
        // A Content-Length header was read, but the stream ends before the
        // blank line that would terminate the header block — a genuine
        // truncation, distinct from the clean-exit case above.
        let cursor = Cursor::new(b"Content-Length: 5\r\n".to_vec());
        let (tx, rx) = mpsc::sync_channel(EVENTS_CHANNEL_BOUND);
        reader_loop(cursor, &tx, &no_op_wake());
        match rx.recv().unwrap() {
            InboundEvent::Eof { error } => assert!(error.is_some()),
            _ => panic!("expected Eof"),
        }
    }

    #[test]
    fn reader_loop_reports_codec_error_as_eof() {
        // No Content-Length header — read_message errors immediately.
        let cursor = Cursor::new(b"garbage\r\n\r\n{}".to_vec());
        let (tx, rx) = mpsc::sync_channel(EVENTS_CHANNEL_BOUND);
        reader_loop(cursor, &tx, &no_op_wake());
        match rx.recv().unwrap() {
            InboundEvent::Eof { error } => assert!(error.is_some()),
            _ => panic!("expected Eof"),
        }
        // Exactly one event — the loop must not resynchronize and retry.
        assert!(rx.try_recv().is_err());
    }

    #[test]
    fn reader_loop_wakes_per_message_and_eof() {
        let mut buf = Vec::new();
        codec::write_message(
            &mut buf,
            &Message::Notification {
                method: "one".to_string(),
                params: serde_json::Value::Null,
            },
        )
        .unwrap();
        let cursor = Cursor::new(buf);
        let (tx, _rx) = mpsc::sync_channel(EVENTS_CHANNEL_BOUND);
        let (wake, rx_wake) = counting_wake();
        reader_loop(cursor, &tx, &wake);

        rx_wake
            .recv_timeout(std::time::Duration::from_secs(1))
            .expect("wake after message");
        rx_wake
            .recv_timeout(std::time::Duration::from_secs(1))
            .expect("wake after Eof");
        assert!(
            rx_wake.try_recv().is_err(),
            "exactly one message plus Eof should mean exactly two wakes"
        );
    }

    #[test]
    fn writer_loop_writes_all_queued_messages() {
        let (tx, rx) = mpsc::channel();
        tx.send(Message::Notification {
            method: "a".to_string(),
            params: serde_json::Value::Null,
        })
        .unwrap();
        tx.send(Message::Notification {
            method: "b".to_string(),
            params: serde_json::Value::Null,
        })
        .unwrap();
        drop(tx);

        let mut buf = Vec::new();
        writer_loop(&mut buf, rx);

        let mut cursor = Cursor::new(buf);
        match codec::read_message(&mut cursor).unwrap() {
            Message::Notification { method, .. } => assert_eq!(method, "a"),
            _ => panic!("expected Notification"),
        }
        match codec::read_message(&mut cursor).unwrap() {
            Message::Notification { method, .. } => assert_eq!(method, "b"),
            _ => panic!("expected Notification"),
        }
    }

    #[test]
    fn stderr_loop_forwards_lines() {
        let cursor = Cursor::new(b"first line\nsecond line\n".to_vec());
        let (tx, rx) = mpsc::sync_channel(STDERR_CHANNEL_BOUND);
        stderr_loop(cursor, &tx, &no_op_wake());
        assert_eq!(rx.recv().unwrap(), "first line");
        assert_eq!(rx.recv().unwrap(), "second line");
        assert!(rx.try_recv().is_err());
    }

    #[test]
    fn stderr_loop_wakes_on_forwarded_lines() {
        let cursor = Cursor::new(b"first line\nsecond line\n".to_vec());
        let (tx, _rx) = mpsc::sync_channel(STDERR_CHANNEL_BOUND);
        let (wake, rx_wake) = counting_wake();
        stderr_loop(cursor, &tx, &wake);
        rx_wake
            .recv_timeout(std::time::Duration::from_secs(1))
            .expect("wake for first line");
        rx_wake
            .recv_timeout(std::time::Duration::from_secs(1))
            .expect("wake for second line");
        assert!(rx_wake.try_recv().is_err(), "exactly two lines, two wakes");
    }

    #[test]
    fn stderr_flood_wakes_only_for_lines_that_were_actually_forwarded() {
        // Bound of 2, 5 lines — 3 are dropped by `try_send`'s `Full` arm and
        // must not wake (see `stderr_loop`'s doc): only 2 wakes expected.
        let cursor = Cursor::new(b"a\nb\nc\nd\ne\n".to_vec());
        let (tx, _rx) = mpsc::sync_channel(2);
        let (wake, rx_wake) = counting_wake();
        stderr_loop(cursor, &tx, &wake);
        rx_wake
            .recv_timeout(std::time::Duration::from_secs(1))
            .expect("wake 1");
        rx_wake
            .recv_timeout(std::time::Duration::from_secs(1))
            .expect("wake 2");
        assert!(
            rx_wake.try_recv().is_err(),
            "dropped lines under the flood must not also wake"
        );
    }

    #[test]
    fn stderr_flood_drops_lines_but_the_loop_terminates() {
        // Bound of 2, 5 lines, receiver never drained during the loop —
        // `try_send` must drop the overflow rather than block, so the loop
        // still returns instead of hanging.
        let cursor = Cursor::new(b"a\nb\nc\nd\ne\n".to_vec());
        let (tx, rx) = mpsc::sync_channel(2);
        stderr_loop(cursor, &tx, &no_op_wake());

        let mut received = Vec::new();
        while let Ok(line) = rx.try_recv() {
            received.push(line);
        }
        assert_eq!(
            received.len(),
            2,
            "only the channel's capacity should have been retained: {received:?}"
        );
    }

    #[test]
    fn stderr_loop_exits_when_the_receiver_is_gone() {
        let cursor = Cursor::new(b"first line\nsecond line\n".to_vec());
        let (tx, rx) = mpsc::sync_channel(STDERR_CHANNEL_BOUND);
        drop(rx);
        // Must return promptly on the Disconnected arm, not panic or loop.
        stderr_loop(cursor, &tx, &no_op_wake());
    }

    #[test]
    fn reader_loop_delivers_through_a_bounded_channel_in_order() {
        // Capacity of 1 forces `reader_loop`'s `send` to block between the
        // two messages until the reader below drains — this exercises the
        // `SyncSender` blocking-when-full path (not just the non-blocking
        // `try_recv` used elsewhere), without a real flooding process (which
        // would need timing assertions and be flaky by construction —
        // `Stdio::piped()`'s own pipe backpressure is what actually
        // engages in production; this test only pins that `reader_loop`
        // functions correctly against a bounded channel).
        let mut buf = Vec::new();
        codec::write_message(
            &mut buf,
            &Message::Notification {
                method: "one".to_string(),
                params: serde_json::Value::Null,
            },
        )
        .unwrap();
        codec::write_message(
            &mut buf,
            &Message::Notification {
                method: "two".to_string(),
                params: serde_json::Value::Null,
            },
        )
        .unwrap();
        let cursor = Cursor::new(buf);
        let (tx, rx) = mpsc::sync_channel(1);
        let handle = thread::spawn(move || reader_loop(cursor, &tx, &no_op_wake()));

        match rx.recv().unwrap() {
            InboundEvent::Message(Message::Notification { method, .. }) => {
                assert_eq!(method, "one")
            }
            other => panic!("expected 'one', got {other:?}"),
        }
        match rx.recv().unwrap() {
            InboundEvent::Message(Message::Notification { method, .. }) => {
                assert_eq!(method, "two")
            }
            other => panic!("expected 'two', got {other:?}"),
        }
        match rx.recv().unwrap() {
            InboundEvent::Eof { error } => assert!(error.is_none()),
            other => panic!("expected Eof, got {other:?}"),
        }
        handle.join().unwrap();
    }

    // ── wait_for_finish ──────────────────────────────────────────────────────

    #[test]
    fn wait_for_finish_returns_true_once_thread_completes() {
        let handle = thread::spawn(|| {
            std::thread::sleep(std::time::Duration::from_millis(20));
        });
        assert!(wait_for_finish(
            &handle,
            std::time::Duration::from_millis(500)
        ));
        let _ = handle.join();
    }

    #[test]
    fn wait_for_finish_times_out_on_a_thread_that_never_finishes_in_time() {
        let (tx, rx) = mpsc::channel::<()>();
        let handle = thread::spawn(move || {
            // Blocks until `tx` is dropped below.
            let _ = rx.recv();
        });
        assert!(!wait_for_finish(
            &handle,
            std::time::Duration::from_millis(50)
        ));
        drop(tx);
        let _ = handle.join();
    }

    #[test]
    #[cfg(unix)]
    fn cat_echoes_frames_and_drop_reaps_without_hanging() {
        let root = std::env::current_dir().unwrap();
        let mut handle =
            ServerHandle::spawn("/bin/cat", &[], &root, no_op_wake()).expect("spawn cat");

        let sent = Message::Request {
            id: RequestId::Int(1),
            method: "echo".to_string(),
            params: serde_json::json!({"ping": true}),
        };
        handle.send(sent);

        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        let mut received = None;
        while std::time::Instant::now() < deadline {
            for ev in handle.try_recv_all() {
                if let InboundEvent::Message(m) = ev {
                    received = Some(m);
                }
            }
            if received.is_some() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }

        match received.expect("cat should echo the frame back") {
            Message::Request { id, method, params } => {
                assert_eq!(id, RequestId::Int(1));
                assert_eq!(method, "echo");
                assert_eq!(params, serde_json::json!({"ping": true}));
            }
            other => panic!("expected Request, got {other:?}"),
        }

        // Drop runs kill -> wait -> join; must return promptly, not hang.
        drop(handle);
    }

    #[test]
    #[cfg(unix)]
    fn cat_echo_fires_waker() {
        let root = std::env::current_dir().unwrap();
        let (wake, rx_wake) = counting_wake();
        let handle = ServerHandle::spawn("/bin/cat", &[], &root, wake).expect("spawn cat");

        handle.send(Message::Notification {
            method: "ping".to_string(),
            params: serde_json::Value::Null,
        });

        rx_wake
            .recv_timeout(std::time::Duration::from_secs(5))
            .expect("reader thread must wake the loop after cat echoes the notification back");

        drop(handle);
    }

    #[test]
    #[cfg(unix)]
    fn drop_does_not_hang_when_stderr_floods_past_the_bound() {
        // Regression for the Drop deadlock fixed alongside the bounded
        // stderr channel: a thread blocked mid-`send` on a full channel is
        // NOT unblocked by `child.kill()` alone (killing only ends a
        // blocking *read*) — `Drop` must also close the receivers. On
        // regression this test hangs (caught by the harness's own test
        // timeout); on a correct `Drop` it returns promptly.
        let root = std::env::current_dir().unwrap();
        let mut handle = ServerHandle::spawn(
            "/bin/sh",
            &["-c".to_string(), "yes flood 1>&2".to_string()],
            &root,
            no_op_wake(),
        )
        .expect("spawn sh");

        // Let stderr fill well past STDERR_CHANNEL_BOUND before draining.
        std::thread::sleep(std::time::Duration::from_millis(50));

        let events = handle.try_recv_all();
        assert!(
            events.iter().any(|e| matches!(e, InboundEvent::Stderr(_))),
            "expected at least one Stderr event from the flood"
        );

        drop(handle);
    }

    // ── needs_cmd_shim ────────────────────────────────────────────────────────

    #[test]
    fn needs_cmd_shim_detects_cmd_extension() {
        assert!(needs_cmd_shim("typescript-language-server.cmd"));
    }

    #[test]
    fn needs_cmd_shim_detects_bat_extension() {
        assert!(needs_cmd_shim("run-server.bat"));
    }

    #[test]
    fn needs_cmd_shim_is_case_insensitive() {
        assert!(needs_cmd_shim("SERVER.CMD"));
        assert!(needs_cmd_shim("server.Bat"));
    }

    #[test]
    fn needs_cmd_shim_false_for_plain_executable() {
        assert!(!needs_cmd_shim("rust-analyzer"));
        assert!(!needs_cmd_shim("rust-analyzer.exe"));
    }

    #[test]
    fn needs_cmd_shim_false_for_extension_in_the_middle() {
        assert!(!needs_cmd_shim("server.cmd.exe"));
    }
}
