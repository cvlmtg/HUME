// The Steel surface for `spawn-async!`/`cancel-async!`.
// Portable half: jobs that never start a live child, so it needs no `sh`.
// See `tests/unix/async_job_steel.rs` for the real-spawn end-to-end
// coverage (happy path, nonzero exit, missing binary and empty-cmd spawn
// failures, kill-on-cancel, cross-kind handles).

use super::*;

#[test]
fn cancel_async_twice_is_a_silent_no_op() {
    let tmp = tempfile::tempdir().unwrap();
    let mut ed = editor_from("-[a]>bc\n");
    run(
        &mut ed,
        tmp.path(),
        r#"(define-command! "try-cancel" "" (lambda ()
             (let ((job (spawn-async! "hume-no-such-program" '() (lambda (result) (void)))))
               (cancel-async! job)
               (cancel-async! job)
               (log! 'info "done"))))"#,
    );
    call(&mut ed, "try-cancel");

    assert_eq!(status(&ed), "done", "errors: {:?}", error_log(&ed));
}

#[test]
fn cancel_async_raises_for_an_integer() {
    let tmp = tempfile::tempdir().unwrap();
    let mut ed = editor_from("-[a]>bc\n");
    run(
        &mut ed,
        tmp.path(),
        r#"(define-command! "try-cancel" "" (lambda () (cancel-async! 0)))"#,
    );
    call(&mut ed, "try-cancel");

    assert!(
        has_error(&ed, "cancel-async!: job: expected a job"),
        "errors: {:?}",
        error_log(&ed)
    );
}
