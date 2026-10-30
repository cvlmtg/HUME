// Steel timer surface: (after! ms thunk),
// (cancel-timer! id), (debounce ms proc).

use super::*;
use hume_platform::dirs::Dirs;
use hume_scripting::ScriptingHost;

#[test]
fn after_fires_once_past_its_deadline() {
    let tmp = tempfile::tempdir().unwrap();
    let mut ed = editor_from("-[a]>bcdef\n");
    let mut host = ScriptingHost::for_testing(&Dirs::none());
    eval_with_real_host(
        &mut ed,
        &mut host,
        r#"(define-typed-command! "start" "" (lambda () (after! 0 (lambda () (call! "move-right" (focused-pane))))))"#,
        tmp.path(),
    );
    ed.scripting = Some(host);

    type_cmd(&mut ed, ":start");
    ed.drain_async_sources();
    ed.settle();

    assert_eq!(
        state(&ed),
        "a-[b]>cdef\n",
        "the thunk must fire once its 0ms deadline passes"
    );
}

#[test]
fn a_timer_not_yet_due_does_not_fire() {
    let tmp = tempfile::tempdir().unwrap();
    let mut ed = editor_from("-[a]>bcdef\n");
    let mut host = ScriptingHost::for_testing(&Dirs::none());
    eval_with_real_host(
        &mut ed,
        &mut host,
        r#"(define-typed-command! "start" "" (lambda () (after! 100000 (lambda () (call! "move-right" (focused-pane))))))"#,
        tmp.path(),
    );
    ed.scripting = Some(host);

    type_cmd(&mut ed, ":start");
    ed.drain_async_sources();
    ed.settle();

    assert_eq!(
        state(&ed),
        "-[a]>bcdef\n",
        "a far-future timer must not fire yet"
    );
}

#[test]
fn cancel_timer_before_it_fires_prevents_the_thunk() {
    let tmp = tempfile::tempdir().unwrap();
    let mut ed = editor_from("-[a]>bcdef\n");
    let mut host = ScriptingHost::for_testing(&Dirs::none());
    eval_with_real_host(
        &mut ed,
        &mut host,
        r#"(define-typed-command! "start-and-cancel" "" (lambda ()
             (define id (after! 0 (lambda () (call! "move-right" (focused-pane)))))
             (cancel-timer! id)))"#,
        tmp.path(),
    );
    ed.scripting = Some(host);

    type_cmd(&mut ed, ":start-and-cancel");
    ed.drain_async_sources();
    ed.settle();

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
    let mut host = ScriptingHost::for_testing(&Dirs::none());
    eval_with_real_host(
        &mut ed,
        &mut host,
        r#"(define tick (debounce 0 (lambda () (call! "move-right" (focused-pane)))))
           (define-typed-command! "burst" "" (lambda () (tick) (tick) (tick)))"#,
        tmp.path(),
    );
    ed.scripting = Some(host);

    type_cmd(&mut ed, ":burst");
    ed.drain_async_sources();
    ed.settle();

    assert_eq!(
        state(&ed),
        "a-[b]>cdef\n",
        "three rapid debounced calls must collapse to exactly one trailing invocation, \
         not fire three times"
    );
}

/// The bug `debounce-by` exists to fix: `debounce` shares one pending timer
/// across every call regardless of args, so a call keyed `"b"` cancels a
/// still-pending call keyed `"a"` (this is exactly `lsp/refresh-hints`'
/// production shape: two attached buffers, each firing `on-diagnostics-
/// changed` independently, sharing one `debounce`-wrapped handler).
#[test]
fn debounce_shares_one_pending_timer_across_all_keys() {
    let tmp = tempfile::tempdir().unwrap();
    let mut ed = editor_from("-[a]>bcdef\n");
    let mut host = ScriptingHost::for_testing(&Dirs::none());
    eval_with_real_host(
        &mut ed,
        &mut host,
        r#"(define tick (debounce 0 (lambda (key) (log! 'warning (string-append "fired:" key)))))
           (define-typed-command! "burst" "" (lambda () (tick "a") (tick "b")))"#,
        tmp.path(),
    );
    ed.scripting = Some(host);

    type_cmd(&mut ed, ":burst");
    ed.drain_async_sources();
    ed.settle();

    let log = ed.state.message_log.format_for_display();
    assert!(
        !log.contains("fired:a") && log.contains("fired:b"),
        "a plain `debounce`'s single shared timer must drop the \"a\" call \
         when \"b\" reschedules it: {log:?}"
    );
}

