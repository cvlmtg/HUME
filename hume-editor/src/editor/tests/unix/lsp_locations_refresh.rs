// The locations drawer (`lsp-references`) follows edits: it remembers the
// symbol it was opened for and, once the line count of a listed buffer
// changes, asks the server again for that symbol and swaps the rows. Loads the
// real shipped `core:lsp` plugin in place (`RealRuntimeGuard`).
//
// Not on Windows: Scheme require strings embed OS paths; backslashes are not
// escaped in Steel string literals (same constraint as tests/plugins.rs).

use std::path::Path;
use std::time::Duration;

use super::*;
use hume_lsp::test_util::{RecordingLspBackend, RequestLog};

/// `foo` sits at line 1, column 4, offset 16.
const FOO_OFFSET: usize = 16;

fn loc(uri: &str, line: u64, character: u64) -> serde_json::Value {
    serde_json::json!({
        "uri": uri,
        "range": {"start": {"line": line, "character": character}, "end": {"line": line, "character": character}}
    })
}

fn setup(
    tmp: &Path,
    references: Vec<serde_json::Value>,
) -> (Editor, RealRuntimeGuard, ServerId, RequestLog) {
    setup_with(tmp, |backend| {
        for answer in references {
            backend.respond_to("textDocument/references", answer);
        }
    })
}

/// [`setup`] with the references answers scripted by `script`, for a test
/// that needs an error among them. The file is
/// "fn main() {\n    foo();\n}\n".
fn setup_with(
    tmp: &Path,
    script: impl FnOnce(&mut RecordingLspBackend),
) -> (Editor, RealRuntimeGuard, ServerId, RequestLog) {
    let (rig, guard) = core_lsp_rig(
        tmp,
        "-[f]>n main() {\n    foo();\n}\n",
        serde_json::json!({"capabilities": {"referencesProvider": true}}),
        |backend, _sid| script(backend),
    );
    let sid = rig.sid("rust-analyzer");
    (rig.ed, guard, sid, rig.requests)
}

fn run_references(ed: &mut Editor) {
    ed.execute_keymap_command("lsp-references".into(), Some(1), false);
    ed.settle();
    ed.drain_lsp();
    ed.settle();
}

/// Lets the refresh's debounce elapse, then delivers the answer.
fn settle_after_refresh(ed: &mut Editor) {
    ed.settle();
    std::thread::sleep(Duration::from_millis(450));
    ed.settle();
    ed.settle();
    ed.drain_lsp();
    ed.settle();
}

fn reference_requests(requests: &RequestLog) -> usize {
    requests
        .borrow()
        .iter()
        .filter(|(_sid, m, _params)| m == "textDocument/references")
        .count()
}

fn last_reference_line(requests: &RequestLog) -> u64 {
    requests
        .borrow()
        .iter()
        .rev()
        .find(|(_sid, m, _params)| m == "textDocument/references")
        .map(|(_sid, _m, params)| params["position"]["line"].as_u64().unwrap())
        .expect("a references request was sent")
}

fn insert_line_above_the_cursor_line(ed: &mut Editor) {
    ed.handle_key(key('O'));
    ed.handle_key(key_esc());
}

fn drawer_is_open(ed: &Editor) -> bool {
    ed.state.views.drawer.read().is_some()
}

#[test]
fn inserting_a_line_above_the_symbol_asks_the_server_again_at_its_new_line() {
    let tmp = safe_tempdir();
    let uri = rust_rig_uri(tmp.path());
    let (mut ed, _guard, _sid, requests) = setup(
        tmp.path(),
        vec![
            serde_json::json!([loc(&uri, 1, 4), loc(&uri, 2, 0)]),
            serde_json::json!([loc(&uri, 2, 4), loc(&uri, 3, 0)]),
        ],
    );
    set_cursor(&mut ed, FOO_OFFSET);
    run_references(&mut ed);
    assert_eq!(drawer_rows(&ed).len(), 2, "setup: the drawer is open");
    assert_eq!(last_reference_line(&requests), 1, "setup: asked at `foo`");

    insert_line_above_the_cursor_line(&mut ed);
    settle_after_refresh(&mut ed);

    assert_eq!(reference_requests(&requests), 2);
    assert_eq!(
        last_reference_line(&requests),
        2,
        "the second request names where `foo` is now"
    );
    let rows = drawer_rows(&ed);
    assert!(rows[0].ends_with("main.rs:3:5"), "rows: {rows:?}");
}

