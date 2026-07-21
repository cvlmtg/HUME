//! `/bin/cat` end-to-end tests, gated once at the `mod unix;` declaration in
//! the parent — the whole module is unix-only since both tests spawn
//! `/bin/cat` as a stand-in server.

use super::super::*;

#[test]
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

#[test]
fn start_threads_the_waker_through_to_the_spawned_server() {
    // Distinct from transport.rs's `cat_echo_fires_waker`, which pins the
    // reader loop's own wake calls: this pins that `ThreadedLspBackend`
    // forwards its own `wake` field into `ServerHandle::spawn` in the
    // first place (rather than, say, a stray no-op).
    let root = std::env::current_dir().unwrap();
    let (tx_wake, rx_wake) = std::sync::mpsc::channel::<()>();
    let wake: WakeCallback = Arc::new(move || {
        let _ = tx_wake.send(());
    });
    let mut backend = ThreadedLspBackend::with_waker(wake);
    let id = backend.start("/bin/cat", &[], &root).expect("spawn cat");

    backend.send(
        id,
        Message::Notification {
            method: "ping".to_string(),
            params: serde_json::Value::Null,
        },
    );

    rx_wake
        .recv_timeout(std::time::Duration::from_secs(5))
        .expect("start() must thread its own wake field into the spawned server");
}
