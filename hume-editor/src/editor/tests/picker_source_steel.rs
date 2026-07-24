// The Steel surface for `picker-source-spawn!` (`docs/FUZZY-FINDERS.md` B5).
// Portable half: the token gate, the empty-cmd/spawn-failure raise paths —
// none of these ever actually spawn a live child, so they need no `sh`.
// See `tests/unix/picker_source_steel.rs` for the real-spawn end-to-end
// coverage (happy path, #:nul, nonzero exit, kill-on-close).

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
fn stale_token_returns_false_without_spawning() {
    let tmp = safe_tempdir();
    let mut ed = editor_from("-[a]>bc\n");
    run(
        &mut ed,
        tmp.path(),
        r#"
        (define tok #f)
        (define-command! "go" "" (lambda ()
          (set! tok (picker! '() (lambda (x) (log! 'info (to-string x)))))))
        (define-command! "spawn-stale" "" (lambda ()
          (log! 'info (to-string
            (picker-source-spawn! (+ tok 1) "definitely-not-a-real-binary-xyz" '())))))
        "#,
    );
    type_cmd(&mut ed, ":go");
    call(&mut ed, "spawn-stale");

    assert_eq!(
        ed.state.status_msg.clone().unwrap(),
        "#false",
        "a stale token must return #f, not raise — the bogus binary name proves nothing was spawned"
    );
    assert!(ed.state.picker.is_some(), "the real picker must stay open");
}

#[test]
fn no_open_picker_returns_false_without_spawning() {
    let tmp = safe_tempdir();
    let mut ed = editor_from("-[a]>bc\n");
    run(
        &mut ed,
        tmp.path(),
        r#"(define-command! "spawn-none" "" (lambda ()
             (log! 'info (to-string
               (picker-source-spawn! 1 "definitely-not-a-real-binary-xyz" '())))))"#,
    );
    call(&mut ed, "spawn-none");

    assert_eq!(ed.state.status_msg.clone().unwrap(), "#false");
    assert!(ed.state.picker.is_none());
}

#[test]
fn spawn_failure_raises_and_leaves_the_picker_open() {
    let tmp = safe_tempdir();
    let mut ed = editor_from("-[a]>bc\n");
    run(
        &mut ed,
        tmp.path(),
        r#"
        (define tok #f)
        (define-command! "go" "" (lambda ()
          (set! tok (picker! '() (lambda (x) (void))))))
        (define-command! "spawn-bad" "" (lambda ()
          (picker-source-spawn! tok "definitely-not-a-real-binary-xyz" '())))
        "#,
    );
    type_cmd(&mut ed, ":go");
    call(&mut ed, "spawn-bad");

    let msg = ed.state.status_msg.clone().unwrap_or_default();
    assert!(
        msg.contains("picker-source-spawn!") && msg.contains("cannot run"),
        "error should name the builtin and the failure, got {msg:?}"
    );
    assert!(
        ed.state.picker.is_some(),
        "a failed spawn must not close the picker"
    );
}

#[test]
fn empty_cmd_raises_naming_the_arg() {
    let tmp = safe_tempdir();
    let mut ed = editor_from("-[a]>bc\n");
    run(
        &mut ed,
        tmp.path(),
        r#"
        (define tok #f)
        (define-command! "go" "" (lambda ()
          (set! tok (picker! '() (lambda (x) (void))))))
        (define-command! "spawn-empty" "" (lambda ()
          (picker-source-spawn! tok "" '())))
        "#,
    );
    type_cmd(&mut ed, ":go");
    call(&mut ed, "spawn-empty");

    let msg = ed.state.status_msg.clone().unwrap_or_default();
    assert!(
        msg.contains("picker-source-spawn!") && msg.contains("cmd"),
        "error should name the builtin and the empty cmd, got {msg:?}"
    );
}
