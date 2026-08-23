// Goto definition family: `lsp-goto-definition` /
// `-declaration` / `-type-definition` / `-implementation`, composing
// `lsp-request`, `lsp-capabilities`, `goto-location!`,
// `show-drawer-list!` (via lib.scm's lsp/show-locations!). Loads the real
// shipped `core:lsp` plugin in place (`RealRuntimeGuard`).
//
// Not on Windows: Scheme require strings embed OS paths; backslashes are not
// escaped in Steel string literals (same constraint as tests/plugins.rs).

use std::path::{Path, PathBuf};

use super::*;
use crate::editor::lsp::LspState;
use hume_lsp::backend::{LspBackend, ServerId};
use hume_lsp::client::LspClient;
use hume_lsp::inline::InlineLspBackend;
use hume_scripting::ScriptingHost;

/// Writes the fixture file up front and returns its `file://` URI — callers
/// need the URI *before* `setup` to build their scripted `Location`
/// response, since `configure` must run before the backend is boxed into
/// `LspState` (trait-erased afterward, per `lsp_hover.rs`).
fn write_fixture_file(file_dir: &Path) -> (PathBuf, String) {
    let file = file_dir.join("main.rs");
    std::fs::write(&file, "fn main() {\n    foo();\n}\n").unwrap();
    let canonical = std::fs::canonicalize(&file).unwrap();
    let uri = hume_lsp::uri::path_to_uri(&canonical)
        .unwrap()
        .as_str()
        .to_string();
    (file, uri)
}

/// Same shape as `lsp_hover.rs::setup`: a real opened file (three lines, so
/// a `Location` can point at a different line for the jump-back test),
/// driven handshake (so `lsp-capabilities` decodes), the real `core:lsp`
/// plugin loaded in place.
fn setup(
    file: &Path,
    tmp: &Path,
    configure: impl FnOnce(&mut InlineLspBackend, ServerId),
) -> (Editor, RealRuntimeGuard, ServerId) {
    let guard = RealRuntimeGuard::new();

    let mut ed = Editor::open(None, std::sync::Arc::new(|| {})).unwrap();

    let mut backend = InlineLspBackend::new();
    backend.respond_to(
        "initialize",
        serde_json::json!({"capabilities": {
            "definitionProvider": true, "declarationProvider": true,
            "typeDefinitionProvider": true, "implementationProvider": true
        }}),
    );
    let sid = backend
        .start("rust-analyzer", &[], Path::new("."), &[])
        .unwrap();
    configure(&mut backend, sid);
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

    (ed, guard, sid)
}

fn run_goto(ed: &mut Editor, cmd: &str) {
    type_cmd(ed, cmd);
    // Drain the Command-mode entry/exit's on-mode-change hooks now (mirrors
    // the real interactive loop, which drains after every keystroke) before
    // the async response arrives — same ordering fix as lsp_hover.rs.
    ed.settle();
    ed.drain_lsp();
    ed.settle();
}

fn loc(uri: &str, line: u64, character: u64) -> serde_json::Value {
    serde_json::json!({
        "uri": uri,
        "range": {"start": {"line": line, "character": character}, "end": {"line": line, "character": character}}
    })
}

fn location_link(uri: &str, line: u64, character: u64) -> serde_json::Value {
    serde_json::json!({
        "targetUri": uri,
        "targetRange": {"start": {"line": line, "character": character}, "end": {"line": line, "character": character + 3}},
        "targetSelectionRange": {"start": {"line": line, "character": character}, "end": {"line": line, "character": character + 3}}
    })
}

#[test]
fn null_result_reports_no_definition_found() {
    let tmp = safe_tempdir();
    let file_dir = safe_tempdir();
    let (file, _uri) = write_fixture_file(file_dir.path());
    let (mut ed, _guard, _sid) = setup(&file, tmp.path(), |backend, _sid| {
        backend.respond_to("textDocument/definition", serde_json::Value::Null);
    });

    run_goto(&mut ed, ":lsp-goto-definition");

    let msg = ed.state.status_msg.clone().unwrap_or_default();
    assert!(
        msg.to_lowercase().contains("no definition found"),
        "expected a no-definition message, got {msg:?}"
    );
}

