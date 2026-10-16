// Document and workspace symbol pickers: `lsp/document-symbols` and
// `lsp/workspace-symbols`. Loads the real shipped `core:lsp` plugin in place
// (`RealRuntimeDirs`).
//
// Not on Windows: Scheme require strings embed OS paths; backslashes are not
// escaped in Steel string literals (same constraint as tests/plugins.rs).

use std::path::Path;

use super::*;
use hume_lsp::backend::ServerId;
use hume_lsp::test_util::{RecordingLspBackend, RequestLog};

/// Char offsets of the positions the tests jump to: (line 1, col 4) and
/// (line 2, col 0).
const FOO_CALL: usize = 12 + 4;
const CLOSING_BRACE: usize = 12 + 11;

fn setup(
    tmp: &Path,
    capabilities: serde_json::Value,
    configure: impl FnOnce(&mut RecordingLspBackend, ServerId),
) -> (Editor, RealRuntimeDirs, RequestLog) {
    let (rig, guard) = core_lsp_rig(
        tmp,
        "-[f]>n main() {\n    foo();\n}\n",
        serde_json::json!({ "capabilities": capabilities }),
        configure,
    );
    (rig.ed, guard, rig.requests)
}

fn pump(ed: &mut Editor) {
    ed.settle();
    ed.drain_lsp();
    ed.settle();
}

fn run(ed: &mut Editor, command: &'static str) {
    ed.execute_keymap_command(command.into(), Some(1), false);
    pump(ed);
}

fn range(line: u64, start: u64, end: u64) -> serde_json::Value {
    serde_json::json!({
        "start": {"line": line, "character": start},
        "end": {"line": line, "character": end}
    })
}

fn loc(uri: &str, line: u64, character: u64) -> serde_json::Value {
    serde_json::json!({"uri": uri, "range": range(line, character, character)})
}

fn picker_rows(ed: &Editor) -> Vec<String> {
    ed.state
        .input
        .picker()
        .expect("a picker must be open")
        .window(20)
        .map(str::to_string)
        .collect()
}

fn head(ed: &Editor) -> usize {
    ed.current_view().primary().head().offset().index()
}

fn nested_reply() -> serde_json::Value {
    serde_json::json!([
        {
            "name": "main", "kind": 12,
            "range": range(0, 0, 11), "selectionRange": range(0, 3, 7),
            "children": [
                {
                    "name": "foo", "kind": 12,
                    "range": range(1, 4, 9), "selectionRange": range(1, 4, 7)
                }
            ]
        },
        {
            "name": "Bar", "kind": 23,
            "range": range(2, 0, 1), "selectionRange": range(2, 0, 1)
        }
    ])
}

