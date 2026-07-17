// Rename: `lsp-rename` composing `lsp-request`, `lsp-capabilities`,
// `apply-workspace-edit!`, `prompt!`, `symbol-under-cursor`. Loads the real
// shipped `core:lsp` plugin in place
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

/// Writes "fn main() {\n    helper();\n}\n" and returns its (path, uri) —
/// cursor lands inside "helper" on line 1 (0-indexed), matching the search
/// each test does before invoking `lsp-rename`.
#[cfg(not(windows))]
fn write_fixture_file(file_dir: &Path) -> (PathBuf, String) {
    let file = file_dir.join("main.rs");
    std::fs::write(&file, "fn main() {\n    helper();\n}\n").unwrap();
    let canonical = std::fs::canonicalize(&file).unwrap();
    let uri = hume_lsp::uri::path_to_uri(&canonical)
        .unwrap()
        .as_str()
        .to_string();
    (file, uri)
}

#[cfg(not(windows))]
fn setup(
    file: &Path,
    tmp: &Path,
    configure: impl FnOnce(&mut InlineLspBackend, ServerId),
) -> (Editor, RealRuntimeGuard, ServerId) {
    let guard = RealRuntimeGuard::new();

    let mut ed = Editor::open(None).unwrap();

    let mut backend = InlineLspBackend::new();
    backend.respond_to(
        "initialize",
        serde_json::json!({"capabilities": {"renameProvider": true}}),
    );
    let sid = backend.start("rust-analyzer", &[], Path::new(".")).unwrap();
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
    // Land the cursor inside "helper" (line 1, col 4) so
    // symbol-under-cursor has something real to extract.
    let pid = ed.state.focused_pane_id;
    let pbs = ed
        .state
        .panes
        .state
        .get_mut(pid)
        .and_then(|by_buf| by_buf.get_mut(bid))
        .expect("pane buffer state must exist");
    pbs.selections = hume_editing::selection::SelectionSet::single(
        hume_editing::selection::Selection::collapsed(16), // 'h' of "helper" on line 1
    );

    let (sid2, ev) = ed.lsp.backend_mut().drain().into_iter().next().unwrap();
    let actions = ed.lsp.client_for_test(sid2).unwrap().on_event(ev);
    for action in actions {
        ed.dispatch_lsp_action(sid2, action);
    }

    let mut host = ScriptingHost::new();
    eval_with_real_host(&mut ed, &mut host, r#"(load-plugin "core:lsp")"#, tmp);
    ed.scripting = Some(host);

    (ed, guard, sid)
}

#[cfg(not(windows))]
fn run_rename(ed: &mut Editor) {
    type_cmd(ed, ":lsp-rename");
    ed.drain_hooks();
}

#[test]
#[cfg(not(windows))]
fn prompt_prefill_shows_the_symbol_under_cursor() {
    let tmp = safe_tempdir();
    let file_dir = safe_tempdir();
    let (file, _uri) = write_fixture_file(file_dir.path());
    let (mut ed, _guard, _sid) = setup(&file, tmp.path(), |_backend, _sid| {});

    run_rename(&mut ed);

    let mb = ed.state.minibuf.as_ref().expect("prompt must be open");
    assert_eq!(
        mb.input, "helper",
        "prefill must be the word under the cursor"
    );
}

#[test]
#[cfg(not(windows))]
fn cancel_sends_no_rename_request() {
    let tmp = safe_tempdir();
    let file_dir = safe_tempdir();
    let (file, uri) = write_fixture_file(file_dir.path());
    // Script a response that WOULD apply visibly if the request were sent
    // despite the cancel — proves the guard, not just "nothing crashed".
    let (mut ed, _guard, _sid) = setup(&file, tmp.path(), |backend, _sid| {
        backend.respond_to(
            "textDocument/rename",
            serde_json::json!({"changes": {uri: [
                {"range": {"start": {"line": 1, "character": 4}, "end": {"line": 1, "character": 10}}, "newText": "SHOULD_NOT_APPLY"}
            ]}}),
        );
    });
    let before = ed.doc().text().to_string();

    run_rename(&mut ed);
    ed.feed_key(key_esc());
    ed.drain_pending_steel_calls();
    ed.drain_lsp();
    ed.drain_pending_steel_calls();

    assert_eq!(
        ed.doc().text().to_string(),
        before,
        "cancel must not apply any edit"
    );
    assert!(
        !ed.state
            .status_msg
            .clone()
            .unwrap_or_default()
            .contains("buffers modified"),
        "cancel must not send the rename request at all"
    );
}

#[test]
#[cfg(not(windows))]
fn null_result_reports_nothing_to_rename() {
    let tmp = safe_tempdir();
    let file_dir = safe_tempdir();
    let (file, _uri) = write_fixture_file(file_dir.path());
    let (mut ed, _guard, _sid) = setup(&file, tmp.path(), |backend, _sid| {
        backend.respond_to("textDocument/rename", serde_json::Value::Null);
    });

    run_rename(&mut ed);
    ed.feed_key(key('X'));
    ed.feed_key(key_enter());
    ed.drain_pending_steel_calls();
    ed.drain_lsp();
    ed.drain_pending_steel_calls();

    let msg = ed.state.status_msg.clone().unwrap_or_default();
    assert!(
        msg.to_lowercase().contains("nothing to rename"),
        "expected a nothing-to-rename message, got {msg:?}"
    );
}

#[test]
#[cfg(not(windows))]
fn multi_file_workspace_edit_applies_and_logs_the_summary() {
    let tmp = safe_tempdir();
    let file_dir = safe_tempdir();
    let (file, uri) = write_fixture_file(file_dir.path());
    let other_file = file_dir.path().join("lib.rs");
    std::fs::write(&other_file, "fn helper() {}\n").unwrap();
    let other_uri = hume_lsp::uri::path_to_uri(&std::fs::canonicalize(&other_file).unwrap())
        .unwrap()
        .as_str()
        .to_string();

    let (mut ed, _guard, _sid) = setup(&file, tmp.path(), move |backend, _sid| {
        backend.respond_to(
            "textDocument/rename",
            serde_json::json!({"changes": {
                uri.clone(): [
                    {"range": {"start": {"line": 1, "character": 4}, "end": {"line": 1, "character": 10}}, "newText": "renamed"}
                ],
                other_uri: [
                    {"range": {"start": {"line": 0, "character": 3}, "end": {"line": 0, "character": 9}}, "newText": "renamed"}
                ]
            }}),
        );
    });

    run_rename(&mut ed);
    // Accept the "helper" prefill as-is (typing nothing, just Enter) — the
    // exact new name doesn't matter, only that it's non-empty so `when
    // new-name` fires.
    ed.feed_key(key_enter());
    ed.drain_pending_steel_calls();
    ed.drain_lsp();
    ed.drain_pending_steel_calls();

    assert_eq!(
        ed.doc().text().to_string(),
        "fn main() {\n    renamed();\n}\n",
        "the currently-open file's edit must apply"
    );
    let msg = ed.state.status_msg.clone().unwrap_or_default();
    assert!(
        msg.contains('2') && msg.contains("buffers modified"),
        "expected the 2-buffer summary, got {msg:?}"
    );
}