#[test]
fn single_location_hashmap_jumps_directly() {
    let tmp = safe_tempdir();
    let file_dir = safe_tempdir();
    let (file, uri) = write_fixture_file(file_dir.path());
    let (mut ed, _guard, _sid) = setup(&file, tmp.path(), |backend, _sid| {
        backend.respond_to("textDocument/definition", loc(&uri, 1, 4));
    });

    run_goto(&mut ed, ":lsp-goto-definition");

    assert_eq!(
        ed.doc()
            .text()
            .char_to_line(ed.current_selections().primary().head()),
        1,
        "a single Location must jump directly to line 1 (0-indexed)"
    );
}

#[test]
fn single_element_array_jumps_directly() {
    let tmp = safe_tempdir();
    let file_dir = safe_tempdir();
    let (file, uri) = write_fixture_file(file_dir.path());
    let (mut ed, _guard, _sid) = setup(&file, tmp.path(), |backend, _sid| {
        backend.respond_to(
            "textDocument/definition",
            serde_json::json!([loc(&uri, 1, 4)]),
        );
    });

    run_goto(&mut ed, ":lsp-goto-definition");

    assert_eq!(
        ed.doc()
            .text()
            .char_to_line(ed.current_selections().primary().head()),
        1,
        "a length-1 Location[] must jump directly, not open the drawer"
    );
    assert!(ed.state.config.drawer.is_none());
}

#[test]
fn multi_element_array_opens_the_drawer_and_row_select_jumps() {
    let tmp = safe_tempdir();
    let file_dir = safe_tempdir();
    let (file, uri) = write_fixture_file(file_dir.path());
    let (mut ed, _guard, _sid) = setup(&file, tmp.path(), |backend, _sid| {
        backend.respond_to(
            "textDocument/definition",
            serde_json::json!([loc(&uri, 0, 0), loc(&uri, 1, 4), loc(&uri, 2, 0)]),
        );
    });

    run_goto(&mut ed, ":lsp-goto-definition");

    let rows = {
        let guard = ed.state.drawer_view.read().unwrap();
        guard
            .as_ref()
            .expect("drawer must open for a multi-entry array")
            .rows
            .clone()
    };
    assert_eq!(rows.len(), 3);

    // Select row index 1 (the second entry, line 1).
    ed.handle_key(key('j'));
    ed.handle_key(key_enter());
    ed.settle();

    assert_eq!(
        ed.doc()
            .text()
            .char_to_line(ed.current_selections().primary().head()),
        1,
        "selecting row 2 in the drawer must jump to that entry's line"
    );
}

/// A Windows drive-letter `file://` URI (`file:///C:/...`) must display in
/// the drawer without a leading `/` before the drive letter —
/// `lsp/uri->display-path`'s plain 7-char scheme strip alone leaves one in
/// ("/C:/foo"), not a valid Windows path. The second location's file need
/// not exist: opening the drawer only formats row labels, it doesn't touch
/// the filesystem (only selecting a row and jumping would).
#[test]
fn windows_drive_letter_uri_displays_without_leading_slash() {
    let tmp = safe_tempdir();
    let file_dir = safe_tempdir();
    let (file, uri) = write_fixture_file(file_dir.path());
    let win_uri = "file:///C:/Users/x/main.rs";
    let (mut ed, _guard, _sid) = setup(&file, tmp.path(), |backend, _sid| {
        backend.respond_to(
            "textDocument/definition",
            serde_json::json!([loc(&uri, 0, 0), loc(win_uri, 1, 0)]),
        );
    });

    run_goto(&mut ed, ":lsp-goto-definition");

    let rows = {
        let guard = ed.state.drawer_view.read().unwrap();
        guard
            .as_ref()
            .expect("drawer must open for a multi-entry array")
            .rows
            .clone()
    };
    assert_eq!(rows.len(), 2);
    assert!(
        rows[1].starts_with("C:/Users/x/main.rs"),
        "Windows drive-letter URI must display without a leading '/' \
         before the drive letter, got {:?}",
        rows[1]
    );
}

