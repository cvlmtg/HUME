// Rename: `lsp-rename` composing `lsp-request!`, `lsp-capabilities`,
// `apply-workspace-edit!`, `prompt!`, `symbol-under-cursor`. Loads the real
// shipped `core:lsp` plugin in place
// (`RealRuntimeGuard`).
//
// Not on Windows: Scheme require strings embed OS paths; backslashes are not
// escaped in Steel string literals (same constraint as tests/plugins.rs).

use std::path::Path;

use super::*;
use hume_lsp::backend::ServerId;
use hume_lsp::test_util::RecordingLspBackend;

/// A [`core_lsp_rig`] over "fn main() {\n    helper();\n}\n" with a
/// rename provider. The cursor sits on the 'h' of "helper" (line 1, col 4)
/// so `symbol-under-cursor` has something real to extract.
fn setup(
    tmp: &Path,
    configure: impl FnOnce(&mut RecordingLspBackend, ServerId),
) -> (Editor, RealRuntimeGuard, ServerId) {
    let (rig, guard) = core_lsp_rig(
        tmp,
        "fn main() {\n    -[h]>elper();\n}\n",
        serde_json::json!({"capabilities": {"renameProvider": true}}),
        configure,
    );
    let sid = rig.sid("rust-analyzer");
    (rig.ed, guard, sid)
}

fn run_rename(ed: &mut Editor) {
    // lsp-rename is key-bindable, not typed: dispatch through the keymap
    // pipeline, the way its bound key (`G R`) would.
    ed.execute_keymap_command("lsp-rename".into(), Some(1), false);
    ed.settle();
}

#[test]
fn prompt_prefill_shows_the_symbol_under_cursor() {
    let tmp = safe_tempdir();
    let (mut ed, _guard, _sid) = setup(tmp.path(), |_backend, _sid| {});

    run_rename(&mut ed);

    let mb = ed.state.minibuf().expect("prompt must be open");
    assert_eq!(
        mb.input, "helper",
        "prefill must be the word under the cursor"
    );
}

#[test]
fn cancel_sends_no_rename_request() {
    let tmp = safe_tempdir();
    let uri = rust_rig_uri(tmp.path());
    // Script a response that WOULD apply visibly if the request were sent
    // despite the cancel. This proves the guard, not just "nothing crashed".
    let (mut ed, _guard, _sid) = setup(tmp.path(), |backend, _sid| {
        backend.respond_to(
            "textDocument/rename",
            serde_json::json!({"changes": {uri: [
                {"range": {"start": {"line": 1, "character": 4}, "end": {"line": 1, "character": 10}}, "newText": "SHOULD_NOT_APPLY"}
            ]}}),
        );
    });
    let before = ed.doc().text().to_string();

    run_rename(&mut ed);
    ed.feed_key(key_esc());
    ed.settle();
    ed.drain_lsp();
    ed.settle();

    assert_eq!(
        ed.doc().text().to_string(),
        before,
        "cancel must not apply any edit"
    );
    assert!(
        !ed.state
            .status_msg
            .clone()
            .unwrap_or_default()
            .contains("buffers modified"),
        "cancel must not send the rename request at all"
    );
}

#[test]
fn null_result_reports_nothing_to_rename() {
    let tmp = safe_tempdir();
    let (mut ed, _guard, _sid) = setup(tmp.path(), |backend, _sid| {
        backend.respond_to("textDocument/rename", serde_json::Value::Null);
    });

    run_rename(&mut ed);
    ed.feed_key(key('X'));
    ed.feed_key(key_enter());
    ed.settle();
    ed.drain_lsp();
    ed.settle();

    let msg = ed.state.status_msg.clone().unwrap_or_default();
    assert!(
        msg.to_lowercase().contains("nothing to rename"),
        "expected a nothing-to-rename message, got {msg:?}"
    );
}

