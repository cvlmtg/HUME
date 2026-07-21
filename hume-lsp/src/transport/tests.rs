use super::*;
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

#[cfg(unix)]
mod unix;
