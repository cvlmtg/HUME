// The Steel surface for `spawn-async!`/`cancel-async!`.
// Portable half: the empty-cmd raise path and cancelling an unknown id —
// neither ever actually spawns a live child, so they need no `sh`. See
// `tests/unix/async_job_steel.rs` for the real-spawn end-to-end coverage
// (happy path, nonzero exit, missing binary, kill-on-cancel).

use std::path::Path;

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
fn empty_cmd_raises_naming_the_arg() {
    let tmp = safe_tempdir();
    let mut ed = editor_from("-[a]>bc\n");
    run(
        &mut ed,
        tmp.path(),
        r#"(define-command! "spawn-empty" "" (lambda ()
             (spawn-async! "" '() #f (lambda (out err code) (void)))))"#,
    );
    call(&mut ed, "spawn-empty");

    let msg = ed.state.status_msg.clone().unwrap_or_default();
    assert!(
        msg.contains("spawn-async!") && msg.contains("cmd"),
        "error should name the builtin and the empty cmd, got {msg:?}"
    );
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
