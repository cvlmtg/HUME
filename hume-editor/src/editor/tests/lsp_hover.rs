// F1 (docs/lsp/step-4.md) — hover: `lsp-hover` composing B2 (lsp-request),
// B3 (lsp-capabilities), U4 (show-popup!), U6 (show-drawer-list!). Loads the
// real shipped `core:lsp` plugin in place (`RealRuntimeGuard` points
// HUME_RUNTIME at the actual on-disk runtime/ dir) so tests exercise the
// actual code, not a hand-rolled stand-in.
//
// Not on Windows: Scheme require strings embed OS paths; backslashes are not
// escaped in Steel string literals (same constraint as tests/plugins.rs).

use std::path::{Path, PathBuf};

use super::*;
use crate::editor::lsp::LspState;
use crate::editor::scripting_setup::make_init_host;
use hume_engine::pipeline::RenderContext;
use hume_lsp::backend::{LspBackend, ServerId};
use hume_lsp::client::LspClient;
use hume_lsp::inline::InlineLspBackend;
use hume_scripting::ScriptingHost;

#[cfg(not(windows))]
fn eval_with_real_host(ed: &mut Editor, host: &mut ScriptingHost, source: &str, tmp: &Path) {
    let init_path = tmp.join("init.scm");
    std::fs::write(&init_path, source).unwrap();
    let mut ih = make_init_host(&mut ed.state, &mut ed.view);
    host.eval_init(&init_path, 10_000, &mut ih, Default::default())
        .expect("eval_init");
}

