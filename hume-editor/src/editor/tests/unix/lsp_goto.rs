// Goto definition family: `lsp-goto-definition` /
// `-declaration` / `-type-definition` / `-implementation`, composing
// `lsp-request`, `lsp-capabilities`, `goto-location!`,
// `show-drawer-list!` (via lib.scm's lsp/show-locations!). Loads the real
// shipped `core:lsp` plugin in place (`RealRuntimeGuard`).
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

/// Writes the fixture file up front and returns its `file://` URI — callers
/// need the URI *before* `setup` to build their scripted `Location`
/// response, since `configure` must run before the backend is boxed into
/// `LspState` (trait-erased afterward, per `lsp_hover.rs`).
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

/// Same shape as `lsp_hover.rs::setup`: a real opened file (three lines, so
/// a `Location` can point at a different line for the jump-back test),
/// driven handshake (so `lsp-capabilities` decodes), the real `core:lsp`
/// plugin loaded in place.
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
        serde_json::json!({"capabilities": {
            "definitionProvider": true, "declarationProvider": true,
            "typeDefinitionProvider": true, "implementationProvider": true
        }}),
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

fn run_goto(ed: &mut Editor, cmd: &str) {
    type_cmd(ed, cmd);
    // Drain the Command-mode entry/exit's on-mode-change hooks now (mirrors
    // the real interactive loop, which drains after every keystroke) before
    // the async response arrives — same ordering fix as lsp_hover.rs.
    ed.drain_hooks();
    ed.drain_lsp();
    ed.drain_pending_steel_calls();
    ed.drain_hooks();
}

fn loc(uri: &str, line: u64, character: u64) -> serde_json::Value {
    serde_json::json!({
        "uri": uri,
        "range": {"start": {"line": line, "character": character}, "end": {"line": line, "character": character}}
    })
}

fn location_link(uri: &str, line: u64, character: u64) -> serde_json::Value {
    serde_json::json!({
        "targetUri": uri,
        "targetRange": {"start": {"line": line, "character": character}, "end": {"line": line, "character": character + 3}},
        "targetSelectionRange": {"start": {"line": line, "character": character}, "end": {"line": line, "character": character + 3}}
    })
}

#[test]
fn null_result_reports_no_definition_found() {
    let tmp = safe_tempdir();
    let file_dir = safe_tempdir();
    let (file, _uri) = write_fixture_file(file_dir.path());
    let (mut ed, _guard, _sid) = setup(&file, tmp.path(), |backend, _sid| {
        backend.respond_to("textDocument/definition", serde_json::Value::Null);
    });

    run_goto(&mut ed, ":lsp-goto-definition");

    let msg = ed.state.status_msg.clone().unwrap_or_default();
    assert!(
        msg.to_lowercase().contains("no definition found"),
        "expected a no-definition message, got {msg:?}"
    );
}

#[test]
fn single_location_hashmap_jumps_directly() {
    let tmp = safe_tempdir();
    let file_dir = safe_tempdir();
    let (file, uri) = write_fixture_file(file_dir.path());
    let (mut ed, _guard, _sid) = setup(&file, tmp.path(), |backend, _sid| {
        backend.respond_to("textDocument/definition", loc(&uri, 1, 4));
    });

    run_goto(&mut ed, ":lsp-goto-definition");

    assert_eq!(
        ed.doc()
            .text()
            .char_to_line(ed.current_selections().primary().head()),
        1,
        "a single Location must jump directly to line 1 (0-indexed)"
    );
}

#[test]
fn single_element_array_jumps_directly() {
    let tmp = safe_tempdir();
    let file_dir = safe_tempdir();
    let (file, uri) = write_fixture_file(file_dir.path());
    let (mut ed, _guard, _sid) = setup(&file, tmp.path(), |backend, _sid| {
        backend.respond_to(
            "textDocument/definition",
            serde_json::json!([loc(&uri, 1, 4)]),
        );
    });

    run_goto(&mut ed, ":lsp-goto-definition");

    assert_eq!(
        ed.doc()
            .text()
            .char_to_line(ed.current_selections().primary().head()),
        1,
        "a length-1 Location[] must jump directly, not open the drawer"
    );
    assert!(ed.state.drawer.is_none());
}

#[test]
fn multi_element_array_opens_the_drawer_and_row_select_jumps() {
    let tmp = safe_tempdir();
    let file_dir = safe_tempdir();
    let (file, uri) = write_fixture_file(file_dir.path());
    let (mut ed, _guard, _sid) = setup(&file, tmp.path(), |backend, _sid| {
        backend.respond_to(
            "textDocument/definition",
            serde_json::json!([loc(&uri, 0, 0), loc(&uri, 1, 4), loc(&uri, 2, 0)]),
        );
    });

    run_goto(&mut ed, ":lsp-goto-definition");

    let rows = {
        let guard = ed.state.drawer_view.read().unwrap();
        guard
            .as_ref()
            .expect("drawer must open for a multi-entry array")
            .rows
            .clone()
    };
    assert_eq!(rows.len(), 3);

    // Select row index 1 (the second entry, line 1).
    ed.handle_key(key('j'));
    ed.handle_key(key_enter());
    ed.drain_pending_steel_calls();

    assert_eq!(
        ed.doc()
            .text()
            .char_to_line(ed.current_selections().primary().head()),
        1,
        "selecting row 2 in the drawer must jump to that entry's line"
    );
}

