// Code actions: `lsp-code-actions`, composing `lsp-request!`,
// `lsp-capabilities`, `diagnostics-for-buffer`'s `raw`
// field (echoed back as context.diagnostics; servers gate diagnostic-
// derived quickfixes on this), `apply-workspace-edit!`, `show-menu!`.
// Loads the real shipped `core:lsp` plugin in place (`RealRuntimeGuard`).
//
// Not on Windows: Scheme require strings embed OS paths; backslashes are not
// escaped in Steel string literals (same constraint as tests/plugins.rs).

use std::path::Path;

use super::*;
use hume_lsp::codec::{Message, RequestId};
use hume_lsp::test_util::{RecordingLspBackend, RequestLog};

/// Every test's buffer content unless a test needs a different one.
const FIXTURE: &str = "fn main() {\n    let x = 1;\n}\n";

fn setup(
    tmp: &Path,
    configure: impl FnOnce(&mut RecordingLspBackend, ServerId),
) -> (Editor, RealRuntimeGuard, ServerId, RequestLog) {
    setup_with_capabilities(
        tmp,
        serde_json::json!({"codeActionProvider": true}),
        configure,
    )
}

/// Like `setup`, but with caller-chosen `initialize` capabilities, needed
/// for the resolve tests, which require `codeActionProvider` to be the
/// CodeActionOptions hash shape (`{"resolveProvider": true}`), not the bare
/// boolean `setup`'s default uses.
fn setup_with_capabilities(
    tmp: &Path,
    capabilities: serde_json::Value,
    configure: impl FnOnce(&mut RecordingLspBackend, ServerId),
) -> (Editor, RealRuntimeGuard, ServerId, RequestLog) {
    setup_over(tmp, FIXTURE, capabilities, configure)
}

/// A [`core_lsp_rig`] over `content` (cursor at its start).
fn setup_over(
    tmp: &Path,
    content: &str,
    capabilities: serde_json::Value,
    configure: impl FnOnce(&mut RecordingLspBackend, ServerId),
) -> (Editor, RealRuntimeGuard, ServerId, RequestLog) {
    let (rig, guard) = core_lsp_rig(
        tmp,
        &marked_at_start(content),
        serde_json::json!({"capabilities": capabilities}),
        configure,
    );
    let sid = rig.sid("rust-analyzer");
    (rig.ed, guard, sid, rig.requests)
}

fn run_actions(ed: &mut Editor) {
    // lsp-code-actions is key-bindable, not typed: dispatch through the
    // keymap pipeline, the way its bound key (`z a`) would.
    ed.execute_keymap_command("lsp-code-actions".into(), Some(1), false);
    ed.settle();
    ed.drain_lsp();
    ed.settle();
}

fn menu_items(ed: &Editor) -> Vec<String> {
    ed.state
        .input
        .menu()
        .map(|m| m.rows.iter().map(|r| r.main.to_string()).collect())
        .unwrap_or_default()
}

fn last_request_params(requests: &RequestLog, method: &str) -> serde_json::Value {
    requests
        .borrow()
        .iter()
        .rev()
        .find(|(_sid, m, _params)| m == method)
        .map(|(_sid, _m, params)| params.clone())
        .unwrap_or_else(|| panic!("no {method} request was sent"))
}

fn edit_action(title: &str, uri: &str) -> serde_json::Value {
    serde_json::json!({
        "title": title,
        "edit": {"changes": {uri: [
            {"range": {"start": {"line": 1, "character": 12}, "end": {"line": 1, "character": 13}}, "newText": "2"}
        ]}}
    })
}

fn command_action(title: &str) -> serde_json::Value {
    serde_json::json!({
        "title": title,
        "command": {"title": title, "command": "smoke.doThing", "arguments": []}
    })
}

fn disabled_action(title: &str) -> serde_json::Value {
    serde_json::json!({"title": title, "disabled": {"reason": "not applicable here"}})
}

/// A lazily-resolved CodeAction: neither "edit" nor "command", per spec;
/// the client must send `codeAction/resolve` to get either.
fn unresolved_action(title: &str) -> serde_json::Value {
    serde_json::json!({"title": title})
}

