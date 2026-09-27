// Generic LSP bridge: lsp-request, lsp-notify,
// on-lsp-notification, delivered through the queued-Steel-call mechanism.

#[cfg(unix)]
use std::cell::RefCell;
use std::path::{Path, PathBuf};
#[cfg(unix)]
use std::rc::Rc;

use super::*;
use crate::editor::lsp::LspState;
use hume_lsp::backend::{LspBackend, ServerId};
use hume_lsp::client::{LspClient, ServerState};
use hume_lsp::codec::Message;
use hume_lsp::inline::InlineLspBackend;
use hume_lsp::test_util::{NotificationLog, RecordingLspBackend, RequestLog};
#[cfg(unix)]
use hume_lsp::transport::InboundEvent;
use hume_scripting::ScriptingHost;

/// Wires a scripted backend with a Running client attached to the focused
/// buffer's `lsp_server` and registered under language `"rust"`, enough
/// for both a `bid`-resolved lookup and a `"rust"`-named one.
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
/// other: both are independent, both fire.
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
        r#"(define-typed-command! "test-cmd" "" (lambda (bid)
             (lsp-request bid "textDocument/completion" (hash)
               (lambda (err result) (log! 'trace (string-append "marker-" (json-ref result "marker")))))
             (lsp-request bid "textDocument/completion" (hash)
               (lambda (err result) (log! 'trace (string-append "marker-" (json-ref result "marker")))))))"#,
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
/// server, alongside its existing timed-out-callback contract. Otherwise a
/// stopped server's stale request id could linger in the map forever.
#[test]
fn lsp_stop_clears_supersede_entries() {
    let tmp = safe_tempdir();
    let mut ed = editor_from("-[a]>bcdef\n");
    setup_with(&mut ed, |_b, _sid| {
        // No canned response: the request stays pending until :lsp-stop.
    });
    let mut host = ScriptingHost::new();
    eval_with_real_host(
        &mut ed,
        &mut host,
        r#"(define-typed-command! "test-cmd" "" (lambda (bid)
             (lsp-request bid "textDocument/completion" (hash)
               (lambda (err result) (log! 'trace "fired"))
               #:supersede "k")))"#,
        tmp.path(),
    );
    ed.scripting = Some(host);

    type_cmd(&mut ed, ":test-cmd");
    assert_eq!(ed.lsp.supersede_count_for_test(), 1, "sanity: key tracked");

    ed.lsp_stop(&hume_scripting::LspServerTarget::Language(
        "rust".to_string(),
    ));

    assert_eq!(
        ed.lsp.supersede_count_for_test(),
        0,
        "supersede entries for the stopped server must not linger"
    );
}

#[test]
fn response_delivers_a_handle_to_callback() {
    let tmp = safe_tempdir();
    let mut ed = editor_from("-[a]>bcdef\n");
    setup_with(&mut ed, |b, _sid| {
        b.respond_to("textDocument/hover", serde_json::json!({"contents": "hi"}));
    });
    let mut host = ScriptingHost::new();
    eval_with_real_host(
        &mut ed,
        &mut host,
        r#"(define-typed-command! "test-cmd" "" (lambda (bid)
             (lsp-request bid "textDocument/hover" (hash) (lambda (err result)
               (when (equal? (json-ref result "contents") "hi")
                 (call! "move-right" bid))))))"#,
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

/// Every `lsp-request` response crosses as an opaque handle, not a
/// hashmap, for a real (non-null) response.
#[test]
fn request_delivers_an_opaque_handle_not_a_hashmap() {
    let tmp = safe_tempdir();
    let mut ed = editor_from("-[a]>bcdef\n");
    setup_with(&mut ed, |b, _sid| {
        b.respond_to(
            "textDocument/completion",
            serde_json::json!({"items": [], "isIncomplete": false}),
        );
    });
    let mut host = ScriptingHost::new();
    eval_with_real_host(
        &mut ed,
        &mut host,
        r#"(define-typed-command! "test-cmd" "" (lambda (bid)
             (lsp-request bid "textDocument/completion" (hash) (lambda (err result)
               (when (not (hash? result))
                 (call! "move-right" bid))))))"#,
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
        "a response must not decode into a hashmap"
    );
}

/// A `null` response still crosses as `Void`: the `(void? result)` check
/// every LSP feature already uses to detect a server declining with no
/// data stays meaningful.
#[test]
fn request_with_a_null_response_still_gives_void() {
    let tmp = safe_tempdir();
    let mut ed = editor_from("-[a]>bcdef\n");
    setup_with(&mut ed, |b, _sid| {
        b.respond_to("textDocument/completion", serde_json::Value::Null);
    });
    let mut host = ScriptingHost::new();
    eval_with_real_host(
        &mut ed,
        &mut host,
        r#"(define-typed-command! "test-cmd" "" (lambda (bid)
             (lsp-request bid "textDocument/completion" (hash) (lambda (err result)
               (when (void? result)
                 (call! "move-right" bid))))))"#,
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
        "a null response must still cross as Void"
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
        r#"(define-typed-command! "test-cmd" "" (lambda (bid)
             (lsp-request bid "textDocument/hover" (hash) (lambda (err result)
               (when (and (hash? err) (equal? (hash-ref err "code") -32601))
                 (call! "move-right" bid))))))"#,
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
    // No canned response for textDocument/hover, so it sits pending forever
    // until the (zeroed) deadline scan in `take_completed` claims it.
    setup_with(&mut ed, |_b, _sid| {});
    ed.state.settings.lsp_request_timeout_ms = 0;
    let mut host = ScriptingHost::new();
    eval_with_real_host(
        &mut ed,
        &mut host,
        r#"(define-typed-command! "test-cmd" "" (lambda (bid)
             (lsp-request bid "textDocument/hover" (hash) (lambda (err result)
               (when (equal? err "timeout")
                 (call! "move-right" bid))))))"#,
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
             (when (equal? (json-ref params "x") 1)
               (call! "move-right" (focused-pane)))))"#,
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
        "no handler registered; must fall back to the existing Trace log: {log:?}"
    );
}