/// The multi-entry-array case exercises `lsp-locations->display-parts`'s
/// `LocationLink` decoding (`hume_lsp::location::decode_location`'s
/// `targetUri`/`targetSelectionRange` branch), unlike the single-entry jump
/// tests above, which reach the same decoder through `goto-location!`
/// instead.
#[test]
fn multi_element_location_link_array_opens_the_drawer_and_row_select_jumps() {
    let tmp = safe_tempdir();
    let file_dir = safe_tempdir();
    let (file, uri) = write_fixture_file(file_dir.path());
    let (mut ed, _guard, _sid) = setup(&file, tmp.path(), |backend, _sid| {
        backend.respond_to(
            "textDocument/definition",
            serde_json::json!([
                location_link(&uri, 0, 0),
                location_link(&uri, 1, 4),
                location_link(&uri, 2, 0)
            ]),
        );
    });

    run_goto(&mut ed, ":lsp-goto-definition");

    let rows = {
        let guard = ed.state.drawer_view.read().unwrap();
        guard
            .as_ref()
            .expect("drawer must open for a multi-entry LocationLink array")
            .rows
            .clone()
    };
    assert_eq!(rows.len(), 3);
    assert!(
        rows[1].ends_with(":2:5"),
        "row text must be built from targetSelectionRange (1-based line:col), got {:?}",
        rows[1]
    );

    // Select row index 1 (the second entry, line 1).
    ed.handle_key(key('j'));
    ed.handle_key(key_enter());
    ed.settle();

    assert_eq!(
        ed.doc()
            .text()
            .char_to_line(ed.current_selections().primary().head()),
        1,
        "selecting a LocationLink drawer row must jump to targetSelectionRange's line"
    );
}

#[test]
fn location_link_array_prefers_target_selection_range() {
    let tmp = safe_tempdir();
    let file_dir = safe_tempdir();
    let (file, uri) = write_fixture_file(file_dir.path());
    let (mut ed, _guard, _sid) = setup(&file, tmp.path(), |backend, _sid| {
        backend.respond_to(
            "textDocument/definition",
            serde_json::json!([location_link(&uri, 1, 4)]),
        );
    });

    run_goto(&mut ed, ":lsp-goto-definition");

    assert_eq!(
        ed.doc()
            .text()
            .char_to_line(ed.current_selections().primary().head()),
        1,
        "a single-entry LocationLink[] must jump using targetSelectionRange"
    );
}

#[test]
fn jump_back_returns_to_the_origin_after_a_jump() {
    let tmp = safe_tempdir();
    let file_dir = safe_tempdir();
    let (file, uri) = write_fixture_file(file_dir.path());
    let (mut ed, _guard, _sid) = setup(&file, tmp.path(), |backend, _sid| {
        backend.respond_to("textDocument/definition", loc(&uri, 1, 4));
    });
    let before = state(&ed);

    run_goto(&mut ed, ":lsp-goto-definition");
    assert_ne!(
        state(&ed),
        before,
        "sanity: the jump must have moved the cursor"
    );

    ed.handle_key(key_ctrl('o'));
    assert_eq!(
        state(&ed),
        before,
        "Ctrl+o must return to the pre-jump position"
    );
}

/// `goto-location!`'s wire (`Location` hashmap) path opens the target file
/// via `lsp::edits::resolve_or_open` → `buffer::lifecycle::
/// open_or_dedup_and_notify` when it isn't already open — which can't detect
/// language inline (see that function's doc), so it queues the buffer onto
/// `EditorState.pending_language_detection`, drained at the tail of
/// `apply_script_effects` once this eval (`run_goto`'s `:lsp-goto-definition`
/// dispatch) returns.
///
/// Fail oracle: revert `resolve_or_open` to call the bare (pre-fix)
/// `lifecycle::open_or_dedup` — the newly-opened buffer never gets a
/// `language`.
#[test]
fn goto_to_an_unopened_file_detects_its_language() {
    let tmp = safe_tempdir();
    let file_dir = safe_tempdir();
    let (file, _uri) = write_fixture_file(file_dir.path());
    let other_file = file_dir.path().join("other.rs");
    std::fs::write(&other_file, "fn other() {}\n").unwrap();
    let other_canonical = std::fs::canonicalize(&other_file).unwrap();
    let other_uri = hume_lsp::uri::path_to_uri(&other_canonical)
        .unwrap()
        .as_str()
        .to_string();

    let (mut ed, _guard, _sid) = setup(&file, tmp.path(), move |backend, _sid| {
        backend.respond_to("textDocument/definition", loc(&other_uri, 0, 3));
    });
    ed.state
        .config
        .languages
        .register_identity_no_rebuild("rust", &["rs"], &[], &[], None);
    ed.state
        .config
        .languages
        .rebuild_glob_set()
        .expect("rebuild ok");

    run_goto(&mut ed, ":lsp-goto-definition");

    let bid = ed
        .state
        .buffers
        .find_by_path(&other_canonical)
        .expect("goto-location! must have opened the target file");
    assert_eq!(
        ed.state.buffers.get(bid).language,
        ed.state.config.languages.id_of("rust"),
        "the goto-opened file must have its language detected"
    );
}

