// `:ls` lists open buffers in the bottom drawer. `Enter` on a row switches
// the focused pane to that buffer and leaves the drawer open.

use pretty_assertions::assert_eq;

use super::*;
use crate::editor::input_stack::DrawerRows;

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

/// The buffers the drawer's rows name, in row order.
fn listed(ed: &Editor) -> Vec<BufferId> {
    match &ed.state.input.drawer().expect("drawer must be open").rows {
        DrawerRows::Buffers { buffers, .. } => buffers.clone(),
        DrawerRows::Script(_) => panic!("expected a buffer-list drawer"),
    }
}

fn store_order(ed: &Editor) -> Vec<BufferId> {
    ed.state.buffers.iter().map(|(id, _)| id).collect()
}

fn frame(ed: &mut Editor) {
    let mut ctx = hume_engine::pipeline::RenderContext::new();
    ed.settle();
    ed.prepare_frame(&mut ctx);
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
    assert_eq!(drawer_rows(&ed).len(), count);
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
    ed.report(Severity::Warning, "note");
    ed.execute_typed("messages", None).unwrap();
    ed.execute_typed("ls", None).unwrap();

    insta::assert_snapshot!(drawer_rows(&ed).join("\n"), @"
       1  #+  *scratch*
       2  %   [messages]
    ");
}

/// The current buffer's row is `markup.bold`. An unsaved buffer's row is
/// `diagnostic.warning.message-text`, and while it is the selected row the
/// whole row, its text included, is `diagnostic.warning.message`. The unsaved
/// scopes win on the current buffer.
#[test]
fn rows_scope_the_current_buffer_bold_and_unsaved_buffers_as_warning() {
    const UNSAVED_TEXT: &str = "diagnostic.warning.message-text";
    const UNSAVED: &str = "diagnostic.warning.message";
    let (p1, _t1) = temp_file("file1\n");
    let (p2, _t2) = temp_file("file2\n");
    let mut ed = editor_from("-[h]>ello\n");
    ed.handle_key(key('i'));
    ed.handle_key(key('x'));
    ed.handle_key(key_esc());
    ed.execute_typed("e", Some(p1.to_str().unwrap())).unwrap();
    ed.execute_typed("e", Some(p2.to_str().unwrap())).unwrap();
    ed.execute_typed("ls", None).unwrap();
    let len = |ed: &Editor, i: usize| drawer_rows(ed)[i].chars().count();

    let (scratch, current) = (len(&ed, 0), len(&ed, 2));
    assert_drawer_styles(
        &mut ed,
        &[
            (Some(UNSAVED), &[(0, scratch, UNSAVED_TEXT, Some(UNSAVED))]),
            (None, &[]),
            (None, &[(0, current, "markup.bold", None)]),
        ],
    );

    ed.handle_key(key('d'));
    frame(&mut ed);

    let (scratch, current) = (len(&ed, 0), len(&ed, 2));
    assert_drawer_styles(
        &mut ed,
        &[
            (Some(UNSAVED), &[(0, scratch, UNSAVED_TEXT, Some(UNSAVED))]),
            (None, &[]),
            (Some(UNSAVED), &[(0, current, UNSAVED_TEXT, Some(UNSAVED))]),
        ],
    );
}

/// A row wider than the drawer is cut at its tail, so its number and
/// markers stay.
#[test]
fn a_row_wider_than_the_drawer_is_cut_at_its_tail() {
    let mut ed = editor_from("-[h]>ello\n");
    ed.execute_typed("ls", None).unwrap();
    ed.resize_terminal(12, 24);
    frame(&mut ed);

    let row = {
        let guard = ed.state.views.drawer.read();
        let rows = &guard.as_ref().expect("drawer must be open").rows;
        rows[0]
            .as_ref()
            .expect("the row is shown")
            .text()
            .to_owned()
    };
    insta::assert_snapshot!(format!("{row:?}"), @r#""   1  %  …""#);
}

#[test]
fn a_buffers_label_shows_in_the_name_column() {
    let mut ed = editor_from("-[h]>ello\n");
    let fp = FocusedPane::current(&ed.view);
    ed.open_read_only_view(fp, &view_label("custom"), "text\n", None);
    ed.execute_typed("ls", None).unwrap();
    insta::assert_snapshot!(drawer_rows(&ed).join("\n"), @"
       1  #   *scratch*
       2  %   [custom]
    ");
}

#[test]
fn opening_pushes_no_jump_entry() {
    let mut ed = editor_from("-[h]>ello\n");
    let bid = ed.focused_buffer_id();
    let pid = ed.view.focused_pane();
    ed.execute_typed("ls", None).unwrap();
    assert!(!ed.state.panes.entries[pid].jumps.entries_for_buffer(bid));
}

#[test]
fn enter_switches_buffer_with_a_jump_and_keeps_the_drawer_open() {
    let (mut ed, _tmp) = three_buffers();
    let departed = ed.focused_buffer_id();
    let pid = ed.view.focused_pane();
    ed.handle_key(key_up());
    ed.handle_key(key_up());
    assert_eq!(selected(&ed), 0);

    ed.handle_key(key_enter());

    assert_eq!(ed.focused_buffer_id(), listed(&ed)[0]);
    assert!(ed.doc().path().is_none(), "row 0 is the scratch buffer");
    assert!(drawer_open(&ed));
    assert!(
        ed.state.panes.entries[pid]
            .jumps
            .entries_for_buffer(departed)
    );
    assert_eq!(ed.state.buffers.second_most_recent(), Some(departed));
}

#[test]
fn enter_moves_the_markers_with_the_switch() {
    let mut ed = editor_from("-[h]>ello\n");
    ed.report(Severity::Warning, "note");
    ed.execute_typed("messages", None).unwrap();
    ed.execute_typed("ls", None).unwrap();
    ed.handle_key(key_up());

    ed.handle_key(key_enter());

    insta::assert_snapshot!(drawer_rows(&ed).join("\n"), @"
       1  %   *scratch*
       2  #   [messages]
    ");
}

#[test]
fn row_numbers_name_the_buffer_b_resolves() {
    let (mut ed, _tmp) = three_buffers();
    ed.handle_key(key_esc());
    for (i, id) in store_order(&ed).into_iter().enumerate() {
        ed.execute_typed("b", Some(&(i + 1).to_string())).unwrap();
        assert_eq!(ed.focused_buffer_id(), id);
    }
}

#[test]
fn rows_follow_the_buffer_store_after_a_close() {
    let (mut ed, _tmp) = three_buffers();
    ed.execute_typed("b", Some("2")).unwrap();
    ed.execute_typed("bd", None).unwrap();

    frame(&mut ed);

    assert_eq!(listed(&ed), store_order(&ed));
    assert_eq!(drawer_rows(&ed).len(), 2);
    assert_eq!(listed(&ed)[selected(&ed)], ed.focused_buffer_id());
}

#[test]
fn rows_gain_a_buffer_opened_while_the_drawer_is_open() {
    let (mut ed, _tmp) = three_buffers();
    let (path, _t) = temp_file("file3\n");
    ed.execute_typed("e", Some(path.to_str().unwrap())).unwrap();

    frame(&mut ed);

    assert_eq!(listed(&ed), store_order(&ed));
    assert_eq!(listed(&ed).last(), Some(&ed.focused_buffer_id()));
}

#[test]
fn enter_on_a_closed_buffer_reports_and_drops_its_row() {
    let (mut ed, _tmp) = three_buffers();
    ed.execute_typed("bd", None).unwrap();
    let focused = ed.focused_buffer_id();
    assert!(drawer_open(&ed));

    ed.handle_key(key_enter());

    assert_eq!(ed.focused_buffer_id(), focused);
    assert!(
        ed.state
            .status_msg
            .text()
            .is_some_and(|m| m.contains("closed")),
        "{:?}",
        ed.state.status_msg
    );
    assert_eq!(listed(&ed), store_order(&ed));
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
  (show-drawer-list! pane (map (lambda (row) (cons row row)) '("a" "b")) on-select)))
(define-typed-command! "selections" "" (lambda (pane) (log! 'info selections)))"##,
    );
    type_cmd(&mut ed, ":go");

    type_cmd(&mut ed, ":ls");
    ed.settle();
    type_cmd(&mut ed, ":selections");

    assert_eq!(ed.state.status_msg.text(), Some("#f;"));
}
