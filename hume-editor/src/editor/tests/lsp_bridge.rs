// Generic LSP bridge: lsp-request, lsp-notify,
// on-lsp-notification, delivered through the queued-Steel-call mechanism.

use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use super::*;
use crate::editor::lsp::LspState;
use crate::editor::scripting_setup::make_init_host;
use hume_lsp::backend::{LspBackend, ServerId};
use hume_lsp::client::{LspClient, ServerState};
use hume_lsp::codec::Message;
use hume_lsp::inline::InlineLspBackend;
use hume_lsp::transport::InboundEvent;
use hume_scripting::ScriptingHost;

/// Wires a scripted backend with a Running client attached to the focused
/// buffer, registered under language `"rust"` — enough for both `server =
/// #f` (focused buffer) and `server = "rust"` (named) resolution.
fn setup_with(
    ed: &mut Editor,
    configure: impl FnOnce(&mut InlineLspBackend, ServerId),
) -> ServerId {
    let mut backend = InlineLspBackend::new();
    let sid = backend.start("rust-analyzer", &[], Path::new(".")).unwrap();
    configure(&mut backend, sid);
    ed.lsp = LspState::from_backend_for_test(Box::new(backend));
    let mut client = LspClient::new(sid, PathBuf::from("."));
    client.set_state_for_test(ServerState::Running);
    ed.lsp.insert_client_for_test(client);
    ed.lsp
        .insert_server_key_for_test("rust".to_string(), PathBuf::from("."), sid);
    let bid = ed.focused_buffer_id();
    ed.state.buffers.get_mut(bid).lsp_server = Some(sid);
    sid
}

/// Evaluates `source` against a *real* editor host (unlike `MockHost`, this
/// makes `define-command!`/`on-lsp-notification` register into the live
/// editor) — same pattern as `lsp_status.rs`'s `eval_register`.
fn eval_with_real_host(ed: &mut Editor, host: &mut ScriptingHost, source: &str, tmp: &Path) {
    let init_path = tmp.join("init.scm");
    std::fs::write(&init_path, source).unwrap();
    let mut ih = make_init_host(&mut ed.state, &mut ed.view);
    host.eval_init(&init_path, 10_000, &mut ih, Default::default())
        .expect("eval_init");
}

#[test]
fn response_delivers_decoded_result_to_callback() {
    let tmp = tempfile::tempdir().unwrap();
    let mut ed = editor_from("-[a]>bcdef\n");
    setup_with(&mut ed, |b, _sid| {
        b.respond_to("textDocument/hover", serde_json::json!({"contents": "hi"}));
    });
    let mut host = ScriptingHost::new();
    eval_with_real_host(
        &mut ed,
        &mut host,
        r#"(define-command! "test-cmd" "" (lambda ()
             (lsp-request #f "textDocument/hover" (hash) (lambda (err result)
               (when (equal? (hash-ref result "contents") "hi")
                 (call! "move-right"))))))"#,
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
        "callback must decode the response and move the cursor"
    );
}

#[test]
fn protocol_error_delivers_err_hashmap_to_callback() {
    let tmp = tempfile::tempdir().unwrap();
    let mut ed = editor_from("-[a]>bcdef\n");
    setup_with(&mut ed, |b, _sid| {
        b.fail_with("textDocument/hover", -32601, "nope");
    });
    let mut host = ScriptingHost::new();
    eval_with_real_host(
        &mut ed,
        &mut host,
        r#"(define-command! "test-cmd" "" (lambda ()
             (lsp-request #f "textDocument/hover" (hash) (lambda (err result)
               (when (and (hash? err) (equal? (hash-ref err "code") -32601))
                 (call! "move-right"))))))"#,
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
        "callback must receive the protocol error as a {{code message}} hashmap"
    );
}