/// `debounce-by`'s fix: two independently-keyed calls in the same burst both
/// fire; neither key's pending timer cancels the other's.
#[test]
fn debounce_by_keys_pending_timers_independently() {
    let tmp = tempfile::tempdir().unwrap();
    let mut ed = editor_from("-[a]>bcdef\n");
    let mut host = ScriptingHost::for_testing(&Dirs::none());
    eval_with_real_host(
        &mut ed,
        &mut host,
        r#"(define tick (debounce-by 0 (lambda (key) (log! 'warning (string-append "fired:" key)))))
           (define-typed-command! "burst" "" (lambda ()
             (tick "a") (tick "a") (tick "a")
             (tick "b") (tick "b")))"#,
        tmp.path(),
    );
    ed.scripting = Some(host);

    type_cmd(&mut ed, ":burst");
    ed.drain_async_sources();
    ed.settle();

    let log = ed.state.message_log.format_for_display();
    assert_eq!(
        log.matches("fired:a").count(),
        1,
        "three same-key calls must still collapse to one trailing fire: {log:?}"
    );
    assert_eq!(
        log.matches("fired:b").count(),
        1,
        "a different key's calls must fire independently, not get cancelled \
         by \"a\"'s reschedule: {log:?}"
    );
}

#[test]
fn an_erroring_thunk_lands_in_the_message_log_and_the_wheel_survives() {
    let tmp = tempfile::tempdir().unwrap();
    let mut ed = editor_from("-[a]>bcdef\n");
    let mut host = ScriptingHost::for_testing(&Dirs::none());
    eval_with_real_host(
        &mut ed,
        &mut host,
        r#"(define-typed-command! "boom" "" (lambda () (after! 0 (lambda () (car '())))))
           (define-typed-command! "start" "" (lambda () (after! 0 (lambda () (call! "move-right" (focused-pane))))))"#,
        tmp.path(),
    );
    ed.scripting = Some(host);

    // First drain cycle: the erroring thunk is the only one queued, so its
    // error can't swallow anything else. Isolates the "reported, not
    // panicking" assertion from the "first error aborts the rest of this
    // batch" semantics (same `run_steel_calls`).
    type_cmd(&mut ed, ":boom");
    ed.drain_async_sources();
    ed.settle(); // must not panic

    let log = ed.state.message_log.format_for_display();
    assert!(
        log.contains("steel call error"),
        "the erroring thunk must be reported, not crash: {log:?}"
    );

    // Second, separate drain cycle: a fresh timer still schedules and fires
    // normally; the wheel/thunk table weren't left in a broken state.
    type_cmd(&mut ed, ":start");
    ed.drain_async_sources();
    ed.settle();

    assert_eq!(
        state(&ed),
        "a-[b]>cdef\n",
        "a later, unrelated timer must still fire after an earlier one errored"
    );
}

// ── A thunk that is due but has not run ─────────────────────────────────────

/// An editor whose `go` command runs `go` and whose `stop` command runs
/// `stop`, after the Scheme `prelude`.
fn timer_commands(prelude: &str, go: &str, stop: &str) -> Editor {
    let tmp = tempfile::tempdir().unwrap();
    let mut ed = editor_from("-[a]>bcdef\n");
    run(
        &mut ed,
        tmp.path(),
        &format!(
            r#"{prelude}
               (define-command! "go" "" (lambda () {go}))
               (define-command! "stop" "" (lambda () {stop}))"#
        ),
    );
    ed
}

