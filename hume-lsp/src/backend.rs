//! The seam the editor holds: `Box<dyn LspBackend>`, mirroring
//! `parse_worker: Box<dyn ParseBackend>` in `hume-treesitter`.
//!
//! Transport-flavored only — no capabilities, no `text_gen`, no buffer
//! knowledge. That client-level state lives above this trait.

use std::path::Path;

use crate::codec::Message;
use crate::transport::{InboundEvent, ServerHandle};

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct ServerId(pub u32);

pub trait LspBackend {
    /// Spawn (threaded) or register (inline) a server. Handshake is the
    /// client layer's job — this is transport-level only.
    fn start(&mut self, cmd: &str, args: &[String], root: &Path) -> std::io::Result<ServerId>;
    fn send(&mut self, server: ServerId, msg: Message);
    /// All events that arrived since the last drain, in arrival order.
    fn drain(&mut self) -> Vec<(ServerId, InboundEvent)>;
    /// Any undrained event? Feeds the wake predicate.
    fn has_pending(&self) -> bool;
    fn shutdown(&mut self, server: ServerId);
}

/// Production backend: one real server process per registration.
pub struct ThreadedLspBackend {
    servers: std::collections::HashMap<ServerId, ServerHandle>,
    next: u32,
}

impl ThreadedLspBackend {
    pub fn new() -> Self {
        Self {
            servers: std::collections::HashMap::new(),
            next: 0,
        }
    }
}

impl Default for ThreadedLspBackend {
    fn default() -> Self {
        Self::new()
    }
}

impl LspBackend for ThreadedLspBackend {
    fn start(&mut self, cmd: &str, args: &[String], root: &Path) -> std::io::Result<ServerId> {
        let handle = ServerHandle::spawn(cmd, args, root)?;
        let id = ServerId(self.next);
        self.next += 1;
        self.servers.insert(id, handle);
        Ok(id)
    }

    fn send(&mut self, server: ServerId, msg: Message) {
        if let Some(handle) = self.servers.get(&server) {
            handle.send(msg);
        }
    }

    fn drain(&mut self) -> Vec<(ServerId, InboundEvent)> {
        let mut out = Vec::new();
        for (&id, handle) in self.servers.iter_mut() {
            for ev in handle.try_recv_all() {
                out.push((id, ev));
            }
        }
        out
    }

    /// A raw `mpsc::Receiver` can't be peeked without consuming, and this
    /// trait deliberately carries no request/response bookkeeping (that's
    /// client-level state). Wake-up while servers are running is
    /// driven by the editor-side `LspState`'s heartbeat deadline instead.
    fn has_pending(&self) -> bool {
        false
    }

    fn shutdown(&mut self, server: ServerId) {
        // Removing the handle drops it: Drop runs kill -> wait -> join.
        self.servers.remove(&server);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[cfg(unix)]
    fn start_send_drain_round_trips_through_cat() {
        let root = std::env::current_dir().unwrap();
        let mut backend = ThreadedLspBackend::new();
        let id = backend.start("/bin/cat", &[], &root).expect("spawn cat");

        backend.send(
            id,
            Message::Notification {
                method: "ping".to_string(),
                params: serde_json::Value::Null,
            },
        );

        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        let mut got = false;
        while std::time::Instant::now() < deadline {
            for (sid, ev) in backend.drain() {
                if sid == id
                    && let InboundEvent::Message(Message::Notification { method, .. }) = ev
                {
                    assert_eq!(method, "ping");
                    got = true;
                }
            }
            if got {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        assert!(got, "cat should echo the notification back");

        backend.shutdown(id);
        // A second drain after shutdown must not panic or find the removed server.
        assert!(backend.drain().is_empty());
    }
}
