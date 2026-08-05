// Generic LSP bridge: lsp-request, lsp-notify,
// on-lsp-notification, delivered through the queued-Steel-call mechanism.

use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use super::*;
use crate::editor::lsp::LspState;
use hume_lsp::backend::{LspBackend, ServerId};
use hume_lsp::client::{LspClient, ServerState};
use hume_lsp::codec::Message;
use hume_lsp::inline::InlineLspBackend;
use hume_lsp::test_util::{NotificationLog, RecordingLspBackend, RequestLog};
use hume_lsp::transport::InboundEvent;
use hume_scripting::ScriptingHost;

/// Wires a scripted backend with a Running client attached to the focused
/// buffer, registered under language `"rust"` — enough for both `server =
/// #f` (focused buffer) and `server = "rust"` (named) resolution.
pub(super) fn setup_with(
    ed: &mut Editor,
    configure: impl FnOnce(&mut InlineLspBackend, ServerId),
) -> ServerId {
    let mut backend = InlineLspBackend::new();
    let sid = backend
        .start("rust-analyzer", &[], Path::new("."), &[])
        .unwrap();
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

/// Same wiring as `setup_with`, but over `RecordingLspBackend` so outgoing
/// requests/notifications (e.g. `$/cancelRequest`) stay observable after
/// the backend is boxed into `LspState`.
pub(super) fn setup_with_recording(
    ed: &mut Editor,
    configure: impl FnOnce(&mut RecordingLspBackend, ServerId),
) -> (ServerId, NotificationLog, RequestLog) {
    let (mut backend, log, requests) = RecordingLspBackend::new();
    let sid = backend
        .start("rust-analyzer", &[], Path::new("."), &[])
        .unwrap();
    configure(&mut backend, sid);
    ed.lsp = LspState::from_backend_for_test(Box::new(backend));
    let mut client = LspClient::new(sid, PathBuf::from("."));
    client.set_state_for_test(ServerState::Running);
    ed.lsp.insert_client_for_test(client);
    ed.lsp
        .insert_server_key_for_test("rust".to_string(), PathBuf::from("."), sid);
    let bid = ed.focused_buffer_id();
    ed.state.buffers.get_mut(bid).lsp_server = Some(sid);
    (sid, log, requests)
}

// ── #:supersede ──────────────────────────────────────────────────────────────

/// Two `lsp-request` calls with no `#:supersede` key must never cancel each
/// other — both are independent, both fire.
#[test]
fn requests_without_a_supersede_key_do_not_cancel_each_other() {
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
               (lambda (err result) (log! 'trace (string-append "marker-" (hash-ref result "marker")))))
             (lsp-request #f "textDocument/completion" (hash)
               (lambda (err result) (log! 'trace (string-append "marker-" (hash-ref result "marker")))))))"#,
        tmp.path(),
    );
    ed.scripting = Some(host);

    type_cmd(&mut ed, ":test-cmd");
    ed.drain_lsp();
    ed.settle();

    let log = ed.state.message_log.format_for_display();
    assert!(
        log.contains("marker-A"),
        "both callbacks must fire: {log:?}"
    );
    assert!(
        log.contains("marker-B"),
        "both callbacks must fire: {log:?}"
    );
    assert!(
        !notifications
            .borrow()
            .iter()
            .any(|(method, _)| method == "$/cancelRequest"),
        "no supersede key means no cancellation"
    );
}

/// `:lsp-stop` must clear any tracked supersede-key entries for that
/// server, alongside its existing timed-out-callback contract — otherwise a
/// stopped server's stale request id could linger in the map forever.
#[test]
fn lsp_stop_clears_supersede_entries() {
    let tmp = safe_tempdir();
    let mut ed = editor_from("-[a]>bcdef\n");
    setup_with(&mut ed, |_b, _sid| {
        // No canned response — the request stays pending until :lsp-stop.
    });
    let mut host = ScriptingHost::new();
    eval_with_real_host(
        &mut ed,
        &mut host,
        r#"(define-command! "test-cmd" "" (lambda ()
             (lsp-request #f "textDocument/completion" (hash)
               (lambda (err result) (log! 'trace "fired"))
               #:supersede "k")))"#,
        tmp.path(),
    );
    ed.scripting = Some(host);

    type_cmd(&mut ed, ":test-cmd");
    assert_eq!(ed.lsp.supersede_count_for_test(), 1, "sanity: key tracked");

    ed.lsp_stop(Some("rust"));

    assert_eq!(
        ed.lsp.supersede_count_for_test(),
        0,
        "supersede entries for the stopped server must not linger"
    );
}

#[test]
fn response_delivers_decoded_result_to_callback() {
    let tmp = safe_tempdir();
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
    ed.settle();

    assert_ne!(
        state(&ed),
        before,
        "callback must decode the response and move the cursor"
    );
}

#[test]
fn protocol_error_delivers_err_hashmap_to_callback() {
    let tmp = safe_tempdir();
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
    ed.settle();

    assert_ne!(
        state(&ed),
        before,
        "callback must receive the protocol error as a {{code message}} hashmap"
    );
}

#[test]
fn timeout_delivers_err_string_timeout_to_callback() {
    let tmp = safe_tempdir();
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
    ed.settle();

    assert_ne!(
        state(&ed),
        before,
        "callback must receive the string \"timeout\" as err"
    );
}

#[test]
fn on_lsp_notification_fires_the_registered_handler() {
    let tmp = safe_tempdir();
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
    ed.settle();

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
    let tmp = safe_tempdir();
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
    ed.settle();
    let after_first = state(&ed);
    assert_ne!(start, after_first, "first callback must have fired exactly");

    // The second request was only just queued by the first callback — it
    // cannot have been answered (let alone re-entrantly evaluated) within
    // the same drain/eval pass that sent it.
    ed.drain_lsp();
    ed.settle();
    let after_second = state(&ed);
    assert_ne!(
        after_first, after_second,
        "second callback must fire on its own, later drain cycle"
    );
}

#[test]
fn callback_error_lands_in_message_log_not_a_crash() {
    let tmp = safe_tempdir();
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
    ed.settle(); // must not panic

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
pub(super) struct OrderedLogBackend {
    inner: InlineLspBackend,
    log: Rc<RefCell<Vec<String>>>,
}

impl OrderedLogBackend {
    pub(super) fn new() -> (Self, Rc<RefCell<Vec<String>>>) {
        let log = Rc::new(RefCell::new(Vec::new()));
        (
            Self {
                inner: InlineLspBackend::new(),
                log: log.clone(),
            },
            log,
        )
    }

    pub(super) fn respond_to(&mut self, method: &str, result: serde_json::Value) {
        self.inner.respond_to(method, result);
    }
}

impl LspBackend for OrderedLogBackend {
    fn start(
        &mut self,
        cmd: &str,
        args: &[String],
        root: &Path,
        env: &[(String, String)],
    ) -> std::io::Result<ServerId> {
        self.inner.start(cmd, args, root, env)
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

    fn shutdown(&mut self, server: ServerId) {
        self.inner.shutdown(server);
    }
}

#[test]
fn lsp_request_with_unknown_server_reports_an_error_and_fires_callback_with_err() {
    // Regression: a resolution failure must never silently drop the
    // callback — the documented `(err result)` contract (exactly one
    // non-`#f`) must hold even when no request/response pair could ever
    // exist. Before the fix, the callback simply never fired here.
    let tmp = safe_tempdir();
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
    ed.settle();

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
    let tmp = safe_tempdir();
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
    ed.settle();

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
    let tmp = safe_tempdir();
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
