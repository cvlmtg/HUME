// F9 (docs/lsp/step-4.md) — code actions: `lsp-code-actions`, composing B2
// (lsp-request), B3 (lsp-capabilities), B5 (diagnostics-for-buffer's `raw`
// field — echoed back as context.diagnostics; servers gate diagnostic-
// derived quickfixes on this), B6 (apply-workspace-edit!), U5 (show-menu!).
// Loads the real shipped `core:lsp` plugin in place (`RealRuntimeGuard`).
//
// Not on Windows: Scheme require strings embed OS paths; backslashes are not
// escaped in Steel string literals (same constraint as tests/plugins.rs).

use std::path::{Path, PathBuf};

use super::*;
use crate::editor::lsp::LspState;
use crate::editor::scripting_setup::make_init_host;
use hume_lsp::backend::{LspBackend, ServerId};
use hume_lsp::client::LspClient;
use hume_lsp::codec::{Message, RequestId};
use hume_lsp::test_util::{RecordingLspBackend, RequestLog};
use hume_scripting::ScriptingHost;

#[cfg(not(windows))]
fn eval_with_real_host(ed: &mut Editor, host: &mut ScriptingHost, source: &str, tmp: &Path) {
    let init_path = tmp.join("init.scm");
    std::fs::write(&init_path, source).unwrap();
    let mut ih = make_init_host(&mut ed.state, &mut ed.view);
    host.eval_init(&init_path, 10_000, &mut ih, Default::default())
        .expect("eval_init");
}

#[cfg(not(windows))]
fn write_fixture_file(file_dir: &Path) -> (PathBuf, String) {
    let file = file_dir.join("main.rs");
    std::fs::write(&file, "fn main() {\n    let x = 1;\n}\n").unwrap();
    let canonical = std::fs::canonicalize(&file).unwrap();
    let uri = hume_lsp::uri::path_to_uri(&canonical).unwrap().as_str().to_string();
    (file, uri)
}

