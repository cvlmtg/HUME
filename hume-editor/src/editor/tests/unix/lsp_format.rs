// Formatting: `:lsp-fmt`, composing `lsp-request`,
// `lsp-capabilities`, `selection-spans-full-line?`, `apply-text-edits!`.
// Loads the real shipped `core:lsp` plugin in place (`RealRuntimeGuard`).
//
// Not on Windows: Scheme require strings embed OS paths; backslashes are not
// escaped in Steel string literals (same constraint as tests/plugins.rs).

use std::path::{Path, PathBuf};

use super::*;
use crate::editor::lsp::LspState;
use hume_editing::selection::{Selection, SelectionSet};
use hume_lsp::backend::{LspBackend, ServerId};
use hume_lsp::client::LspClient;
use hume_lsp::inline::InlineLspBackend;
use hume_scripting::ScriptingHost;

/// "line1\nline2\nline3\n" — char offsets: line0 'line1' = 0..5 (+\n at 5),
/// line1 'line2' = 6..11 (+\n at 11), line2 'line3' = 12..17 (+\n at 17).
fn setup(
    file: &Path,
    tmp: &Path,
    configure: impl FnOnce(&mut InlineLspBackend, ServerId),
) -> (Editor, RealRuntimeGuard) {
    let guard = RealRuntimeGuard::new();
    std::fs::write(file, "line1\nline2\nline3\n").unwrap();

    let mut backend = InlineLspBackend::new();
    backend.respond_to(
        "initialize",
        serde_json::json!({"capabilities": {
            "documentFormattingProvider": true,
            "documentRangeFormattingProvider": true
        }}),
    );
    let sid = backend.start("rust-analyzer", &[], Path::new(".")).unwrap();
    configure(&mut backend, sid);

    let mut ed = Editor::open(None).unwrap();
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

    (ed, guard)
}

fn select_full_line_1(ed: &mut Editor) {
    // 'line1\n' — chars [0, 6).
    let bid = ed.focused_buffer_id();
    let pid = ed.state.focused_pane_id;
    let pbs = ed
        .state
        .panes
        .state
        .get_mut(pid)
        .and_then(|by_buf| by_buf.get_mut(bid))
        .expect("pane buffer state must exist");
    pbs.selections = SelectionSet::single(Selection::new(0, 5));
}

fn run_fmt(ed: &mut Editor) {
    type_cmd(ed, ":lsp-fmt");
    ed.drain_hooks();
    ed.drain_lsp();
    ed.drain_pending_steel_calls();
}

fn text_edit(sl: u64, sc: u64, el: u64, ec: u64, new_text: &str) -> serde_json::Value {
    serde_json::json!({
        "range": {"start": {"line": sl, "character": sc}, "end": {"line": el, "character": ec}},
        "newText": new_text
    })
}

#[test]
fn whole_buffer_edit_is_one_undo_step() {
    let tmp = safe_tempdir();
    let file_dir = safe_tempdir();
    let (mut ed, _guard) = setup(
        &file_dir.path().join("main.rs"),
        tmp.path(),
        |backend, _sid| {
            backend.respond_to(
                "textDocument/formatting",
                serde_json::json!([text_edit(
                    0,
                    0,
                    3,
                    0,
                    "formatted1\nformatted2\nformatted3\n"
                )]),
            );
        },
    );
    let before = ed.doc().text().to_string();

    run_fmt(&mut ed);

    assert_eq!(
        ed.doc().text().to_string(),
        "formatted1\nformatted2\nformatted3\n",
        "the whole-buffer replacement edit must apply"
    );
    ed.handle_key(key('u'));
    assert_eq!(
        ed.doc().text().to_string(),
        before,
        "a single 'u' must fully restore the pre-format text"
    );
}

#[test]
fn sub_line_selection_still_formats_the_whole_buffer() {
    // Default cursor: a bare collapsed selection — never spans a full line.
    let tmp = safe_tempdir();
    let file_dir = safe_tempdir();
    let (mut ed, _guard) = setup(
        &file_dir.path().join("main.rs"),
        tmp.path(),
        |backend, _sid| {
            backend.respond_to(
                "textDocument/formatting",
                serde_json::json!([text_edit(0, 0, 3, 0, "WHOLE_BUFFER\n")]),
            );
            // If the decision were wrong and a sub-line selection triggered
            // range formatting instead, this response would apply and the
            // assertion below would fail loudly (not silently match).
            backend.respond_to(
                "textDocument/rangeFormatting",
                serde_json::json!([text_edit(0, 0, 1, 0, "WRONG_RANGE_PATH\n")]),
            );
        },
    );

    run_fmt(&mut ed);

    assert_eq!(ed.doc().text().to_string(), "WHOLE_BUFFER\n");
}

#[test]
fn full_line_selection_sends_range_formatting() {
    let tmp = safe_tempdir();
    let file_dir = safe_tempdir();
    let (mut ed, _guard) = setup(
        &file_dir.path().join("main.rs"),
        tmp.path(),
        |backend, _sid| {
            backend.respond_to(
                "textDocument/rangeFormatting",
                serde_json::json!([text_edit(0, 0, 1, 0, "RANGE_FORMATTED\n")]),
            );
            backend.respond_to(
                "textDocument/formatting",
                serde_json::json!([text_edit(0, 0, 3, 0, "WRONG_WHOLE_BUFFER_PATH\n")]),
            );
        },
    );
    select_full_line_1(&mut ed);

    run_fmt(&mut ed);

    assert_eq!(
        ed.doc().text().to_string(),
        "RANGE_FORMATTED\nline2\nline3\n",
        "a full-line selection must send rangeFormatting, not whole-buffer formatting"
    );
}

#[test]
fn null_result_reports_already_formatted() {
    let tmp = safe_tempdir();
    let file_dir = safe_tempdir();
    let (mut ed, _guard) = setup(
        &file_dir.path().join("main.rs"),
        tmp.path(),
        |backend, _sid| {
            backend.respond_to("textDocument/formatting", serde_json::Value::Null);
        },
    );

    run_fmt(&mut ed);

    let msg = ed.state.status_msg.clone().unwrap_or_default();
    assert!(
        msg.to_lowercase().contains("already formatted"),
        "expected an already-formatted message, got {msg:?}"
    );
}

#[test]
fn loading_the_plugin_registers_no_save_hook() {
    let tmp = safe_tempdir();
    let file_dir = safe_tempdir();
    let (mut ed, _guard) = setup(
        &file_dir.path().join("main.rs"),
        tmp.path(),
        |backend, _sid| {
            // If a save hook incorrectly fired :lsp-fmt, this response landing
            // would visibly rewrite the buffer.
            backend.respond_to(
                "textDocument/formatting",
                serde_json::json!([text_edit(0, 0, 3, 0, "SHOULD_NOT_APPEAR\n")]),
            );
        },
    );
    let before = ed.doc().text().to_string();

    let bid = ed.focused_buffer_id();
    ed.fire_hook_buffer_save(bid);
    ed.drain_hooks();
    ed.drain_lsp();
    ed.drain_pending_steel_calls();

    assert_eq!(
        ed.doc().text().to_string(),
        before,
        "loading core:lsp must not register an on-buffer-save formatter — v1 is manual :lsp-fmt only"
    );
}