#[test]
fn cancel_timer_drops_a_thunk_already_queued() {
    let mut ed = timer_commands(
        "(define id #f)",
        r#"(set! id (after! 0 (lambda () (log! 'info "fired"))))"#,
        "(cancel-timer! id)",
    );
    call(&mut ed, "go");
    ed.drain_async_sources();
    assert!(
        !ed.state.config.pending_work.is_empty(),
        "setup: the thunk is queued"
    );
    call(&mut ed, "stop");
    ed.settle();
    assert_eq!(status(&ed), "");
}

#[test]
fn reload_forgets_a_thunk_already_queued() {
    let mut ed = timer_commands(
        "(define id #f)",
        r#"(set! id (after! 0 (lambda () (log! 'info "fired"))))"#,
        "(cancel-timer! id)",
    );
    call(&mut ed, "go");
    ed.drain_async_sources();
    assert!(
        !ed.state.config.pending_work.is_empty(),
        "setup: the thunk is queued"
    );

    ed.reset_config_state();

    assert!(ed.timers.is_empty());
}

#[test]
fn debounce_called_again_while_its_thunk_is_queued_runs_once() {
    let mut ed = timer_commands(
        r#"(define count 0)
           (define tick (debounce 0 (lambda ()
             (set! count (+ count 1))
             (log! 'info (number->string count)))))"#,
        "(tick)",
        "(tick)",
    );
    call(&mut ed, "go");
    ed.drain_async_sources();
    assert!(
        !ed.state.config.pending_work.is_empty(),
        "setup: the thunk is queued"
    );
    call(&mut ed, "stop");
    ed.settle();
    ed.drain_async_sources();
    ed.settle();
    assert_eq!(status(&ed), "1");
}

#[test]
fn cancelling_a_thunk_leaves_a_native_timer_scheduled() {
    let mut ed = editor_from("-[a]>bcdef\n");
    let bid = ed.focused_buffer_id();
    let native = ed.timers.schedule(
        std::time::Duration::from_millis(10),
        crate::editor::timer_bridge::TimerPayload::DiagnosticPullDebounce(bid),
    );
    ed.timers.cancel_thunk(native);
    let due = ed
        .timers
        .take_due(std::time::Instant::now() + std::time::Duration::from_millis(20));
    assert_eq!(due.len(), 1);
}

#[test]
fn a_queued_thunk_lost_to_the_drain_cap_frees_its_timer() {
    let mut ed = timer_commands(
        "(define id #f)",
        r#"(set! id (after! 0 (lambda () (log! 'info "fired"))))"#,
        "(cancel-timer! id)",
    );
    call(&mut ed, "go");
    ed.drain_async_sources();
    // Past `MAX_EVENT_DRAIN` (1000, `event/pump.rs`), so the whole batch,
    // the queued thunk included, is dropped unrun.
    for _ in 0..1100 {
        ed.state
            .config
            .pending_work
            .push_back(crate::editor::event::PendingWork::Event(
                crate::editor::event::EditorEvent::OnFocusGained,
            ));
    }
    assert!(!ed.drain_pending_work());
    assert!(ed.timers.is_empty());
}

#[test]
fn settle_until_idle_runs_a_chain_of_timers_to_the_end() {
    let tmp = tempfile::tempdir().unwrap();
    let mut ed = editor_from("-[a]>bcdef\n");
    let mut host = ScriptingHost::for_testing(&Dirs::none());
    eval_with_real_host(
        &mut ed,
        &mut host,
        r#"(define-typed-command! "start" ""
             (lambda ()
               (after! 20 (lambda ()
                 (call! "move-right" (focused-pane))
                 (after! 20 (lambda () (call! "move-right" (focused-pane))))))))"#,
        tmp.path(),
    );
    ed.scripting = Some(host);

    let started = std::time::Instant::now();
    type_cmd(&mut ed, ":start");
    settle_until_idle(&mut ed);

    assert_eq!(state(&ed), "ab-[c]>def\n");
    assert_eq!(ed.wake_timeout(), None);
    assert!(
        started.elapsed() >= std::time::Duration::from_millis(40),
        "the second timer cannot fire before both delays have passed"
    );
}