fn diagnostic_params(uri: &str) -> serde_json::Value {
    serde_json::json!({"uri": uri, "diagnostics": [
        {"range": {"start": {"line": 0, "character": 0}, "end": {"line": 0, "character": 10}},
         "severity": 4, "message": "unused import", "code": "unused_imports"}
    ]})
}

#[test]
fn titles_are_listed_in_the_menu() {
    let tmp = safe_tempdir();
    let uri = rust_rig_uri(tmp.path());
    let (mut ed, _guard, _sid, _requests) = setup(tmp.path(), |backend, _sid| {
        backend.respond_to(
            "textDocument/codeAction",
            serde_json::json!([
                edit_action("Fix the thing", &uri),
                command_action("Run the thing")
            ]),
        );
    });

    run_actions(&mut ed);

    assert_eq!(
        menu_items(&ed),
        vec!["Fix the thing".to_string(), "Run the thing".to_string()]
    );
}

#[test]
fn selecting_an_edit_action_applies_it() {
    let tmp = safe_tempdir();
    let uri = rust_rig_uri(tmp.path());
    let (mut ed, _guard, _sid, _requests) = setup(tmp.path(), |backend, _sid| {
        backend.respond_to(
            "textDocument/codeAction",
            serde_json::json!([edit_action("Fix the thing", &uri)]),
        );
    });

    run_actions(&mut ed);
    ed.handle_key(key_enter());
    ed.settle();

    assert_eq!(
        ed.doc().text().to_string(),
        "fn main() {\n    let x = 2;\n}\n"
    );
}

#[test]
fn selecting_a_command_action_runs_the_full_server_loop() {
    let tmp = safe_tempdir();
    let uri = rust_rig_uri(tmp.path());
    let (mut ed, _guard, sid, _requests) = setup(tmp.path(), |backend, sid| {
        backend.respond_to(
            "textDocument/codeAction",
            serde_json::json!([command_action("Run the thing")]),
        );
        backend.respond_to("workspace/executeCommand", serde_json::Value::Null);
        // Simulate the server's real behavior: after executing the command,
        // it pushes an unsolicited workspace/applyEdit *request* back. That is the
        // full loop this test proves, not just "a request was sent".
        backend.push_from_server(
            sid,
            Message::Request {
                id: RequestId::Int(9000),
                method: "workspace/applyEdit".to_string(),
                params: serde_json::json!({"edit": {"changes": {uri: [
                    {"range": {"start": {"line": 1, "character": 12}, "end": {"line": 1, "character": 13}}, "newText": "99"}
                ]}}}),
            },
        );
    });

    run_actions(&mut ed);
    ed.handle_key(key_enter());
    ed.settle();
    ed.drain_lsp();
    ed.settle();
    let _ = sid;

    assert_eq!(
        ed.doc().text().to_string(),
        "fn main() {\n    let x = 99;\n}\n",
        "the server's follow-up workspace/applyEdit request must land"
    );
}

/// `workspace/executeCommand`'s params carry no `textDocument`, so it has
/// no self-derived buffer id to anchor a staleness check against the way
/// `textDocument/codeAction` or `textDocument/hover` do. It relies on
/// `#:allow-stale #t` (`lsp/exec-command`, `actions.scm`) instead. An edit
/// between the command being sent and its response draining must not
/// suppress this error report.
///
/// Without `#:allow-stale` on `lsp/exec-command`, the bridge's bid-anchored
/// staleness check (applied to every request, `textDocument`-bearing or not)
/// would drop this response and the error would never reach the log.
#[test]
fn a_command_execution_error_is_still_reported_after_an_intervening_edit() {
    let tmp = safe_tempdir();
    let (mut ed, _guard, _sid, _requests) = setup(tmp.path(), |backend, _sid| {
        backend.respond_to(
            "textDocument/codeAction",
            serde_json::json!([command_action("Run the thing")]),
        );
        backend.fail_with("workspace/executeCommand", -32603, "command exploded");
    });

    run_actions(&mut ed);
    ed.handle_key(key_enter());
    ed.settle();
    // Edit the buffer before draining executeCommand's response.
    ed.feed_key(key('i'));
    ed.feed_key(key('X'));
    ed.feed_key(key_esc());
    ed.drain_lsp();
    ed.settle();

    let errors: Vec<String> = ed
        .state
        .message_log
        .entries()
        .filter(|e| e.severity == Severity::Error)
        .map(|e| e.text.clone())
        .collect();
    assert!(
        errors
            .iter()
            .any(|e| e.contains("code action") && e.contains("command exploded")),
        "an executeCommand error must still be reported after an intervening edit, got {errors:?}"
    );
}

