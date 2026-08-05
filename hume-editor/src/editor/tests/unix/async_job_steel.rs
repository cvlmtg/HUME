// `spawn-async!`/`cancel-async!` end-to-end through real Steel source with a
// real spawned child (`sh`/`sleep`) — unix-only. See `async_job_steel.rs`
// (portable, at the crate's `tests/` root) for the argument-validation
// coverage that never actually spawns anything, and `unix/async_job.rs` for
// the Rust-only registry/drain coverage that skips Steel entirely.

use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use super::*;
use crate::editor::dispatch::ArgSource;
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
fn happy_path_delivers_stdout_stderr_and_exit_code() {
    let tmp = safe_tempdir();
    let mut ed = editor_from("-[a]>bc\n");
    run(
        &mut ed,
        tmp.path(),
        r#"
        (define-command! "go" "" (lambda ()
          (spawn-async! "sh" (list "-c" "printf hi") #f
            (lambda (out err code)
              (log! 'info (string-append out "|" err "|" (number->string code)))))))
        "#,
    );
    call(&mut ed, "go");

    drain_until(&mut ed, |ed| ed.state.status_msg.is_some());

    assert_eq!(ed.state.status_msg.clone().unwrap(), "hi||0");
}

#[test]
fn nonzero_exit_and_stderr_reach_the_callback() {
    let tmp = safe_tempdir();
    let mut ed = editor_from("-[a]>bc\n");
    run(
        &mut ed,
        tmp.path(),
        r#"
        (define-command! "go" "" (lambda ()
          (spawn-async! "sh" (list "-c" "echo boom >&2; exit 3") #f
            (lambda (out err code)
              (log! 'info (string-append err "|" (number->string code)))))))
        "#,
    );
    call(&mut ed, "go");

    drain_until(&mut ed, |ed| ed.state.status_msg.is_some());

    assert_eq!(ed.state.status_msg.clone().unwrap(), "boom\n|3");
}

#[test]
fn missing_binary_fires_the_callback_with_code_negative_one() {
    let tmp = safe_tempdir();
    let mut ed = editor_from("-[a]>bc\n");
    run(
        &mut ed,
        tmp.path(),
        r#"
        (define-command! "go" "" (lambda ()
          (spawn-async! "definitely-not-a-real-binary-xyz" '() #f
            (lambda (out err code)
              (log! 'info (number->string code))))))
        "#,
    );
    call(&mut ed, "go");
    // A spawn failure fires its callback synchronously, inside
    // `spawn-async!` itself — no `drain_async_sources` needed, only the
    // queued-call drain.
    ed.settle();

    assert_eq!(ed.state.status_msg.clone().unwrap(), "-1");
}

#[test]
fn empty_cmd_fires_the_callback_instead_of_raising() {
    let tmp = safe_tempdir();
    let mut ed = editor_from("-[a]>bc\n");
    run(
        &mut ed,
        tmp.path(),
        r#"
        (define-command! "go" "" (lambda ()
          (spawn-async! "" '() #f
            (lambda (out err code)
              (log! 'info (number->string code))))))
        "#,
    );
    call(&mut ed, "go");
    // Same "fires synchronously inside spawn-async!" shape as the missing-
    // binary case above — no `drain_async_sources` needed.
    ed.settle();

    assert_eq!(
        ed.state.status_msg.clone().unwrap(),
        "-1",
        "an empty cmd must fire the documented failure triple, never raise"
    );
}

#[test]
fn spawn_failure_wakes_the_event_loop() {
    let tmp = safe_tempdir();
    let mut ed = editor_from("-[a]>bc\n");
    let woken = Arc::new(AtomicUsize::new(0));
    let counted = Arc::clone(&woken);
    ed.state.wake = Arc::new(move || {
        counted.fetch_add(1, Ordering::SeqCst);
    });
    run(
        &mut ed,
        tmp.path(),
        r#"
        (define-command! "go" "" (lambda ()
          (spawn-async! "definitely-not-a-real-binary-xyz" '() #f
            (lambda (out err code) (void)))))
        "#,
    );
    call(&mut ed, "go");

    assert_eq!(
        woken.load(Ordering::SeqCst),
        1,
        "a spawn failure must wake the loop the same way a completing job does, \
         so a callback chained from inside a queued Steel call isn't stranded \
         until the next keystroke"
    );
}

#[test]
fn cancel_async_prevents_the_callback_and_kills_the_child() {
    let tmp = safe_tempdir();
    let mut ed = editor_from("-[a]>bc\n");
    run(
        &mut ed,
        tmp.path(),
        r#"
        (define job-id #f)
        (define-command! "go" "" (lambda ()
          (set! job-id (spawn-async! "sleep" (list "30") #f
            (lambda (out err code) (log! 'info "must-not-fire"))))))
        (define-command! "cancel-it" "" (lambda () (cancel-async! job-id)))
        "#,
    );
    call(&mut ed, "go");

    let pid = ed
        .state
        .config
        .async_jobs
        .values()
        .next()
        .expect("job registered")
        .job
        .pid();

    let started = Instant::now();
    call(&mut ed, "cancel-it");
    assert!(
        started.elapsed() < Duration::from_secs(5),
        "cancel-async! must not block on the child's remaining lifetime, took {:?}",
        started.elapsed()
    );

    let alive = std::process::Command::new("kill")
        .args(["-0", &pid.to_string()])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .expect("spawn kill -0")
        .success();
    assert!(!alive, "cancel-async! must kill its job's child");

    ed.drain_async_sources();
    ed.settle();
    assert!(
        ed.state.status_msg.is_none(),
        "a cancelled job's callback must never fire"
    );
}
