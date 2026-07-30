use super::*;
use std::path::Path;

use super::super::lsp_bridge::{OrderedLogBackend, setup_with, setup_with_recording};
use crate::editor::lsp::LspState;
use hume_lsp::backend::{LspBackend, ServerId};
use hume_lsp::client::{LspClient, ServerState};
use hume_lsp::inline::InlineLspBackend;
use hume_scripting::ScriptingHost;

/// Two `#:supersede "k"` requests queued in the same command dispatch (so
/// both flush in one batch, the first still pending when the second sends)
/// — the second must cancel the first: exactly one `$/cancelRequest` on the
/// wire, the first callback never fires, the second does, and neither the
/// callback nor the supersede-key entry leaks.
#[test]
fn supersede_cancels_the_prior_request_under_the_same_key() {
    let tmp = safe_tempdir();
    let mut ed = editor_from("-[a]>bcdef\n");
    let (_sid, notifications, _requests) = setup_with_recording(&mut ed, |b, _sid| {
        b.respond_to(
            "textDocument/completion",
            serde_json::json!({"marker": "A"}),
        );
        b.respond_to(
            "textDocument/completion",
            serde_json::json!({"marker": "B"}),
        );
    });
    let mut host = ScriptingHost::new();
    eval_with_real_host(
        &mut ed,
        &mut host,
        r#"(define-command! "test-cmd" "" (lambda ()
             (lsp-request #f "textDocument/completion" (hash)
               (lambda (err result) (log! 'trace (string-append "marker-" (hash-ref result "marker"))))
               #:supersede "k")
             (lsp-request #f "textDocument/completion" (hash)
               (lambda (err result) (log! 'trace (string-append "marker-" (hash-ref result "marker"))))
               #:supersede "k")))"#,
        tmp.path(),
    );
    ed.scripting = Some(host);

    type_cmd(&mut ed, ":test-cmd");
    ed.drain_lsp();
    ed.drain_pending_steel_calls();

    let log = ed.state.message_log.format_for_display();
    assert!(
        !log.contains("marker-A"),
        "the superseded request's callback must never fire: {log:?}"
    );
    assert!(
        log.contains("marker-B"),
        "the superseding request's callback must fire: {log:?}"
    );

    let cancels: Vec<_> = notifications
        .borrow()
        .iter()
        .filter(|(method, _)| method == "$/cancelRequest")
        .cloned()
        .collect();
    assert_eq!(
        cancels.len(),
        1,
        "expected exactly one $/cancelRequest, got: {cancels:?}"
    );
    assert_eq!(cancels[0].1, serde_json::json!({"id": 1}));

    assert_eq!(
        ed.lsp.callback_count_for_test(),
        0,
        "the superseded request's callback must not leak"
    );
    assert_eq!(
        ed.lsp.supersede_count_for_test(),
        0,
        "the supersede-key entry must be cleared once its request completes"
    );
}

/// Opens a real file (so `Buffer.path()` is `Some(canonical)`), wires a
/// scripted server, and attaches the newly-opened buffer to it. Returns the
/// buffer id and the `file://` URI a request's `textDocument.uri` must use
/// to hit the staleness check against this buffer.
fn setup_with_real_file(
    ed: &mut Editor,
    file_dir: &Path,
    configure: impl FnOnce(&mut InlineLspBackend, ServerId),
) -> (hume_engine::pipeline::BufferId, String) {
    let file = file_dir.join("main.rs");
    std::fs::write(&file, "abcdef\n").unwrap();
    let sid = setup_with(ed, configure);
    ed.execute_typed("e", Some(file.to_str().unwrap())).unwrap();
    let bid = ed.focused_buffer_id();
    ed.state.buffers.get_mut(bid).lsp_server = Some(sid);
    let canonical = std::fs::canonicalize(&file).unwrap();
    let uri = hume_lsp::uri::path_to_uri(&canonical)
        .unwrap()
        .as_str()
        .to_string();
    (bid, uri)
}

/// Flip oracle for the two staleness tests below: with the same setup but no
/// intervening edit, the callback fires normally (proves the harness itself
/// isn't what's suppressing it).
#[test]
fn callback_fires_normally_without_an_intervening_edit() {
    let tmp = safe_tempdir();
    let file_dir = safe_tempdir();
    let mut ed = editor_from("-[x]>\n");
    let (_bid, uri) = setup_with_real_file(&mut ed, file_dir.path(), |b, _sid| {
        b.respond_to("textDocument/hover", serde_json::json!({"contents": "ok"}));
    });

    let mut host = ScriptingHost::new();
    eval_with_real_host(
        &mut ed,
        &mut host,
        &format!(
            r#"(define-command! "test-cmd" "" (lambda ()
                 (lsp-request #f "textDocument/hover" (hash "textDocument" (hash "uri" "{uri}")) (lambda (err result)
                   (call! "move-right")))))"#
        ),
        tmp.path(),
    );
    ed.scripting = Some(host);

    let before = state(&ed);
    type_cmd(&mut ed, ":test-cmd");
    ed.drain_lsp();
    ed.drain_pending_steel_calls();

    assert_ne!(
        state(&ed),
        before,
        "callback must fire when the buffer never moved on"
    );
}

#[test]
fn stale_response_is_dropped_without_allow_stale() {
    let tmp = safe_tempdir();
    let file_dir = safe_tempdir();
    let mut ed = editor_from("-[x]>\n");
    let (_bid, uri) = setup_with_real_file(&mut ed, file_dir.path(), |b, _sid| {
        b.respond_to("textDocument/hover", serde_json::json!({"contents": "ok"}));
    });

    let mut host = ScriptingHost::new();
    eval_with_real_host(
        &mut ed,
        &mut host,
        &format!(
            r#"(define-command! "test-cmd" "" (lambda ()
                 (lsp-request #f "textDocument/hover" (hash "textDocument" (hash "uri" "{uri}")) (lambda (err result)
                   (call! "move-right")))))"#
        ),
        tmp.path(),
    );
    ed.scripting = Some(host);

    type_cmd(&mut ed, ":test-cmd");
    // Move the buffer's text_gen past what the request was sent against
    // (`:e` left focus on this buffer, so these keys land on it directly).
    ed.feed_key(key('i'));
    ed.feed_key(key('X'));
    ed.feed_key(key_esc());

    let before = state(&ed);
    ed.drain_lsp();
    ed.drain_pending_steel_calls();

    assert_eq!(
        state(&ed),
        before,
        "a stale response (buffer moved on, no #:allow-stale) must be dropped silently"
    );
}

