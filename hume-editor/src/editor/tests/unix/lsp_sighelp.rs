// Signature help: trigger chars fire a debounced
// textDocument/signatureHelp, composing `lsp-request`,
// `lsp-capabilities`, debounce, `on-lsp-attach`, `on-trigger-char`.
// Loads the real shipped `core:lsp` plugin in place (`RealRuntimeGuard`).
//
// Not on Windows: Scheme require strings embed OS paths; backslashes are not
// escaped in Steel string literals (same constraint as tests/plugins.rs).

use std::path::{Path, PathBuf};
use std::time::Duration;

use super::*;
use crate::editor::lsp::LspState;
use hume_editing::selection::{Selection, SelectionSet};
use hume_engine::pipeline::RenderContext;
use hume_lsp::backend::{LspBackend, ServerId};
use hume_lsp::client::LspClient;
use hume_lsp::test_util::{RecordingLspBackend, RequestLog};
use hume_scripting::ScriptingHost;

fn write_fixture_file(file_dir: &Path) -> PathBuf {
    let file = file_dir.join("main.rs");
    // "foo\n" — char 3 is the trailing newline; a collapsed selection
    // there puts Insert mode's cursor right after "foo", ready to type "(".
    std::fs::write(&file, "foo\n").unwrap();
    file
}

