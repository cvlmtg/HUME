// Signature help: trigger chars fire a debounced
// textDocument/signatureHelp, composing `lsp-request!`,
// `lsp-capabilities`, debounce, `on-lsp-attach`, `on-trigger-char`.
// Loads the real shipped `core:lsp` plugin in place (`RealRuntimeGuard`).
//
// Not on Windows: Scheme require strings embed OS paths; backslashes are not
// escaped in Steel string literals (same constraint as tests/plugins.rs).

use std::path::Path;
use std::time::Duration;

use super::*;
use crate::editor::commands::open_pane_in_layout;
use hume_lsp::backend::ServerId;
use hume_lsp::test_util::{RecordingLspBackend, RequestLog};

fn setup(
    file: &Path,
    tmp: &Path,
    configure: impl FnOnce(&mut RecordingLspBackend, ServerId),
) -> (Editor, RealRuntimeGuard, RequestLog) {
    setup_trigger_char_feature(
        file,
        tmp,
        serde_json::json!({"signatureHelpProvider": {"triggerCharacters": ["(", ","]}}),
        configure,
    )
}

/// Char 3 of the fixture's "foo\n" is the trailing newline; a collapsed
/// selection there puts Insert mode's cursor right after "foo".
fn position_after_foo(ed: &mut Editor) {
    set_cursor(ed, 3);
}

fn type_char_and_settle(ed: &mut Editor, ch: char) {
    ed.feed_key(key(ch));
    ed.settle(); // on-trigger-char fires, schedules the debounce timer
    std::thread::sleep(Duration::from_millis(250));
    ed.drain_async_sources(); // debounce timer fires, sends the request
    ed.drain_lsp(); // scripted response arrives
    ed.settle(); // callback runs, shows/updates the popup
}

