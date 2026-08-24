// References: `lsp-references`, reusing the goto-definition family's
// worker shape but always presenting the drawer (never auto-jumping even
// for a single result). Loads the real shipped `core:lsp` plugin in place
// (`RealRuntimeGuard`).
//
// Not on Windows: Scheme require strings embed OS paths; backslashes are not
// escaped in Steel string literals (same constraint as tests/plugins.rs).

use std::path::{Path, PathBuf};

use super::*;
use crate::editor::lsp::LspState;
use hume_lsp::backend::{LspBackend, ServerId};
use hume_lsp::client::LspClient;
use hume_lsp::inline::InlineLspBackend;
use hume_scripting::ScriptingHost;

fn write_fixture_file(file_dir: &Path) -> (PathBuf, String) {
    let file = file_dir.join("main.rs");
    std::fs::write(&file, "fn main() {\n    foo();\n}\n").unwrap();
    let canonical = std::fs::canonicalize(&file).unwrap();
    let uri = hume_lsp::uri::path_to_uri(&canonical)
        .unwrap()
        .as_str()
        .to_string();
    (file, uri)
}

fn setup(
    file: &Path,
    tmp: &Path,
    configure: impl FnOnce(&mut InlineLspBackend, ServerId),
) -> (Editor, RealRuntimeGuard, ServerId) {
    let guard = RealRuntimeGuard::new();

    let mut ed = Editor::open(None, std::sync::Arc::new(|| {})).unwrap();

    let mut backend = InlineLspBackend::new();
    backend.respond_to(
        "initialize",
        serde_json::json!({"capabilities": {"referencesProvider": true}}),
    );
    let sid = backend
        .start("rust-analyzer", &[], Path::new("."), &[])
        .unwrap();
    configure(&mut backend, sid);
    ed.lsp = LspState::from_backend_for_test(Box::new(backend));
    let mut client = LspClient::new(sid, PathBuf::from("."));
    client.start_handshake(ed.lsp.backend_mut());
    ed.lsp.insert_client_for_test(client);
    ed.lsp
        .insert_server_key_for_test("rust".to_string(), PathBuf::from("."), sid);

    ed.execute_typed("e", Some(file.to_str().unwrap())).unwrap();
    let bid = ed.focused_buffer_id();
    ed.state.buffers.get_mut(bid).lsp_server = Some(sid);

    let (sid2, ev) = ed.lsp.backend_mut().drain().into_iter().next().unwrap();
    let actions = ed.lsp.client_for_test(sid2).unwrap().on_event(ev);
    for action in actions {
        ed.dispatch_lsp_action(sid2, action);
    }

    let mut host = ScriptingHost::new();
    eval_with_real_host(
        &mut ed,
        &mut host,
        r#"(load-plugin "core:stdlib")
(load-plugin "core:lsp")"#,
        tmp,
    );
    ed.scripting = Some(host);

    (ed, guard, sid)
}

fn run_references(ed: &mut Editor) {
    type_cmd(ed, ":lsp-references");
    ed.settle();
    ed.drain_lsp();
    ed.settle();
}

fn loc(uri: &str, line: u64, character: u64) -> serde_json::Value {
    serde_json::json!({
        "uri": uri,
        "range": {"start": {"line": line, "character": character}, "end": {"line": line, "character": character}}
    })
}

#[test]
fn three_locations_list_three_rows() {
    let tmp = safe_tempdir();
    let file_dir = safe_tempdir();
    let (file, uri) = write_fixture_file(file_dir.path());
    let (mut ed, _guard, _sid) = setup(&file, tmp.path(), |backend, _sid| {
        backend.respond_to(
            "textDocument/references",
            serde_json::json!([loc(&uri, 0, 0), loc(&uri, 1, 4), loc(&uri, 2, 0)]),
        );
    });

    run_references(&mut ed);

    let rows = {
        let guard = ed.state.drawer_view.read().unwrap();
        guard.as_ref().expect("drawer must open").rows.clone()
    };
    assert_eq!(rows.len(), 3);
}

#[test]
fn enter_jumps_and_drawer_stays_open() {
    let tmp = safe_tempdir();
    let file_dir = safe_tempdir();
    let (file, uri) = write_fixture_file(file_dir.path());
    let (mut ed, _guard, _sid) = setup(&file, tmp.path(), |backend, _sid| {
        backend.respond_to(
            "textDocument/references",
            serde_json::json!([loc(&uri, 0, 0), loc(&uri, 1, 4), loc(&uri, 2, 0)]),
        );
    });

    run_references(&mut ed);
    ed.handle_key(key('j'));
    ed.handle_key(key_enter());
    ed.settle();

    assert_eq!(
        ed.doc()
            .text()
            .char_to_line(ed.current_selections().primary().head()),
        1,
        "Enter on row 2 must jump to that entry's line"
    );
    assert!(
        ed.state.config.drawer.is_some(),
        "the references drawer must stay open after a jump (drawer browse behavior)"
    );
}

#[test]
fn single_result_still_opens_the_drawer() {
    let tmp = safe_tempdir();
    let file_dir = safe_tempdir();
    let (file, uri) = write_fixture_file(file_dir.path());
    let (mut ed, _guard, _sid) = setup(&file, tmp.path(), |backend, _sid| {
        backend.respond_to(
            "textDocument/references",
            serde_json::json!([loc(&uri, 1, 4)]),
        );
    });

    run_references(&mut ed);

    assert!(
        ed.state.config.drawer.is_some(),
        "references must always drawer-list, even a single result (unlike goto's auto-jump)"
    );
}

#[test]
fn null_result_reports_no_references() {
    let tmp = safe_tempdir();
    let file_dir = safe_tempdir();
    let (file, _uri) = write_fixture_file(file_dir.path());
    let (mut ed, _guard, _sid) = setup(&file, tmp.path(), |backend, _sid| {
        backend.respond_to("textDocument/references", serde_json::Value::Null);
    });

    run_references(&mut ed);

    let msg = ed.state.status_msg.clone().unwrap_or_default();
    assert!(
        msg.to_lowercase().contains("no references"),
        "expected a no-references message, got {msg:?}"
    );
}
