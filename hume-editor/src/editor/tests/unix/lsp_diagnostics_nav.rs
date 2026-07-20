// Diagnostics navigation: `goto-next-diagnostic`,
// `goto-prev-diagnostic`, `:diagnostics` drawer. No LSP request — reads the
// diagnostics store via `diagnostics-for-buffer`. Depends on `core:stdlib`
// (`stdlib/cursor-char-index`), loaded alongside `core:lsp` via
// `RealRuntimeGuard` (both resolve from the real on-disk runtime/ dir).
//
// Not on Windows: Scheme require strings embed OS paths; backslashes are not
// escaped in Steel string literals (same constraint as tests/plugins.rs).

use std::path::Path;

use super::*;
use crate::editor::lsp::LspState;
use hume_lsp::backend::LspBackend;
use hume_lsp::client::LspClient;
use hume_lsp::inline::InlineLspBackend;
use hume_scripting::ScriptingHost;

/// `((start_line, start_char), (end_line, end_char), severity, message)`.
type DiagFixture<'a> = ((u32, u32), (u32, u32), i64, &'a str);

fn publish_diagnostics_notification(uri: &str, diags: &[DiagFixture]) -> hume_lsp::codec::Message {
    let diagnostics: Vec<serde_json::Value> = diags
        .iter()
        .map(|((sl, sc), (el, ec), sev, msg)| {
            serde_json::json!({
                "range": {"start": {"line": sl, "character": sc}, "end": {"line": el, "character": ec}},
                "severity": sev,
                "message": msg,
            })
        })
        .collect();
    hume_lsp::codec::Message::Notification {
        method: "textDocument/publishDiagnostics".to_string(),
        params: serde_json::json!({"uri": uri, "diagnostics": diagnostics}),
    }
}

/// Fixture buffer: "aa\nbb\ncc\ndd\n" — char offsets: line0 'aa' = 0..2,
/// line1 'bb' = 3..5, line2 'cc' = 6..8, line3 'dd' = 9..11. Diagnostic A
/// covers 'bb' (char start 3); diagnostic B covers 'dd' (char start 9) —
/// leaves line0 genuinely "before A" and line2 genuinely "between A and B".
fn setup(file: &Path, tmp: &Path, diags: &[DiagFixture]) -> (Editor, RealRuntimeGuard) {
    let guard = RealRuntimeGuard::new();
    std::fs::write(file, "aa\nbb\ncc\ndd\n").unwrap();

    let mut backend = InlineLspBackend::new();
    let sid = backend.start("rust-analyzer", &[], Path::new(".")).unwrap();
    let uri = hume_lsp::uri::path_to_uri(file).unwrap();
    if !diags.is_empty() {
        backend.push_from_server(sid, publish_diagnostics_notification(uri.as_str(), diags));
    }

    let mut ed = Editor::open(None).unwrap();
    ed.lsp = LspState::from_backend_for_test(Box::new(backend));
    ed.lsp
        .insert_client_for_test(LspClient::new(sid, file.parent().unwrap().to_path_buf()));
    ed.execute_typed("e", Some(file.to_str().unwrap())).unwrap();
    ed.drain_lsp();

    let mut host = ScriptingHost::new();
    eval_with_real_host(
        &mut ed,
        &mut host,
        r#"(load-plugin "core:stdlib") (load-plugin "core:lsp")"#,
        tmp,
    );
    ed.scripting = Some(host);

    (ed, guard)
}

fn run(ed: &mut Editor, cmd: &str) {
    type_cmd(ed, cmd);
    ed.drain_hooks();
    ed.drain_pending_steel_calls();
}

fn set_cursor(ed: &mut Editor, char_offset: usize) {
    let bid = ed.focused_buffer_id();
    let pid = ed.state.focused_pane_id;
    let pbs = ed
        .state
        .panes
        .state
        .get_mut(pid)
        .and_then(|by_buf| by_buf.get_mut(bid))
        .expect("pane buffer state must exist");
    pbs.selections = hume_editing::selection::SelectionSet::single(
        hume_editing::selection::Selection::collapsed(char_offset),
    );
}

const DIAG_A: DiagFixture = ((1, 0), (1, 2), 1, "problem A");
const DIAG_B: DiagFixture = ((3, 0), (3, 2), 2, "problem B");

