//! `/bin/cat` + `/bin/sh` end-to-end tests that spawn a real child
//! process, gated once at the `mod unix;` declaration in the parent.

use super::*;
use crate::codec::RequestId;

#[test]
fn cat_echoes_frames_and_drop_reaps_without_hanging() {
    let root = std::env::current_dir().unwrap();
    let handle = ServerHandle::spawn(
        "/bin/cat",
        &[],
        Launch::with_process_path(&root),
        &[],
        no_op_wake(),
    )
    .expect("spawn cat");

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
    let handle = ServerHandle::spawn("/bin/cat", &[], Launch::with_process_path(&root), &[], wake)
        .expect("spawn cat");

    handle.send(Message::Notification {
        method: "ping".to_string(),
        params: serde_json::Value::Null,
    });

    rx_wake
        .recv_timeout(std::time::Duration::from_secs(5))
        .expect("reader thread must wake the loop after cat echoes the notification back");

    drop(handle);
}

/// `env` reaches the spawned process's actual environment: the real-process
/// link in the `#:env` chain (Steel decode → `LspServerConfig` → here).
/// Additive, not replacing: the child also inherits `PATH` well enough to
/// resolve `/bin/sh` itself, so this only proves the *extra* pair arrives,
/// not that the whole environment was clobbered.
#[test]
fn env_reaches_the_spawned_process() {
    let root = std::env::current_dir().unwrap();
    let handle = ServerHandle::spawn(
        "/bin/sh",
        &["-c".to_string(), "printenv HUME_TEST_VAR 1>&2".to_string()],
        Launch::with_process_path(&root),
        &[("HUME_TEST_VAR".to_string(), "steel-lsp-home".to_string())],
        no_op_wake(),
    )
    .expect("spawn sh");

    let stderr = stderr_until(&handle, "steel-lsp-home");
    assert!(
        stderr.contains("steel-lsp-home"),
        "child process must see HUME_TEST_VAR from `env`, got stderr: {stderr:?}"
    );

    drop(handle);
}

/// A server cannot answer a credential prompt, so git run by it fails at
/// once instead of waiting on the terminal.
#[test]
fn the_server_runs_with_git_terminal_prompts_off() {
    let root = std::env::current_dir().unwrap();
    let handle = ServerHandle::spawn(
        "/bin/sh",
        &[
            "-c".to_string(),
            "echo \"prompt=${GIT_TERMINAL_PROMPT-unset}\" 1>&2".to_string(),
        ],
        Launch::with_process_path(&root),
        &[],
        no_op_wake(),
    )
    .expect("spawn sh");

    assert_eq!(stderr_until(&handle, "prompt="), "prompt=0");
}

/// The server's stderr lines, joined, once one contains `needle` or five
/// seconds have passed.
fn stderr_until(handle: &ServerHandle, needle: &str) -> String {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    let mut stderr = String::new();
    while std::time::Instant::now() < deadline {
        for ev in handle.try_recv_all() {
            if let InboundEvent::Stderr(s) = ev {
                stderr.push_str(&s);
            }
        }
        if stderr.contains(needle) {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    stderr
}

#[test]
fn drop_does_not_hang_when_stderr_floods_past_the_bound() {
    // Guards a `Drop` deadlock with the bounded stderr channel: a thread blocked mid-`send` on a full channel is
    // NOT unblocked by `child.kill()` alone (killing only ends a
    // blocking *read*), so `Drop` must also close the receivers. On
    // regression this test hangs (caught by the harness's own test
    // timeout); on a correct `Drop` it returns promptly.
    let root = std::env::current_dir().unwrap();
    let handle = ServerHandle::spawn(
        "/bin/sh",
        &["-c".to_string(), "yes flood 1>&2".to_string()],
        Launch::with_process_path(&root),
        &[],
        no_op_wake(),
    )
    .expect("spawn sh");

    // Let stderr fill well past STDERR_CHANNEL_BOUND before draining. Poll
    // instead of a single fixed sleep: a loaded CI runner can take longer
    // than one short sleep to spawn the child and produce output.
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    let mut events = Vec::new();
    while std::time::Instant::now() < deadline {
        std::thread::sleep(std::time::Duration::from_millis(50));
        events = handle.try_recv_all();
        if events.iter().any(|e| matches!(e, InboundEvent::Stderr(_))) {
            break;
        }
    }

    assert!(
        events.iter().any(|e| matches!(e, InboundEvent::Stderr(_))),
        "expected at least one Stderr event from the flood"
    );

    drop(handle);
}
