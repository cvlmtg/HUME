//! Server process management: spawns a language server over piped stdio and
//! bridges its stdout/stdin/stderr to `mpsc` channels via three threads.
//!
//! Mirrors `hume-treesitter`'s `ThreadedParseBackend` — thread + channel
//! ownership, the `Option<Sender>` close-to-signal pattern, `Drop` ordering.

use std::io::{BufRead, BufReader, Write};
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::sync::mpsc;
use std::thread;

use crate::codec::{self, Message};

/// One event surfaced by the reader or stderr thread.
pub enum InboundEvent {
    Message(Message),
    /// One line of stderr output, already utf8-lossy decoded.
    Stderr(String),
    /// The reader hit EOF or a codec error — the connection is dead.
    Eof { error: Option<String> },
}

/// A running server process plus its bridging threads.
pub struct ServerHandle {
    /// Writer thread input; `None` after `Drop` closes it to signal the
    /// writer thread to exit.
    tx: Option<mpsc::Sender<Message>>,
    /// Reader + stderr thread output.
    rx: mpsc::Receiver<InboundEvent>,
    child: Child,
    threads: Vec<thread::JoinHandle<()>>,
}

impl ServerHandle {
    /// Spawns the process (cwd = `root`) and its three bridging threads.
    pub fn spawn(cmd: &str, args: &[String], root: &Path) -> std::io::Result<ServerHandle> {
        let mut child = Command::new(cmd)
            .args(args)
            .current_dir(root)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()?;

        let stdin = child.stdin.take().expect("piped stdin");
        let stdout = child.stdout.take().expect("piped stdout");
        let stderr = child.stderr.take().expect("piped stderr");

        let (tx_events, rx_events) = mpsc::channel::<InboundEvent>();
        let (tx_out, rx_out) = mpsc::channel::<Message>();

        let reader_tx = tx_events.clone();
        let reader = thread::Builder::new()
            .name("hume-lsp-reader".into())
            .spawn(move || reader_loop(BufReader::new(stdout), &reader_tx))
            .expect("failed to spawn LSP reader thread");

        let writer = thread::Builder::new()
            .name("hume-lsp-writer".into())
            .spawn(move || writer_loop(stdin, rx_out))
            .expect("failed to spawn LSP writer thread");

        let stderr_thread = thread::Builder::new()
            .name("hume-lsp-stderr".into())
            .spawn(move || stderr_loop(BufReader::new(stderr), &tx_events))
            .expect("failed to spawn LSP stderr thread");

        Ok(ServerHandle {
            tx: Some(tx_out),
            rx: rx_events,
            child,
            threads: vec![reader, writer, stderr_thread],
        })
    }

    /// Send a message to the server. Silently dropped if the connection is
    /// already dead — the crash is reported via the `Eof` event instead.
    pub fn send(&self, msg: Message) {
        if let Some(tx) = &self.tx {
            let _ = tx.send(msg);
        }
    }

    /// Drains all events that have arrived since the last call, in arrival
    /// order across both the reader and stderr threads.
    pub fn try_recv_all(&mut self) -> Vec<InboundEvent> {
        let mut out = Vec::new();
        while let Ok(ev) = self.rx.try_recv() {
            out.push(ev);
        }
        out
    }
}

impl Drop for ServerHandle {
    fn drop(&mut self) {
        // Closing tx makes the writer thread's `for msg in rx` end.
        self.tx = None;
        // Killing the child closes its stdout/stderr, which ends the reader
        // and stderr threads' blocking reads.
        let _ = self.child.kill();
        let _ = self.child.wait();
        for t in self.threads.drain(..) {
            let _ = t.join();
        }
    }
}

/// Reads frames until EOF or a codec error, forwarding each as an event.
/// Factored over `impl BufRead` so it's testable with in-memory pipes.
fn reader_loop(mut r: impl BufRead, tx: &mpsc::Sender<InboundEvent>) {
    loop {
        match codec::read_message(&mut r) {
            Ok(msg) => {
                if tx.send(InboundEvent::Message(msg)).is_err() {
                    return;
                }
            }
            Err(e) => {
                let _ = tx.send(InboundEvent::Eof {
                    error: Some(e.to_string()),
                });
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

/// Forwards each stderr line as an event. Unstructured text — never parsed,
/// just relayed (the editor glue logs it).
fn stderr_loop(r: impl BufRead, tx: &mpsc::Sender<InboundEvent>) {
    for line in r.lines() {
        match line {
            Ok(l) => {
                if tx.send(InboundEvent::Stderr(l)).is_err() {
                    return;
                }
            }
            Err(_) => return,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::codec::RequestId;
    use std::io::Cursor;

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
        let (tx, rx) = mpsc::channel();
        reader_loop(cursor, &tx);

        match rx.recv().unwrap() {
            InboundEvent::Message(Message::Notification { method, .. }) => {
                assert_eq!(method, "one");
            }
            _ => panic!("expected Message"),
        }
        match rx.recv().unwrap() {
            InboundEvent::Eof { .. } => {}
            _ => panic!("expected Eof after stream end"),
        }
    }

    #[test]
    fn reader_loop_reports_codec_error_as_eof() {
        // No Content-Length header — read_message errors immediately.
        let cursor = Cursor::new(b"garbage\r\n\r\n{}".to_vec());
        let (tx, rx) = mpsc::channel();
        reader_loop(cursor, &tx);
        match rx.recv().unwrap() {
            InboundEvent::Eof { error } => assert!(error.is_some()),
            _ => panic!("expected Eof"),
        }
        // Exactly one event — the loop must not resynchronize and retry.
        assert!(rx.try_recv().is_err());
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
        let (tx, rx) = mpsc::channel();
        stderr_loop(cursor, &tx);
        match rx.recv().unwrap() {
            InboundEvent::Stderr(l) => assert_eq!(l, "first line"),
            _ => panic!("expected Stderr"),
        }
        match rx.recv().unwrap() {
            InboundEvent::Stderr(l) => assert_eq!(l, "second line"),
            _ => panic!("expected Stderr"),
        }
        assert!(rx.try_recv().is_err());
    }

    #[test]
    #[cfg(unix)]
    fn cat_echoes_frames_and_drop_reaps_without_hanging() {
        let root = std::env::current_dir().unwrap();
        let mut handle = ServerHandle::spawn("/bin/cat", &[], &root).expect("spawn cat");

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
}
