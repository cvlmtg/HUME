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

fn editor_with(source: &str) -> (Editor, tempfile::TempDir) {
    let tmp = safe_tempdir();
    let mut ed = editor_from("-[a]>bc\n");
    run(&mut ed, tmp.path(), source);
    (ed, tmp)
}

#[test]
fn happy_path_streams_lines_and_accept_returns_the_raw_line() {
    // Spawns "sh" by unqualified name — see `Global::Env`'s doc.
    let _lock = TEST_GLOBALS.claim(Global::Env);
    let (mut ed, _tmp) = editor_with(
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
    ed.sync_viewport_dims(40, 12);
    ed.settle();
    ed.prepare_frame(&mut ctx);
    ed.feed_key(key_enter()); // top-ranked row (insertion order, empty query) is "a"
    ed.settle();

    assert_eq!(
        ed.state.status_msg.clone().unwrap(),
        "a",
        "on-select must receive the raw streamed line as payload (display == payload)"
    );
    assert!(ed.state.config.picker.is_none());
}

#[test]
fn nul_delimited_source_splits_on_nul() {
    // Spawns "sh" by unqualified name — see `Global::Env`'s doc.
    let _lock = TEST_GLOBALS.claim(Global::Env);
    let (mut ed, _tmp) = editor_with(
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
    // Spawns "sh" by unqualified name — see `Global::Env`'s doc.
    let _lock = TEST_GLOBALS.claim(Global::Env);
    let (mut ed, _tmp) = editor_with(
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
fn ok_exit_codes_silences_the_allowlisted_code_but_not_others() {
    // Spawns "sh" by unqualified name — see `Global::Env`'s doc.
    let _lock = TEST_GLOBALS.claim(Global::Env);
    let (mut ed, _tmp) = editor_with(
        r#"
        (define tok #f)
        (define-command! "go" "" (lambda ()
          (set! tok (picker! '() (lambda (x) (void))))))
        (define-command! "spawn-no-matches" "" (lambda ()
          (picker-source-spawn! tok "sh" (list "-c" "exit 1") #:ok-exit-codes '(0 1))))
        (define-command! "spawn-bad-regex" "" (lambda ()
          (picker-source-spawn! tok "sh" (list "-c" "echo boom >&2; exit 2") #:ok-exit-codes '(0 1))))
        "#,
    );
    type_cmd(&mut ed, ":go");
    call(&mut ed, "spawn-no-matches");
    assert!(
        ed.state
            .config
            .picker
            .as_ref()
            .is_some_and(|p| p.has_source()),
        "spawn must have attached a source — otherwise the drain_until below \
         would pass vacuously on the very first poll"
    );
    drain_until(&mut ed, source_detached);
    assert!(
        ed.state.status_msg.is_none(),
        "an allowlisted exit code (rg's 'no matches') must not report"
    );

    call(&mut ed, "spawn-bad-regex");
    drain_until(&mut ed, |ed| ed.state.status_msg.is_some());
    let msg = ed.state.status_msg.clone().unwrap();
    assert!(
        msg.contains("boom"),
        "a non-allowlisted exit code must still report, got: {msg}"
    );
}

#[test]
fn picker_source_stop_kills_the_child_and_no_further_rows_land() {
    // Spawns "sh" and "kill" by unqualified name — see `Global::Env`'s doc.
    let _lock = TEST_GLOBALS.claim(Global::Env);
    let (mut ed, _tmp) = editor_with(
        r#"
        (define tok #f)
        (define-command! "go" "" (lambda ()
          (set! tok (picker! '() (lambda (x) (void))))))
        (define-command! "spawn-it" "" (lambda ()
          (picker-source-spawn! tok "sh"
            (list "-c" "for i in 1 2 3 4 5 6 7 8 9 10; do echo $i; sleep 0.2; done"))))
        (define-command! "stop-it" "" (lambda ()
          (picker-source-stop! tok)))
        "#,
    );
    type_cmd(&mut ed, ":go");
    call(&mut ed, "spawn-it");

    drain_until(&mut ed, |ed| {
        ed.state
            .config
            .picker
            .as_ref()
            .is_some_and(|p| p.total_len() >= 1)
    });

    let pid = ed
        .state
        .config
        .picker
        .as_ref()
        .unwrap()
        .source_pid_for_test()
        .expect("source attached");

    call(&mut ed, "stop-it");
    let stopped_total = ed.state.config.picker.as_ref().unwrap().total_len();

    // Give the (now-dead) child's would-be remaining output a real window to
    // land, then confirm nothing did.
    std::thread::sleep(Duration::from_millis(500));
    ed.settle();

    assert!(
        !ed.state.config.picker.as_ref().unwrap().has_source(),
        "picker-source-stop! must detach the source"
    );
    assert_eq!(
        ed.state.config.picker.as_ref().unwrap().total_len(),
        stopped_total,
        "no further rows may land once the source is stopped"
    );

    assert!(
        !process_is_alive(pid),
        "picker-source-stop! must kill its source child"
    );
}

#[test]
fn respawn_reports_an_already_exited_outgoing_source() {
    // Spawns "sh" by unqualified name — see `Global::Env`'s doc.
    let _lock = TEST_GLOBALS.claim(Global::Env);
    let (mut ed, _tmp) = editor_with(
        r#"
        (define tok #f)
        (define-command! "go" "" (lambda ()
          (set! tok (picker! '() (lambda (x) (void))))))
        (define-command! "spawn-first" "" (lambda ()
          (picker-source-spawn! tok "sh" (list "-c" "echo boom >&2; exit 2"))))
        (define-command! "spawn-second" "" (lambda ()
          (picker-source-spawn! tok "sh" (list "-c" "exit 0"))))
        "#,
    );
    type_cmd(&mut ed, ":go");
    ed.state.status_msg = None;
    call(&mut ed, "spawn-first");

    // Poll the child's own OS exit status directly — never `ed.settle()`
    // here, which would drain and report it through the ordinary disconnect
    // path this test is deliberately racing ahead of with a respawn.
    let deadline = Instant::now() + Duration::from_secs(2);
    while !ed
        .state
        .config
        .picker
        .as_ref()
        .unwrap()
        .source_has_exited_for_test()
    {
        assert!(Instant::now() < deadline, "first child never exited");
        std::thread::sleep(Duration::from_millis(5));
    }

    call(&mut ed, "spawn-second");

    let msg = ed
        .state
        .status_msg
        .clone()
        .expect("a re-spawn must report the outgoing source's exit if it had already exited");
    assert!(msg.contains("boom"), "got: {msg}");
}

#[test]
fn respawn_does_not_report_a_still_running_outgoing_source() {
    // Spawns "sh"/"sleep" by unqualified name — see `Global::Env`'s doc.
    let _lock = TEST_GLOBALS.claim(Global::Env);
    let (mut ed, _tmp) = editor_with(
        r#"
        (define tok #f)
        (define-command! "go" "" (lambda ()
          (set! tok (picker! '() (lambda (x) (void))))))
        (define-command! "spawn-first" "" (lambda ()
          (picker-source-spawn! tok "sleep" (list "30"))))
        (define-command! "spawn-second" "" (lambda ()
          (picker-source-spawn! tok "sh" (list "-c" "exit 0"))))
        "#,
    );
    type_cmd(&mut ed, ":go");
    ed.state.status_msg = None;
    call(&mut ed, "spawn-first");

    call(&mut ed, "spawn-second"); // supersedes the still-running `sleep 30`

    assert!(
        ed.state.status_msg.is_none(),
        "a still-running source superseded mid-stream must not report a spurious exit"
    );
}

#[test]
fn picker_close_kills_the_source_child() {
    // Spawns "sleep" and "kill" by unqualified name — see `Global::Env`'s doc.
    let _lock = TEST_GLOBALS.claim(Global::Env);
    let (mut ed, _tmp) = editor_with(
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
    ed.settle();
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

    assert!(
        !process_is_alive(pid),
        "closing the picker must kill its source child"
    );

    // Keep interacting past the terminal action.
    ed.feed_key(key('i'));
    ed.feed_key(key('Z'));
    ed.feed_key(key_esc());
    ed.settle();
    let bid = ed.focused_buffer_id();
    let text = ed.state.buffers.get(bid).text().to_string();
    assert!(
        text.contains('Z'),
        "keys after close must behave as plain input"
    );
}

#[test]
fn live_picker_seed_spawns_keystroke_respawns_and_backspace_to_empty_clears() {
    // Spawns "sh" by unqualified name — see `Global::Env`'s doc.
    let _lock = TEST_GLOBALS.claim(Global::Env);
    let (mut ed, _tmp) = editor_with(
        r#"
        (define tok #f)
        (define-command! "go" "" (lambda ()
          (set! tok (live-picker! (lambda (x) (log! 'info (to-string x)))
            #:query "a"
            #:debounce-ms 0
            #:command (lambda (q)
              (and (not (equal? q ""))
                   (list "sh" "-c" (string-append "printf 'row-" q "\n'"))))))))
        "#,
    );
    type_cmd(&mut ed, ":go");

    drain_until_picker_total(&mut ed, 1);
    assert_eq!(
        ed.state
            .config
            .picker
            .as_ref()
            .unwrap()
            .window(10)
            .collect::<Vec<_>>(),
        vec!["row-a"],
        "the #:query seed must have spawned synchronously"
    );

    ed.feed_key(key('b'));
    // The stop half runs immediately, before the debounced respawn — same
    // ordering the portable
    // `live_picker_keystroke_keeps_previous_rows_until_the_new_search_delivers`
    // test pins without a real spawn. The previous pattern's row stays on
    // screen, not cleared, until the new search's own first batch swaps it
    // in below.
    ed.settle();
    assert_eq!(
        ed.state
            .config
            .picker
            .as_ref()
            .unwrap()
            .window(10)
            .collect::<Vec<_>>(),
        vec!["row-a"],
        "the previous pattern's row must survive the keystroke, not clear immediately"
    );
    assert!(
        ed.state.config.picker.as_ref().unwrap().is_pending(),
        "a live query change must mark the session pending even before the \
         debounced respawn fires"
    );

    drain_until(&mut ed, |ed| {
        ed.state
            .config
            .picker
            .as_ref()
            .is_some_and(|p| p.window(10).collect::<Vec<_>>() == vec!["row-ab"])
    });

    ed.feed_key(key_backspace());
    ed.feed_key(key_backspace());
    // Two settles, as the portable debounce-ms-0 tests document: the first
    // drains the two queued wrapped-callback calls (stopping the source,
    // arming then re-arming the 0ms timer for the latest, now-empty,
    // query); the second lets that surviving timer fire — calling
    // #:command with "" here, which returns #f, which is what actually
    // clears the rows (see `spawn-for`'s #f branch in bootstrap.scm).
    ed.settle();
    ed.settle();
    assert!(
        !ed.state.config.picker.as_ref().unwrap().has_source(),
        "backspacing to empty must leave no source attached"
    );
    assert_eq!(
        ed.state.config.picker.as_ref().unwrap().total_len(),
        0,
        "backspacing to empty must clear rows and spawn nothing new"
    );
}

#[test]
fn live_picker_requery_with_no_output_clears_the_previous_rows() {
    // Spawns "sh" by unqualified name — see `Global::Env`'s doc.
    let _lock = TEST_GLOBALS.claim(Global::Env);
    let (mut ed, _tmp) = editor_with(
        r#"
        (define-command! "go" "" (lambda ()
          (live-picker! (lambda (x) (log! 'info (to-string x)))
            #:query "a"
            #:debounce-ms 0
            #:ok-exit-codes '(0 1)
            #:command (lambda (q)
              (and (not (equal? q ""))
                   (if (equal? q "a")
                       (list "sh" "-c" "printf 'row-a\n'")
                       (list "sh" "-c" "exit 1")))))))
        "#,
    );
    type_cmd(&mut ed, ":go");

    drain_until_picker_total(&mut ed, 1);
    assert_eq!(
        ed.state
            .config
            .picker
            .as_ref()
            .unwrap()
            .window(10)
            .collect::<Vec<_>>(),
        vec!["row-a"],
        "the #:query seed must have spawned synchronously"
    );

    // "az" spawns `sh -c "exit 1"` — an allowlisted (`#:ok-exit-codes '(0 1)`)
    // but silent exit. It never delivers a batch to swap the old row out
    // (`PickerSession::push`'s `take_supersede` branch), so
    // `drain_picker_source`'s disconnect-with-nothing-delivered check is
    // what has to clear it instead — see `picker_source.rs`'s doc.
    ed.feed_key(key('z'));
    ed.settle();
    ed.settle();

    drain_until(&mut ed, |ed| {
        ed.state
            .config
            .picker
            .as_ref()
            .is_some_and(|p| p.total_len() == 0)
    });
    assert!(
        !ed.state.config.picker.as_ref().unwrap().has_source(),
        "a requery source that delivers nothing must leave no source attached once it exits"
    );
}