#[cfg(not(windows))]
fn setup(
    file: &Path,
    tmp: &Path,
    configure: impl FnOnce(&mut RecordingLspBackend, ServerId),
) -> (Editor, RealRuntimeGuard, ServerId, RequestLog) {
    let guard = RealRuntimeGuard::new();

    let (mut backend, _notifications, requests) = RecordingLspBackend::with_default_handshake_and_requests();
    backend.respond_to(
        "initialize",
        serde_json::json!({"capabilities": {"codeActionProvider": true}}),
    );
    let sid = backend.start("rust-analyzer", &[], Path::new(".")).unwrap();
    configure(&mut backend, sid);

    let mut ed = Editor::open(None).unwrap();
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
    eval_with_real_host(&mut ed, &mut host, r#"(load-plugin "core:lsp")"#, tmp);
    ed.scripting = Some(host);

    (ed, guard, sid, requests)
}

#[cfg(not(windows))]
fn run_actions(ed: &mut Editor) {
    type_cmd(ed, ":lsp-code-actions");
    ed.drain_hooks();
    ed.drain_lsp();
    ed.drain_pending_steel_calls();
}

#[cfg(not(windows))]
fn menu_items(ed: &Editor) -> Vec<String> {
    ed.state.menu.as_ref().map(|m| m.items.clone()).unwrap_or_default()
}

#[cfg(not(windows))]
fn last_request_params(requests: &RequestLog, method: &str) -> serde_json::Value {
    requests
        .borrow()
        .iter()
        .rev()
        .find(|(_sid, m, _params)| m == method)
        .map(|(_sid, _m, params)| params.clone())
        .unwrap_or_else(|| panic!("no {method} request was sent"))
}

fn edit_action(title: &str, uri: &str) -> serde_json::Value {
    serde_json::json!({
        "title": title,
        "edit": {"changes": {uri: [
            {"range": {"start": {"line": 1, "character": 12}, "end": {"line": 1, "character": 13}}, "newText": "2"}
        ]}}
    })
}

fn command_action(title: &str) -> serde_json::Value {
    serde_json::json!({
        "title": title,
        "command": {"title": title, "command": "smoke.doThing", "arguments": []}
    })
}

fn disabled_action(title: &str) -> serde_json::Value {
    serde_json::json!({"title": title, "disabled": {"reason": "not applicable here"}})
}

fn diagnostic_params(uri: &str) -> serde_json::Value {
    serde_json::json!({"uri": uri, "diagnostics": [
        {"range": {"start": {"line": 0, "character": 0}, "end": {"line": 0, "character": 10}},
         "severity": 4, "message": "unused import", "code": "unused_imports"}
    ]})
}

#[test]
#[cfg(not(windows))]
fn titles_are_listed_in_the_menu() {
    let tmp = safe_tempdir();
    let file_dir = safe_tempdir();
    let (file, uri) = write_fixture_file(file_dir.path());
    let (mut ed, _guard, _sid, _requests) = setup(&file, tmp.path(), |backend, _sid| {
        backend.respond_to(
            "textDocument/codeAction",
            serde_json::json!([edit_action("Fix the thing", &uri), command_action("Run the thing")]),
        );
    });

    run_actions(&mut ed);

    assert_eq!(menu_items(&ed), vec!["Fix the thing".to_string(), "Run the thing".to_string()]);
}

#[test]
#[cfg(not(windows))]
fn selecting_an_edit_action_applies_it() {
    let tmp = safe_tempdir();
    let file_dir = safe_tempdir();
    let (file, uri) = write_fixture_file(file_dir.path());
    let (mut ed, _guard, _sid, _requests) = setup(&file, tmp.path(), |backend, _sid| {
        backend.respond_to("textDocument/codeAction", serde_json::json!([edit_action("Fix the thing", &uri)]));
    });

    run_actions(&mut ed);
    ed.handle_key(key_enter());
    ed.drain_pending_steel_calls();

    assert_eq!(ed.doc().text().to_string(), "fn main() {\n    let x = 2;\n}\n");
}

#[test]
#[cfg(not(windows))]
fn selecting_a_command_action_runs_the_full_server_loop() {
    let tmp = safe_tempdir();
    let file_dir = safe_tempdir();
    let (file, uri) = write_fixture_file(file_dir.path());
    let (mut ed, _guard, sid, _requests) = setup(&file, tmp.path(), |backend, sid| {
        backend.respond_to("textDocument/codeAction", serde_json::json!([command_action("Run the thing")]));
        backend.respond_to("workspace/executeCommand", serde_json::Value::Null);
        // Simulate the server's real behavior: after executing the command,
        // it pushes an unsolicited workspace/applyEdit *request* back — the
        // full loop this test proves, not just "a request was sent".
        backend.push_from_server(
            sid,
            Message::Request {
                id: RequestId::Int(9000),
                method: "workspace/applyEdit".to_string(),
                params: serde_json::json!({"edit": {"changes": {uri: [
                    {"range": {"start": {"line": 1, "character": 12}, "end": {"line": 1, "character": 13}}, "newText": "99"}
                ]}}}),
            },
        );
    });

    run_actions(&mut ed);
    ed.handle_key(key_enter());
    ed.drain_pending_steel_calls();
    ed.drain_lsp();
    ed.drain_pending_steel_calls();
    let _ = sid;

    assert_eq!(
        ed.doc().text().to_string(),
        "fn main() {\n    let x = 99;\n}\n",
        "the server's follow-up workspace/applyEdit request must land"
    );
}

#[test]
#[cfg(not(windows))]
fn disabled_actions_are_filtered_out() {
    let tmp = safe_tempdir();
    let file_dir = safe_tempdir();
    let (file, uri) = write_fixture_file(file_dir.path());
    let (mut ed, _guard, _sid, _requests) = setup(&file, tmp.path(), |backend, _sid| {
        backend.respond_to(
            "textDocument/codeAction",
            serde_json::json!([disabled_action("Not available"), edit_action("Fix the thing", &uri)]),
        );
    });

    run_actions(&mut ed);

    assert_eq!(menu_items(&ed), vec!["Fix the thing".to_string()], "disabled actions must never appear");
}

#[test]
#[cfg(not(windows))]
fn empty_response_reports_no_code_actions_and_opens_no_menu() {
    let tmp = safe_tempdir();
    let file_dir = safe_tempdir();
    let (file, _uri) = write_fixture_file(file_dir.path());
    let (mut ed, _guard, _sid, _requests) = setup(&file, tmp.path(), |backend, _sid| {
        backend.respond_to("textDocument/codeAction", serde_json::Value::Null);
    });

    run_actions(&mut ed);

    assert!(ed.state.menu.is_none());
    let msg = ed.state.status_msg.clone().unwrap_or_default();
    assert!(
        msg.to_lowercase().contains("no code actions"),
        "expected a no-code-actions message, got {msg:?}"
    );
}

#[test]
#[cfg(not(windows))]
fn context_diagnostics_echoes_the_raw_diagnostic_overlapping_the_cursor() {
    let tmp = safe_tempdir();
    let file_dir = safe_tempdir();
    let (file, uri) = write_fixture_file(file_dir.path());
    let (mut ed, _guard, sid, requests) = setup(&file, tmp.path(), |backend, _sid| {
        backend.respond_to("textDocument/codeAction", serde_json::Value::Null);
    });
    ed.ingest_publish_diagnostics(sid, diagnostic_params(&uri));

    run_actions(&mut ed);

    let params = last_request_params(&requests, "textDocument/codeAction");
    let diags = params["context"]["diagnostics"].as_array().expect("diagnostics must be an array");
    assert_eq!(diags.len(), 1, "the cursor-overlapping diagnostic must be echoed back, got: {diags:?}");
    assert_eq!(diags[0]["message"], "unused import");
    assert_eq!(diags[0]["code"], "unused_imports");
    // Distinguishes the raw wire Diagnostic from B5's flat char-indexed
    // shape (which also carries "message"/"code" at the top level, so the
    // two prior asserts alone wouldn't catch passing the wrong one):
    // only the raw shape nests its bounds under "range".
    assert_eq!(diags[0]["range"]["start"]["line"], 0);
    assert_eq!(diags[0]["range"]["start"]["character"], 0);
    assert!(diags[0].get("start").is_none(), "must be the raw wire Diagnostic, not B5's flat shape");
}
