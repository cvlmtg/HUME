// Steel timer surface: (after ms thunk),
// (cancel-timer! id), (debounce ms proc).

use super::*;
use crate::editor::scripting_setup::make_init_host;
use hume_scripting::ScriptingHost;

fn eval_with_real_host(ed: &mut Editor, host: &mut ScriptingHost, source: &str, tmp: &std::path::Path) {
    let init_path = tmp.join("init.scm");
    std::fs::write(&init_path, source).unwrap();
    let mut ih = make_init_host(&mut ed.state, &mut ed.view);
    host.eval_init(&init_path, 10_000, &mut ih, Default::default())
        .expect("eval_init");
}

#[test]
fn after_fires_once_past_its_deadline() {
    let tmp = tempfile::tempdir().unwrap();
    let mut ed = editor_from("-[a]>bcdef\n");
    let mut host = ScriptingHost::new();
    eval_with_real_host(
        &mut ed,
        &mut host,
        r#"(define-command! "start" "" (lambda () (after 0 (lambda () (call! "move-right")))))"#,
        tmp.path(),
    );
    ed.scripting = Some(host);

    type_cmd(&mut ed, ":start");
    ed.drain_async_sources();
    ed.drain_pending_steel_calls();

    assert_eq!(state(&ed), "a-[b]>cdef\n", "the thunk must fire once its 0ms deadline passes");
}

#[test]
fn a_timer_not_yet_due_does_not_fire() {
    let tmp = tempfile::tempdir().unwrap();
    let mut ed = editor_from("-[a]>bcdef\n");
    let mut host = ScriptingHost::new();
    eval_with_real_host(
        &mut ed,
        &mut host,
        r#"(define-command! "start" "" (lambda () (after 100000 (lambda () (call! "move-right")))))"#,
        tmp.path(),
    );
    ed.scripting = Some(host);

    type_cmd(&mut ed, ":start");
    ed.drain_async_sources();
    ed.drain_pending_steel_calls();

    assert_eq!(state(&ed), "-[a]>bcdef\n", "a far-future timer must not fire yet");
}

#[test]
fn cancel_timer_before_it_fires_prevents_the_thunk() {
    let tmp = tempfile::tempdir().unwrap();
    let mut ed = editor_from("-[a]>bcdef\n");
    let mut host = ScriptingHost::new();
    eval_with_real_host(
        &mut ed,
        &mut host,
        r#"(define-command! "start-and-cancel" "" (lambda ()
             (define id (after 0 (lambda () (call! "move-right"))))
             (cancel-timer! id)))"#,
        tmp.path(),
    );
    ed.scripting = Some(host);

    type_cmd(&mut ed, ":start-and-cancel");
    ed.drain_async_sources();
    ed.drain_pending_steel_calls();

    assert_eq!(
        state(&ed),
        "-[a]>bcdef\n",
        "a cancelled timer must never fire, even past its original deadline"
    );
}

#[test]
fn debounce_collapses_a_rapid_burst_into_one_trailing_call() {
    let tmp = tempfile::tempdir().unwrap();
    let mut ed = editor_from("-[a]>bcdef\n");
    let mut host = ScriptingHost::new();
    eval_with_real_host(
        &mut ed,
        &mut host,
        r#"(define tick (debounce 0 (lambda () (call! "move-right"))))
           (define-command! "burst" "" (lambda () (tick) (tick) (tick)))"#,
        tmp.path(),
    );
    ed.scripting = Some(host);

    type_cmd(&mut ed, ":burst");
    ed.drain_async_sources();
    ed.drain_pending_steel_calls();

    assert_eq!(
        state(&ed),
        "a-[b]>cdef\n",
        "three rapid debounced calls must collapse to exactly one trailing invocation, \
         not fire three times"
    );
}

#[test]
fn an_erroring_thunk_lands_in_the_message_log_and_the_wheel_survives() {
    let tmp = tempfile::tempdir().unwrap();
    let mut ed = editor_from("-[a]>bcdef\n");
    let mut host = ScriptingHost::new();
    eval_with_real_host(
        &mut ed,
        &mut host,
        r#"(define-command! "boom" "" (lambda () (after 0 (lambda () (car '())))))
           (define-command! "start" "" (lambda () (after 0 (lambda () (call! "move-right")))))"#,
        tmp.path(),
    );
    ed.scripting = Some(host);

    // First drain cycle: the erroring thunk is the only one queued, so its
    // error can't swallow anything else — isolates the "reported, not
    // panicking" assertion from the "first error aborts the rest of this
    // batch" semantics (same `run_steel_calls`).
    type_cmd(&mut ed, ":boom");
    ed.drain_async_sources();
    ed.drain_pending_steel_calls(); // must not panic

    let log = ed.state.message_log.format_for_display();
    assert!(
        log.contains("steel call error"),
        "the erroring thunk must be reported, not crash: {log:?}"
    );

    // Second, separate drain cycle: a fresh timer still schedules and fires
    // normally — the wheel/thunk table weren't left in a broken state.
    type_cmd(&mut ed, ":start");
    ed.drain_async_sources();
    ed.drain_pending_steel_calls();

    assert_eq!(
        state(&ed),
        "a-[b]>cdef\n",
        "a later, unrelated timer must still fire after an earlier one errored"
    );
}