#[test]
fn allow_stale_delivers_despite_buffer_moving_on() {
    let tmp = safe_tempdir();
    let file_dir = safe_tempdir();
    let mut ed = editor_from("-[x]>\n");
    let (_bid, uri) = setup_with_real_file(&mut ed, file_dir.path(), |b, _sid| {
        b.respond_to("textDocument/hover", serde_json::json!({"contents": "ok"}));
    });

    let mut host = ScriptingHost::new();
    eval_with_real_host(
        &mut ed,
        &mut host,
        &format!(
            r#"(define-command! "test-cmd" "" (lambda ()
                 (lsp-request #f "textDocument/hover" (hash "textDocument" (hash "uri" "{uri}")) (lambda (err result)
                   (call! "move-right")) #:allow-stale #t)))"#
        ),
        tmp.path(),
    );
    ed.scripting = Some(host);

    type_cmd(&mut ed, ":test-cmd");
    ed.feed_key(key('i'));
    ed.feed_key(key('X'));
    ed.feed_key(key_esc());

    let before = state(&ed);
    ed.drain_lsp();
    ed.drain_pending_steel_calls();

    assert_ne!(
        state(&ed),
        before,
        "#:allow-stale must opt out of the staleness drop"
    );
}

/// Regression: a Steel command that edits the buffer (queuing an LSP
/// `didChange`) and then immediately fires an `lsp-request` — the same
/// shape as a trigger-char hook firing right after the edit that triggered
/// it — must put the `didChange` on the wire *before* the request. Before
/// the fix, `send_one_lsp_request` sent the request straight away and left
/// the queued edit sitting in `Buffer.lsp_pending` until the next frame's
/// `prepare_frame`, so the request reached the server ahead of the edit it
/// was computed against.
#[test]
fn didchange_reaches_the_wire_before_a_same_dispatch_request() {
    let tmp = safe_tempdir();
    let file_dir = safe_tempdir();
    let file = file_dir.path().join("main.rs");
    std::fs::write(&file, "abcdef\n").unwrap();

    let mut ed = editor_from("-[a]>bcdef\n");
    let (mut raw_backend, log) = OrderedLogBackend::new();
    raw_backend.respond_to("textDocument/hover", serde_json::json!({"contents": "hi"}));
    let sid = raw_backend
        .start("rust-analyzer", &[], Path::new("."), &[])
        .unwrap();
    ed.lsp = LspState::from_backend_for_test(Box::new(raw_backend));
    let mut client = LspClient::new(sid, PathBuf::from("."));
    client.set_state_for_test(ServerState::Running);
    ed.lsp.insert_client_for_test(client);
    ed.lsp
        .insert_server_key_for_test("rust".to_string(), PathBuf::from("."), sid);

    ed.execute_typed("e", Some(file.to_str().unwrap())).unwrap();
    let bid = ed.focused_buffer_id();
    ed.state.buffers.get_mut(bid).lsp_server = Some(sid);

    let mut host = ScriptingHost::new();
    eval_with_real_host(
        &mut ed,
        &mut host,
        r#"(define-command! "test-cmd" "" (lambda ()
             (apply-text-edits! (current-buffer) (list (list (cons 0 0) (cons 0 0) "Z")))
             (lsp-request #f "textDocument/hover" (hash) (lambda (err result) (begin)))))"#,
        tmp.path(),
    );
    ed.scripting = Some(host);

    type_cmd(&mut ed, ":test-cmd");

    let methods = log.borrow();
    assert_eq!(
        methods.as_slice(),
        ["textDocument/didChange", "textDocument/hover"],
        "the queued edit's didChange must reach the wire before the request \
         fired in the same dispatch, got: {methods:?}"
    );
}