#[test]
fn disabled_actions_are_filtered_out() {
    let tmp = safe_tempdir();
    let uri = rust_rig_uri(tmp.path());
    let (mut ed, _guard, _sid, _requests) = setup(tmp.path(), |backend, _sid| {
        backend.respond_to(
            "textDocument/codeAction",
            serde_json::json!([
                disabled_action("Not available"),
                edit_action("Fix the thing", &uri)
            ]),
        );
    });

    run_actions(&mut ed);

    assert_eq!(
        menu_items(&ed),
        vec!["Fix the thing".to_string()],
        "disabled actions must never appear"
    );
}

#[test]
fn empty_response_reports_no_code_actions_and_opens_no_menu() {
    let tmp = safe_tempdir();
    let (mut ed, _guard, _sid, _requests) = setup(tmp.path(), |backend, _sid| {
        backend.respond_to("textDocument/codeAction", serde_json::Value::Null);
    });

    run_actions(&mut ed);

    assert!(ed.state.input.menu().is_none());
    let msg = ed.state.status_msg.clone().unwrap_or_default();
    assert!(
        msg.to_lowercase().contains("no code actions"),
        "expected a no-code-actions message, got {msg:?}"
    );
}

#[test]
fn context_diagnostics_echoes_the_raw_diagnostic_overlapping_the_cursor() {
    let tmp = safe_tempdir();
    let uri = rust_rig_uri(tmp.path());
    let (mut ed, _guard, sid, requests) = setup(tmp.path(), |backend, _sid| {
        backend.respond_to("textDocument/codeAction", serde_json::Value::Null);
    });
    ed.ingest_typed_publish_for_test(
        sid,
        serde_json::from_value(diagnostic_params(&uri)).unwrap(),
    );

    run_actions(&mut ed);

    let params = last_request_params(&requests, "textDocument/codeAction");
    let diags = params["context"]["diagnostics"]
        .as_array()
        .expect("diagnostics must be an array");
    assert_eq!(
        diags.len(),
        1,
        "the cursor-overlapping diagnostic must be echoed back, got: {diags:?}"
    );
    assert_eq!(diags[0]["message"], "unused import");
    assert_eq!(diags[0]["code"], "unused_imports");
    // Distinguishes the raw wire Diagnostic from the diagnostics store's flat
    // char-indexed shape (which also carries "message"/"code" at the top
    // level, so the two prior asserts alone wouldn't catch passing the wrong
    // one): only the raw shape nests its bounds under "range".
    assert_eq!(diags[0]["range"]["start"]["line"], 0);
    assert_eq!(diags[0]["range"]["start"]["character"], 0);
    assert!(
        diags[0].get("start").is_none(),
        "must be the raw wire Diagnostic, not the flat shape"
    );
}

#[test]
fn context_diagnostics_covers_a_selection_over_a_multi_char_cluster() {
    // The cursor sits on "e" + U+0301, one cluster of two chars. A diagnostic
    // on the combining mark alone overlaps what the cursor covers.
    let tmp = safe_tempdir();
    let uri = rust_rig_uri(tmp.path());
    let (mut ed, _guard, sid, requests) = setup_over(
        tmp.path(),
        "e\u{301}x\n",
        serde_json::json!({"codeActionProvider": true}),
        |backend, _sid| {
            backend.respond_to("textDocument/codeAction", serde_json::Value::Null);
        },
    );
    ed.ingest_typed_publish_for_test(
        sid,
        serde_json::from_value(serde_json::json!({"uri": uri, "diagnostics": [
            {"range": {"start": {"line": 0, "character": 1}, "end": {"line": 0, "character": 2}},
             "severity": 4, "message": "on the mark", "code": "mark"}
        ]}))
        .unwrap(),
    );

    run_actions(&mut ed);

    let params = last_request_params(&requests, "textDocument/codeAction");
    let diags = params["context"]["diagnostics"]
        .as_array()
        .expect("diagnostics must be an array");
    assert_eq!(diags.len(), 1, "got: {diags:?}");
    assert_eq!(diags[0]["message"], "on the mark");
}