#[test]
fn a_closed_drawer_is_not_refreshed_and_releases_the_position() {
    let tmp = safe_tempdir();
    let uri = rust_rig_uri(tmp.path());
    let (mut ed, _guard, _sid, requests) = setup(
        tmp.path(),
        vec![serde_json::json!([loc(&uri, 1, 4), loc(&uri, 2, 0)])],
    );
    set_cursor(&mut ed, FOO_OFFSET);
    run_references(&mut ed);
    ed.handle_key(key_esc());
    ed.settle();
    assert!(!drawer_is_open(&ed), "setup: Esc closed the drawer");
    assert_eq!(ed.state.panes.tracked.len(), 0, "the position is released");

    insert_line_above_the_cursor_line(&mut ed);
    settle_after_refresh(&mut ed);

    assert_eq!(reference_requests(&requests), 1);
}

#[test]
fn several_inserts_within_the_debounce_ask_once() {
    let tmp = safe_tempdir();
    let uri = rust_rig_uri(tmp.path());
    let (mut ed, _guard, _sid, requests) = setup(
        tmp.path(),
        vec![
            serde_json::json!([loc(&uri, 1, 4), loc(&uri, 2, 0)]),
            serde_json::json!([loc(&uri, 4, 4), loc(&uri, 5, 0)]),
        ],
    );
    set_cursor(&mut ed, FOO_OFFSET);
    run_references(&mut ed);

    for _ in 0..3 {
        insert_line_above_the_cursor_line(&mut ed);
        ed.settle();
    }
    settle_after_refresh(&mut ed);

    assert_eq!(reference_requests(&requests), 2);
    assert_eq!(last_reference_line(&requests), 4);
}

#[test]
fn an_edit_that_keeps_the_line_count_asks_nothing() {
    let tmp = safe_tempdir();
    let uri = rust_rig_uri(tmp.path());
    let (mut ed, _guard, _sid, requests) = setup(
        tmp.path(),
        vec![serde_json::json!([loc(&uri, 1, 4), loc(&uri, 2, 0)])],
    );
    set_cursor(&mut ed, FOO_OFFSET);
    run_references(&mut ed);

    ed.handle_key(key('i'));
    ed.handle_key(key('x'));
    ed.handle_key(key_esc());
    settle_after_refresh(&mut ed);

    assert_eq!(reference_requests(&requests), 1);
}

#[test]
fn a_refresh_that_finds_nothing_closes_the_drawer_with_a_message() {
    let tmp = safe_tempdir();
    let uri = rust_rig_uri(tmp.path());
    let (mut ed, _guard, _sid, requests) = setup(
        tmp.path(),
        vec![
            serde_json::json!([loc(&uri, 1, 4), loc(&uri, 2, 0)]),
            serde_json::json!([]),
        ],
    );
    set_cursor(&mut ed, FOO_OFFSET);
    run_references(&mut ed);

    insert_line_above_the_cursor_line(&mut ed);
    settle_after_refresh(&mut ed);

    assert_eq!(reference_requests(&requests), 2);
    assert!(!drawer_is_open(&ed), "nothing left to list");
    assert_eq!(ed.state.panes.tracked.len(), 0);
    assert_eq!(ed.state.status_msg.as_deref(), Some("No references found"));
}

#[test]
fn a_drawer_replaced_by_another_list_is_not_refreshed() {
    let tmp = safe_tempdir();
    let uri = rust_rig_uri(tmp.path());
    let (mut ed, _guard, sid, requests) = setup(
        tmp.path(),
        vec![serde_json::json!([loc(&uri, 1, 4), loc(&uri, 2, 0)])],
    );
    set_cursor(&mut ed, FOO_OFFSET);
    run_references(&mut ed);

    ed.ingest_typed_publish_for_test(
        sid,
        serde_json::from_value(serde_json::json!({"uri": uri, "diagnostics": [
            {"range": {"start": {"line": 1, "character": 4}, "end": {"line": 1, "character": 7}},
             "severity": 1, "message": "boom", "code": "E0"}
        ]}))
        .unwrap(),
    );
    type_cmd(&mut ed, ":diagnostics");
    ed.settle();
    assert_eq!(ed.state.panes.tracked.len(), 0, "the position is released");

    insert_line_above_the_cursor_line(&mut ed);
    settle_after_refresh(&mut ed);

    assert_eq!(reference_requests(&requests), 1);
}

fn selected_row(ed: &Editor) -> usize {
    ed.state
        .views
        .drawer
        .read()
        .as_ref()
        .expect("drawer must be open")
        .selected
}

