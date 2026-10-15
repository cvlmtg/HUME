//! `on-before-buffer-save`: handlers run before the write, and a handler
//! taking `done` holds the write until it calls it.

use super::*;
use pretty_assertions::assert_eq;

/// Editor on `ab` whose file on disk holds `zz`, so a write that has not
/// happened yet is distinguishable from one that wrote the buffer as it was.
fn dirty_editor() -> (Editor, tempfile::TempPath, tempfile::TempDir) {
    let (ed, file) = editor_with_file("-[a]>b\n", "zz\n");
    (ed, file, tempfile::tempdir().unwrap())
}

fn on_disk(file: &tempfile::TempPath) -> String {
    std::fs::read_to_string(file).unwrap()
}

fn errors(ed: &Editor) -> usize {
    ed.state
        .message_log
        .entries()
        .filter(|e| e.severity == Severity::Error)
        .count()
}

const ASYNC_HANDLER: &str = r#"
(define held-done #f)
(register-hook! 'on-before-buffer-save (lambda (pane done) (set! held-done done)))
(define-typed-command! "finish" ""
  (lambda ()
    (call! "delete" (focused-pane))
    (held-done)
    (held-done)))
"#;

#[test]
fn sync_handler_edit_lands_in_the_written_file() {
    let (mut ed, file, dir) = dirty_editor();
    run(
        &mut ed,
        dir.path(),
        r#"(register-hook! 'on-before-buffer-save (lambda (pane) (call! "delete" (focused-pane))))"#,
    );

    type_cmd(&mut ed, ":w");
    ed.settle();

    assert_eq!(on_disk(&file), "b\n");
    let bid = ed.focused_buffer_id();
    assert!(!ed.state.has_unsaved_changes(bid));
}

#[test]
fn async_handler_holds_the_write_until_done() {
    let (mut ed, file, dir) = dirty_editor();
    run(&mut ed, dir.path(), ASYNC_HANDLER);

    type_cmd(&mut ed, ":w");
    ed.settle();
    assert_eq!(on_disk(&file), "zz\n", "write must wait for done");

    type_cmd(&mut ed, ":finish");
    ed.settle();
    assert_eq!(on_disk(&file), "b\n");
    assert_eq!(errors(&ed), 0, "a second done call must do nothing");
}

#[test]
fn timeout_counts_from_when_the_handlers_are_raised() {
    let (mut ed, file, dir) = dirty_editor();
    run(
        &mut ed,
        dir.path(),
        r#"(register-hook! 'on-before-buffer-save (lambda (pane) (call! "delete" (focused-pane))))"#,
    );
    type_cmd(&mut ed, ":set global before-save-timeout-ms=1");

    type_cmd(&mut ed, ":w");
    std::thread::sleep(std::time::Duration::from_millis(30));
    ed.settle();

    assert_eq!(on_disk(&file), "b\n");
    assert_eq!(warnings(&ed), 0);
}

#[test]
fn write_proceeds_when_a_handler_never_calls_done() {
    let (mut ed, file, dir) = dirty_editor();
    run(
        &mut ed,
        dir.path(),
        r#"(register-hook! 'on-before-buffer-save (lambda (pane done) (begin)))"#,
    );
    type_cmd(&mut ed, ":set global before-save-timeout-ms=1");

    type_cmd(&mut ed, ":w");
    ed.settle();
    assert_eq!(on_disk(&file), "zz\n");

    std::thread::sleep(std::time::Duration::from_millis(30));
    ed.settle();

    assert_eq!(on_disk(&file), "ab\n");
    assert!(
        ed.state
            .message_log
            .entries()
            .any(|e| e.severity == Severity::Warning),
        "the timeout must be reported"
    );
}

#[test]
fn a_failing_handler_does_not_block_the_write() {
    let (mut ed, file, dir) = dirty_editor();
    run(
        &mut ed,
        dir.path(),
        r#"(register-hook! 'on-before-buffer-save (lambda (pane done) (error "boom")))"#,
    );

    type_cmd(&mut ed, ":w");
    ed.settle();

    assert_eq!(on_disk(&file), "ab\n");
    assert_eq!(errors(&ed), 1, "the handler's error must be reported");
}

#[test]
fn write_quit_quits_only_after_the_deferred_write() {
    let (mut ed, file, dir) = dirty_editor();
    run(&mut ed, dir.path(), ASYNC_HANDLER);

    type_cmd(&mut ed, ":wq");
    ed.settle();
    assert!(!ed.state.should_quit, "must not quit before the write");
    assert_eq!(on_disk(&file), "zz\n");

    type_cmd(&mut ed, ":finish");
    ed.settle();
    assert_eq!(on_disk(&file), "b\n");
    assert!(ed.state.should_quit);
}

#[test]
fn second_write_while_one_is_pending_is_refused() {
    let (mut ed, file, dir) = dirty_editor();
    run(&mut ed, dir.path(), ASYNC_HANDLER);

    type_cmd(&mut ed, ":w");
    ed.settle();
    type_cmd(&mut ed, ":w");
    ed.settle();

    assert_eq!(on_disk(&file), "zz\n");
    assert!(
        ed.state
            .status_msg
            .as_deref()
            .is_some_and(|m| m.contains("in progress")),
        "got {:?}",
        ed.state.status_msg
    );
}

#[test]
fn done_after_the_write_committed_does_nothing() {
    let (mut ed, file, dir) = dirty_editor();
    run(
        &mut ed,
        dir.path(),
        r#"(define held-done #f)
           (register-hook! 'on-before-buffer-save (lambda (pane done) (set! held-done done) (done)))
           (define-typed-command! "late" "" (lambda () (held-done)))"#,
    );

    type_cmd(&mut ed, ":w");
    ed.settle();
    assert_eq!(on_disk(&file), "ab\n");

    type_cmd(&mut ed, ":late");
    ed.settle();
    assert_eq!(errors(&ed), 0);
}

#[test]
fn write_all_raises_the_event_for_each_dirty_buffer_only() {
    let (mut ed, file_a, dir) = dirty_editor();
    run(
        &mut ed,
        dir.path(),
        r#"(register-hook! 'on-before-buffer-save (lambda (pane) (log! 'trace "before")))"#,
    );
    let (file_b, _guard_b) = temp_file("yy\n");
    type_cmd(&mut ed, &format!(":e {}", file_b.display()));
    ed.handle_key(key('i'));
    ed.handle_key(key('x'));
    ed.handle_key(key_esc());

    type_cmd(&mut ed, ":wa");
    ed.settle();

    assert_eq!(on_disk(&file_a), "zz\n");
    assert_eq!(std::fs::read_to_string(&file_b).unwrap(), "xyy\n");
    let raised = ed
        .state
        .message_log
        .entries()
        .filter(|e| e.severity == Severity::Trace && e.text == "before")
        .count();
    assert_eq!(raised, 1);
}

#[test]
fn export_of_a_read_only_buffer_does_not_hold_for_handlers() {
    let (mut ed, _file, dir) = dirty_editor();
    run(&mut ed, dir.path(), ASYNC_HANDLER);
    let bid = ed.focused_buffer_id();
    ed.state.buffers.get_mut(bid).read_only = true;
    let (target, _guard) = temp_file("old\n");

    type_cmd(&mut ed, &format!(":w! {}", target.display()));

    assert_eq!(std::fs::read_to_string(&target).unwrap(), "ab\n");
}

#[test]
fn pathless_buffer_is_refused_without_holding() {
    let mut ed = editor_from("-[a]>b\n");
    let dir = tempfile::tempdir().unwrap();
    run(&mut ed, dir.path(), ASYNC_HANDLER);

    type_cmd(&mut ed, ":w");

    assert!(
        ed.state
            .status_msg
            .as_deref()
            .is_some_and(|m| m.contains("no file name")),
        "got {:?}",
        ed.state.status_msg
    );
}

fn traces(ed: &Editor) -> Vec<String> {
    ed.state
        .message_log
        .entries()
        .filter(|e| e.severity == Severity::Trace)
        .map(|e| e.text.to_string())
        .collect()
}

fn warnings(ed: &Editor) -> usize {
    ed.state
        .message_log
        .entries()
        .filter(|e| e.severity == Severity::Warning)
        .count()
}

const RELEASE_HANDLER: &str = r#"
(define held-done #f)
(register-hook! 'on-before-buffer-save (lambda (pane done) (set! held-done done)))
(define-typed-command! "release" "" (lambda () (held-done)))
"#;

#[test]
fn closing_the_buffer_drops_its_parked_write_without_a_warning() {
    let (mut ed, file, dir) = dirty_editor();
    run(&mut ed, dir.path(), ASYNC_HANDLER);
    type_cmd(&mut ed, ":set global before-save-timeout-ms=1");

    type_cmd(&mut ed, ":w");
    ed.settle();
    type_cmd(&mut ed, ":bd!");
    ed.settle();
    std::thread::sleep(std::time::Duration::from_millis(30));
    ed.settle();

    assert_eq!(on_disk(&file), "zz\n");
    assert_eq!(warnings(&ed), 0);
}

#[test]
fn reload_releases_a_parked_write() {
    let (mut ed, file, dir) = dirty_editor();
    run(&mut ed, dir.path(), ASYNC_HANDLER);

    type_cmd(&mut ed, ":w");
    ed.settle();
    assert_eq!(on_disk(&file), "zz\n");

    ed.reset_config_state();
    ed.settle();

    assert_eq!(on_disk(&file), "ab\n");
    assert_eq!(warnings(&ed), 0);
}

#[test]
fn event_dropped_by_the_drain_cap_still_writes_without_a_warning() {
    let (mut ed, file, dir) = dirty_editor();
    run(&mut ed, dir.path(), ASYNC_HANDLER);

    type_cmd(&mut ed, ":w");
    for _ in 0..1001 {
        ed.state
            .queue_event(crate::editor::event::EditorEvent::OnFocusGained);
    }
    ed.settle();

    assert_eq!(on_disk(&file), "ab\n");
    assert_eq!(warnings(&ed), 0);
}

#[test]
fn write_quit_released_after_focus_moved_does_not_quit_the_other_buffer() {
    let (mut ed, file, dir) = dirty_editor();
    run(&mut ed, dir.path(), RELEASE_HANDLER);
    let (other, _guard) = temp_file("yy\n");

    type_cmd(&mut ed, ":wq");
    ed.settle();
    type_cmd(&mut ed, &format!(":e {}", other.display()));
    ed.settle();
    type_cmd(&mut ed, ":release");
    ed.settle();

    assert_eq!(on_disk(&file), "ab\n");
    assert!(!ed.state.should_quit, "the other buffer must stay open");
    assert!(
        ed.state
            .status_msg
            .as_deref()
            .is_some_and(|m| m.contains("quit cancelled")),
        "got {:?}",
        ed.state.status_msg
    );
}

#[test]
fn event_raised_for_a_pane_that_moved_on_still_reaches_the_handlers() {
    let (mut ed, file, dir) = dirty_editor();
    run(
        &mut ed,
        dir.path(),
        r#"(define held-done #f)
           (register-hook! 'on-before-buffer-save
             (lambda (pane done) (log! 'trace "ran") (set! held-done done)))
           (define-typed-command! "release" "" (lambda () (held-done)))"#,
    );
    let (other, _guard) = temp_file("yy\n");

    type_cmd(&mut ed, ":w");
    type_cmd(&mut ed, &format!(":e {}", other.display()));
    ed.settle();
    type_cmd(&mut ed, ":release");
    ed.settle();

    assert_eq!(traces(&ed), ["ran"]);
    assert_eq!(on_disk(&file), "ab\n");
}

#[test]
fn write_all_skips_a_buffer_whose_write_is_waiting_and_writes_the_rest() {
    let (mut ed, _file_a, dir) = dirty_editor();
    run(
        &mut ed,
        dir.path(),
        r#"(register-hook! 'on-before-buffer-save
             (lambda (pane done) (log! 'trace "raised")))"#,
    );
    ed.handle_key(key('i'));
    ed.handle_key(key('x'));
    ed.handle_key(key_esc());
    type_cmd(&mut ed, ":w");
    ed.settle();
    let (file_b, _guard_b) = temp_file("yy\n");
    type_cmd(&mut ed, &format!(":e {}", file_b.display()));
    ed.handle_key(key('i'));
    ed.handle_key(key('x'));
    ed.handle_key(key_esc());

    type_cmd(&mut ed, ":wa");
    ed.settle();

    assert_eq!(traces(&ed), ["raised", "raised"], "B must still be raised");
    assert!(
        ed.state
            .message_log
            .entries()
            .any(|e| e.severity == Severity::Warning
                && e.text.starts_with("Skipped (save in progress)")),
        "the waiting buffer must be listed as skipped"
    );
}
