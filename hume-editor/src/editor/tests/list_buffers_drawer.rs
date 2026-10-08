// `:ls` lists open buffers in the bottom drawer. `Enter` on a row switches
// the focused pane to that buffer and leaves the drawer open.

use pretty_assertions::assert_eq;
use termina::event::{KeyCode, KeyEvent, Modifiers};

use super::*;

fn key_shift_up() -> KeyEvent {
    KeyEvent::new(KeyCode::Up, Modifiers::SHIFT)
}

/// The drawer's rows, which must all be rendered (a native drawer has no
/// lazy rows).
fn rows(ed: &Editor) -> Vec<String> {
    let guard = ed.state.views.drawer.read();
    guard
        .as_ref()
        .expect("drawer must be open")
        .rows
        .iter()
        .map(|r| r.clone().expect("native rows are rendered up front"))
        .collect()
}

fn selected(ed: &Editor) -> usize {
    ed.state
        .input
        .drawer()
        .expect("drawer must be open")
        .selected
}

fn drawer_open(ed: &Editor) -> bool {
    ed.state.input.drawer().is_some()
}

/// Scratch buffer plus two files, the second one focused, then `:ls` run.
fn three_buffers() -> (Editor, [tempfile::TempPath; 2]) {
    let (p1, t1) = temp_file("file1\n");
    let (p2, t2) = temp_file("file2\n");
    let mut ed = editor_from("-[h]>ello\n");
    ed.execute_typed("e", Some(p1.to_str().unwrap())).unwrap();
    ed.execute_typed("e", Some(p2.to_str().unwrap())).unwrap();
    ed.execute_typed("ls", None).unwrap();
    (ed, [t1, t2])
}

#[test]
fn ls_opens_a_drawer_row_per_buffer_and_no_buffer() {
    let (p1, _t1) = temp_file("file1\n");
    let (p2, _t2) = temp_file("file2\n");
    let mut ed = editor_from("-[h]>ello\n");
    ed.execute_typed("e", Some(p1.to_str().unwrap())).unwrap();
    ed.execute_typed("e", Some(p2.to_str().unwrap())).unwrap();
    let focused = ed.focused_buffer_id();
    let count = ed.state.buffers.iter().count();

    ed.execute_typed("ls", None).unwrap();

    assert!(drawer_open(&ed));
    assert_eq!(rows(&ed).len(), count);
    assert_eq!(selected(&ed), count - 1, "the focused buffer is selected");
    assert_eq!(ed.state.buffers.iter().count(), count);
    assert_eq!(ed.focused_buffer_id(), focused);
    assert!(!ed.doc().is_read_only());
}

#[test]
fn list_buffers_alias_opens_the_drawer() {
    let mut ed = editor_from("-[h]>ello\n");
    ed.execute_typed("list-buffers", None).unwrap();
    assert!(drawer_open(&ed));
}

#[test]
fn rows_mark_current_alternate_and_dirty() {
    let mut ed = editor_from("-[h]>ello\n");
    ed.handle_key(key('i'));
    ed.handle_key(key('x'));
    ed.handle_key(key_esc());
    ed.report(Severity::Warning, "note".to_string());
    ed.execute_typed("messages", None).unwrap();
    ed.execute_typed("ls", None).unwrap();

    insta::assert_snapshot!(rows(&ed).join("\n"), @"
       1  #+  *scratch*
       2  %   [messages]
    ");
}

#[test]
fn a_buffers_label_shows_instead_of_its_path() {
    let (mut ed, _tmp) = editor_with_file("-[h]>ello\n", "hello\n");
    ed.doc_mut().label = Some("[custom]".to_string());
    ed.execute_typed("ls", None).unwrap();
    assert!(rows(&ed)[0].contains("[custom]"), "{:?}", rows(&ed));
}

#[test]
fn opening_pushes_no_jump_entry() {
    let mut ed = editor_from("-[h]>ello\n");
    let bid = ed.focused_buffer_id();
    let pid = ed.state.focus.id();
    ed.execute_typed("ls", None).unwrap();
    assert!(!ed.state.panes.jumps[pid].entries_for_buffer(bid));
}

#[test]
fn enter_switches_buffer_with_a_jump_and_keeps_the_drawer_open() {
    let (mut ed, _tmp) = three_buffers();
    let departed = ed.focused_buffer_id();
    let pid = ed.state.focus.id();
    ed.handle_key(key_shift_up());
    ed.handle_key(key_shift_up());
    assert_eq!(selected(&ed), 0);

    ed.handle_key(key_enter());

    assert_ne!(ed.focused_buffer_id(), departed);
    assert!(ed.doc().path().is_none(), "row 0 is the scratch buffer");
    assert!(drawer_open(&ed));
    assert!(ed.state.panes.jumps[pid].entries_for_buffer(departed));
    let rows = rows(&ed);
    assert!(
        rows[0].contains('%'),
        "the marker follows the switch: {rows:?}"
    );
    assert!(
        rows[2].contains('#'),
        "the departed buffer is the alternate: {rows:?}"
    );
}

#[test]
fn row_numbers_name_the_buffer_b_resolves() {
    let (mut ed, _tmp) = three_buffers();
    ed.handle_key(key_esc());
    ed.execute_typed("b", Some("1")).unwrap();
    ed.execute_typed("ls", None).unwrap();

    for (i, row) in rows(&ed).iter().enumerate() {
        let n: usize = row.split_whitespace().next().unwrap().parse().unwrap();
        assert_eq!(n, i + 1);
        ed.execute_typed("b", Some(&n.to_string())).unwrap();
        let name = ed.doc().display_name().to_string();
        assert!(row.contains(&name), "row {n} {row:?} vs buffer {name:?}");
    }
}

#[test]
fn enter_on_a_closed_buffer_reports_and_stays_put() {
    let (mut ed, _tmp) = three_buffers();
    ed.execute_typed("bd", None).unwrap();
    let focused = ed.focused_buffer_id();
    assert!(drawer_open(&ed));

    ed.handle_key(key_enter());

    assert_eq!(ed.focused_buffer_id(), focused);
    assert!(
        ed.state
            .status_msg
            .as_deref()
            .is_some_and(|m| m.contains("closed")),
        "{:?}",
        ed.state.status_msg
    );
}

#[test]
fn replacing_a_steel_drawer_tells_its_owner() {
    let tmp = tempfile::tempdir().unwrap();
    let mut ed = editor_from("-[x]>abc\n");
    run(
        &mut ed,
        tmp.path(),
        r##"(define selections "")
(define (on-select i token)
  (set! selections (string-append selections (if i (number->string i) "#f") ";")))
(define-typed-command! "go" "" (lambda (pane)
  (show-drawer-list! pane '("a" "b") on-select)))
(define-typed-command! "selections" "" (lambda (pane) (log! 'info selections)))"##,
    );
    type_cmd(&mut ed, ":go");

    type_cmd(&mut ed, ":ls");
    ed.settle();
    type_cmd(&mut ed, ":selections");

    assert_eq!(ed.state.status_msg.as_deref(), Some("#f;"));
}