/// A refresh the server answers with an error keeps the rows it had, and the
/// session keeps its position for the next refresh.
#[test]
fn a_refresh_answered_with_an_error_keeps_the_rows() {
    let tmp = safe_tempdir();
    let uri = rust_rig_uri(tmp.path());
    let (mut ed, _guard, _sid, requests) = setup_with(tmp.path(), |backend| {
        backend.respond_to(
            "textDocument/references",
            serde_json::json!([loc(&uri, 1, 4), loc(&uri, 2, 0)]),
        );
        backend.fail_with("textDocument/references", -32603, "internal error");
    });
    set_cursor(&mut ed, FOO_OFFSET);
    run_references(&mut ed);
    let before = drawer_rows(&ed);

    insert_line_above_the_cursor_line(&mut ed);
    settle_after_refresh(&mut ed);

    assert_eq!(reference_requests(&requests), 2, "setup: the refresh asked");
    assert_eq!(drawer_rows(&ed), before);
    assert_eq!(ed.state.panes.tracked.len(), 1);
}

/// The selected row stays selected across a refresh.
#[test]
fn a_refresh_keeps_the_selected_row() {
    let tmp = safe_tempdir();
    let uri = rust_rig_uri(tmp.path());
    let (mut ed, _guard, _sid, _requests) = setup(
        tmp.path(),
        vec![
            serde_json::json!([loc(&uri, 0, 0), loc(&uri, 1, 4), loc(&uri, 2, 0)]),
            serde_json::json!([loc(&uri, 1, 0), loc(&uri, 2, 4), loc(&uri, 3, 0)]),
        ],
    );
    set_cursor(&mut ed, FOO_OFFSET);
    run_references(&mut ed);
    ed.handle_key(key_shift_down());
    assert_eq!(selected_row(&ed), 1, "setup: the second row is selected");

    insert_line_above_the_cursor_line(&mut ed);
    settle_after_refresh(&mut ed);

    assert!(
        drawer_rows(&ed)[1].ends_with("main.rs:3:5"),
        "setup: swapped"
    );
    assert_eq!(selected_row(&ed), 1);
}

/// A refresh that lists fewer rows than the selected index clamps it to the
/// last row.
#[test]
fn a_refresh_with_fewer_rows_clamps_the_selected_row() {
    let tmp = safe_tempdir();
    let uri = rust_rig_uri(tmp.path());
    let (mut ed, _guard, _sid, _requests) = setup(
        tmp.path(),
        vec![
            serde_json::json!([loc(&uri, 0, 0), loc(&uri, 1, 4), loc(&uri, 2, 0)]),
            serde_json::json!([loc(&uri, 2, 4)]),
        ],
    );
    set_cursor(&mut ed, FOO_OFFSET);
    run_references(&mut ed);
    ed.handle_key(key_shift_down());
    ed.handle_key(key_shift_down());
    assert_eq!(selected_row(&ed), 2, "setup: the last row is selected");

    insert_line_above_the_cursor_line(&mut ed);
    settle_after_refresh(&mut ed);

    assert_eq!(drawer_rows(&ed).len(), 1, "setup: swapped");
    assert_eq!(selected_row(&ed), 0);
}

/// A change to the line count of another listed buffer, not the one the
/// list was asked from, refreshes it too.
#[test]
fn a_line_count_change_in_another_listed_buffer_refreshes_the_list() {
    let tmp = safe_tempdir();
    let file = rig_root(tmp.path()).join("src/main.rs");
    let uri = rust_rig_uri(tmp.path());
    let other = rig_root(tmp.path()).join("other.rs");
    std::fs::write(&other, "fn bar() {\n    foo();\n}\n").unwrap();
    let other_uri = hume_lsp::uri::path_to_uri(&std::fs::canonicalize(&other).unwrap())
        .unwrap()
        .as_str()
        .to_string();
    let answer = serde_json::json!([loc(&uri, 1, 4), loc(&other_uri, 1, 4)]);
    let (mut ed, _guard, _sid, requests) = setup(tmp.path(), vec![answer.clone(), answer]);
    ed.execute_typed("e", Some(other.to_str().unwrap()))
        .unwrap();
    ed.execute_typed("e", Some(file.to_str().unwrap())).unwrap();
    set_cursor(&mut ed, FOO_OFFSET);
    run_references(&mut ed);
    assert_eq!(drawer_rows(&ed).len(), 2, "setup: the drawer is open");

    ed.execute_typed("e", Some(other.to_str().unwrap()))
        .unwrap();
    insert_line_above_the_cursor_line(&mut ed);
    settle_after_refresh(&mut ed);

    assert_eq!(reference_requests(&requests), 2);
}