#[test]
fn timeout_delivers_err_string_timeout_to_callback() {
    let tmp = tempfile::tempdir().unwrap();
    let mut ed = editor_from("-[a]>bcdef\n");
    // No canned response for textDocument/hover — it sits pending forever
    // until the (zeroed) deadline scan in `take_completed` claims it.
    setup_with(&mut ed, |_b, _sid| {});
    ed.state.settings.lsp_request_timeout_ms = 0;
    let mut host = ScriptingHost::new();
    eval_with_real_host(
        &mut ed,
        &mut host,
        r#"(define-command! "test-cmd" "" (lambda ()
             (lsp-request #f "textDocument/hover" (hash) (lambda (err result)
               (when (equal? err "timeout")
                 (call! "move-right"))))))"#,
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
        "callback must receive the string \"timeout\" as err"
    );
}

/// Opens a real file (so `Buffer.path()` is `Some(canonical)`), wires a
/// scripted server, and attaches the newly-opened buffer to it. Returns the
/// buffer id and the `file://` URI a request's `textDocument.uri` must use
/// to hit the staleness check against this buffer.
#[cfg(not(windows))]
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
#[cfg(not(windows))]
fn callback_fires_normally_without_an_intervening_edit() {
    let tmp = tempfile::tempdir().unwrap();
    let file_dir = tempfile::tempdir().unwrap();
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
#[cfg(not(windows))]
fn stale_response_is_dropped_without_allow_stale() {
    let tmp = tempfile::tempdir().unwrap();
    let file_dir = tempfile::tempdir().unwrap();
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
#[cfg(not(windows))]
fn allow_stale_delivers_despite_buffer_moving_on() {
    let tmp = tempfile::tempdir().unwrap();
    let file_dir = tempfile::tempdir().unwrap();
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

#[test]
fn on_lsp_notification_fires_the_registered_handler() {
    let tmp = tempfile::tempdir().unwrap();
    let mut ed = editor_from("-[a]>bcdef\n");
    // `push_from_server` (not `send`, which is outbound client->server) is
    // the double's way to simulate a server-initiated notification.
    setup_with(&mut ed, |b, sid| {
        b.push_from_server(
            sid,
            hume_lsp::codec::Message::Notification {
                method: "custom/event".to_string(),
                params: serde_json::json!({"x": 1}),
            },
        );
    });

    let mut host = ScriptingHost::new();
    eval_with_real_host(
        &mut ed,
        &mut host,
        r#"(on-lsp-notification "custom/event" (lambda (server params)
             (when (equal? (hash-ref params "x") 1)
               (call! "move-right"))))"#,
        tmp.path(),
    );
    ed.scripting = Some(host);

    let before = state(&ed);
    ed.drain_lsp();
    ed.drain_pending_steel_calls();

    assert_ne!(
        state(&ed),
        before,
        "registered on-lsp-notification handler must fire with decoded params"
    );
}

#[test]
fn unhandled_notification_without_a_registered_handler_only_logs_trace() {
    let mut ed = editor_from("-[a]>bcdef\n");
    setup_with(&mut ed, |b, sid| {
        b.push_from_server(
            sid,
            hume_lsp::codec::Message::Notification {
                method: "custom/unhandled".to_string(),
                params: serde_json::Value::Null,
            },
        );
    });

    ed.drain_lsp();

    let log = ed.state.message_log.format_for_display();
    assert!(
        log.contains("unhandled notification custom/unhandled"),
        "no handler registered — must fall back to the existing Trace log: {log:?}"
    );
}

/// A callback that itself calls `lsp-request` must not evaluate the second
/// request's callback synchronously within the same Steel session — it only
/// resolves on a later drain cycle, one cursor move per completed cycle.
#[test]
fn callback_calling_lsp_request_does_not_reenter_synchronously() {
    let tmp = tempfile::tempdir().unwrap();
    let mut ed = editor_from("-[a]>bcdefgh\n");
    setup_with(&mut ed, |b, _sid| {
        b.respond_to(
            "textDocument/hover",
            serde_json::json!({"contents": "first"}),
        );
        b.respond_to(
            "textDocument/definition",
            serde_json::json!({"contents": "second"}),
        );
    });
    let mut host = ScriptingHost::new();
    eval_with_real_host(
        &mut ed,
        &mut host,
        r#"(define-command! "test-cmd" "" (lambda ()
             (lsp-request #f "textDocument/hover" (hash) (lambda (err result)
               (call! "move-right")
               (lsp-request #f "textDocument/definition" (hash) (lambda (err2 result2)
                 (call! "move-right")))))))"#,
        tmp.path(),
    );
    ed.scripting = Some(host);

    let start = state(&ed);
    type_cmd(&mut ed, ":test-cmd");
    ed.drain_lsp();
    ed.drain_pending_steel_calls();
    let after_first = state(&ed);
    assert_ne!(start, after_first, "first callback must have fired exactly");

    // The second request was only just queued by the first callback — it
    // cannot have been answered (let alone re-entrantly evaluated) within
    // the same drain/eval pass that sent it.
    ed.drain_lsp();
    ed.drain_pending_steel_calls();
    let after_second = state(&ed);
    assert_ne!(
        after_first, after_second,
        "second callback must fire on its own, later drain cycle"
    );
}

