// The Steel surface for `picker-source-spawn!`.
// Portable half: the token gate, the empty-cmd/spawn-failure raise paths —
// none of these ever actually spawn a live child, so they need no `sh`.
// See `tests/unix/picker_source_steel.rs` for the real-spawn end-to-end
// coverage (happy path, #:nul, nonzero exit, kill-on-close).

use super::*;
use crate::editor::dispatch::ArgSource;

fn call(ed: &mut Editor, name: &str) {
    ed.execute_keymap_command(name.to_string().into(), None, false, ArgSource::Keymap);
}

fn editor_with(source: &str) -> (Editor, tempfile::TempDir) {
    let tmp = safe_tempdir();
    let mut ed = editor_from("-[a]>bc\n");
    run(&mut ed, tmp.path(), source);
    (ed, tmp)
}

#[test]
fn stale_token_returns_false_without_spawning() {
    let (mut ed, _tmp) = editor_with(
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
    assert!(
        ed.state.config.picker.is_some(),
        "the real picker must stay open"
    );
}

#[test]
fn no_open_picker_returns_false_without_spawning() {
    let (mut ed, _tmp) = editor_with(
        r#"(define-command! "spawn-none" "" (lambda ()
             (log! 'info (to-string
               (picker-source-spawn! 1 "definitely-not-a-real-binary-xyz" '())))))"#,
    );
    call(&mut ed, "spawn-none");

    assert_eq!(ed.state.status_msg.clone().unwrap(), "#false");
    assert!(ed.state.config.picker.is_none());
}

#[test]
fn spawn_failure_raises_and_leaves_the_picker_open() {
    let (mut ed, _tmp) = editor_with(
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
        ed.state.config.picker.is_some(),
        "a failed spawn must not close the picker"
    );
}

#[test]
fn empty_cmd_raises_naming_the_arg() {
    let (mut ed, _tmp) = editor_with(
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

// ── #:ok-exit-codes rejects values outside i32's range ──────────────────────
//
// `#:ok-exit-codes` decodes before the spawn call (see
// `ui::picker_source_spawn`), so an out-of-range code raises before anything
// is spawned — the bogus binary name below proves that.

#[test]
fn ok_exit_codes_rejects_a_value_outside_i32_range() {
    let (mut ed, _tmp) = editor_with(
        r#"
        (define tok #f)
        (define-command! "go" "" (lambda ()
          (set! tok (picker! '() (lambda (x) (void))))))
        (define-command! "spawn-it" "" (lambda ()
          (picker-source-spawn! tok "definitely-not-a-real-binary-xyz" '()
            #:ok-exit-codes (list 4294967297))))
        "#,
    );
    type_cmd(&mut ed, ":go");
    call(&mut ed, "spawn-it");

    let msg = ed.state.status_msg.clone().unwrap_or_default();
    assert!(
        msg.contains("ok-exit-codes") && msg.contains("range"),
        "error should name the offending argument, got {msg:?}"
    );
    assert!(
        ed.state
            .config
            .picker
            .as_ref()
            .is_some_and(|p| p.total_len() == 0),
        "a rejected argument must not spawn anything"
    );
}
