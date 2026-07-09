// The full completion flow: trigger (Ctrl+Space + server trigger chars) ->
// textDocument/completion -> completion-begin!; on-completion-accept applies
// additionalTextEdits or resolves; on-completion-refilter re-requests while
// isIncomplete. Named lsp_completion_feature.rs (not lsp_completion.rs — that
// file already covers the completion-begin!/update-filter!/top/
// accept!/dismiss! orchestration directly; this file drives the same
// primitives through the real shipped plugin and a real LSP round trip).
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
    std::fs::write(&file, "foo\n").unwrap();
    file
}

/// Same plugin-before-handshake ordering as the signature-help setup: `on-lsp-attach`'s
/// handler (registers trigger chars) must already be installed when the
/// `Running` transition fires it, once, at attach time.
#[cfg(not(windows))]
fn setup(
    file: &Path,
    tmp: &Path,
    capabilities: serde_json::Value,
    configure: impl FnOnce(&mut RecordingLspBackend, ServerId),
) -> (Editor, RealRuntimeGuard, RequestLog) {
    let guard = RealRuntimeGuard::new();

    let (mut backend, _notifications, requests) = RecordingLspBackend::new();
    backend.respond_to(
        "initialize",
        serde_json::json!({"capabilities": capabilities}),
    );
    let sid = backend.start("rust-analyzer", &[], Path::new(".")).unwrap();
    configure(&mut backend, sid);

    let mut ed = Editor::open(None).unwrap();
    ed.lsp = LspState::from_backend_for_test(Box::new(backend));

    let mut host = ScriptingHost::new();
    eval_with_real_host(&mut ed, &mut host, r#"(load-plugin "core:lsp")"#, tmp);
    ed.scripting = Some(host);

    ed.execute_typed("e", Some(file.to_str().unwrap())).unwrap();
    let bid = ed.focused_buffer_id();
    ed.state.buffers.get_mut(bid).lsp_server = Some(sid);

    let mut client = LspClient::new(sid, PathBuf::from("."));
    client.start_handshake(ed.lsp.backend_mut());
    ed.lsp.insert_client_for_test(client);
    ed.lsp
        .insert_server_key_for_test("rust".to_string(), PathBuf::from("."), sid);

    let (sid2, ev) = ed.lsp.backend_mut().drain().into_iter().next().unwrap();
    let actions = ed.lsp.client_for_test(sid2).unwrap().on_event(ev);
    for action in actions {
        ed.dispatch_lsp_action(sid2, action);
    }
    ed.drain_hooks(); // on-lsp-attach registers trigger chars

    (ed, guard, requests)
}

#[cfg(not(windows))]
fn full_completion_caps() -> serde_json::Value {
    serde_json::json!({
        "completionProvider": {"triggerCharacters": ["."], "resolveProvider": true}
    })
}

#[cfg(not(windows))]
fn settle(ed: &mut Editor) {
    ed.drain_hooks();
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
fn status(ed: &Editor) -> String {
    ed.state.status_msg.clone().unwrap_or_default()
}

#[test]
#[cfg(not(windows))]
fn trigger_char_fires_the_completion_request() {
    let tmp = safe_tempdir();
    let file_dir = safe_tempdir();
    let file = write_fixture_file(file_dir.path());
    let (mut ed, _guard, requests) = setup(
        &file,
        tmp.path(),
        full_completion_caps(),
        |backend, _sid| {
            backend.respond_to("textDocument/completion", serde_json::json!([]));
        },
    );

    ed.feed_key(key('i'));
    ed.drain_hooks();
    ed.feed_key(key('.'));
    settle(&mut ed);

    assert_eq!(request_count(&requests, "textDocument/completion"), 1);
}

#[test]
#[cfg(not(windows))]
fn ctrl_space_fires_completion_trigger() {
    let tmp = safe_tempdir();
    let file_dir = safe_tempdir();
    let file = write_fixture_file(file_dir.path());
    let (mut ed, _guard, requests) = setup(
        &file,
        tmp.path(),
        full_completion_caps(),
        |backend, _sid| {
            backend.respond_to("textDocument/completion", serde_json::json!([]));
        },
    );

    ed.feed_key(key('i'));
    ed.drain_hooks();
    ed.feed_key(key_ctrl(' '));
    settle(&mut ed);

    assert_eq!(request_count(&requests, "textDocument/completion"), 1);
}

#[test]
#[cfg(not(windows))]
fn capability_gated_no_completion_provider_sends_no_request() {
    let tmp = safe_tempdir();
    let file_dir = safe_tempdir();
    let file = write_fixture_file(file_dir.path());
    let (mut ed, _guard, requests) = setup(
        &file,
        tmp.path(),
        serde_json::json!({}),
        |_backend, _sid| {},
    );

    ed.feed_key(key('i'));
    ed.drain_hooks();
    ed.feed_key(key_ctrl(' '));
    settle(&mut ed);

    assert_eq!(request_count(&requests, "textDocument/completion"), 0);
    assert!(status(&ed).to_lowercase().contains("not supported"));
}

#[test]
#[cfg(not(windows))]
fn null_response_opens_no_session() {
    let tmp = safe_tempdir();
    let file_dir = safe_tempdir();
    let file = write_fixture_file(file_dir.path());
    let (mut ed, _guard, _requests) = setup(
        &file,
        tmp.path(),
        full_completion_caps(),
        |backend, _sid| {
            backend.respond_to("textDocument/completion", serde_json::Value::Null);
        },
    );

    ed.feed_key(key('i'));
    ed.drain_hooks();
    ed.feed_key(key_ctrl(' '));
    settle(&mut ed);
    assert!(
        !status(&ed).to_lowercase().contains("error"),
        "a null response must be a clean no-op, not fall through to a type error \
         (json null decodes to Steel void, never #f), got status {:?}",
        status(&ed)
    );
    ed.feed_key(key_esc());

    // Directly exercise the session state the completion orchestration tests
    // already cover: no
    // active session means accept! must error.
    let source = r#"(define-command! "try-accept" "" (lambda () (completion-accept! 0)))"#;
    let mut host = ed.scripting.take().unwrap();
    eval_with_real_host(&mut ed, &mut host, source, tmp.path());
    ed.scripting = Some(host);
    type_cmd(&mut ed, ":try-accept");

    assert!(
        status(&ed)
            .to_lowercase()
            .contains("no active completion session"),
        "a null response must never call completion-begin!, got status {:?}",
        status(&ed)
    );
}

#[test]
#[cfg(not(windows))]
fn accept_applies_main_edit_and_additional_text_edits_as_two_undo_steps() {
    let tmp = safe_tempdir();
    let file_dir = safe_tempdir();
    // Blank line 0 (the auto-import destination) + "foo" on line 1 (the
    // completion site) — non-overlapping, matching the real-world shape:
    // an import lands above the cursor's line, not at the exact same spot.
    let file = file_dir.path().join("main.rs");
    std::fs::write(&file, "\nfoo\n").unwrap();
    let (mut ed, _guard, _requests) = setup(
        &file,
        tmp.path(),
        full_completion_caps(),
        |backend, _sid| {
            backend.respond_to(
            "textDocument/completion",
            serde_json::json!([{
                "label": "bar",
                "insertText": "bar",
                "additionalTextEdits": [
                    {"range": {"start": {"line": 0, "character": 0}, "end": {"line": 0, "character": 0}},
                     "newText": "use std::bar;\n"}
                ]
            }]),
        );
        },
    );
    // Char 1 is the start of "foo" on line 1 (char 0 is line 0's newline).
    let bid = ed.focused_buffer_id();
    let pid = ed.state.focused_pane_id;
    let pbs = ed
        .state
        .panes
        .state
        .get_mut(pid)
        .and_then(|by_buf| by_buf.get_mut(bid))
        .expect("pane buffer state must exist");
    pbs.selections = hume_editing::selection::SelectionSet::single(
        hume_editing::selection::Selection::collapsed(1),
    );

    ed.feed_key(key('i'));
    ed.drain_hooks();
    ed.feed_key(key_ctrl(' '));
    settle(&mut ed);

    // Enter is the real acceptance key (insert.rs's completion-menu
    // intercept) — accepts the currently-selected (default: index 0) item.
    ed.feed_key(key_enter());
    settle(&mut ed);

    assert_eq!(
        ed.doc().text().to_string(),
        "use std::bar;\n\nbarfoo\n",
        "the main edit (insertText at the anchor, char 1) and additionalTextEdits \
         (line 0, above it) must both land"
    );

    ed.feed_key(key_esc()); // no menu left open — a plain Insert-mode exit
    ed.handle_key(key('u'));
    assert_eq!(
        ed.doc().text().to_string(),
        "\nbarfoo\n",
        "additionalTextEdits undo in one step (apply-text-edits! transaction)"
    );
    ed.handle_key(key('u'));
    assert_eq!(
        ed.doc().text().to_string(),
        "\nfoo\n",
        "the main completion edit undoes as its own separate step"
    );
}

#[test]
#[cfg(not(windows))]
fn resolve_sent_only_when_item_lacks_additional_text_edits_and_resolve_provider_present() {
    let tmp = safe_tempdir();
    let file_dir = safe_tempdir();
    let file = write_fixture_file(file_dir.path());
    let (mut ed, _guard, requests) = setup(
        &file,
        tmp.path(),
        full_completion_caps(),
        |backend, _sid| {
            backend.respond_to(
                "textDocument/completion",
                serde_json::json!([{"label": "bar", "insertText": "bar"}]),
            );
            backend.respond_to(
                "completionItem/resolve",
                serde_json::json!({"label": "bar"}),
            );
        },
    );
    ed.feed_key(key('i'));
    ed.drain_hooks();
    ed.feed_key(key_ctrl(' '));
    settle(&mut ed);

    ed.feed_key(key_enter());
    settle(&mut ed);

    assert_eq!(
        request_count(&requests, "completionItem/resolve"),
        1,
        "an item with no additionalTextEdits, with resolveProvider present, must resolve"
    );
}

#[test]
#[cfg(not(windows))]
fn null_resolve_response_is_a_clean_no_op() {
    let tmp = safe_tempdir();
    let file_dir = safe_tempdir();
    let file = write_fixture_file(file_dir.path());
    let (mut ed, _guard, requests) = setup(
        &file,
        tmp.path(),
        full_completion_caps(),
        |backend, _sid| {
            backend.respond_to(
                "textDocument/completion",
                serde_json::json!([{"label": "bar", "insertText": "bar"}]),
            );
            backend.respond_to("completionItem/resolve", serde_json::Value::Null);
        },
    );
    ed.feed_key(key('i'));
    ed.drain_hooks();
    ed.feed_key(key_ctrl(' '));
    settle(&mut ed);

    ed.feed_key(key_enter());
    settle(&mut ed);

    assert_eq!(
        request_count(&requests, "completionItem/resolve"),
        1,
        "sanity: resolve must have been sent"
    );
    assert!(
        !status(&ed).to_lowercase().contains("error"),
        "a null resolve response must be a clean no-op, got status {:?}",
        status(&ed)
    );
}

#[test]
#[cfg(not(windows))]
fn resolve_not_sent_when_the_item_already_has_additional_text_edits() {
    let tmp = safe_tempdir();
    let file_dir = safe_tempdir();
    let file = write_fixture_file(file_dir.path());
    let (mut ed, _guard, requests) = setup(
        &file,
        tmp.path(),
        full_completion_caps(),
        |backend, _sid| {
            backend.respond_to(
                "textDocument/completion",
                serde_json::json!([{
                    "label": "bar",
                    "insertText": "bar",
                    "additionalTextEdits": []
                }]),
            );
            backend.respond_to(
                "completionItem/resolve",
                serde_json::json!({"label": "bar"}),
            );
        },
    );
    ed.feed_key(key('i'));
    ed.drain_hooks();
    ed.feed_key(key_ctrl(' '));
    settle(&mut ed);

    ed.feed_key(key_enter());
    settle(&mut ed);

    assert_eq!(
        ed.doc().text().to_string(),
        "barfoo\n",
        "sanity: accept must actually have run (not a zero-effect pass)"
    );
    assert_eq!(
        request_count(&requests, "completionItem/resolve"),
        0,
        "an item that already carries additionalTextEdits (even empty) must not resolve"
    );
}

#[test]
#[cfg(not(windows))]
fn refilter_on_incomplete_session_re_requests() {
    let tmp = safe_tempdir();
    let file_dir = safe_tempdir();
    let file = write_fixture_file(file_dir.path());
    let (mut ed, _guard, requests) = setup(
        &file,
        tmp.path(),
        full_completion_caps(),
        |backend, _sid| {
            backend.respond_to(
            "textDocument/completion",
            serde_json::json!({"isIncomplete": true, "items": [{"label": "foo", "insertText": "foo"}]}),
        );
            backend.respond_to(
            "textDocument/completion",
            serde_json::json!({"isIncomplete": true, "items": [{"label": "foobar", "insertText": "foobar"}]}),
        );
        },
    );
    ed.feed_key(key('i'));
    ed.drain_hooks();
    ed.feed_key(key_ctrl(' '));
    settle(&mut ed);
    assert_eq!(request_count(&requests, "textDocument/completion"), 1);

    ed.feed_key(key('x'));
    settle(&mut ed);

    assert_eq!(
        request_count(&requests, "textDocument/completion"),
        2,
        "typing while the session is isIncomplete must re-request"
    );
}

#[test]
#[cfg(not(windows))]
fn refilter_on_complete_session_does_not_re_request() {
    let tmp = safe_tempdir();
    let file_dir = safe_tempdir();
    let file = write_fixture_file(file_dir.path());
    let (mut ed, _guard, requests) = setup(
        &file,
        tmp.path(),
        full_completion_caps(),
        |backend, _sid| {
            backend.respond_to(
                "textDocument/completion",
                serde_json::json!([{"label": "foo", "insertText": "foo"}]),
            );
        },
    );
    ed.feed_key(key('i'));
    ed.drain_hooks();
    ed.feed_key(key_ctrl(' '));
    settle(&mut ed);
    assert_eq!(request_count(&requests, "textDocument/completion"), 1);

    ed.feed_key(key('x'));
    settle(&mut ed);

    assert_eq!(
        request_count(&requests, "textDocument/completion"),
        1,
        "a complete (non-isIncomplete) session must not re-request on further typing"
    );
}

/// Detach regression: `*completion-chars*`/`"lsp-completion"`'s trigger-char
/// registration is global, set once at attach and (before this fix) never
/// cleared — a trigger char left registered past `:lsp-stop` would still
/// reach `lsp/guard-capability`, which resolves the focused buffer's own
/// (now-detached) server and logs "not supported by server" on every
/// matching keystroke. `on-lsp-detach` clears the registration, so the char
/// no longer matches at all — a true no-op, not a per-keystroke log.
#[test]
#[cfg(not(windows))]
fn detach_clears_completion_trigger_chars_so_a_stale_trigger_is_a_true_no_op() {
    let tmp = safe_tempdir();
    let file_dir = safe_tempdir();
    let file = write_fixture_file(file_dir.path());
    let (mut ed, _guard, requests) = setup(
        &file,
        tmp.path(),
        full_completion_caps(),
        |_backend, _sid| {},
    );

    ed.lsp_stop(Some("rust"));
    ed.drain_hooks(); // on-lsp-detach clears *completion-chars*

    ed.feed_key(key('i'));
    ed.drain_hooks();
    let before = ed.state.status_msg.clone();
    ed.feed_key(key('.'));
    settle(&mut ed);

    assert_eq!(request_count(&requests, "textDocument/completion"), 0);
    assert_eq!(
        ed.state.status_msg, before,
        "a trigger char left registered past detach must be a true no-op, not a \
         guard-capability 'not supported' status message every keystroke"
    );
}

/// An open completion session's `items` are a snapshot already fetched from
/// the server, not a live subscription — but leaving it open after the
/// server stops would keep showing (and let the user accept) suggestions
/// from a server that's no longer running for this buffer.
#[test]
#[cfg(not(windows))]
fn detach_dismisses_an_open_completion_session_for_that_buffer() {
    let tmp = safe_tempdir();
    let file_dir = safe_tempdir();
    let file = write_fixture_file(file_dir.path());
    let (mut ed, _guard, _requests) = setup(
        &file,
        tmp.path(),
        full_completion_caps(),
        |backend, _sid| {
            backend.respond_to(
                "textDocument/completion",
                serde_json::json!([{"label": "bar", "insertText": "bar"}]),
            );
        },
    );
    ed.feed_key(key('i'));
    ed.drain_hooks();
    ed.feed_key(key_ctrl(' '));
    settle(&mut ed);
    assert!(
        ed.state.lsp_completion.is_some(),
        "sanity: a session must be open"
    );

    ed.lsp_stop(Some("rust"));

    assert!(
        ed.state.lsp_completion.is_none(),
        "an open completion session for the detached buffer must be dismissed, \
         not left showing stale items from a server that's no longer running"
    );
}

#[test]
#[cfg(not(windows))]
fn snippet_item_lands_as_stripped_plain_text() {
    let tmp = safe_tempdir();
    let file_dir = safe_tempdir();
    let file = write_fixture_file(file_dir.path());
    let (mut ed, _guard, _requests) = setup(
        &file,
        tmp.path(),
        full_completion_caps(),
        |backend, _sid| {
            backend.respond_to(
                "textDocument/completion",
                serde_json::json!([{
                    "label": "for",
                    "insertText": "for ${1:x} in ${2:iter} {\n    $0\n}",
                    "insertTextFormat": 2
                }]),
            );
        },
    );
    ed.feed_key(key('i'));
    ed.drain_hooks();
    ed.feed_key(key_ctrl(' '));
    settle(&mut ed);

    ed.feed_key(key_enter());
    settle(&mut ed);

    assert_eq!(
        ed.doc().text().to_string(),
        "for x in iter {\n    \n}foo\n",
        "snippet placeholders must be stripped to their default text before insertion"
    );
}