/// A target whose path genuinely can't be opened (here: it's a directory,
/// not a file — `Buffer::from_file_or_new` only tolerates `NotFound`) must
/// still error and leave the cursor untouched.
#[test]
fn goto_target_is_directory_errors_without_moving_the_cursor() {
    let tmp = safe_tempdir();
    let file_dir = safe_tempdir();
    let (file, _uri) = write_fixture_file(file_dir.path());
    let dir_uri = hume_lsp::uri::path_to_uri(&std::fs::canonicalize(tmp.path()).unwrap())
        .unwrap()
        .as_str()
        .to_string();
    let (mut ed, _guard, _sid) = setup(&file, tmp.path(), |backend, _sid| {
        backend.respond_to("textDocument/definition", loc(&dir_uri, 0, 0));
    });
    let before = state(&ed);

    run_goto(&mut ed, ":lsp-goto-definition");

    assert_eq!(state(&ed), before, "a failed goto must not move the cursor");
    let msg = ed.state.status_msg.clone().unwrap_or_default();
    assert!(
        msg.to_lowercase().contains("error") || msg.to_lowercase().contains("goto-location"),
        "expected an error message, got {msg:?}"
    );
}

/// A target whose file doesn't exist yet (but whose path is otherwise valid)
/// must open a new-file buffer and jump to it — the same `:e newfile.txt`
/// tolerance `resolve_or_open` shares with `Editor::resolve_open_path` via
/// `Buffer::from_file_or_new`, not an error. Covers a server-driven
/// definition/rename that points at a file it expects the client to create.
#[test]
fn goto_missing_target_opens_new_file_buffer_and_jumps_to_it() {
    let tmp = safe_tempdir();
    let file_dir = safe_tempdir();
    let (file, _uri) = write_fixture_file(file_dir.path());
    let missing = file_dir.path().join("not_yet_created.rs");
    let missing_uri = format!("file://{}", missing.display());
    let (mut ed, _guard, _sid) = setup(&file, tmp.path(), |backend, _sid| {
        backend.respond_to("textDocument/definition", loc(&missing_uri, 0, 0));
    });
    let start_bid = ed.focused_buffer_id();

    run_goto(&mut ed, ":lsp-goto-definition");

    assert_ne!(
        ed.focused_buffer_id(),
        start_bid,
        "goto must have switched to the new-file buffer"
    );
    assert!(
        ed.doc().is_new_file(),
        "target buffer must be a pending new-file buffer, not an error"
    );
    let msg = ed.state.status_msg.clone().unwrap_or_default();
    assert!(
        !msg.to_lowercase().contains("error"),
        "must not report an error, got {msg:?}"
    );
}

/// The same malformed-location rule `lsp-locations->display-parts` enforces
/// (SPEC.md's Q33b) applies to `goto-location!` too, through the shared
/// `hume_lsp::location::decode_location`: a `Location` missing `range` must
/// error rather than silently jumping to line 0.
///
/// Sabotage oracle: loosening `decode_location` to tolerate a missing
/// `range` would fail this test *and*
/// `column_display_agreement.rs`'s
/// `a_malformed_location_aborts_the_batch_instead_of_a_degraded_row`,
/// proving both paths share the one decoder.
#[test]
fn location_missing_range_errors_instead_of_jumping() {
    let tmp = safe_tempdir();
    let file_dir = safe_tempdir();
    let (file, uri) = write_fixture_file(file_dir.path());
    let (mut ed, _guard, _sid) = setup(&file, tmp.path(), |backend, _sid| {
        backend.respond_to("textDocument/definition", serde_json::json!({"uri": uri}));
    });
    let before = state(&ed);

    run_goto(&mut ed, ":lsp-goto-definition");

    assert_eq!(
        state(&ed),
        before,
        "a malformed location must not move the cursor"
    );
    let msg = ed.state.status_msg.clone().unwrap_or_default();
    assert!(
        msg.contains("goto-location!") && msg.contains("missing range"),
        "expected an error naming the builtin and the missing field, got {msg:?}"
    );
}