#[test]
fn callback_error_lands_in_message_log_not_a_crash() {
    let tmp = tempfile::tempdir().unwrap();
    let mut ed = editor_from("-[a]>bcdef\n");
    setup_with(&mut ed, |b, _sid| {
        b.respond_to("textDocument/hover", serde_json::json!({"contents": "hi"}));
    });
    let mut host = ScriptingHost::new();
    eval_with_real_host(
        &mut ed,
        &mut host,
        r#"(define-command! "test-cmd" "" (lambda ()
             (lsp-request #f "textDocument/hover" (hash) (lambda (err result)
               (car '())))))"#,
        tmp.path(),
    );
    ed.scripting = Some(host);

    type_cmd(&mut ed, ":test-cmd");
    ed.drain_lsp();
    ed.drain_pending_steel_calls(); // must not panic

    let log = ed.state.message_log.format_for_display();
    assert!(
        log.contains("steel call error"),
        "an erroring callback must be reported, not crash the editor: {log:?}"
    );
}

/// Wraps `InlineLspBackend`, logging the method name of every `send()` call
/// — `Request` and `Notification` alike — into one shared, arrival-ordered
/// log. `RecordingLspBackend` (test_util) keeps requests and notifications
/// in two separate logs, which can't answer "did the didChange reach the
/// wire before this request" ordering bug: only a single combined
/// log can.
struct OrderedLogBackend {
    inner: InlineLspBackend,
    log: Rc<RefCell<Vec<String>>>,
}

impl OrderedLogBackend {
    fn new() -> (Self, Rc<RefCell<Vec<String>>>) {
        let log = Rc::new(RefCell::new(Vec::new()));
        (
            Self {
                inner: InlineLspBackend::new(),
                log: log.clone(),
            },
            log,
        )
    }

    fn respond_to(&mut self, method: &str, result: serde_json::Value) {
        self.inner.respond_to(method, result);
    }
}

impl LspBackend for OrderedLogBackend {
    fn start(&mut self, cmd: &str, args: &[String], root: &Path) -> std::io::Result<ServerId> {
        self.inner.start(cmd, args, root)
    }

    fn send(&mut self, server: ServerId, msg: Message) {
        let method = match &msg {
            Message::Request { method, .. } => method.clone(),
            Message::Notification { method, .. } => method.clone(),
            Message::Response { .. } => "<response>".to_string(),
        };
        self.log.borrow_mut().push(method);
        self.inner.send(server, msg);
    }

    fn drain(&mut self) -> Vec<(ServerId, InboundEvent)> {
        self.inner.drain()
    }

    fn has_pending(&self) -> bool {
        self.inner.has_pending()
    }

    fn shutdown(&mut self, server: ServerId) {
        self.inner.shutdown(server);
    }
}

/// Regression: a Steel command that edits the buffer (queuing an LSP
/// `didChange`) and then immediately fires an `lsp-request` — the same
/// shape as a trigger-char hook firing right after the edit that triggered
/// it — must put the `didChange` on the wire *before* the request. Before
/// the fix, `flush_pending_lsp_calls` sent the request straight away and
/// left the queued edit sitting in `Buffer.lsp_pending` until the next
/// frame's `prepare_frame`, so the request reached the server ahead of the
/// edit it was computed against.
#[test]
#[cfg(not(windows))]
fn didchange_reaches_the_wire_before_a_same_dispatch_request() {
    let tmp = tempfile::tempdir().unwrap();
    let file_dir = tempfile::tempdir().unwrap();
    let file = file_dir.path().join("main.rs");
    std::fs::write(&file, "abcdef\n").unwrap();

    let mut ed = editor_from("-[a]>bcdef\n");
    let (mut raw_backend, log) = OrderedLogBackend::new();
    raw_backend.respond_to("textDocument/hover", serde_json::json!({"contents": "hi"}));
    let sid = raw_backend
        .start("rust-analyzer", &[], Path::new("."))
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
             (apply-text-edits! (current-buffer) (list (list (list 0 0) (list 0 0) "Z")))
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

#[test]
fn lsp_request_with_unknown_server_reports_an_error_and_fires_callback_with_err() {
    // Regression: a resolution failure must never silently drop the
    // callback — the documented `(err result)` contract (exactly one
    // non-`#f`) must hold even when no request/response pair could ever
    // exist. Before the fix, the callback simply never fired here.
    let tmp = tempfile::tempdir().unwrap();
    let mut ed = editor_from("-[a]>bcdef\n");
    setup_with(&mut ed, |_b, _sid| {});
    let mut host = ScriptingHost::new();
    eval_with_real_host(
        &mut ed,
        &mut host,
        r#"(define-command! "test-cmd" "" (lambda ()
             (lsp-request "no-such-language" "textDocument/hover" (hash) (lambda (err result)
               (when (string? err)
                 (call! "move-right"))))))"#,
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
        "callback must fire immediately with a string err when the named server doesn't resolve"
    );
    let log = ed.state.message_log.format_for_display();
    assert!(
        log.contains("lsp-request:"),
        "resolution failure must also be reported: {log:?}"
    );
}

#[test]
fn lsp_request_against_a_crashed_server_fires_callback_with_err() {
    // Same contract as the unknown-server case above, for the other
    // resolve_server failure mode: a server that resolved fine at
    // registration time but has since crashed. Without the fix, a plugin
    // relying on the err branch (e.g. sighelp's popup-close-on-error) would
    // never see it — the request would just sit silently dropped.
    let tmp = tempfile::tempdir().unwrap();
    let mut ed = editor_from("-[a]>bcdef\n");
    let sid = setup_with(&mut ed, |_b, _sid| {});
    ed.lsp
        .client_for_test(sid)
        .unwrap()
        .set_state_for_test(ServerState::Crashed);

    let mut host = ScriptingHost::new();
    eval_with_real_host(
        &mut ed,
        &mut host,
        r#"(define-command! "test-cmd" "" (lambda ()
             (lsp-request #f "textDocument/hover" (hash) (lambda (err result)
               (when (string? err)
                 (call! "move-right"))))))"#,
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
        "callback must fire immediately with a string err when the server has crashed"
    );
}

/// Regression: `(lsp-position-params bid)`/`(lsp-range-params bid)` return
/// `#f` when `bid` has no attached server or isn't shown in any pane, and
/// callers pass that result straight through as `params`. Without a check,
/// `#f` would silently reach the wire as JSON `params: false` instead of
/// erroring at the boundary.
#[test]
fn lsp_request_rejects_false_as_params_instead_of_sending_it_on_the_wire() {
    let tmp = tempfile::tempdir().unwrap();
    let mut ed = editor_from("-[a]>bcdef\n");
    setup_with(&mut ed, |_b, _sid| {});
    let mut host = ScriptingHost::new();
    eval_with_real_host(
        &mut ed,
        &mut host,
        r#"(define-command! "test-cmd" "" (lambda ()
             (lsp-request #f "textDocument/hover" #f (lambda (err result) (begin)))))"#,
        tmp.path(),
    );
    ed.scripting = Some(host);

    type_cmd(&mut ed, ":test-cmd");

    let log = ed.state.message_log.format_for_display();
    assert!(
        log.to_lowercase().contains("boolean"),
        "passing #f as params must error loudly, not silently reach the wire as params: false: {log:?}"
    );
}