/// The client and its handshake are constructed *after* the plugin loads
/// (unlike every other feature's `setup`), so `on-lsp-attach`'s handler —
/// which registers trigger chars — is already installed when the
/// `Running` transition fires it. Every other card's tests never depend on
/// that ordering because they trigger everything through `type_cmd`, not
/// through a hook that only fires once, at attach time.
fn setup(
    file: &Path,
    tmp: &Path,
    configure: impl FnOnce(&mut RecordingLspBackend, ServerId),
) -> (Editor, RealRuntimeGuard, RequestLog) {
    let guard = RealRuntimeGuard::new();

    let (mut backend, _notifications, requests) = RecordingLspBackend::new();
    backend.respond_to(
        "initialize",
        serde_json::json!({"capabilities": {
            "signatureHelpProvider": {"triggerCharacters": ["(", ","]}
        }}),
    );
    let sid = backend.start("rust-analyzer", &[], Path::new(".")).unwrap();
    configure(&mut backend, sid);

    let mut ed = Editor::open(None, std::sync::Arc::new(|| {})).unwrap();
    ed.lsp = LspState::from_backend_for_test(Box::new(backend));

    let mut host = ScriptingHost::new();
    eval_with_real_host(&mut ed, &mut host, r#"(load-plugin "core:lsp")"#, tmp);
    ed.scripting = Some(host);

    ed.execute_typed("e", Some(file.to_str().unwrap())).unwrap();
    let bid = ed.focused_buffer_id();
    ed.state.buffers.get_mut(bid).lsp_server = Some(sid);
    // This harness's `eval_init` never loads `languages.scm` (unlike the
    // real `Editor::init_scripting` startup sequence), so `.rs` extension
    // detection never ran — set the language explicitly to match the
    // "rust" server key below, which on-lsp-attach's `server-name` arg
    // (the language) must equal for register-trigger-chars! to route here.
    let lang = ed.state.languages.intern("rust");
    ed.state.buffers.get_mut(bid).language = Some(lang);

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

fn position_after_foo(ed: &mut Editor) {
    let bid = ed.focused_buffer_id();
    let pid = ed.state.focused_pane_id;
    let pbs = ed
        .state
        .panes
        .state
        .get_mut(pid)
        .and_then(|by_buf| by_buf.get_mut(bid))
        .expect("pane buffer state must exist");
    pbs.selections = SelectionSet::single(Selection::collapsed(3));
}

fn type_char_and_settle(ed: &mut Editor, ch: char) {
    ed.feed_key(key(ch));
    ed.drain_hooks(); // on-trigger-char fires, schedules the debounce timer
    std::thread::sleep(Duration::from_millis(250));
    ed.drain_async_sources(); // debounce timer fires, sends the request
    ed.drain_lsp(); // scripted response arrives
    ed.drain_pending_steel_calls(); // callback runs, shows/updates the popup
}

fn popup_lines(ed: &mut Editor) -> Vec<String> {
    let mut ctx = RenderContext::new();
    ed.prepare_frame(80, 25, &mut ctx);
    ed.state
        .popup_view
        .read()
        .unwrap()
        .as_ref()
        .map(|s| s.lines.clone())
        .unwrap_or_default()
}

fn request_count(requests: &RequestLog, method: &str) -> usize {
    requests
        .borrow()
        .iter()
        .filter(|(_sid, m, _params)| m == method)
        .count()
}

fn signature_help_response(
    label: &str,
    param_labels: &[&str],
    active_param: i64,
) -> serde_json::Value {
    serde_json::json!({
        "signatures": [{
            "label": label,
            "parameters": param_labels.iter().map(|l| serde_json::json!({"label": l})).collect::<Vec<_>>(),
        }],
        "activeSignature": 0,
        "activeParameter": active_param,
    })
}

/// Detach must be a true no-op: `*sighelp-chars*`/`"lsp-sighelp"`'s
/// trigger-char registration is global, set once at attach, so
/// `on-lsp-detach` must clear it. The `on-trigger-char` handler also needs
/// its own `lsp/guard-capability` check (unlike completion.scm, which
/// doesn't need one) — without it, a trigger char left registered past
/// `:lsp-stop` would hit `lsp-request`'s server-resolution failure and log
/// an Error, not a polite Info skip, on every matching keystroke.
#[test]
fn detach_clears_sighelp_trigger_chars_so_a_stale_trigger_is_a_true_no_op() {
    let tmp = safe_tempdir();
    let file_dir = safe_tempdir();
    let file = write_fixture_file(file_dir.path());
    let (mut ed, _guard, requests) = setup(&file, tmp.path(), |_backend, _sid| {});
    position_after_foo(&mut ed);

    ed.lsp_stop(Some("rust"));
    ed.drain_hooks(); // on-lsp-detach clears *sighelp-chars*

    ed.feed_key(key('i'));
    ed.drain_hooks();
    let before_log_len = ed.state.message_log.entries().count();
    type_char_and_settle(&mut ed, '(');

    assert_eq!(request_count(&requests, "textDocument/signatureHelp"), 0);
    assert_eq!(
        ed.state.message_log.entries().count(),
        before_log_len,
        "a trigger char left registered past detach must be a true no-op, not an \
         lsp-request server-resolution Error logged every keystroke"
    );
}

#[test]
fn trigger_char_after_debounce_shows_signature_with_marked_param() {
    let tmp = safe_tempdir();
    let file_dir = safe_tempdir();
    let file = write_fixture_file(file_dir.path());
    let (mut ed, _guard, _requests) = setup(&file, tmp.path(), |backend, _sid| {
        backend.respond_to(
            "textDocument/signatureHelp",
            signature_help_response("fn foo(a: i32, b: i32)", &["a: i32", "b: i32"], 0),
        );
    });
    position_after_foo(&mut ed);

    ed.feed_key(key('i'));
    ed.drain_hooks();
    type_char_and_settle(&mut ed, '(');

    assert_eq!(
        popup_lines(&mut ed),
        vec!["fn foo(a: i32, b: i32)".to_string(), "⟨a: i32⟩".to_string()]
    );
}

#[test]
fn comma_advances_the_marked_parameter() {
    let tmp = safe_tempdir();
    let file_dir = safe_tempdir();
    let file = write_fixture_file(file_dir.path());
    let (mut ed, _guard, _requests) = setup(&file, tmp.path(), |backend, _sid| {
        backend.respond_to(
            "textDocument/signatureHelp",
            signature_help_response("fn foo(a: i32, b: i32)", &["a: i32", "b: i32"], 0),
        );
        backend.respond_to(
            "textDocument/signatureHelp",
            signature_help_response("fn foo(a: i32, b: i32)", &["a: i32", "b: i32"], 1),
        );
    });
    position_after_foo(&mut ed);
    ed.feed_key(key('i'));
    ed.drain_hooks();
    type_char_and_settle(&mut ed, '(');

    type_char_and_settle(&mut ed, ',');

    assert_eq!(
        popup_lines(&mut ed),
        vec!["fn foo(a: i32, b: i32)".to_string(), "⟨b: i32⟩".to_string()]
    );
}

#[test]
fn close_paren_closes_the_popup_without_a_request() {
    let tmp = safe_tempdir();
    let file_dir = safe_tempdir();
    let file = write_fixture_file(file_dir.path());
    let (mut ed, _guard, requests) = setup(&file, tmp.path(), |backend, _sid| {
        backend.respond_to(
            "textDocument/signatureHelp",
            signature_help_response("fn foo(a: i32)", &["a: i32"], 0),
        );
    });
    // Auto-pair would insert a matching ")" right after typing "(" and
    // then just skip-over (not insert) an explicitly typed ")" — and
    // `on-trigger-char` only fires on a genuine insertion (mappings/
    // insert.rs). Disable it so this test's own ")" keystroke is a real
    // insertion, exercising the same code path a non-auto-paired ")"
    // (or a language without auto-pairs configured) would take.
    ed.state.settings.auto_pairs_enabled = false;
    position_after_foo(&mut ed);
    ed.feed_key(key('i'));
    ed.drain_hooks();
    type_char_and_settle(&mut ed, '(');
    assert!(
        !popup_lines(&mut ed).is_empty(),
        "popup must be open before closing it"
    );
    let requests_before_close = requests.borrow().len();

    ed.feed_key(key(')'));
    ed.drain_hooks();
    ed.drain_pending_steel_calls();

    assert!(popup_lines(&mut ed).is_empty(), "')' must close the popup");
    assert_eq!(
        requests.borrow().len(),
        requests_before_close,
        "')' must not send a signatureHelp request"
    );
}

#[test]
fn esc_closes_via_the_shared_mode_change_handler() {
    let tmp = safe_tempdir();
    let file_dir = safe_tempdir();
    let file = write_fixture_file(file_dir.path());
    let (mut ed, _guard, _requests) = setup(&file, tmp.path(), |backend, _sid| {
        backend.respond_to(
            "textDocument/signatureHelp",
            signature_help_response("fn foo(a: i32)", &["a: i32"], 0),
        );
    });
    position_after_foo(&mut ed);
    ed.feed_key(key('i'));
    ed.drain_hooks();
    type_char_and_settle(&mut ed, '(');
    assert!(
        !popup_lines(&mut ed).is_empty(),
        "popup must be open before Esc"
    );

    ed.feed_key(key_esc());
    ed.drain_hooks();

    assert!(popup_lines(&mut ed).is_empty(), "Esc must close the popup");
}

#[test]
fn rapid_trigger_chars_coalesce_to_one_request() {
    let tmp = safe_tempdir();
    let file_dir = safe_tempdir();
    let file = write_fixture_file(file_dir.path());
    let (mut ed, _guard, requests) = setup(&file, tmp.path(), |backend, _sid| {
        backend.respond_to(
            "textDocument/signatureHelp",
            signature_help_response("fn foo()", &[], 0),
        );
    });
    position_after_foo(&mut ed);
    ed.feed_key(key('i'));
    ed.drain_hooks();

    // Three trigger chars back to back, no settling in between — each
    // (re)schedules the same 150ms debounce, cancelling the last.
    ed.feed_key(key('('));
    ed.drain_hooks();
    ed.feed_key(key(','));
    ed.drain_hooks();
    ed.feed_key(key(','));
    ed.drain_hooks();

    std::thread::sleep(Duration::from_millis(250));
    ed.drain_async_sources();
    ed.drain_lsp();
    ed.drain_pending_steel_calls();

    let sighelp_requests = requests
        .borrow()
        .iter()
        .filter(|(_sid, method, _params)| method == "textDocument/signatureHelp")
        .count();
    assert_eq!(
        sighelp_requests, 1,
        "a rapid burst must collapse to exactly one request"
    );
}

#[test]
fn null_response_closes_the_popup() {
    let tmp = safe_tempdir();
    let file_dir = safe_tempdir();
    let file = write_fixture_file(file_dir.path());
    let (mut ed, _guard, _requests) = setup(&file, tmp.path(), |backend, _sid| {
        backend.respond_to(
            "textDocument/signatureHelp",
            signature_help_response("fn foo(a: i32)", &["a: i32"], 0),
        );
        backend.respond_to("textDocument/signatureHelp", serde_json::Value::Null);
    });
    position_after_foo(&mut ed);
    ed.feed_key(key('i'));
    ed.drain_hooks();
    type_char_and_settle(&mut ed, '(');
    assert!(
        !popup_lines(&mut ed).is_empty(),
        "popup must be open before the null response"
    );

    type_char_and_settle(&mut ed, ',');

    assert!(
        popup_lines(&mut ed).is_empty(),
        "a null response must close the popup"
    );
}

#[test]
fn offset_form_parameter_label_marks_the_correct_slice() {
    let tmp = safe_tempdir();
    let file_dir = safe_tempdir();
    let file = write_fixture_file(file_dir.path());
    let (mut ed, _guard, _requests) = setup(&file, tmp.path(), |backend, _sid| {
        backend.respond_to(
            "textDocument/signatureHelp",
            serde_json::json!({
                "signatures": [{
                    "label": "fn foo(a: i32, longarg: i32)",
                    // "a: i32" is [7, 13); "longarg: i32" is [15, 27) — offset
                    // form, distinct from the string-form fixtures above.
                    "parameters": [{"label": [7, 13]}, {"label": [15, 27]}],
                }],
                "activeSignature": 0,
                "activeParameter": 1,
            }),
        );
    });
    position_after_foo(&mut ed);
    ed.feed_key(key('i'));
    ed.drain_hooks();
    type_char_and_settle(&mut ed, '(');

    assert_eq!(
        popup_lines(&mut ed),
        vec![
            "fn foo(a: i32, longarg: i32)".to_string(),
            "⟨longarg: i32⟩".to_string()
        ]
    );
}

#[test]
fn offset_form_label_with_an_astral_char_marks_the_correct_slice() {
    let tmp = safe_tempdir();
    let file_dir = safe_tempdir();
    let file = write_fixture_file(file_dir.path());
    let (mut ed, _guard, _requests) = setup(&file, tmp.path(), |backend, _sid| {
        backend.respond_to(
            "textDocument/signatureHelp",
            serde_json::json!({
                "signatures": [{
                    // "😀" is one astral char (U+1F600 -> 2 UTF-16 units,
                    // 1 Steel char). "a" starts at char index 2 but wire
                    // (UTF-16) offset 3 — a param-text impl that treats
                    // the offset as a char index directly would slice the
                    // wrong span or panic out of bounds.
                    "label": "😀 a",
                    "parameters": [{"label": [3, 4]}],
                }],
                "activeSignature": 0,
                "activeParameter": 0,
            }),
        );
    });
    position_after_foo(&mut ed);
    ed.feed_key(key('i'));
    ed.drain_hooks();
    type_char_and_settle(&mut ed, '(');

    assert_eq!(
        popup_lines(&mut ed),
        vec!["😀 a".to_string(), "⟨a⟩".to_string()]
    );
}
