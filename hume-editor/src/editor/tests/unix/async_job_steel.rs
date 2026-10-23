// `spawn-async!`/`cancel-async!` end-to-end through real Steel source with a
// real spawned child (`sh`/`sleep`), unix-only. See `async_job_steel.rs`
// (portable, at the crate's `tests/` root) for the argument-validation
// coverage that never actually spawns anything, and `unix/async_job.rs` for
// the Rust-only registry/drain coverage that skips Steel entirely.

use std::time::{Duration, Instant};

use hume_platform::wake::counting_wake;

use super::*;

#[test]
fn happy_path_delivers_stdout_stderr_and_exit_code() {
    let tmp = tempfile::tempdir().unwrap();
    let mut ed = editor_from("-[a]>bc\n");
    run(
        &mut ed,
        tmp.path(),
        r#"
        (define-command! "go" "" (lambda ()
          (spawn-async! "sh" (list "-c" "printf hi")
            (lambda (out err code)
              (log! 'info (string-append out "|" err "|" (number->string code)))))))
        "#,
    );
    call(&mut ed, "go");

    drain_until(&mut ed, |ed| ed.state.status_msg.is_some());

    assert_eq!(ed.state.status_msg.clone().unwrap(), "hi||0");
}

#[test]
fn cwd_keyword_sets_the_child_working_directory() {
    let tmp = tempfile::tempdir().unwrap();
    let mut ed = editor_from("-[a]>bc\n");
    run(
        &mut ed,
        tmp.path(),
        r#"
        (define-command! "go" "" (lambda ()
          (spawn-async! "pwd" '()
            (lambda (out err code) (log! 'info (trim out)))
            #:cwd "/")))
        "#,
    );
    call(&mut ed, "go");

    drain_until(&mut ed, |ed| ed.state.status_msg.is_some());

    assert_eq!(ed.state.status_msg.clone().unwrap(), "/");
}

#[test]
fn nonzero_exit_and_stderr_reach_the_callback() {
    let tmp = tempfile::tempdir().unwrap();
    let mut ed = editor_from("-[a]>bc\n");
    run(
        &mut ed,
        tmp.path(),
        r#"
        (define-command! "go" "" (lambda ()
          (spawn-async! "sh" (list "-c" "echo boom >&2; exit 3")
            (lambda (out err code)
              (log! 'info (string-append err "|" (number->string code)))))))
        "#,
    );
    call(&mut ed, "go");

    drain_until(&mut ed, |ed| ed.state.status_msg.is_some());

    assert_eq!(ed.state.status_msg.clone().unwrap(), "boom\n|3");
}

#[test]
fn missing_binary_fires_the_callback_with_no_exit_code() {
    let tmp = tempfile::tempdir().unwrap();
    let mut ed = editor_from("-[a]>bc\n");
    run(
        &mut ed,
        tmp.path(),
        r#"
        (define-command! "go" "" (lambda ()
          (spawn-async! "definitely-not-a-real-binary-xyz" '()
            (lambda (out err code)
              (log! 'info (if code (number->string code) "no exit code"))))))
        "#,
    );
    call(&mut ed, "go");
    // A spawn failure fires its callback synchronously, inside
    // `spawn-async!` itself: no `drain_async_sources` needed, only the
    // queued-call drain.
    ed.settle();

    assert_eq!(ed.state.status_msg.clone().unwrap(), "no exit code");
}

#[test]
fn empty_cmd_fires_the_callback_instead_of_raising() {
    let tmp = tempfile::tempdir().unwrap();
    let mut ed = editor_from("-[a]>bc\n");
    run(
        &mut ed,
        tmp.path(),
        r#"
        (define-command! "go" "" (lambda ()
          (spawn-async! "" '()
            (lambda (out err code)
              (log! 'info (if code (number->string code) "no exit code"))))))
        "#,
    );
    call(&mut ed, "go");
    // Same "fires synchronously inside spawn-async!" shape as the missing-
    // binary case above: no `drain_async_sources` needed.
    ed.settle();

    assert_eq!(
        ed.state.status_msg.clone().unwrap(),
        "no exit code",
        "an empty cmd must fire the documented failure triple, never raise"
    );
}

#[test]
fn spawn_failure_wakes_the_event_loop() {
    let tmp = tempfile::tempdir().unwrap();
    let mut ed = editor_from("-[a]>bc\n");
    let (wake, woken) = counting_wake();
    ed.state.wake = wake;
    run(
        &mut ed,
        tmp.path(),
        r#"
        (define-command! "go" "" (lambda ()
          (spawn-async! "definitely-not-a-real-binary-xyz" '()
            (lambda (out err code) (void)))))
        "#,
    );
    call(&mut ed, "go");

    assert_eq!(
        woken.try_iter().count(),
        1,
        "a spawn failure must wake the loop the same way a completing job does, \
         so a callback chained from inside a queued Steel call isn't stranded \
         until the next keystroke"
    );
}

#[test]
fn cancel_async_prevents_the_callback_and_kills_the_child() {
    let tmp = tempfile::tempdir().unwrap();
    let mut ed = editor_from("-[a]>bc\n");
    run(
        &mut ed,
        tmp.path(),
        r#"
        (define job-id #f)
        (define-command! "go" "" (lambda ()
          (set! job-id (spawn-async! "sleep" (list "30")
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
        .process()
        .pid();

    let started = Instant::now();
    call(&mut ed, "cancel-it");
    assert!(
        started.elapsed() < Duration::from_secs(5),
        "cancel-async! must not block on the child's remaining lifetime, took {:?}",
        started.elapsed()
    );

    assert!(!is_alive(pid), "cancel-async! must kill its job's child");

    ed.drain_async_sources();
    ed.settle();
    assert!(
        ed.state.status_msg.is_none(),
        "a cancelled job's callback must never fire"
    );
}

/// An edit a job's callback makes is background work's, not the user's: a
/// bare `p` after it still pastes the kill-ring head.
#[test]
fn an_async_job_callback_edit_leaves_the_ring_stamp_fresh() {
    let tmp = tempfile::tempdir().unwrap();
    let mut ed = editor_from("-[ab]>cd\n");
    seed_clipboard(&mut ed, "CLIP");
    run(
        &mut ed,
        tmp.path(),
        r#"
        (define-command! "go" "" (lambda ()
          (spawn-async! "sh" (list "-c" "true")
            (lambda (out err code)
              (edit-text! (focused-pane)
                (list (hash 'start-char 1 'end-char 1 'text "!")))))))
        "#,
    );
    ed.feed_key(key('d'));
    call(&mut ed, "go");

    drain_until(&mut ed, |ed| ed.doc().text().to_string() == "c!d\n");

    ed.feed_key(key('p'));
    assert_eq!(ed.doc().text().to_string(), "cab!d\n");
}

// ── A callback due when it is cancelled ─────────────────────────────────────

/// Runs `start`, which binds `job-id`, lets its callback (logging `fired`)
/// come due, then cancels it before it runs.
fn cancel_a_due_callback(ed: &mut Editor, start: &str) {
    let tmp = tempfile::tempdir().unwrap();
    run(
        ed,
        tmp.path(),
        &format!(
            r#"
            (define job-id #f)
            (define-command! "stop" "" (lambda () (cancel-async! job-id)))
            (define-command! "go" "" (lambda () (set! job-id {start})))
            "#
        ),
    );
    call(ed, "go");
    drain_sources_until(ed, |ed| !ed.state.config.pending_work.is_empty());
    call(ed, "stop");
    ed.settle();
}

#[test]
fn a_result_due_before_cancel_async_never_fires() {
    let mut ed = editor_from("-[a]>bc\n");
    cancel_a_due_callback(
        &mut ed,
        r#"(spawn-async! "sh" (list "-c" "printf hi")
             (lambda (out err code) (log! 'info "fired")))"#,
    );
    assert_eq!(status(&ed), "");
}

#[test]
fn a_report_due_before_cancel_async_never_fires() {
    let bin = tempfile::tempdir().unwrap();
    hume_platform::path::test_support::write_program(bin.path(), "hume-scan-probe", 0o755);
    let mut ed = editor_from("-[a]>bc\n");
    ed.state.dirs.search_path = Some(bin.path().into());
    cancel_a_due_callback(
        &mut ed,
        r#"(path-programs "hume-scan" '()
             (lambda (names done?) (log! 'info "fired")))"#,
    );
    assert_eq!(status(&ed), "");
}

#[test]
fn a_failed_spawn_cancelled_at_once_never_fires() {
    let tmp = tempfile::tempdir().unwrap();
    let mut ed = editor_from("-[a]>bc\n");
    run(
        &mut ed,
        tmp.path(),
        r#"
        (define-command! "go" "" (lambda ()
          (cancel-async!
            (spawn-async! "definitely-not-a-real-binary-xyz" '()
              (lambda (out err code) (log! 'info "fired"))))))
        "#,
    );
    call(&mut ed, "go");
    ed.settle();

    assert_eq!(status(&ed), "");
}

#[test]
fn a_scan_whose_callback_raises_is_cancelled() {
    let tmp = tempfile::tempdir().unwrap();
    let mut ed = editor_from("-[a]>bc\n");
    ed.state.dirs.search_path = None;
    run(
        &mut ed,
        tmp.path(),
        r#"
        (define-command! "go" "" (lambda ()
          (path-programs "" '() (lambda (names done?) (error "boom")))))
        "#,
    );
    call(&mut ed, "go");
    // The scan the command started is swapped, before any drain reads it,
    // for one that reports once and then waits, so a second report can
    // only come from a scan still registered.
    let (release, gate) = std::sync::mpsc::channel::<()>();
    let gated = hume_platform::snapshots::spawn_snapshots(
        "hume-test-scan",
        std::sync::Arc::clone(&ed.state.wake),
        move |reporter| {
            reporter.send(std::sync::Arc::from([std::sync::Arc::from("first")]));
            let _ = gate.recv();
            reporter.send(std::sync::Arc::from([std::sync::Arc::from("second")]));
        },
    )
    .expect("spawn");
    let Some(crate::editor::async_job::PendingJob::Live { work, .. }) =
        ed.state.config.async_jobs.values_mut().next()
    else {
        panic!("the scan is registered");
    };
    *work = crate::editor::async_job::AsyncWork::Names(gated);

    drain_until(&mut ed, |ed| has_error(ed, "boom"));
    assert!(ed.state.config.async_jobs.is_empty());

    release.send(()).expect("the producer waits");
    std::thread::sleep(Duration::from_millis(50));
    ed.settle();
    let booms = ed
        .state
        .message_log
        .entries()
        .filter(|e| e.text.contains("boom"))
        .count();
    assert_eq!(booms, 1);
}
