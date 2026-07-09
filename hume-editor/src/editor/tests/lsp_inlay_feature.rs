// Inlay hints: debounced textDocument/inlayHint on viewport change and
// diagnostics change, composing `lsp-request`, `lsp-capabilities`, debounce,
// `set-inlay-hints!`, `on-viewport-change`, `on-diagnostics-changed`,
// and rendering (not tested here, its own pinned snapshots cover that).
// Named lsp_inlay_feature.rs — lsp_inlay_hints.rs already covers rendering
// of the decoration store directly; this file drives the same store through
// the real shipped plugin and a real LSP round trip. Loads the real shipped
// `core:lsp` plugin in place (`RealRuntimeGuard`).
//
// Not on Windows: Scheme require strings embed OS paths; backslashes are not
// escaped in Steel string literals (same constraint as tests/plugins.rs).

use std::path::{Path, PathBuf};
use std::time::Duration;

use super::*;
use crate::editor::lsp::LspState;
use crate::editor::scripting_setup::make_init_host;
use hume_engine::pipeline::RenderContext;
use hume_lsp::backend::{LspBackend, ServerId};
use hume_lsp::client::LspClient;
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
fn write_fixture_file(file_dir: &Path) -> PathBuf {
    let file = file_dir.join("main.rs");
    std::fs::write(&file, "let x = 1;\n").unwrap();
    file
}