#[test]
fn each_command_sends_its_own_method() {
    // Wiring smoke test: script a distinctive response only for the exact
    // method each command should send. If a command sent the wrong method,
    // no response would be queued for it and the jump would never happen.
    for (cmd, method) in [
        ("lsp-goto-definition", "textDocument/definition"),
        ("lsp-goto-declaration", "textDocument/declaration"),
        ("lsp-goto-type-definition", "textDocument/typeDefinition"),
        ("lsp-goto-implementation", "textDocument/implementation"),
    ] {
        let tmp = safe_tempdir();
        let file_dir = safe_tempdir();
        let (file, uri) = write_fixture_file(file_dir.path());
        let (mut ed, _guard, _sid) = setup(&file, tmp.path(), |backend, _sid| {
            backend.respond_to(method, loc(&uri, 1, 4));
        });

        run_goto(&mut ed, &format!(":{cmd}"));

        assert_eq!(
            ed.doc()
                .text()
                .char_to_line(ed.current_selections().primary().head()),
            1,
            "{cmd} must send {method} and jump on its response"
        );
    }
}

/// A goto-definition landing on a different, unopened file is a
/// non-interactive `switch-to-buffer!` (via `goto-location!`) — SPEC.md §7's
/// C5 table names this write path specifically, since it never runs through
/// `type_cmd`/`feed_key` at all; the switch happens inside `drain_lsp`'s
/// response handling. Must raise exactly one `OnBufferEnter`, same as every
/// other focus-changing action.
///
/// Fail oracle: `goto-location!`'s buffer switch bypassing `settle()`'s
/// diff (a direct raise wired only into typed/keyed commands) would leave
/// the trace log empty; a duplicate raise on the same switch would produce
/// more than one entry.
#[test]
fn goto_into_another_file_raises_exactly_one_on_buffer_enter() {
    let tmp = safe_tempdir();
    let file_dir = safe_tempdir();
    let (file, _uri) = write_fixture_file(file_dir.path());
    let other_file = file_dir.path().join("other.rs");
    std::fs::write(&other_file, "fn other() {}\n").unwrap();
    let other_canonical = std::fs::canonicalize(&other_file).unwrap();
    let other_uri = hume_lsp::uri::path_to_uri(&other_canonical)
        .unwrap()
        .as_str()
        .to_string();

    let (mut ed, _guard, _sid) = setup(&file, tmp.path(), move |backend, _sid| {
        backend.respond_to("textDocument/definition", loc(&other_uri, 0, 3));
    });

    // Drain the startup OnBufferEnter (no handler registered yet, so
    // nothing gets logged for it) before installing the counting handler.
    ed.settle();
    let mut host = ed.scripting.take().expect("setup() installs a host");
    eval_with_real_host(
        &mut ed,
        &mut host,
        r#"(register-hook! 'on-buffer-enter (lambda (bid) (log! 'trace "entered")))"#,
        tmp.path(),
    );
    ed.scripting = Some(host);

    run_goto(&mut ed, ":lsp-goto-definition");

    let bid = ed
        .state
        .buffers
        .find_by_path(&other_canonical)
        .expect("goto-location! must have opened and switched to the target file");
    assert_eq!(
        ed.focused_buffer_id(),
        bid,
        "sanity: the goto must have landed on the other file"
    );
    let entered = ed
        .state
        .message_log
        .entries()
        .filter(|e| e.severity == Severity::Trace && e.text == "entered")
        .count();
    assert_eq!(
        entered, 1,
        "a goto-definition switch into another file must raise exactly one OnBufferEnter"
    );
}