/// A callback that itself calls `lsp-request` must not evaluate the second
/// request's callback synchronously within the same Steel session. It only
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
        r#"(define-typed-command! "test-cmd" "" (lambda (bid)
             (lsp-request bid "textDocument/hover" (hash) (lambda (err result)
               (call! "move-right" bid)
               (lsp-request bid "textDocument/definition" (hash) (lambda (err2 result2)
                 (call! "move-right" bid)))))))"#,
        tmp.path(),
    );
    ed.scripting = Some(host);

    let start = state(&ed);
    type_cmd(&mut ed, ":test-cmd");
    ed.drain_lsp();
    ed.settle();
    let after_first = state(&ed);
    assert_ne!(start, after_first, "first callback must have fired exactly");

    // The second request was only just queued by the first callback: it
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
        r#"(define-typed-command! "test-cmd" "" (lambda (bid)
             (lsp-request bid "textDocument/hover" (hash) (lambda (err result)
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
/// (`Request` and `Notification` alike) into one shared, arrival-ordered
/// log. `RecordingLspBackend` (test_util) keeps requests and notifications
/// in two separate logs, which can't answer the ordering question "did the
/// didChange reach the wire before this request": only a single combined
/// log can.
#[cfg(unix)]
pub(super) struct OrderedLogBackend {
    inner: InlineLspBackend,
    log: Rc<RefCell<Vec<String>>>,
}

#[cfg(unix)]
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

#[cfg(unix)]
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
fn lsp_request_with_no_attached_server_reports_an_error_and_fires_callback_with_err() {
    // Regression: a resolution failure must never silently drop the
    // callback: the documented `(err result)` contract (exactly one
    // non-`#f`) must hold even when no request/response pair could ever
    // exist.
    let tmp = safe_tempdir();
    let mut ed = editor_from("-[a]>bcdef\n");
    setup_with(&mut ed, |_b, _sid| {});
    // `setup_with` attaches the server to the focused buffer unconditionally.
    // Detach it again so `bid` genuinely resolves to no server,
    // reproducing the resolution-failure path this test targets.
    let focused = ed.focused_buffer_id();
    ed.state.buffers.get_mut(focused).lsp_server = None;
    let mut host = ScriptingHost::new();
    eval_with_real_host(
        &mut ed,
        &mut host,
        r#"(define-typed-command! "test-cmd" "" (lambda (bid)
             (lsp-request bid "textDocument/hover" (hash) (lambda (err result)
               (when (string? err)
                 (call! "move-right" bid))))))"#,
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
        "callback must fire immediately with a string err when the buffer has no attached server"
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
    // registration time but has since crashed. A plugin relying on the err
    // branch (e.g. sighelp's popup-close-on-error) must still see it.
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
        r#"(define-typed-command! "test-cmd" "" (lambda (bid)
             (lsp-request bid "textDocument/hover" (hash) (lambda (err result)
               (when (string? err)
                 (call! "move-right" bid))))))"#,
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