#[test]
fn multi_file_workspace_edit_applies_and_logs_the_summary() {
    let tmp = safe_tempdir();
    let uri = rust_rig_uri(tmp.path());
    let other_file = rig_root(tmp.path()).join("lib.rs");
    std::fs::write(&other_file, "fn helper() {}\n").unwrap();
    let other_uri = hume_lsp::uri::path_to_uri(&std::fs::canonicalize(&other_file).unwrap())
        .unwrap()
        .as_str()
        .to_string();

    let (mut ed, _guard, _sid) = setup(tmp.path(), move |backend, _sid| {
        backend.respond_to(
            "textDocument/rename",
            serde_json::json!({"changes": {
                uri.clone(): [
                    {"range": {"start": {"line": 1, "character": 4}, "end": {"line": 1, "character": 10}}, "newText": "renamed"}
                ],
                other_uri: [
                    {"range": {"start": {"line": 0, "character": 3}, "end": {"line": 0, "character": 9}}, "newText": "renamed"}
                ]
            }}),
        );
    });

    run_rename(&mut ed);
    // Accept the "helper" prefill as-is (typing nothing, just Enter). The
    // exact new name doesn't matter, only that it's non-empty so `when
    // new-name` fires.
    ed.feed_key(key_enter());
    ed.settle();
    ed.drain_lsp();
    ed.settle();

    assert_eq!(
        ed.doc().text().to_string(),
        "fn main() {\n    renamed();\n}\n",
        "the currently-open file's edit must apply"
    );
    let msg = ed.state.status_msg.clone().unwrap_or_default();
    assert!(
        msg.contains('2') && msg.contains("buffers modified"),
        "expected the 2-buffer summary, got {msg:?}"
    );
}

/// `textDocument/rename`'s request carries `#:allow-stale #t`, so an
/// edit landing between confirming the new name and the response draining
/// must not silently drop the rename, but `apply-workspace-edit!`'s own
/// `#:expect-generation` must then refuse to apply it, rather than silently
/// doing nothing or applying against text that has since moved.
#[test]
fn rename_reports_a_stale_buffer_after_an_intervening_edit() {
    let tmp = safe_tempdir();
    let uri = rust_rig_uri(tmp.path());
    let (mut ed, _guard, _sid) = setup(tmp.path(), move |backend, _sid| {
        backend.respond_to(
            "textDocument/rename",
            serde_json::json!({"changes": {
                uri: [
                    {"range": {"start": {"line": 1, "character": 4}, "end": {"line": 1, "character": 10}}, "newText": "renamed"}
                ]
            }}),
        );
    });

    run_rename(&mut ed);
    ed.feed_key(key_enter()); // confirms the prefilled name, sends textDocument/rename
    ed.settle();
    // Edit the buffer before draining the rename response.
    ed.feed_key(key('i'));
    ed.feed_key(key('z'));
    ed.feed_key(key_esc());
    let before_apply = ed.doc().text().to_string();
    ed.drain_lsp();
    ed.settle();

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

/// `apply-workspace-edit!` (the Steel builtin `%apply-workspace-edit!` wraps)
/// opens unopened files via `lsp::edits::resolve_or_open` →
/// `buffer::lifecycle::open_or_dedup_and_notify`, which can't detect language
/// inline (see that function's doc). It queues the buffer onto
/// `EditorState.pending_language_detection`, drained at the tail of
/// `apply_script_effects` once this eval (reached via `settle`'s `Call` arm,
/// the rename response callback) returns.
///
/// If `resolve_or_open` called the bare `lifecycle::open_or_dedup`,
/// `lib.rs`'s buffer would never get a `language`.
#[test]
fn multi_file_workspace_edit_detects_language_of_the_newly_opened_file() {
    let tmp = safe_tempdir();
    let uri = rust_rig_uri(tmp.path());
    let other_file = rig_root(tmp.path()).join("lib.rs");
    std::fs::write(&other_file, "fn helper() {}\n").unwrap();
    let other_canonical = std::fs::canonicalize(&other_file).unwrap();
    let other_uri = hume_lsp::uri::path_to_uri(&other_canonical)
        .unwrap()
        .as_str()
        .to_string();

    let (mut ed, _guard, _sid) = setup(tmp.path(), move |backend, _sid| {
        backend.respond_to(
            "textDocument/rename",
            serde_json::json!({"changes": {
                uri.clone(): [
                    {"range": {"start": {"line": 1, "character": 4}, "end": {"line": 1, "character": 10}}, "newText": "renamed"}
                ],
                other_uri: [
                    {"range": {"start": {"line": 0, "character": 3}, "end": {"line": 0, "character": 9}}, "newText": "renamed"}
                ]
            }}),
        );
    });

    run_rename(&mut ed);
    ed.feed_key(key_enter());
    ed.settle();
    ed.drain_lsp();
    ed.settle();

    let bid = ed
        .state
        .buffers
        .find_by_path(&other_canonical)
        .expect("workspace edit must have opened lib.rs as a buffer");
    assert_eq!(
        ed.state.buffers.get(bid).language,
        ed.state.config.languages.id_of("rust"),
        "the workspace-edit-opened file must have its language detected"
    );
}
