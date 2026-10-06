use super::*;

use super::super::lsp_bridge::{OrderedLogBackend, bridge_initialize_result};

/// Two `#:supersede "k"` requests queued in the same command dispatch (so
/// both flush in one batch, the first still pending when the second sends):
/// the second must cancel the first: exactly one `$/cancelRequest` on the
/// wire, the first callback never fires, the second does, and neither the
/// callback nor its delivery leaks.
#[test]
fn supersede_cancels_the_prior_request_under_the_same_key() {
    let tmp = safe_tempdir();
    let (mut b, _, _) = RecordingLspBackend::new();
    b.respond_to("initialize", bridge_initialize_result());
    b.respond_to(
        "textDocument/completion",
        serde_json::json!({"marker": "A"}),
    );
    b.respond_to(
        "textDocument/completion",
        serde_json::json!({"marker": "B"}),
    );
    let mut rig = LspRig::drained(tmp.path(), RigSpec::rust("-[a]>bcdef\n"), b);
    rig.eval(
        r#"(define-typed-command! "test-cmd" "" (lambda (bid)
             (lsp-request! bid "textDocument/completion" (hash)
               (lambda (err result) (log! 'trace (string-append "marker-" (json-ref result "marker"))))
               #:supersede "k")
             (lsp-request! bid "textDocument/completion" (hash)
               (lambda (err result) (log! 'trace (string-append "marker-" (json-ref result "marker"))))
               #:supersede "k")))"#,
    );
    let sid = rig.sid("rust-analyzer");
    let ed = &mut rig.ed;

    type_cmd(ed, ":test-cmd");
    ed.drain_lsp();
    ed.settle();

    let log = ed.state.message_log.format_for_display();
    assert!(
        !log.contains("marker-A"),
        "the superseded request's callback must never fire: {log:?}"
    );
    assert!(
        log.contains("marker-B"),
        "the superseding request's callback must fire: {log:?}"
    );

    let cancels = rig.sent(sid, "$/cancelRequest");
    // Id 1 is `initialize`, so the superseded request is id 2.
    assert_eq!(
        cancels,
        vec![serde_json::json!({"id": 2})],
        "expected exactly one $/cancelRequest, for the superseded request"
    );

    assert_eq!(
        rig.ed.state.lsp.callback_count_for_test(),
        0,
        "the superseded request's callback must not leak"
    );
    assert_eq!(
        rig.ed.state.lsp.delivery_count_for_test(),
        0,
        "the superseded request's delivery must not leak"
    );
}

/// A rig over a real file (so `Buffer.path()` is `Some(canonical)`) whose
/// running server answers one `textDocument/hover`, with `source(uri)`
/// evaluated in its host: `uri` is the `file://` URI a request's
/// `textDocument.uri` must use to hit the staleness check against the
/// rig's buffer.
fn setup_with_real_file(tmp: &std::path::Path, source: impl FnOnce(&str) -> String) -> Editor {
    let (mut b, _, _) = RecordingLspBackend::new();
    b.respond_to("initialize", bridge_initialize_result());
    b.respond_to("textDocument/hover", serde_json::json!({"contents": "ok"}));
    let mut rig = LspRig::drained(tmp, RigSpec::rust("-[a]>bcdef\n"), b);
    let uri = rig.uri();
    rig.eval(&source(&uri));
    rig.ed
}

/// Same setup as the two staleness tests below, but with no intervening
/// edit: the callback fires normally, so the harness itself isn't what
/// suppresses it there.
#[test]
fn callback_fires_normally_without_an_intervening_edit() {
    let tmp = safe_tempdir();
    let mut ed = setup_with_real_file(tmp.path(), |uri| {
        format!(
            r#"(define-typed-command! "test-cmd" "" (lambda (bid)
                 (lsp-request! bid "textDocument/hover" (hash "textDocument" (hash "uri" "{uri}")) (lambda (err result)
                   (call! "move-right" bid)))))"#
        )
    });

    let before = state(&ed);
    type_cmd(&mut ed, ":test-cmd");
    ed.drain_lsp();
    ed.settle();

    assert_ne!(
        state(&ed),
        before,
        "callback must fire when the buffer never moved on"
    );
}

#[test]
fn stale_response_is_dropped_without_allow_stale() {
    let tmp = safe_tempdir();
    let mut ed = setup_with_real_file(tmp.path(), |uri| {
        format!(
            r#"(define-typed-command! "test-cmd" "" (lambda (bid)
                 (lsp-request! bid "textDocument/hover" (hash "textDocument" (hash "uri" "{uri}")) (lambda (err result)
                   (call! "move-right" bid)))))"#
        )
    });

    type_cmd(&mut ed, ":test-cmd");
    // Move the buffer's generation past what the request was sent against
    // (`:e` left focus on this buffer, so these keys land on it directly).
    ed.feed_key(key('i'));
    ed.feed_key(key('X'));
    ed.feed_key(key_esc());

    let before = state(&ed);
    ed.drain_lsp();
    ed.settle();

    assert_eq!(
        state(&ed),
        before,
        "a stale response (buffer moved on, no #:allow-stale) must be dropped silently"
    );
}

#[test]
fn allow_stale_delivers_despite_buffer_moving_on() {
    let tmp = safe_tempdir();
    let mut ed = setup_with_real_file(tmp.path(), |uri| {
        format!(
            r#"(define-typed-command! "test-cmd" "" (lambda (bid)
                 (lsp-request! bid "textDocument/hover" (hash "textDocument" (hash "uri" "{uri}")) (lambda (err result)
                   (call! "move-right" bid)) #:allow-stale #t)))"#
        )
    });

    type_cmd(&mut ed, ":test-cmd");
    ed.feed_key(key('i'));
    ed.feed_key(key('X'));
    ed.feed_key(key_esc());

    let before = state(&ed);
    ed.drain_lsp();
    ed.settle();

    assert_ne!(
        state(&ed),
        before,
        "#:allow-stale must opt out of the staleness drop"
    );
}

/// Same staleness drop as `stale_response_is_dropped_without_allow_stale`,
/// but with params that carry no `textDocument` at all, proving the check
/// is keyed off the request's own `bid` (mandatory on every `lsp-request!`),
/// not off sniffing `params.textDocument.uri`.
#[test]
fn stale_response_without_text_document_is_dropped() {
    let tmp = safe_tempdir();
    let mut ed = setup_with_real_file(tmp.path(), |_uri| {
        r#"(define-typed-command! "test-cmd" "" (lambda (bid)
             (lsp-request! bid "textDocument/hover" (hash) (lambda (err result)
               (call! "move-right" bid)))))"#
            .to_string()
    });

    type_cmd(&mut ed, ":test-cmd");
    ed.feed_key(key('i'));
    ed.feed_key(key('X'));
    ed.feed_key(key_esc());

    let before = state(&ed);
    ed.drain_lsp();
    ed.settle();

    assert_eq!(
        state(&ed),
        before,
        "params with no textDocument must still be dropped as stale; the \
         check reads the request's bid, not the wire params"
    );
}

/// `#:allow-stale` still opts out with no `textDocument` in params. This is
/// the counterpart of `stale_response_without_text_document_is_dropped`.
#[test]
fn allow_stale_without_text_document_delivers() {
    let tmp = safe_tempdir();
    let mut ed = setup_with_real_file(tmp.path(), |_uri| {
        r#"(define-typed-command! "test-cmd" "" (lambda (bid)
             (lsp-request! bid "textDocument/hover" (hash) (lambda (err result)
               (call! "move-right" bid)) #:allow-stale #t)))"#
            .to_string()
    });

    type_cmd(&mut ed, ":test-cmd");
    ed.feed_key(key('i'));
    ed.feed_key(key('X'));
    ed.feed_key(key_esc());

    let before = state(&ed);
    ed.drain_lsp();
    ed.settle();

    assert_ne!(
        state(&ed),
        before,
        "#:allow-stale must opt out of the staleness drop even with no textDocument in params"
    );
}

/// A Steel command that edits the buffer (queuing an LSP `didChange`) and
/// then immediately fires an `lsp-request!` (the same shape as a
/// trigger-char hook firing right after the edit that triggered it) must
/// put the `didChange` on the wire *before* the request computed against
/// the edited text.
///
/// Opens the file the way `LspRig::open` does, over an `OrderedLogBackend`:
/// only one log holding requests and notifications together can answer the
/// ordering question.
#[test]
fn didchange_reaches_the_wire_before_a_same_dispatch_request() {
    let tmp = safe_tempdir();
    let root = std::fs::canonicalize(tmp.path()).unwrap();
    std::fs::write(root.join("Cargo.toml"), b"").unwrap();
    let file = root.join("main.rs");
    std::fs::write(&file, "abcdef\n").unwrap();

    let mut ed = editor_from("-[\n]>");
    let (mut raw_backend, log) = OrderedLogBackend::new();
    raw_backend.respond_to(
        "initialize",
        serde_json::json!({ "capabilities": { "textDocumentSync": 2, "hoverProvider": true } }),
    );
    raw_backend.respond_to("textDocument/hover", serde_json::json!({"contents": "hi"}));
    // `apply-text-edits!` only accepts a server-tagged wire edit (via a
    // real response); this canned response is what the `:stash` dispatch
    // below turns into one, ahead of (and logged separately from) the
    // dispatch under test.
    raw_backend.respond_to(
        "test/textEdits",
        serde_json::json!([{"range": {"start": {"line": 0, "character": 0}, "end": {"line": 0, "character": 0}}, "newText": "Z"}]),
    );
    ed.state.lsp = LspState::with_backend(Box::new(raw_backend));
    ed.state
        .config
        .languages
        .register_identity("rust", &["rs"], &[], &[], None)
        .unwrap();

    let mut host = ScriptingHost::new();
    eval_with_real_host(
        &mut ed,
        &mut host,
        &format!(
            r#"{RUST_ANALYZER}
           (define stashed-edits (box #f))
           (define-typed-command! "stash" "" (lambda (bid)
             (lsp-request! bid "test/textEdits" (hash) (lambda (err res) (set-box! stashed-edits res)))))
           (define-typed-command! "test-cmd" "" (lambda (bid)
             (apply-text-edits! bid (json-list (unbox stashed-edits)))
             (lsp-request! bid "textDocument/hover" (hash) (lambda (err result) (begin)))))"#
        ),
        &root,
    );
    ed.scripting = Some(host);
    ed.execute_typed("e", Some(file.to_str().unwrap())).unwrap();
    ed.drain_lsp();

    // Runs (and drains) as its own dispatch, well before the one under
    // test, so the didChange it sends doesn't pollute the log below.
    type_cmd(&mut ed, ":stash");
    ed.drain_lsp();
    ed.settle();
    log.borrow_mut().clear();

    type_cmd(&mut ed, ":test-cmd");

    let methods = log.borrow();
    assert_eq!(
        methods.as_slice(),
        ["textDocument/didChange", "textDocument/hover"],
        "the queued edit's didChange must reach the wire before the request \
         fired in the same dispatch, got: {methods:?}"
    );
}