/// Builds an editor with a real opened file attached to a scripted server
/// whose handshake has fully completed (so `lsp-capabilities` decodes real
/// data — a shortcut `client.state = Running` skips that, per
/// `lsp_introspect.rs`), then loads the real shipped `core:lsp` plugin.
/// `lsp-position-params` requires `buf.path()` to be `Some`, so every F1
/// test needs a real file — a bare `editor_from` buffer won't do.
///
/// `configure` scripts any responses beyond `initialize` (e.g.
/// `textDocument/hover`) — it must run *before* the backend is boxed into
/// `LspState`, since `client_and_backend`/`backend_mut` only expose the
/// trait object afterward, which can't reach `InlineLspBackend::respond_to`.
#[cfg(not(windows))]
fn setup(
    file_dir: &Path,
    tmp: &Path,
    initialize_result: serde_json::Value,
    configure: impl FnOnce(&mut InlineLspBackend, ServerId),
) -> (Editor, RealRuntimeGuard, ServerId) {
    let guard = RealRuntimeGuard::new();

    let mut ed = Editor::open(None).unwrap();
    let file = file_dir.join("main.rs");
    std::fs::write(&file, "fn main() {}\n").unwrap();

    let mut backend = InlineLspBackend::new();
    backend.respond_to("initialize", initialize_result);
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

#[cfg(not(windows))]
fn popup_lines(ed: &Editor) -> Option<Vec<String>> {
    ed.state.popup_view.read().unwrap().as_ref().map(|s| s.lines.clone())
}

#[cfg(not(windows))]
fn run_hover(ed: &mut Editor) {
    type_cmd(ed, ":lsp-hover");
    // Drain the Command-mode entry/exit's on-mode-change hooks (which
    // include lsp-hover's own close-on-mode-change dismiss) *before* the
    // async response arrives — mirrors the real interactive loop, which
    // drains hooks after every keystroke, well before any network response
    // could land. Draining hooks only at the end would incorrectly replay
    // those stale mode changes after the popup is shown, closing it.
    ed.drain_hooks();
    ed.drain_lsp();
    ed.drain_pending_steel_calls();
    ed.drain_hooks();
}

#[test]
#[cfg(not(windows))]
fn popup_shows_the_fixture_content() {
    let tmp = safe_tempdir();
    let file_dir = safe_tempdir();
    let (mut ed, _guard, _sid) = setup(
        file_dir.path(),
        tmp.path(),
        serde_json::json!({"capabilities": {"hoverProvider": true}}),
        |backend, _sid| {
            backend.respond_to(
                "textDocument/hover",
                serde_json::json!({
                    "contents": {"kind": "plaintext", "value": "fn main()"},
                    "range": {"start": {"line": 0, "character": 3}, "end": {"line": 0, "character": 7}}
                }),
            );
        },
    );

    run_hover(&mut ed);
    let mut ctx = RenderContext::new();
    ed.prepare_frame(80, 25, &mut ctx);

    assert_eq!(
        popup_lines(&ed),
        Some(vec!["fn main()".to_string()]),
        "the worked fixture's MarkupContent value must render verbatim in the popup"
    );
}

#[test]
#[cfg(not(windows))]
fn null_result_logs_and_shows_no_popup() {
    let tmp = safe_tempdir();
    let file_dir = safe_tempdir();
    let (mut ed, _guard, _sid) = setup(
        file_dir.path(),
        tmp.path(),
        serde_json::json!({"capabilities": {"hoverProvider": true}}),
        |backend, _sid| {
            backend.respond_to("textDocument/hover", serde_json::Value::Null);
        },
    );

    run_hover(&mut ed);
    let mut ctx = RenderContext::new();
    ed.prepare_frame(80, 25, &mut ctx);

    assert!(popup_lines(&ed).is_none(), "null hover result must not open a popup");
    let msg = ed.state.status_msg.clone().unwrap_or_default();
    assert!(
        msg.to_lowercase().contains("no hover info"),
        "expected a 'no hover info' message, got {msg:?}"
    );
}

#[test]
#[cfg(not(windows))]
fn error_reports_via_the_message_log() {
    let tmp = safe_tempdir();
    let file_dir = safe_tempdir();
    let (mut ed, _guard, _sid) = setup(
        file_dir.path(),
        tmp.path(),
        serde_json::json!({"capabilities": {"hoverProvider": true}}),
        |backend, _sid| {
            backend.fail_with("textDocument/hover", -32603, "boom");
        },
    );

    run_hover(&mut ed);
    let mut ctx = RenderContext::new();
    ed.prepare_frame(80, 25, &mut ctx);

    assert!(popup_lines(&ed).is_none(), "a protocol error must not open a popup");
    let msg = ed.state.status_msg.clone().unwrap_or_default();
    assert!(
        msg.to_lowercase().contains("boom"),
        "expected the server's error message surfaced, got {msg:?}"
    );
}

#[test]
#[cfg(not(windows))]
fn tall_content_falls_back_to_the_drawer() {
    let tmp = safe_tempdir();
    let file_dir = safe_tempdir();
    // No on-viewport-change has fired in this test, so the popup/drawer
    // threshold falls back to 15 lines (lib.scm) — 20 lines must overflow.
    let tall = (0..20).map(|i| format!("line{i}")).collect::<Vec<_>>().join("\n");
    let (mut ed, _guard, _sid) = setup(
        file_dir.path(),
        tmp.path(),
        serde_json::json!({"capabilities": {"hoverProvider": true}}),
        |backend, _sid| {
            backend.respond_to("textDocument/hover", serde_json::json!({"contents": tall}));
        },
    );

    run_hover(&mut ed);
    let mut ctx = RenderContext::new();
    ed.prepare_frame(80, 25, &mut ctx);

    assert!(popup_lines(&ed).is_none(), "tall content must not use the popup");
    assert!(ed.state.drawer.is_some(), "tall content must fall back to the drawer");
}

#[test]
#[cfg(not(windows))]
fn capability_gate_skips_the_request_when_hover_unsupported() {
    // No response scripted for "textDocument/hover" — if the capability
    // gate failed open (called the request thunk anyway), the request
    // would go unanswered and `status_msg` would stay unset, not mention
    // "not supported"; this is a sufficient oracle without needing to
    // inspect the (trait-erased, post-boxing unreachable) sent log.
    let tmp = safe_tempdir();
    let file_dir = safe_tempdir();
    // No hoverProvider in the advertised capabilities.
    let (mut ed, _guard, _sid) = setup(
        file_dir.path(),
        tmp.path(),
        serde_json::json!({"capabilities": {}}),
        |_backend, _sid| {},
    );

    run_hover(&mut ed);

    let msg = ed.state.status_msg.clone().unwrap_or_default();
    assert!(
        msg.to_lowercase().contains("not supported"),
        "expected a not-supported message, got {msg:?}"
    );
}

#[test]
#[cfg(not(windows))]
fn allow_stale_is_honored_despite_an_intervening_edit() {
    let tmp = safe_tempdir();
    let file_dir = safe_tempdir();
    let (mut ed, _guard, _sid) = setup(
        file_dir.path(),
        tmp.path(),
        serde_json::json!({"capabilities": {"hoverProvider": true}}),
        |backend, _sid| {
            backend.respond_to("textDocument/hover", serde_json::json!({"contents": "fn main()"}));
        },
    );

    type_cmd(&mut ed, ":lsp-hover");
    // Drain the Command-mode entry/exit's on-mode-change hooks now, exactly
    // as the real interactive loop would (after every keystroke) — well
    // before the async response arrives, so lsp-hover's close-on-mode-
    // change dismiss can't replay against a popup that isn't open yet.
    ed.drain_hooks();

    // Bump the buffer's text_gen between send and drain — without
    // #:allow-stale this response would be dropped (B2).
    ed.feed_key(key('i'));
    ed.drain_hooks();
    ed.feed_key(key('X'));
    ed.feed_key(key_esc());
    ed.drain_hooks();

    ed.drain_lsp();
    ed.drain_pending_steel_calls();
    ed.drain_hooks();
    let mut ctx = RenderContext::new();
    ed.prepare_frame(80, 25, &mut ctx);

    assert_eq!(
        popup_lines(&ed),
        Some(vec!["fn main()".to_string()]),
        "hover must pass #:allow-stale #t and still show the popup after an intervening edit"
    );
}