#[test]
fn nested_document_symbols_list_flattened_with_their_container() {
    let tmp = tempfile::tempdir().unwrap();
    let (mut ed, _guard, _requests) = setup(
        tmp.path(),
        serde_json::json!({"documentSymbolProvider": true}),
        |b, _| {
            b.respond_to("textDocument/documentSymbol", nested_reply());
        },
    );

    run(&mut ed, "lsp/document-symbols");

    insta::assert_snapshot!(picker_rows(&ed).join("\n"), @r"
    main  function
    foo  function  main
    Bar  struct
    ");
}

#[test]
fn accepting_a_nested_symbol_jumps_to_its_selection_range() {
    let tmp = tempfile::tempdir().unwrap();
    let (mut ed, _guard, _requests) = setup(
        tmp.path(),
        serde_json::json!({"documentSymbolProvider": true}),
        |b, _| {
            b.respond_to("textDocument/documentSymbol", nested_reply());
        },
    );

    run(&mut ed, "lsp/document-symbols");
    ed.feed_key(key_down());
    ed.feed_key(key_enter());
    pump(&mut ed);

    assert_eq!(head(&ed), FOO_CALL);
}

#[test]
fn flat_symbol_information_lists_and_jumps_by_its_own_location() {
    let tmp = tempfile::tempdir().unwrap();
    let uri = rust_rig_uri(tmp.path());
    let (mut ed, _guard, _requests) = setup(
        tmp.path(),
        serde_json::json!({"documentSymbolProvider": true}),
        |b, _| {
            b.respond_to(
                "textDocument/documentSymbol",
                serde_json::json!([
                    {"name": "Bar", "kind": 23, "location": loc(&uri, 2, 0)},
                    {"name": "main", "kind": 12, "containerName": "crate",
                     "location": loc(&uri, 0, 3)}
                ]),
            );
        },
    );

    run(&mut ed, "lsp/document-symbols");

    insta::assert_snapshot!(picker_rows(&ed).join("\n"), @r"
    Bar  struct
    main  function  crate
    ");

    ed.feed_key(key_enter());
    pump(&mut ed);
    assert_eq!(head(&ed), CLOSING_BRACE);
}

#[test]
fn null_optional_fields_read_as_absent() {
    let tmp = tempfile::tempdir().unwrap();
    let uri = rust_rig_uri(tmp.path());
    let (mut ed, _guard, _requests) = setup(
        tmp.path(),
        serde_json::json!({"documentSymbolProvider": true}),
        |b, _| {
            b.respond_to(
                "textDocument/documentSymbol",
                serde_json::json!([
                    {"name": "Bar", "kind": 23, "containerName": null,
                     "location": loc(&uri, 2, 0)},
                    {"name": "main", "kind": 12, "children": null,
                     "range": range(0, 0, 11), "selectionRange": range(0, 3, 7)}
                ]),
            );
        },
    );

    run(&mut ed, "lsp/document-symbols");

    insta::assert_snapshot!(picker_rows(&ed).join("\n"), @r"
    Bar  struct
    main  function
    ");
}

#[test]
fn a_file_with_no_symbols_reports_it_and_opens_no_picker() {
    let tmp = tempfile::tempdir().unwrap();
    let (mut ed, _guard, _requests) = setup(
        tmp.path(),
        serde_json::json!({"documentSymbolProvider": true}),
        |b, _| {
            b.respond_to("textDocument/documentSymbol", serde_json::json!([]));
        },
    );

    run(&mut ed, "lsp/document-symbols");

    assert!(ed.state.input.picker().is_none());
    let msg = ed.state.status_msg.clone().unwrap_or_default();
    assert!(msg.to_lowercase().contains("no symbols"), "got {msg:?}");
}

#[test]
fn document_symbols_without_a_provider_report_unsupported() {
    let tmp = tempfile::tempdir().unwrap();
    let (mut ed, _guard, requests) = setup(tmp.path(), serde_json::json!({}), |_, _| {});

    run(&mut ed, "lsp/document-symbols");

    assert!(ed.state.input.picker().is_none());
    assert_eq!(request_count(&requests, "textDocument/documentSymbol"), 0);
    let msg = ed.state.status_msg.clone().unwrap_or_default();
    assert!(msg.contains("not supported"), "got {msg:?}");
}

fn workspace_setup(tmp: &Path) -> (Editor, RealRuntimeDirs, RequestLog) {
    let uri = rust_rig_uri(tmp);
    setup(
        tmp,
        serde_json::json!({"workspaceSymbolProvider": true}),
        move |b, _| {
            b.respond_to(
                "workspace/symbol",
                serde_json::json!([
                    {"name": "foo", "kind": 12, "location": loc(&uri, 1, 4)},
                    {"name": "main", "kind": 12, "containerName": "crate",
                     "location": loc(&uri, 0, 3)}
                ]),
            );
        },
    )
}

fn type_query(ed: &mut Editor, query: &str) {
    for c in query.chars() {
        ed.feed_key(key(c));
    }
    poll_until(
        ed,
        |ed| {
            ed.settle();
            ed.drain_lsp();
            ed.settle();
        },
        |ed| ed.state.input.picker().is_some_and(|p| p.total_len() == 2),
    );
}

#[test]
fn typing_queries_the_server_and_lists_rows_with_their_position() {
    let tmp = tempfile::tempdir().unwrap();
    let (mut ed, _guard, requests) = workspace_setup(tmp.path());

    run(&mut ed, "lsp/workspace-symbols");
    assert_eq!(picker_rows(&ed), Vec::<String>::new());
    type_query(&mut ed, "fo");

    let queries: Vec<_> = requests
        .borrow()
        .iter()
        .filter(|(_, method, _)| method == "workspace/symbol")
        .map(|(_, _, params)| params["query"].as_str().unwrap().to_string())
        .collect();
    assert_eq!(queries.last().map(String::as_str), Some("fo"));

    let rows = picker_rows(&ed);
    assert!(
        rows[0].starts_with("foo  function  ") && rows[0].ends_with("main.rs:2:5"),
        "{rows:?}"
    );
    assert!(
        rows[1].starts_with("main  function  ") && rows[1].ends_with("main.rs:1:4"),
        "{rows:?}"
    );
}

#[test]
fn a_failed_workspace_query_settles_the_picker_with_no_rows() {
    let tmp = tempfile::tempdir().unwrap();
    let (mut ed, _guard, requests) = setup(
        tmp.path(),
        serde_json::json!({"workspaceSymbolProvider": true}),
        |b, _| b.fail_with("workspace/symbol", -32603, "boom"),
    );

    run(&mut ed, "lsp/workspace-symbols");
    ed.feed_key(key('f'));
    poll_until(&mut ed, Editor::settle, |_| {
        request_count(&requests, "workspace/symbol") == 1
    });
    poll_until(&mut ed, pump, |ed| {
        !ed.state.input.picker().unwrap().is_pending()
    });

    assert_eq!(ed.state.input.picker().unwrap().total_len(), 0);
}

#[test]
fn an_answer_for_a_deleted_query_does_not_fill_the_picker() {
    let tmp = tempfile::tempdir().unwrap();
    let (mut ed, _guard, requests) = workspace_setup(tmp.path());

    run(&mut ed, "lsp/workspace-symbols");
    ed.feed_key(key('f'));
    poll_until(&mut ed, Editor::settle, |_| {
        request_count(&requests, "workspace/symbol") == 1
    });
    ed.feed_key(key_backspace());
    poll_until(&mut ed, Editor::settle, |ed| {
        !ed.state.input.picker().unwrap().is_pending()
    });
    pump(&mut ed);

    let picker = ed.state.input.picker().unwrap();
    assert_eq!(picker.total_len(), 0);
    assert!(!picker.is_pending());
}

#[test]
fn accepting_a_workspace_symbol_jumps_to_it() {
    let tmp = tempfile::tempdir().unwrap();
    let (mut ed, _guard, _requests) = workspace_setup(tmp.path());

    run(&mut ed, "lsp/workspace-symbols");
    type_query(&mut ed, "fo");
    ed.feed_key(key_enter());
    pump(&mut ed);

    assert_eq!(head(&ed), FOO_CALL);
}

#[test]
fn workspace_symbols_without_a_provider_report_unsupported() {
    let tmp = tempfile::tempdir().unwrap();
    let (mut ed, _guard, _requests) = setup(tmp.path(), serde_json::json!({}), |_, _| {});

    run(&mut ed, "lsp/workspace-symbols");

    assert!(ed.state.input.picker().is_none());
    let msg = ed.state.status_msg.clone().unwrap_or_default();
    assert!(msg.contains("not supported"), "got {msg:?}");
}