#[cfg(not(windows))]
fn setup(
    file: &Path,
    tmp: &Path,
    configure: impl FnOnce(&mut RecordingLspBackend, ServerId),
) -> (Editor, RealRuntimeGuard, RequestLog) {
    let guard = RealRuntimeGuard::new();

    let (mut backend, _notifications, requests) = RecordingLspBackend::new();
    backend.respond_to(
        "initialize",
        serde_json::json!({"capabilities": {"inlayHintProvider": true}}),
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

    (ed, guard, requests)
}

#[cfg(not(windows))]
fn fire_viewport_change(ed: &mut Editor) {
    let mut ctx = RenderContext::new();
    ed.prepare_frame(80, 25, &mut ctx);
    let pid = ed.state.focused_pane_id;
    ed.fire_hook_viewport_change(pid);
}

/// The debounced thunk itself is only *queued* by `drain_due_timers`
/// (`queue_steel_call`) — actually running it, and thus actually sending
/// the wire request, waits for `drain_pending_steel_calls`. That call's
/// own `flush_pending_lsp_calls` sends the request and the scripted
/// backend auto-queues its response synchronously, but nothing drains
/// *that* within the same call — an extra `drain_lsp` +
/// `drain_pending_steel_calls` round is needed to actually invoke the
/// response callback. `prepare_frame` does exactly this pair internally
/// every real frame; a test not calling it needs the pair explicitly.
#[cfg(not(windows))]
fn settle_after_debounce(ed: &mut Editor) {
    ed.drain_hooks();
    std::thread::sleep(Duration::from_millis(300));
    ed.drain_async_sources();
    ed.drain_lsp();
    ed.drain_pending_steel_calls();
    ed.drain_lsp();
    ed.drain_pending_steel_calls();
}

#[cfg(not(windows))]
fn request_count(requests: &RequestLog, method: &str) -> usize {
    requests
        .borrow()
        .iter()
        .filter(|(_sid, m, _params)| m == method)
        .count()
}

#[cfg(not(windows))]
fn inlay_hint_response(entries: &[(u32, u32, serde_json::Value)]) -> serde_json::Value {
    serde_json::Value::Array(
        entries
            .iter()
            .map(|(line, character, label)| {
                serde_json::json!({
                    "position": {"line": line, "character": character},
                    "label": label,
                })
            })
            .collect(),
    )
}

#[test]
#[cfg(not(windows))]
fn viewport_change_triggers_one_debounced_request() {
    let tmp = safe_tempdir();
    let file_dir = safe_tempdir();
    let file = write_fixture_file(file_dir.path());
    let (mut ed, _guard, requests) = setup(&file, tmp.path(), |backend, _sid| {
        backend.respond_to("textDocument/inlayHint", inlay_hint_response(&[]));
    });
    ed.state.settings.lsp_inlay_hints = true;

    fire_viewport_change(&mut ed);
    settle_after_debounce(&mut ed);

    assert_eq!(request_count(&requests, "textDocument/inlayHint"), 1);
}

#[test]
#[cfg(not(windows))]
fn setting_off_sends_no_request() {
    let tmp = safe_tempdir();
    let file_dir = safe_tempdir();
    let file = write_fixture_file(file_dir.path());
    let (mut ed, _guard, requests) = setup(&file, tmp.path(), |backend, _sid| {
        backend.respond_to("textDocument/inlayHint", inlay_hint_response(&[]));
    });
    // lsp_inlay_hints defaults to false — left untouched.

    fire_viewport_change(&mut ed);
    settle_after_debounce(&mut ed);

    assert_eq!(request_count(&requests, "textDocument/inlayHint"), 0);
}

#[test]
#[cfg(not(windows))]
fn hints_land_in_the_store_at_the_correct_char_offset() {
    let tmp = safe_tempdir();
    let file_dir = safe_tempdir();
    let file = write_fixture_file(file_dir.path());
    // "let x = 1;\n" — wire {line:0, character:4} is 'x' (char offset 4,
    // ASCII text, UTF-16 code units == char offsets).
    let (mut ed, _guard, _requests) = setup(&file, tmp.path(), |backend, _sid| {
        backend.respond_to(
            "textDocument/inlayHint",
            inlay_hint_response(&[(0, 4, serde_json::json!(": i32"))]),
        );
    });
    ed.state.settings.lsp_inlay_hints = true;
    let bid = ed.focused_buffer_id();

    fire_viewport_change(&mut ed);
    settle_after_debounce(&mut ed);

    let hints = ed.state.decorations.inlay_hints_for(bid);
    assert_eq!(hints.len(), 1);
    assert_eq!(hints[0].pos, 4);
    assert_eq!(hints[0].text, ": i32");
    assert!(hints[0].before);
}

#[test]
#[cfg(not(windows))]
fn label_parts_concatenate_and_padding_becomes_literal_spaces() {
    let tmp = safe_tempdir();
    let file_dir = safe_tempdir();
    let file = write_fixture_file(file_dir.path());
    let (mut ed, _guard, _requests) = setup(&file, tmp.path(), |backend, _sid| {
        backend.respond_to(
            "textDocument/inlayHint",
            serde_json::json!([{
                "position": {"line": 0, "character": 4},
                "label": [{"value": ":"}, {"value": " i32"}],
                "paddingLeft": true,
                "paddingRight": true
            }]),
        );
    });
    ed.state.settings.lsp_inlay_hints = true;
    let bid = ed.focused_buffer_id();

    fire_viewport_change(&mut ed);
    settle_after_debounce(&mut ed);

    let hints = ed.state.decorations.inlay_hints_for(bid);
    assert_eq!(hints.len(), 1);
    assert_eq!(
        hints[0].text, " : i32 ",
        "label parts must concatenate in order, then get padding spaces on both sides"
    );
}

#[test]
#[cfg(not(windows))]
fn diagnostics_changed_also_refreshes_hints() {
    let tmp = safe_tempdir();
    let file_dir = safe_tempdir();
    let file = write_fixture_file(file_dir.path());
    let (mut ed, _guard, requests) = setup(&file, tmp.path(), |backend, _sid| {
        backend.respond_to("textDocument/inlayHint", inlay_hint_response(&[]));
        backend.respond_to("textDocument/inlayHint", inlay_hint_response(&[]));
    });
    ed.state.settings.lsp_inlay_hints = true;
    let sid = ed
        .state
        .buffers
        .get(ed.focused_buffer_id())
        .lsp_server
        .expect("buffer must be attached");

    // Establish a viewport first — on-diagnostics-changed alone has no
    // range to work with (lib.scm's tracker gates on this).
    fire_viewport_change(&mut ed);
    settle_after_debounce(&mut ed);
    assert_eq!(request_count(&requests, "textDocument/inlayHint"), 1);

    let bid = ed.focused_buffer_id();
    ed.ingest_publish_diagnostics(
        sid,
        serde_json::json!({"uri": hume_lsp::uri::path_to_uri(&std::fs::canonicalize(&file).unwrap()).unwrap().as_str(), "diagnostics": []}),
    );
    ed.fire_hook_diagnostics_changed(bid);
    settle_after_debounce(&mut ed);

    assert_eq!(
        request_count(&requests, "textDocument/inlayHint"),
        2,
        "on-diagnostics-changed must also trigger a refresh once a viewport is known"
    );
}

#[test]
#[cfg(not(windows))]
fn no_viewport_seen_yet_skips_diagnostics_triggered_refresh() {
    let tmp = safe_tempdir();
    let file_dir = safe_tempdir();
    let file = write_fixture_file(file_dir.path());
    let (mut ed, _guard, requests) = setup(&file, tmp.path(), |backend, _sid| {
        backend.respond_to("textDocument/inlayHint", inlay_hint_response(&[]));
    });
    ed.state.settings.lsp_inlay_hints = true;
    let sid = ed
        .state
        .buffers
        .get(ed.focused_buffer_id())
        .lsp_server
        .expect("buffer must be attached");
    let bid = ed.focused_buffer_id();

    // No fire_viewport_change call — the tracker has never seen this buffer.
    ed.ingest_publish_diagnostics(
        sid,
        serde_json::json!({"uri": hume_lsp::uri::path_to_uri(&std::fs::canonicalize(&file).unwrap()).unwrap().as_str(), "diagnostics": []}),
    );
    ed.fire_hook_diagnostics_changed(bid);
    settle_after_debounce(&mut ed);

    assert_eq!(request_count(&requests, "textDocument/inlayHint"), 0);
}
