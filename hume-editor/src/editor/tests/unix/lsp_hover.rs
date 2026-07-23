// Hover: `lsp-hover` composing `lsp-request`, `lsp-capabilities`,
// `show-popup!`, `show-drawer-list!`. Loads the
// real shipped `core:lsp` plugin in place (`RealRuntimeGuard` points
// HUME_RUNTIME at the actual on-disk runtime/ dir) so tests exercise the
// actual code, not a hand-rolled stand-in.
//
// Not on Windows: Scheme require strings embed OS paths; backslashes are not
// escaped in Steel string literals (same constraint as tests/plugins.rs).

use std::path::{Path, PathBuf};

use super::*;
use crate::editor::lsp::LspState;
use hume_engine::pipeline::RenderContext;
use hume_lsp::backend::{LspBackend, ServerId};
use hume_lsp::client::LspClient;
use hume_lsp::inline::InlineLspBackend;
use hume_scripting::ScriptingHost;

/// Builds an editor with a real opened file attached to a scripted server
/// whose handshake has fully completed (so `lsp-capabilities` decodes real
/// data — a shortcut `client.set_state_for_test(Running)` skips that, per
/// `lsp_introspect.rs`), then loads the real shipped `core:lsp` plugin.
/// `lsp-position-params` requires `buf.path()` to be `Some`, so every hover
/// test needs a real file — a bare `editor_from` buffer won't do.
///
/// `configure` scripts any responses beyond `initialize` (e.g.
/// `textDocument/hover`) — it must run *before* the backend is boxed into
/// `LspState`, since `client_and_backend`/`backend_mut` only expose the
/// trait object afterward, which can't reach `InlineLspBackend::respond_to`.
fn setup(
    file_dir: &Path,
    tmp: &Path,
    initialize_result: serde_json::Value,
    configure: impl FnOnce(&mut InlineLspBackend, ServerId),
) -> (Editor, RealRuntimeGuard, ServerId) {
    let guard = RealRuntimeGuard::new();

    let mut ed = Editor::open(None, std::sync::Arc::new(|| {})).unwrap();
    let file = file_dir.join("main.rs");
    // 30 lines — comfortably taller than the default pane height's ⅓-cap
    // (`Pane::new`'s default viewport is 24 rows tall; `(viewport-range bid)`
    // resolves against this immediately, no `prepare_frame` needed), so a
    // one-line hover response lands well under the popup/drawer threshold —
    // a tiny 1-2 line fixture would make even trivial hover content overflow
    // to the drawer, which isn't what these tests are checking.
    let filler = (0..29)
        .map(|i| format!("// line {i}"))
        .collect::<Vec<_>>()
        .join("\n");
    std::fs::write(&file, format!("fn main() {{}}\n{filler}\n")).unwrap();

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

fn popup_lines(ed: &Editor) -> Option<Vec<String>> {
    ed.state
        .popup_view
        .read()
        .unwrap()
        .as_ref()
        .map(|s| s.lines.clone())
}

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

    assert!(
        popup_lines(&ed).is_none(),
        "null hover result must not open a popup"
    );
    let msg = ed.state.status_msg.clone().unwrap_or_default();
    assert!(
        msg.to_lowercase().contains("no hover info"),
        "expected a 'no hover info' message, got {msg:?}"
    );
}

#[test]
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

    assert!(
        popup_lines(&ed).is_none(),
        "a protocol error must not open a popup"
    );
    let msg = ed.state.status_msg.clone().unwrap_or_default();
    assert!(
        msg.to_lowercase().contains("boom"),
        "expected the server's error message surfaced, got {msg:?}"
    );
}

#[test]
fn popup_is_scrollable_and_closes_on_any_key_except_ctrl_u_d() {
    let tmp = safe_tempdir();
    let file_dir = safe_tempdir();
    let (mut ed, _guard, _sid) = setup(
        file_dir.path(),
        tmp.path(),
        serde_json::json!({"capabilities": {"hoverProvider": true}}),
        |backend, _sid| {
            backend.respond_to(
                "textDocument/hover",
                serde_json::json!({"contents": {"kind": "plaintext", "value": "fn main()"}}),
            );
        },
    );

    run_hover(&mut ed);
    let mut ctx = RenderContext::new();
    ed.prepare_frame(80, 25, &mut ctx);
    assert!(popup_lines(&ed).is_some(), "sanity: popup shown");
    assert!(
        matches!(
            ed.state.popup.as_ref().map(|p| &p.dismiss),
            Some(crate::ui::popup::PopupDismiss::KeyExceptScroll)
        ),
        "hover must open a scrollable popup (`#:scroll #t`), not the old mode-change-only one"
    );

    // Ctrl+d/Ctrl+u scroll the popup instead of closing it.
    ed.feed_key(key_ctrl('d'));
    assert!(
        ed.state.popup.is_some(),
        "Ctrl+d must scroll the hover popup, not close it"
    );
    ed.feed_key(key_ctrl('u'));
    assert!(
        ed.state.popup.is_some(),
        "Ctrl+u must scroll the hover popup, not close it"
    );

    // Any other key (here, cursor movement) dismisses it — the fix for
    // hover only closing on a mode change.
    ed.feed_key(key('j'));
    assert!(
        ed.state.popup.is_none(),
        "cursor movement must dismiss the hover popup, not just a mode change"
    );
}

#[test]
fn tall_content_falls_back_to_the_drawer() {
    let tmp = safe_tempdir();
    let file_dir = safe_tempdir();
    // The fixture file is ~30 lines against the default 24-row pane height,
    // so the popup/drawer threshold (⅓ of visible lines) lands around 8 —
    // 20 lines must overflow to the drawer regardless of the exact figure.
    let tall = (0..20)
        .map(|i| format!("line{i}"))
        .collect::<Vec<_>>()
        .join("\n");
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

    assert!(
        popup_lines(&ed).is_none(),
        "tall content must not use the popup"
    );
    assert!(
        ed.state.drawer.is_some(),
        "tall content must fall back to the drawer"
    );
}

#[test]
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
fn allow_stale_is_honored_despite_an_intervening_edit() {
    let tmp = safe_tempdir();
    let file_dir = safe_tempdir();
    let (mut ed, _guard, _sid) = setup(
        file_dir.path(),
        tmp.path(),
        serde_json::json!({"capabilities": {"hoverProvider": true}}),
        |backend, _sid| {
            backend.respond_to(
                "textDocument/hover",
                serde_json::json!({"contents": "fn main()"}),
            );
        },
    );

    type_cmd(&mut ed, ":lsp-hover");
    // Drain the Command-mode entry/exit's on-mode-change hooks now, exactly
    // as the real interactive loop would (after every keystroke) — well
    // before the async response arrives, so lsp-hover's close-on-mode-
    // change dismiss can't replay against a popup that isn't open yet.
    ed.drain_hooks();

    // Bump the buffer's text_gen between send and drain — without
    // #:allow-stale this response would be dropped.
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
