//! Steel values the editor keeps between evaluations survive a full
//! collection, with the heap cells they capture intact.

use super::*;

/// `:churn` forces a full collection, then allocates past every slot it
/// freed, so a cell nothing roots is overwritten with `"junk"`. Forcing
/// compaction instead would take ten collections and a heap of millions of
/// slots.
const CHURN: &str = r#"
    (define (churn!)
      (#%gc-collect)
      (let loop ((i 0)) (when (< i 200000) (box "junk") (loop (+ i 1)))))
    (define-typed-command! "churn" "" (lambda () (churn!)))
"#;

fn log_text(ed: &Editor) -> String {
    ed.state.message_log.format_for_display()
}

#[test]
fn a_command_keeps_its_captured_state_across_a_collection() {
    let tmp = tempfile::tempdir().unwrap();
    let mut ed = editor_from("-[a]>\n");
    run(
        &mut ed,
        tmp.path(),
        &format!(
            r#"{CHURN}
            (define-typed-command! "count" ""
              (let ((n 0))
                (lambda ()
                  (set! n (+ n 1))
                  (log! 'warn (string-append "count:" (number->string n))))))"#
        ),
    );

    type_cmd(&mut ed, ":count");
    type_cmd(&mut ed, ":churn");
    type_cmd(&mut ed, ":count");

    let log = log_text(&ed);
    assert!(log.contains("count:2"), "{log}");
}

/// A `debounce`d call's `after!` thunk, held by the editor's timers, keeps
/// the arguments it captured.
#[test]
fn a_timer_thunk_keeps_its_captured_arguments_across_a_collection() {
    let tmp = tempfile::tempdir().unwrap();
    let mut ed = editor_from("-[a]>\n");
    run(
        &mut ed,
        tmp.path(),
        &format!(
            r#"{CHURN}
            (define tick (debounce 0 (lambda (msg) (log! 'warn (unbox msg)))))
            (define-typed-command! "arm" "" (lambda () (tick (box "alive"))))"#
        ),
    );

    type_cmd(&mut ed, ":arm");
    type_cmd(&mut ed, ":churn");
    ed.settle();

    let log = log_text(&ed);
    assert!(log.contains("[warning] alive"), "{log}");
}

/// A queued callback keeps what it captured while an earlier callback of
/// the same batch runs and collects.
#[test]
fn a_queued_callback_keeps_its_captured_state_while_the_batch_runs() {
    let tmp = tempfile::tempdir().unwrap();
    let mut ed = editor_from("-[a]>\n");
    run(
        &mut ed,
        tmp.path(),
        &format!(
            r#"{CHURN}
            (define (request-logging! pane msg)
              (lsp-request! pane "test/none" (hash)
                (lambda (err result) (log! 'warn (unbox msg)))))
            (define-typed-command! "requests" ""
              (lambda (pane)
                (lsp-request! pane "test/none" (hash) (lambda (err result) (churn!)))
                (request-logging! pane (box "alive"))))"#
        ),
    );

    type_cmd(&mut ed, ":requests");
    ed.settle();

    let log = log_text(&ed);
    assert!(log.contains("[warning] alive"), "{log}");
}

/// An open picker keeps its on-select callback and its items' payloads
/// through a collection that runs while it waits for a choice.
#[test]
fn an_open_picker_keeps_its_callback_and_payloads_across_a_collection() {
    let tmp = tempfile::tempdir().unwrap();
    let mut ed = editor_from("-[a]>\n");
    run(
        &mut ed,
        tmp.path(),
        &format!(
            r#"{CHURN}
            (define (open-picker! pane msg)
              (picker! pane (list (cons "one" (hash 'cell (box "payload-alive"))))
                (lambda (choice)
                  (log! 'warn (unbox msg))
                  (log! 'warn (unbox (hash-ref choice 'cell))))))
            (define-typed-command! "open" ""
              (lambda (pane)
                (after! 0 churn!)
                (open-picker! pane (box "alive"))))"#
        ),
    );

    type_cmd(&mut ed, ":open");
    ed.settle();
    assert!(ed.state.input.picker().is_some(), "{}", log_text(&ed));
    ed.feed_key(key_enter());
    ed.settle();

    let log = log_text(&ed);
    assert!(log.contains("[warning] alive"), "{log}");
    assert!(log.contains("[warning] payload-alive"), "{log}");
}

/// A test editor keeps one engine: a second source evaluates into it, so a
/// closure the first source left with the editor still runs against its
/// own heap.
#[test]
fn a_second_source_evaluates_into_the_editors_engine() {
    let tmp = tempfile::tempdir().unwrap();
    let mut ed = editor_from("-[a]>\n");
    run(
        &mut ed,
        tmp.path(),
        r#"(define-typed-command! "arm" ""
             (lambda ()
               (after! 0 (let ((cell (box "alive"))) (lambda () (log! 'warn (unbox cell)))))))"#,
    );
    type_cmd(&mut ed, ":arm");

    run(&mut ed, tmp.path(), "(define unrelated 1)");
    ed.settle();

    let log = log_text(&ed);
    assert!(log.contains("[warning] alive"), "{log}");
}