#[test]
fn selecting_an_unresolved_action_sends_resolve_then_applies_it() {
    let tmp = safe_tempdir();
    let uri = rust_rig_uri(tmp.path());
    let (mut ed, _guard, _sid, requests) = setup_with_capabilities(
        tmp.path(),
        serde_json::json!({"codeActionProvider": {"resolveProvider": true}}),
        |backend, _sid| {
            backend.respond_to(
                "textDocument/codeAction",
                serde_json::json!([unresolved_action("Fix the thing")]),
            );
            backend.respond_to("codeAction/resolve", edit_action("Fix the thing", &uri));
        },
    );

    run_actions(&mut ed);
    ed.handle_key(key_enter());
    ed.settle();
    ed.drain_lsp();
    ed.settle();

    last_request_params(&requests, "codeAction/resolve");
    assert_eq!(
        ed.doc().text().to_string(),
        "fn main() {\n    let x = 2;\n}\n",
        "the resolved action's edit must apply after the resolve round-trip"
    );
}

/// `codeAction/resolve` requests `#:allow-stale #t`, so an edit landing
/// between picking the action and the response draining must not silently
/// drop the resolve, but `apply-workspace-edit!`'s own `#:expect-generation`
/// must then refuse to apply it, rather than doing nothing (a request-side
/// drop) or applying against text that has since moved.
#[test]
fn selecting_an_unresolved_action_reports_a_stale_buffer_after_an_intervening_edit() {
    let tmp = safe_tempdir();
    let uri = rust_rig_uri(tmp.path());
    let (mut ed, _guard, _sid, requests) = setup_with_capabilities(
        tmp.path(),
        serde_json::json!({"codeActionProvider": {"resolveProvider": true}}),
        |backend, _sid| {
            backend.respond_to(
                "textDocument/codeAction",
                serde_json::json!([unresolved_action("Fix the thing")]),
            );
            backend.respond_to("codeAction/resolve", edit_action("Fix the thing", &uri));
        },
    );

    run_actions(&mut ed);
    ed.handle_key(key_enter()); // selects the action, sends codeAction/resolve
    ed.settle();
    // Edit the buffer before draining the resolve response.
    ed.feed_key(key('i'));
    ed.feed_key(key('z'));
    ed.feed_key(key_esc());
    let before_apply = ed.doc().text().to_string();
    ed.drain_lsp();
    ed.settle();

    last_request_params(&requests, "codeAction/resolve");
    assert_eq!(
        ed.doc().text().to_string(),
        before_apply,
        "a stale-generation apply must be refused, leaving only the intervening edit"
    );
    let errors: Vec<String> = ed
        .state
        .message_log
        .entries()
        .filter(|e| e.severity == Severity::Error)
        .map(|e| e.text.clone())
        .collect();
    assert!(
        errors
            .iter()
            .any(|e| e.contains("apply-workspace-edit") && e.contains("changed")),
        "expected a generation-mismatch error from apply-workspace-edit!, got {errors:?}"
    );
}

#[test]
fn selecting_an_unresolved_action_without_resolve_support_reports_it() {
    let tmp = safe_tempdir();
    let (mut ed, _guard, _sid, requests) = setup_with_capabilities(
        tmp.path(),
        serde_json::json!({"codeActionProvider": true}),
        |backend, _sid| {
            backend.respond_to(
                "textDocument/codeAction",
                serde_json::json!([unresolved_action("Fix the thing")]),
            );
        },
    );

    run_actions(&mut ed);
    ed.handle_key(key_enter());
    ed.settle();

    assert!(
        requests
            .borrow()
            .iter()
            .all(|(_sid, m, _params)| m != "codeAction/resolve"),
        "must never send codeAction/resolve when the server didn't advertise it"
    );
    let msg = ed.state.status_msg.clone().unwrap_or_default();
    assert!(
        msg.to_lowercase().contains("no edit or command"),
        "expected an unsupported-action message, got {msg:?}"
    );
}

