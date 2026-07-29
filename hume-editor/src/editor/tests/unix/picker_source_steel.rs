// `picker-source-spawn!` end-to-end through real Steel source with a real
// spawned child (`sh`) — unix-only. See `picker_source_steel.rs` (portable)
// for the token-gate/raise-path coverage that never actually spawns
// anything, and `unix/picker_source.rs` for the Rust-only drain coverage
// that skips Steel entirely.

use std::path::Path;
use std::time::{Duration, Instant};

use super::*;
use crate::editor::dispatch::ArgSource;
use hume_engine::pipeline::RenderContext;
use hume_scripting::ScriptingHost;

fn run(ed: &mut Editor, tmp: &Path, source: &str) {
    let mut host = ScriptingHost::new();
    eval_with_real_host(ed, &mut host, source, tmp);
    ed.scripting = Some(host);
}

fn call(ed: &mut Editor, name: &str) {
    ed.execute_keymap_command(name.to_string().into(), None, false, ArgSource::Keymap);
}

#[test]
fn happy_path_streams_lines_and_accept_returns_the_raw_line() {
    let tmp = safe_tempdir();
    let mut ed = editor_from("-[a]>bc\n");
    run(
        &mut ed,
        tmp.path(),
        r#"
        (define tok #f)
        (define-command! "go" "" (lambda ()
          (set! tok (picker! '() (lambda (x) (log! 'info (to-string x)))))))
        (define-command! "spawn-it" "" (lambda ()
          (picker-source-spawn! tok "sh" (list "-c" "printf 'a\nb\nc\n'"))))
        "#,
    );
    type_cmd(&mut ed, ":go");
    call(&mut ed, "spawn-it");

    drain_until_picker_total(&mut ed, 3);

    let mut ctx = RenderContext::new();
    ed.prepare_frame(40, 12, &mut ctx);
    ed.feed_key(key_enter()); // top-ranked row (insertion order, empty query) is "a"
    ed.drain_pending_steel_calls();

    assert_eq!(
        ed.state.status_msg.clone().unwrap(),
        "a",
        "on-select must receive the raw streamed line as payload (display == payload)"
    );
    assert!(ed.state.config.picker.is_none());
}

#[test]
fn nul_delimited_source_splits_on_nul() {
    let tmp = safe_tempdir();
    let mut ed = editor_from("-[a]>bc\n");
    run(
        &mut ed,
        tmp.path(),
        r#"
        (define tok #f)
        (define-command! "go" "" (lambda ()
          (set! tok (picker! '() (lambda (x) (void))))))
        (define-command! "spawn-it" "" (lambda ()
          (picker-source-spawn! tok "sh" (list "-c" "printf 'a\\0b'") #:nul #t)))
        "#,
    );
    type_cmd(&mut ed, ":go");
    call(&mut ed, "spawn-it");

    drain_until_picker_total(&mut ed, 2);

    let picker = ed.state.config.picker.as_ref().unwrap();
    assert_eq!(picker.window(10).collect::<Vec<_>>(), vec!["a", "b"]);
}

#[test]
fn nonzero_exit_reports_a_status_message() {
    let tmp = safe_tempdir();
    let mut ed = editor_from("-[a]>bc\n");
    run(
        &mut ed,
        tmp.path(),
        r#"
        (define tok #f)
        (define-command! "go" "" (lambda ()
          (set! tok (picker! '() (lambda (x) (void))))))
        (define-command! "spawn-it" "" (lambda ()
          (picker-source-spawn! tok "sh" (list "-c" "echo boom >&2; exit 4"))))
        "#,
    );
    type_cmd(&mut ed, ":go");
    ed.state.status_msg = None;
    call(&mut ed, "spawn-it");

    drain_until(&mut ed, |ed| ed.state.status_msg.is_some());

    let msg = ed.state.status_msg.clone().unwrap();
    assert!(msg.contains("boom"), "got: {msg}");
}

#[test]
fn picker_close_kills_the_source_child() {
    let tmp = safe_tempdir();
    let mut ed = editor_from("-[a]>bc\n");
    run(
        &mut ed,
        tmp.path(),
        r#"
        (define tok #f)
        (define-command! "go" "" (lambda ()
          (set! tok (picker! '() (lambda (x) (log! 'info (to-string x)))))))
        (define-command! "spawn-it" "" (lambda ()
          (picker-source-spawn! tok "sleep" (list "30"))))
        "#,
    );
    type_cmd(&mut ed, ":go");
    call(&mut ed, "spawn-it");

    let pid = ed
        .state
        .config
        .picker
        .as_ref()
        .unwrap()
        .source_pid_for_test()
        .expect("source attached");

    ed.state.status_msg = None;
    let started = Instant::now();
    ed.feed_key(key_esc());
    ed.drain_pending_steel_calls();
    // `sleep 30` makes a broken kill observable two ways: the liveness
    // check below (a wait()-only Drop still reaps it, just 30s later) AND —
    // the check that actually catches that case fast — Esc itself must not
    // block for the child's remaining lifetime.
    assert!(
        started.elapsed() < Duration::from_secs(5),
        "closing the picker must not block on the source child's lifetime, took {:?}",
        started.elapsed()
    );

    assert!(ed.state.config.picker.is_none());
    assert_eq!(
        ed.state.status_msg.clone().unwrap(),
        "#false",
        "on-select must fire with #f exactly once on close"
    );

    let alive = std::process::Command::new("kill")
        .args(["-0", &pid.to_string()])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .expect("spawn kill -0")
        .success();
    assert!(!alive, "closing the picker must kill its source child");

    // LESSONS.md L4: keep interacting past the terminal action.
    ed.feed_key(key('i'));
    ed.feed_key(key('Z'));
    ed.feed_key(key_esc());
    ed.drain_pending_steel_calls();
    let bid = ed.focused_buffer_id();
    let text = ed.state.buffers.get(bid).text().to_string();
    assert!(
        text.contains('Z'),
        "keys after close must behave as plain input"
    );
}
