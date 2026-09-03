// The Steel surface for `spawn-async!`/`cancel-async!`.
// Portable half: cancelling an unknown id, which never actually spawns a
// live child, so it needs no `sh`. See `tests/unix/async_job_steel.rs` for
// the real-spawn end-to-end coverage (happy path, nonzero exit, missing
// binary and empty-cmd spawn failures, kill-on-cancel).

use super::*;

fn call(ed: &mut Editor, name: &str) {
    ed.execute_keymap_command(name.to_string().into(), None, false);
}

#[test]
fn cancel_async_on_an_unknown_id_is_a_silent_no_op() {
    let tmp = safe_tempdir();
    let mut ed = editor_from("-[a]>bc\n");
    run(
        &mut ed,
        tmp.path(),
        r#"(define-command! "try-cancel" "" (lambda ()
             (cancel-async! 42)
             (log! 'info "done")))"#,
    );
    call(&mut ed, "try-cancel");

    assert_eq!(
        ed.state.status_msg.clone().unwrap(),
        "done",
        "cancel-async! on an unknown id must not raise — execution must reach past it"
    );
}