#[test]
fn next_from_before_a_jumps_to_a() {
    let tmp = safe_tempdir();
    let file_dir = safe_tempdir();
    let (mut ed, _guard) = setup(
        &file_dir.path().join("main.rs"),
        tmp.path(),
        &[DIAG_A, DIAG_B],
    );
    set_cursor(&mut ed, 0);

    run(&mut ed, ":goto-next-diagnostic");

    assert_eq!(
        ed.current_selections().primary().head(),
        3,
        "must land on diagnostic A's start"
    );
}

#[test]
fn next_from_as_start_of_a_jumps_to_b_not_a() {
    let tmp = safe_tempdir();
    let file_dir = safe_tempdir();
    let (mut ed, _guard) = setup(
        &file_dir.path().join("main.rs"),
        tmp.path(),
        &[DIAG_A, DIAG_B],
    );
    set_cursor(&mut ed, 3); // sitting exactly on A's start

    run(&mut ed, ":goto-next-diagnostic");

    assert_eq!(
        ed.current_selections().primary().head(),
        9,
        "sitting on A must advance to B, not stay on A (next = strictly-after start)"
    );
}

#[test]
fn next_from_after_b_wraps_to_a() {
    let tmp = safe_tempdir();
    let file_dir = safe_tempdir();
    let (mut ed, _guard) = setup(
        &file_dir.path().join("main.rs"),
        tmp.path(),
        &[DIAG_A, DIAG_B],
    );
    set_cursor(&mut ed, 11); // past both diagnostics

    run(&mut ed, ":goto-next-diagnostic");

    assert_eq!(
        ed.current_selections().primary().head(),
        3,
        "must wrap around to A"
    );
}

#[test]
fn prev_from_after_b_jumps_to_b() {
    let tmp = safe_tempdir();
    let file_dir = safe_tempdir();
    let (mut ed, _guard) = setup(
        &file_dir.path().join("main.rs"),
        tmp.path(),
        &[DIAG_A, DIAG_B],
    );
    set_cursor(&mut ed, 11);

    run(&mut ed, ":goto-prev-diagnostic");

    assert_eq!(
        ed.current_selections().primary().head(),
        9,
        "must land on diagnostic B's start"
    );
}

#[test]
fn prev_from_before_a_wraps_to_b() {
    let tmp = safe_tempdir();
    let file_dir = safe_tempdir();
    let (mut ed, _guard) = setup(
        &file_dir.path().join("main.rs"),
        tmp.path(),
        &[DIAG_A, DIAG_B],
    );
    set_cursor(&mut ed, 0);

    run(&mut ed, ":goto-prev-diagnostic");

    assert_eq!(
        ed.current_selections().primary().head(),
        9,
        "must wrap around to B (the last entry)"
    );
}

#[test]
fn empty_buffer_reports_no_diagnostics() {
    let tmp = safe_tempdir();
    let file_dir = safe_tempdir();
    let (mut ed, _guard) = setup(&file_dir.path().join("main.rs"), tmp.path(), &[]);
    let before = state(&ed);

    run(&mut ed, ":goto-next-diagnostic");

    assert_eq!(state(&ed), before, "no diagnostics means no movement");
    let msg = ed.state.status_msg.clone().unwrap_or_default();
    assert!(
        msg.to_lowercase().contains("no diagnostics"),
        "expected a no-diagnostics message, got {msg:?}"
    );
}

#[test]
fn drawer_lists_severity_glyph_and_message_and_enter_jumps() {
    let tmp = safe_tempdir();
    let file_dir = safe_tempdir();
    let (mut ed, _guard) = setup(
        &file_dir.path().join("main.rs"),
        tmp.path(),
        &[DIAG_A, DIAG_B],
    );

    run(&mut ed, ":diagnostics");

    let rows = {
        let guard = ed.state.drawer_view.read().unwrap();
        guard.as_ref().expect("drawer must open").rows.clone()
    };
    assert_eq!(rows.len(), 2);
    assert!(
        rows[0].contains("problem A"),
        "row must include the diagnostic's message: {rows:?}"
    );
    assert!(
        rows[0].contains('✘'),
        "severity 1 (Error) must render as the error glyph: {rows:?}"
    );
    assert!(rows[1].contains("problem B"));
    assert!(
        rows[1].contains('⚠'),
        "severity 2 (Warning) must render as the warning glyph: {rows:?}"
    );

    ed.handle_key(key('j'));
    ed.handle_key(key_enter());
    ed.drain_pending_steel_calls();
    assert_eq!(
        ed.current_selections().primary().head(),
        9,
        "selecting row 2 (B) in the drawer must jump to B's start"
    );
}