/// The multi-entry-array case exercises `lsp/location-display`'s row
/// formatting (`lsp/normalize-location`), unlike the single-entry jump
/// tests above (which only ever reach `goto-location!` directly) — the
/// only path in core:lsp that reads a `LocationLink`'s `targetUri`/
/// `targetSelectionRange` outside Rust's own dual-shape dispatch.
#[test]
fn multi_element_location_link_array_opens_the_drawer_and_row_select_jumps() {
    let tmp = safe_tempdir();
    let file_dir = safe_tempdir();
    let (file, uri) = write_fixture_file(file_dir.path());
    let (mut ed, _guard, _sid) = setup(&file, tmp.path(), |backend, _sid| {
        backend.respond_to(
            "textDocument/definition",
            serde_json::json!([
                location_link(&uri, 0, 0),
                location_link(&uri, 1, 4),
                location_link(&uri, 2, 0)
            ]),
        );
    });

    run_goto(&mut ed, ":lsp-goto-definition");

    let rows = {
        let guard = ed.state.drawer_view.read().unwrap();
        guard
            .as_ref()
            .expect("drawer must open for a multi-entry LocationLink array")
            .rows
            .clone()
    };
    assert_eq!(rows.len(), 3);
    assert!(
        rows[1].ends_with(":2:5"),
        "row text must be built from targetSelectionRange (1-based line:col), got {:?}",
        rows[1]
    );

    // Select row index 1 (the second entry, line 1).
    ed.handle_key(key('j'));
    ed.handle_key(key_enter());
    ed.drain_pending_steel_calls();

    assert_eq!(
        ed.doc()
            .text()
            .char_to_line(ed.current_selections().primary().head()),
        1,
        "selecting a LocationLink drawer row must jump to targetSelectionRange's line"
    );
}

#[test]
fn location_link_array_prefers_target_selection_range() {
    let tmp = safe_tempdir();
    let file_dir = safe_tempdir();
    let (file, uri) = write_fixture_file(file_dir.path());
    let (mut ed, _guard, _sid) = setup(&file, tmp.path(), |backend, _sid| {
        backend.respond_to(
            "textDocument/definition",
            serde_json::json!([location_link(&uri, 1, 4)]),
        );
    });

    run_goto(&mut ed, ":lsp-goto-definition");

    assert_eq!(
        ed.doc()
            .text()
            .char_to_line(ed.current_selections().primary().head()),
        1,
        "a single-entry LocationLink[] must jump using targetSelectionRange"
    );
}

#[test]
fn jump_back_returns_to_the_origin_after_a_jump() {
    let tmp = safe_tempdir();
    let file_dir = safe_tempdir();
    let (file, uri) = write_fixture_file(file_dir.path());
    let (mut ed, _guard, _sid) = setup(&file, tmp.path(), |backend, _sid| {
        backend.respond_to("textDocument/definition", loc(&uri, 1, 4));
    });
    let before = state(&ed);

    run_goto(&mut ed, ":lsp-goto-definition");
    assert_ne!(
        state(&ed),
        before,
        "sanity: the jump must have moved the cursor"
    );

    ed.handle_key(key_ctrl('o'));
    assert_eq!(
        state(&ed),
        before,
        "Ctrl+o must return to the pre-jump position"
    );
}

#[test]
fn nonexistent_target_errors_without_moving_the_cursor() {
    let tmp = safe_tempdir();
    let file_dir = safe_tempdir();
    let (file, _uri) = write_fixture_file(file_dir.path());
    let bogus_uri = "file:///nonexistent/path/that/does/not/exist.rs";
    let (mut ed, _guard, _sid) = setup(&file, tmp.path(), |backend, _sid| {
        backend.respond_to("textDocument/definition", loc(bogus_uri, 0, 0));
    });
    let before = state(&ed);

    run_goto(&mut ed, ":lsp-goto-definition");

    assert_eq!(state(&ed), before, "a failed goto must not move the cursor");
    let msg = ed.state.status_msg.clone().unwrap_or_default();
    assert!(
        msg.to_lowercase().contains("error") || msg.to_lowercase().contains("goto-location"),
        "expected an error message, got {msg:?}"
    );
}

#[test]
fn each_command_sends_its_own_method() {
    // Wiring smoke test: script a distinctive response only for the exact
    // method each command should send. If a command sent the wrong method,
    // no response would be queued for it and the jump would never happen.
    for (cmd, method) in [
        ("lsp-goto-definition", "textDocument/definition"),
        ("lsp-goto-declaration", "textDocument/declaration"),
        ("lsp-goto-type-definition", "textDocument/typeDefinition"),
        ("lsp-goto-implementation", "textDocument/implementation"),
    ] {
        let tmp = safe_tempdir();
        let file_dir = safe_tempdir();
        let (file, uri) = write_fixture_file(file_dir.path());
        let (mut ed, _guard, _sid) = setup(&file, tmp.path(), |backend, _sid| {
            backend.respond_to(method, loc(&uri, 1, 4));
        });

        run_goto(&mut ed, &format!(":{cmd}"));

        assert_eq!(
            ed.doc()
                .text()
                .char_to_line(ed.current_selections().primary().head()),
            1,
            "{cmd} must send {method} and jump on its response"
        );
    }
}