fn popup_lines(ed: &mut Editor) -> Vec<String> {
    crate::editor::tests::render(ed);
    ed.state
        .views
        .popup
        .read()
        .as_ref()
        .map(|s| (*s.lines).clone())
        .unwrap_or_default()
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
/// doesn't need one). Without it, a trigger char left registered past
/// `:lsp-stop` would hit `lsp-request!`'s server-resolution failure and log
/// an Error, not a polite Info skip, on every matching keystroke.
#[test]
fn detach_clears_sighelp_trigger_chars_so_a_stale_trigger_is_a_true_no_op() {
    let tmp = safe_tempdir();
    let file_dir = safe_tempdir();
    let file = write_foo_fixture(file_dir.path());
    let (mut ed, _guard, requests) = setup(&file, tmp.path(), |_backend, _sid| {});
    position_after_foo(&mut ed);

    ed.lsp_stop(&hume_scripting::LspServerTarget::Language(
        "rust".to_string(),
    ));
    ed.settle(); // on-lsp-detach clears *sighelp-chars*

    ed.feed_key(key('i'));
    ed.settle();
    let before_log_len = ed.state.message_log.entries().count();
    type_char_and_settle(&mut ed, '(');

    assert_eq!(request_count(&requests, "textDocument/signatureHelp"), 0);
    assert_eq!(
        ed.state.message_log.entries().count(),
        before_log_len,
        "a trigger char left registered past detach must be a true no-op, not an \
         lsp-request! server-resolution Error logged every keystroke"
    );
}

#[test]
fn trigger_char_after_debounce_shows_signature_with_marked_param() {
    let tmp = safe_tempdir();
    let file_dir = safe_tempdir();
    let file = write_foo_fixture(file_dir.path());
    let (mut ed, _guard, _requests) = setup(&file, tmp.path(), |backend, _sid| {
        backend.respond_to(
            "textDocument/signatureHelp",
            signature_help_response("fn foo(a: i32, b: i32)", &["a: i32", "b: i32"], 0),
        );
    });
    position_after_foo(&mut ed);

    ed.feed_key(key('i'));
    ed.settle();
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
    let file = write_foo_fixture(file_dir.path());
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
    ed.settle();
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
    let file = write_foo_fixture(file_dir.path());
    let (mut ed, _guard, requests) = setup(&file, tmp.path(), |backend, _sid| {
        backend.respond_to(
            "textDocument/signatureHelp",
            signature_help_response("fn foo(a: i32)", &["a: i32"], 0),
        );
    });
    // Auto-pair would insert a matching ")" right after typing "(" and
    // then just skip-over (not insert) an explicitly typed ")", and
    // `on-trigger-char` only fires on a genuine insertion (mappings/
    // insert.rs). Disable it so this test's own ")" keystroke is a real
    // insertion, exercising the same code path a non-auto-paired ")"
    // (or a language without auto-pairs configured) would take.
    ed.state.settings.auto_pairs_enabled = false;
    position_after_foo(&mut ed);
    ed.feed_key(key('i'));
    ed.settle();
    type_char_and_settle(&mut ed, '(');
    assert!(
        !popup_lines(&mut ed).is_empty(),
        "popup must be open before closing it"
    );
    let requests_before_close = requests.borrow().len();

    ed.feed_key(key(')'));
    ed.settle();

    assert!(popup_lines(&mut ed).is_empty(), "')' must close the popup");
    assert_eq!(
        requests.borrow().len(),
        requests_before_close,
        "')' must not send a signatureHelp request"
    );
}

#[test]
fn esc_ending_insert_closes_the_sticky_popup() {
    let tmp = safe_tempdir();
    let file_dir = safe_tempdir();
    let file = write_foo_fixture(file_dir.path());
    let (mut ed, _guard, _requests) = setup(&file, tmp.path(), |backend, _sid| {
        backend.respond_to(
            "textDocument/signatureHelp",
            signature_help_response("fn foo(a: i32)", &["a: i32"], 0),
        );
    });
    position_after_foo(&mut ed);
    ed.feed_key(key('i'));
    ed.settle();
    type_char_and_settle(&mut ed, '(');
    assert!(
        !popup_lines(&mut ed).is_empty(),
        "popup must be open before Esc"
    );

    ed.feed_key(key_esc());
    ed.settle();

    assert!(popup_lines(&mut ed).is_empty(), "Esc must close the popup");
}

#[test]
fn rapid_trigger_chars_coalesce_to_one_request() {
    let tmp = safe_tempdir();
    let file_dir = safe_tempdir();
    let file = write_foo_fixture(file_dir.path());
    let (mut ed, _guard, requests) = setup(&file, tmp.path(), |backend, _sid| {
        backend.respond_to(
            "textDocument/signatureHelp",
            signature_help_response("fn foo()", &[], 0),
        );
    });
    position_after_foo(&mut ed);
    ed.feed_key(key('i'));
    ed.settle();

    // Three trigger chars back to back, no settling in between; each
    // (re)schedules the same 150ms debounce, cancelling the last.
    ed.feed_key(key('('));
    ed.settle();
    ed.feed_key(key(','));
    ed.settle();
    ed.feed_key(key(','));
    ed.settle();

    std::thread::sleep(Duration::from_millis(250));
    ed.drain_async_sources();
    ed.drain_lsp();
    ed.settle();

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
    let file = write_foo_fixture(file_dir.path());
    let (mut ed, _guard, _requests) = setup(&file, tmp.path(), |backend, _sid| {
        backend.respond_to(
            "textDocument/signatureHelp",
            signature_help_response("fn foo(a: i32)", &["a: i32"], 0),
        );
        backend.respond_to("textDocument/signatureHelp", serde_json::Value::Null);
    });
    position_after_foo(&mut ed);
    ed.feed_key(key('i'));
    ed.settle();
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
    let file = write_foo_fixture(file_dir.path());
    let (mut ed, _guard, _requests) = setup(&file, tmp.path(), |backend, _sid| {
        backend.respond_to(
            "textDocument/signatureHelp",
            serde_json::json!({
                "signatures": [{
                    "label": "fn foo(a: i32, longarg: i32)",
                    // "a: i32" is [7, 13); "longarg: i32" is [15, 27), in offset
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
    ed.settle();
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
    let file = write_foo_fixture(file_dir.path());
    let (mut ed, _guard, _requests) = setup(&file, tmp.path(), |backend, _sid| {
        backend.respond_to(
            "textDocument/signatureHelp",
            serde_json::json!({
                "signatures": [{
                    // "😀" is one astral char (U+1F600 -> 2 UTF-16 units,
                    // 1 Steel char). "a" starts at char index 2 but wire
                    // (UTF-16) offset 3, so a param-text impl that treats
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
    ed.settle();
    type_char_and_settle(&mut ed, '(');

    assert_eq!(
        popup_lines(&mut ed),
        vec!["😀 a".to_string(), "⟨a⟩".to_string()]
    );
}

#[test]
fn offset_form_label_is_read_in_the_negotiated_encoding_not_always_utf16() {
    let tmp = safe_tempdir();
    let file_dir = safe_tempdir();
    let file = write_foo_fixture(file_dir.path());
    let (mut ed, _guard, _requests) = setup_trigger_char_feature(
        &file,
        tmp.path(),
        serde_json::json!({
            "signatureHelpProvider": {"triggerCharacters": ["(", ","]},
            "positionEncoding": "utf-8",
        }),
        |backend, _sid| {
            backend.respond_to(
                "textDocument/signatureHelp",
                serde_json::json!({
                    "signatures": [{
                        // "é" is 2 UTF-8 bytes but 1 UTF-16 unit, so the two
                        // encodings disagree about every offset past it.
                        // "a: i32" spans bytes [6, 12) and UTF-16 units
                        // [5, 11); this server negotiated utf-8, so it
                        // sends the byte pair. Reading it as UTF-16 slices
                        // ": i32)" instead.
                        "label": "fn é(a: i32)",
                        "parameters": [{"label": [6, 12]}],
                    }],
                    "activeSignature": 0,
                    "activeParameter": 0,
                }),
            );
        },
    );
    position_after_foo(&mut ed);
    ed.feed_key(key('i'));
    ed.settle();
    type_char_and_settle(&mut ed, '(');

    assert_eq!(
        popup_lines(&mut ed),
        vec!["fn é(a: i32)".to_string(), "⟨a: i32⟩".to_string()]
    );
}

/// The user is free to switch panes while a debounced signature-help
/// request is in flight, the same async-round-trip race `lsp-hover`'s own
/// `#:require-focus` guards against. A response for a buffer that's no
/// longer focused must not open a popup over whatever pane the user
/// switched to.
///
/// A split (not `:e`) moves focus without hiding the original buffer:
/// `lsp-position-params` still needs it shown *somewhere* to build the
/// request in the first place, since the debounce timer only fires (and
/// the request only gets built) after this switch, not before it.
///
/// This depends on `lsp/sighelp-request` (`sighelp.scm`) passing
/// `#:require-focus` to its `lsp-request!`.
#[test]
fn stale_response_after_a_pane_switch_shows_no_popup() {
    let tmp = safe_tempdir();
    let file_dir = safe_tempdir();
    let file = write_foo_fixture(file_dir.path());
    let (mut ed, _guard, _requests) = setup(&file, tmp.path(), |backend, _sid| {
        backend.respond_to(
            "textDocument/signatureHelp",
            signature_help_response("fn foo(a: i32, b: i32)", &["a: i32", "b: i32"], 0),
        );
    });
    position_after_foo(&mut ed);
    ed.feed_key(key('i'));
    ed.settle();
    ed.feed_key(key('('));
    ed.settle(); // on-trigger-char fires, schedules the debounce timer

    let extra = file_dir.path().join("other.rs");
    std::fs::write(&extra, "fn other() {}\n").unwrap();
    let other_bid = ed
        .open_extra_file(&extra)
        .expect("extra file must open as a buffer");
    let start_pid = ed.state.focus.id();
    let other_pid = open_pane_in_layout(
        &mut ed.state,
        &mut ed.view,
        start_pid,
        other_bid,
        hume_engine::pipeline::Direction::Horizontal,
    )
    .unwrap();
    ed.state.focus.set_for_test(other_pid);

    std::thread::sleep(Duration::from_millis(250));
    ed.drain_async_sources(); // debounce fires, request sent and (dropped) delivered
    ed.settle();

    assert_eq!(
        popup_lines(&mut ed),
        Vec::<String>::new(),
        "a signature-help response for a pane that's no longer focused must not open a popup"
    );
}