/// `lsp/run-action`'s resolve branch must not re-enter itself
/// unconditionally on a resolved action. A non-conforming server that
/// resolves an action still lacking both "edit" and "command" would send
/// a *second* `codeAction/resolve` request rather than reporting the
/// action as unsupported: an unbounded round trip for a server that never
/// produces a resolvable shape. `#:resolved?` bounds this to one attempt.
#[test]
fn selecting_an_unresolved_action_whose_resolve_is_still_bare_reports_it_once() {
    let tmp = safe_tempdir();
    let (mut ed, _guard, _sid, requests) = setup_with_capabilities(
        tmp.path(),
        serde_json::json!({"codeActionProvider": {"resolveProvider": true}}),
        |backend, _sid| {
            backend.respond_to(
                "textDocument/codeAction",
                serde_json::json!([unresolved_action("Fix the thing")]),
            );
            // Only one canned response queued (FIFO): a second resolve
            // request sent by an unbounded recursion would go unanswered.
            backend.respond_to("codeAction/resolve", unresolved_action("Fix the thing"));
        },
    );

    run_actions(&mut ed);
    ed.handle_key(key_enter());
    ed.settle();
    ed.drain_lsp();
    ed.settle();

    let resolve_requests = requests
        .borrow()
        .iter()
        .filter(|(_sid, m, _params)| m == "codeAction/resolve")
        .count();
    assert_eq!(
        resolve_requests, 1,
        "a still-bare resolved action must not trigger a second codeAction/resolve"
    );
    let msg = ed.state.status_msg.clone().unwrap_or_default();
    assert!(
        msg.to_lowercase().contains("no edit or command"),
        "expected an unsupported-action message after the single resolve, got {msg:?}"
    );
}

#[test]
fn selecting_an_unresolved_action_whose_resolve_errors_reports_it() {
    let tmp = safe_tempdir();
    let (mut ed, _guard, _sid, _requests) = setup_with_capabilities(
        tmp.path(),
        serde_json::json!({"codeActionProvider": {"resolveProvider": true}}),
        |backend, _sid| {
            backend.respond_to(
                "textDocument/codeAction",
                serde_json::json!([unresolved_action("Fix the thing")]),
            );
            backend.fail_with("codeAction/resolve", -32603, "resolve exploded");
        },
    );

    run_actions(&mut ed);
    ed.handle_key(key_enter());
    ed.settle();
    ed.drain_lsp();
    ed.settle();

    let errors: Vec<String> = ed
        .state
        .message_log
        .entries()
        .filter(|e| e.severity == Severity::Error)
        .map(|e| e.text.clone())
        .collect();
    assert!(
        errors
            .iter()
            .any(|e| e.contains("code action") && e.contains("resolve exploded")),
        "expected a reported codeAction/resolve error, got {errors:?}"
    );
}

/// The user is free to switch buffers while a `textDocument/codeAction`
/// request is in flight: the same async-round-trip race `lsp-hover`'s own
/// `#:require-focus` guards against. A response for a buffer that's no
/// longer focused must not open a menu over whatever the user switched to.
#[test]
fn stale_response_after_a_buffer_switch_opens_no_menu() {
    let tmp = safe_tempdir();
    let uri = rust_rig_uri(tmp.path());
    let (mut ed, _guard, _sid, _requests) = setup(tmp.path(), |backend, _sid| {
        backend.respond_to(
            "textDocument/codeAction",
            serde_json::json!([edit_action("Fix the thing", &uri)]),
        );
    });

    // Sends the request synchronously; no settle() before the
    // switch below. settle() unconditionally drains LSP, which would
    // deliver the response (and close the race window) before the switch
    // ever happens. Same technique as `lsp_hover.rs`'s own
    // `stale_response_after_a_buffer_switch_shows_no_popup`.
    ed.execute_keymap_command("lsp-code-actions".into(), Some(1), false);

    let other = rig_root(tmp.path()).join("other.rs");
    std::fs::write(&other, "\n").unwrap();
    ed.execute_typed("e", Some(other.to_str().unwrap()))
        .unwrap();

    ed.drain_lsp();
    ed.settle();

    assert_eq!(
        menu_items(&ed),
        Vec::<String>::new(),
        "a code-action response for a buffer that's no longer focused must not open a menu"
    );
}