/// Regression: `(lsp-position-params bid)`/`(lsp-primary-range-params bid)`
/// return `#f` when `bid` has no attached server or isn't shown in any pane, and
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
        r#"(define-typed-command! "test-cmd" "" (lambda (bid)
             (lsp-request bid "textDocument/hover" #f (lambda (err result) (begin)))))"#,
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

// ── #:require-focus ──────────────────────────────────────────────────────────

/// `#:require-focus #t` drops the callback if the focused buffer has moved
/// on by the time the response arrives: the one Rust-side check every
/// cursor-anchored async opener (hover, signature help, a code-action menu)
/// shares, rather than each plugin re-implementing its own
/// `(equal? bid (focused-pane))` guard.
#[test]
fn require_focus_drops_the_callback_after_a_buffer_switch() {
    let tmp = safe_tempdir();
    let file_dir = safe_tempdir();
    let mut ed = editor_from("-[a]>bcdef\n");
    setup_with(&mut ed, |b, _sid| {
        b.respond_to("textDocument/hover", serde_json::json!({"contents": "hi"}));
    });
    let mut host = ScriptingHost::new();
    eval_with_real_host(
        &mut ed,
        &mut host,
        r#"(define-typed-command! "test-cmd" "" (lambda (bid)
             (lsp-request bid "textDocument/hover" (hash) (lambda (err result) (call! "move-right" bid))
               #:require-focus #t)))"#,
        tmp.path(),
    );
    ed.scripting = Some(host);

    type_cmd(&mut ed, ":test-cmd");

    // Switch focus before draining. No settle() until after, since settle()
    // unconditionally drains LSP and would deliver the response first.
    let other = file_dir.path().join("other.txt");
    std::fs::write(&other, "abc\n").unwrap();
    ed.execute_typed("e", Some(other.to_str().unwrap()))
        .unwrap();
    let after_switch = state(&ed);

    ed.drain_lsp();
    ed.settle();

    assert_eq!(
        state(&ed),
        after_switch,
        "the callback must not have fired: the switched-to buffer's own state is untouched"
    );
}

/// `#:require-focus #t` must drop the callback when focus moves to a
/// *different pane still showing the same buffer*, as well as on a buffer
/// switch. A split that moves focus off the requesting pane (onto a sibling
/// pane showing the very same buffer) must not deliver the response into
/// whichever pane happens to be focused.
///
/// Comparing `self.focused_buffer_id()` against `anchor.bid` in
/// `anchor_admits` would miss this, since the split changes only the pane.
#[test]
fn require_focus_drops_the_callback_after_a_pane_split_on_the_same_buffer() {
    use hume_scripting::host::CommandHost;

    let tmp = safe_tempdir();
    let mut ed = editor_from("-[a]>bcdef\n");
    setup_with(&mut ed, |b, _sid| {
        b.respond_to("textDocument/hover", serde_json::json!({"contents": "hi"}));
    });
    let mut host = ScriptingHost::new();
    eval_with_real_host(
        &mut ed,
        &mut host,
        r#"(define-typed-command! "test-cmd" "" (lambda (bid)
             (lsp-request bid "textDocument/hover" (hash) (lambda (err result) (log! 'trace "callback-fired"))
               #:require-focus #t)))"#,
        tmp.path(),
    );
    ed.scripting = Some(host);

    type_cmd(&mut ed, ":test-cmd");

    // Split the focused (requesting) pane. `pane-vsplit` focuses the new
    // pane, so the requesting pane is no longer focused even though both
    // panes show the same buffer.
    let pane = focused_pane(&ed);
    {
        let mut host = live_host!(ed);
        host.run_command_sync("pane-vsplit", pane, Some(1), false, None)
            .expect("pane-vsplit must succeed");
    }

    ed.drain_lsp();
    ed.settle();

    let log = ed.state.message_log.format_for_display();
    assert!(
        !log.contains("callback-fired"),
        "the callback must not fire once focus moved to a sibling pane on the same buffer: {log:?}"
    );
}

/// Without `#:require-focus` (the default), the callback still fires after
/// a buffer switch: every background request (formatting, rename,
/// completion, diagnostics) must keep delivering regardless of focus.
///
/// Proved via `log!`, not a native `call!`: a native command now requires
/// its `bid` to be the *focused* buffer (`run_command_sync`'s own contract),
/// so a callback that ran `(call! "move-right" bid)` against the requesting
/// buffer after focus moved elsewhere would correctly error instead of
/// moving anything, which would prove the wrong thing here. `log!` has no
/// such buffer-targeting constraint, so it isolates "did the callback fire
/// at all" from "which buffer can a native command act on".
#[test]
fn no_require_focus_still_delivers_after_a_buffer_switch() {
    let tmp = safe_tempdir();
    let file_dir = safe_tempdir();
    let mut ed = editor_from("-[a]>bcdef\n");
    setup_with(&mut ed, |b, _sid| {
        b.respond_to("textDocument/hover", serde_json::json!({"contents": "hi"}));
    });
    let mut host = ScriptingHost::new();
    eval_with_real_host(
        &mut ed,
        &mut host,
        r#"(define-typed-command! "test-cmd" "" (lambda (bid)
             (lsp-request bid "textDocument/hover" (hash) (lambda (err result) (log! 'trace "callback-fired")))))"#,
        tmp.path(),
    );
    ed.scripting = Some(host);

    type_cmd(&mut ed, ":test-cmd");

    let other = file_dir.path().join("other.txt");
    std::fs::write(&other, "abc\n").unwrap();
    ed.execute_typed("e", Some(other.to_str().unwrap()))
        .unwrap();

    ed.drain_lsp();
    ed.settle();

    let log = ed.state.message_log.format_for_display();
    assert!(
        log.contains("callback-fired"),
        "the callback must still fire, even though focus moved: {log:?}"
    );
}

/// Two responses land in the same LSP drain, so both are
/// admitted by `dispatch_completed`'s drain-time `anchor_admits` check:
/// neither callback has run yet, so focus hasn't moved. Only once they're
/// dequeued does callback 1 actually execute and switch focus away.
/// Callback 2 (`#:require-focus #t`) must be re-checked *at that point*, not
/// just once back at drain time, or it fires over the wrong buffer anyway.
///
/// Without `Editor::run_pending_batch`'s per-call re-check, callback 2
/// would run once dequeued and "b-fired" would land in the message log
/// despite the switch.
#[test]
fn queued_callback_reanchors_against_an_earlier_sibling_in_the_same_batch() {
    let tmp = safe_tempdir();
    let file_dir = safe_tempdir();
    let mut ed = editor_from("-[a]>bcdef\n");
    setup_with(&mut ed, |b, _sid| {
        b.respond_to("textDocument/hover", serde_json::json!({"contents": "one"}));
        b.respond_to(
            "textDocument/completion",
            serde_json::json!({"contents": "two"}),
        );
    });

    let other = file_dir.path().join("other.txt");
    std::fs::write(&other, "abc\n").unwrap();
    let other_path = steel_path(&other);

    let mut host = ScriptingHost::new();
    eval_with_real_host(
        &mut ed,
        &mut host,
        &format!(
            r#"(define-typed-command! "test-cmd" "" (lambda (bid)
                 (lsp-request bid "textDocument/hover" (hash)
                   (lambda (err result) (switch-to-buffer! bid (open-buffer! {other_path}))))
                 (lsp-request bid "textDocument/completion" (hash)
                   (lambda (err result) (log! 'trace "b-fired"))
                   #:require-focus #t)))"#
        ),
        tmp.path(),
    );
    ed.scripting = Some(host);

    type_cmd(&mut ed, ":test-cmd");
    // Both responses land here, both admitted at drain time, since focus
    // is still on the requesting buffer and neither callback has run.
    ed.drain_lsp();
    // Callback 1 runs first (queued first), switches focus; callback 2's
    // re-check then sees the moved focus and must drop it.
    ed.settle();

    let log = ed.state.message_log.format_for_display();
    assert!(
        !log.contains("b-fired"),
        "callback 2 must be dropped once callback 1's switch moves focus away: {log:?}"
    );
}

/// The staleness counterpart of the test above: callback 1 edits `bid` (bumping its
/// `text_gen`), callback 2 has no `#:allow-stale`. Both land in the same
/// drain, both admitted at drain time (neither has run, so `bid`'s
/// `text_gen` still matches both anchors). Only a re-check at dequeue,
/// after callback 1's edit has actually landed, catches the staleness.
#[test]
fn queued_callback_restales_against_an_earlier_siblings_edit_in_the_same_batch() {
    let tmp = safe_tempdir();
    let mut ed = editor_from("-[a]>bcdef\n");
    setup_with(&mut ed, |b, _sid| {
        b.respond_to(
            "test/edit",
            serde_json::json!([{
                "range": {"start": {"line": 0, "character": 0}, "end": {"line": 0, "character": 1}},
                "newText": "X",
            }]),
        );
        b.respond_to(
            "textDocument/completion",
            serde_json::json!({"contents": "hi"}),
        );
    });

    let mut host = ScriptingHost::new();
    eval_with_real_host(
        &mut ed,
        &mut host,
        r#"(define-typed-command! "test-cmd" "" (lambda (bid)
             (lsp-request bid "test/edit" (hash)
               (lambda (err res) (apply-text-edits! bid (json-list res))))
             (lsp-request bid "textDocument/completion" (hash)
               (lambda (err result) (log! 'trace "b-fired")))))"#,
        tmp.path(),
    );
    ed.scripting = Some(host);

    type_cmd(&mut ed, ":test-cmd");
    ed.drain_lsp();
    ed.settle();

    let log = ed.state.message_log.format_for_display();
    assert!(
        !log.contains("b-fired"),
        "callback 2 must be dropped once callback 1's edit bumps text_gen: {log:?}"
    );
}
