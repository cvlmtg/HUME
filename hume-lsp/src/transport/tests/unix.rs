//! `/bin/cat` + `/bin/sh` end-to-end tests that spawn a real child
//! process, gated once at the `mod unix;` declaration in the parent.

use super::*;
use crate::codec::RequestId;

#[test]
fn cat_echoes_frames_and_drop_reaps_without_hanging() {
    let root = std::env::current_dir().unwrap();
    let mut handle = ServerHandle::spawn("/bin/cat", &[], &root, no_op_wake()).expect("spawn cat");

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
