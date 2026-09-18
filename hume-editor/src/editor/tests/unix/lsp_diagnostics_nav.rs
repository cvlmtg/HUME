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

/// Everything `setup` builds and keeps alive for the test's duration — a
/// struct, not a tuple, so the next field added doesn't churn every call
/// site (the same reason `super::DiagSetup` is a struct). `_guard` must
/// stay explicitly bound (it holds the env lock); `sid` is `Copy`, so a
/// caller that doesn't need it may drop it with `..`.
struct NavSetup {
    ed: Editor,
    _guard: RealRuntimeGuard,
    sid: hume_lsp::backend::ServerId,
}

/// Builds the `publishDiagnostics` notification for `file` — shared by
/// `setup` (pushed at the backend, drained via `drain_lsp`) and `republish`
/// (dispatched straight through the production single-shot path), so the
/// two can't drift on params shape.
fn publish_msg(file: &Path, diags: &[DiagFixture]) -> hume_lsp::codec::Message {
    let uri = hume_lsp::uri::path_to_uri(file).unwrap();
    publish_diagnostics_notification(uri.as_str(), diags)
}

/// Fixture buffer: "aa\nbb\ncc\ndd\n" — char offsets: line0 'aa' = 0..2,
/// line1 'bb' = 3..5, line2 'cc' = 6..8, line3 'dd' = 9..11. Diagnostic A
/// covers 'bb' (char start 3); diagnostic B covers 'dd' (char start 9) —
/// leaves line0 genuinely "before A" and line2 genuinely "between A and B".
fn setup(file: &Path, tmp: &Path, diags: &[DiagFixture]) -> NavSetup {
    let guard = RealRuntimeGuard::new();
    std::fs::write(file, "aa\nbb\ncc\ndd\n").unwrap();

    let mut backend = InlineLspBackend::new();
    let sid = backend
        .start("rust-analyzer", &[], Path::new("."), &[])
        .unwrap();
    if !diags.is_empty() {
        backend.push_from_server(sid, publish_msg(file, diags));
    }

    let mut ed = Editor::open(None, std::sync::Arc::new(|| {})).unwrap();
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

    NavSetup {
        ed,
        _guard: guard,
        sid,
    }
}

/// The open drawer's rows — every drawer assertion reads through this, so
/// none reaches into `views.drawer` by hand.
fn drawer_rows(ed: &Editor) -> Vec<String> {
    let guard = ed.state.views.drawer.read();
    guard.as_ref().expect("drawer must be open").rows.to_vec()
}

/// Republishes diagnostics for `file` through the production single-shot
/// path (`dispatch_lsp_action`, the same ingest + `queue_diagnostics_changed`
/// pair `drain_lsp`'s batch loop runs) and settles, so the queued
/// `on-diagnostics-changed` hook — including the drawer's own refresh —
/// has run by the time this returns.
fn republish(
    ed: &mut Editor,
    sid: hume_lsp::backend::ServerId,
    file: &Path,
    diags: &[DiagFixture],
) {
    let hume_lsp::codec::Message::Notification { params, .. } = publish_msg(file, diags) else {
        panic!("publish_msg must build a Notification");
    };
    let params: lsp_types::PublishDiagnosticsParams = serde_json::from_value(params).unwrap();
    ed.dispatch_lsp_action(sid, hume_lsp::client::ClientAction::Diagnostics(params));
    ed.settle();
}

/// Dispatches `goto-next-diagnostic`/`goto-prev-diagnostic` — key-bindable,
/// not typed — through the keymap pipeline, the way their bound keys
/// (`g n`/`g p`) would. `:diagnostics` (typed) dispatches via `type_cmd`
/// directly at its one call site instead.
fn run(ed: &mut Editor, cmd: &str) {
    ed.execute_keymap_command(cmd.to_owned().into(), Some(1), false);
    ed.settle();
}

const DIAG_A: DiagFixture = ((1, 0), (1, 2), 1, "problem A");
const DIAG_B: DiagFixture = ((3, 0), (3, 2), 2, "problem B");

#[test]
fn next_from_before_a_jumps_to_a() {
    let tmp = safe_tempdir();
    let file_dir = safe_tempdir();
    let NavSetup { mut ed, _guard, .. } = setup(
        &file_dir.path().join("main.rs"),
        tmp.path(),
        &[DIAG_A, DIAG_B],
    );
    set_cursor(&mut ed, 0);

    run(&mut ed, "goto-next-diagnostic");

    assert_eq!(
        ed.current_selections().primary().head(),
        co(3),
        "must land on diagnostic A's start"
    );
}

#[test]
fn next_from_as_start_of_a_jumps_to_b_not_a() {
    let tmp = safe_tempdir();
    let file_dir = safe_tempdir();
    let NavSetup { mut ed, _guard, .. } = setup(
        &file_dir.path().join("main.rs"),
        tmp.path(),
        &[DIAG_A, DIAG_B],
    );
    set_cursor(&mut ed, 3); // sitting exactly on A's start

    run(&mut ed, "goto-next-diagnostic");

    assert_eq!(
        ed.current_selections().primary().head(),
        co(9),
        "sitting on A must advance to B, not stay on A (next = strictly-after start)"
    );
}

#[test]
fn next_from_after_b_wraps_to_a() {
    let tmp = safe_tempdir();
    let file_dir = safe_tempdir();
    let NavSetup { mut ed, _guard, .. } = setup(
        &file_dir.path().join("main.rs"),
        tmp.path(),
        &[DIAG_A, DIAG_B],
    );
    set_cursor(&mut ed, 11); // past both diagnostics

    run(&mut ed, "goto-next-diagnostic");

    assert_eq!(
        ed.current_selections().primary().head(),
        co(3),
        "must wrap around to A"
    );
}

#[test]
fn prev_from_after_b_jumps_to_b() {
    let tmp = safe_tempdir();
    let file_dir = safe_tempdir();
    let NavSetup { mut ed, _guard, .. } = setup(
        &file_dir.path().join("main.rs"),
        tmp.path(),
        &[DIAG_A, DIAG_B],
    );
    set_cursor(&mut ed, 11);

    run(&mut ed, "goto-prev-diagnostic");

    assert_eq!(
        ed.current_selections().primary().head(),
        co(9),
        "must land on diagnostic B's start"
    );
}

#[test]
fn prev_from_before_a_wraps_to_b() {
    let tmp = safe_tempdir();
    let file_dir = safe_tempdir();
    let NavSetup { mut ed, _guard, .. } = setup(
        &file_dir.path().join("main.rs"),
        tmp.path(),
        &[DIAG_A, DIAG_B],
    );
    set_cursor(&mut ed, 0);

    run(&mut ed, "goto-prev-diagnostic");

    assert_eq!(
        ed.current_selections().primary().head(),
        co(9),
        "must wrap around to B (the last entry)"
    );
}

#[test]
fn empty_buffer_reports_no_diagnostics() {
    let tmp = safe_tempdir();
    let file_dir = safe_tempdir();
    let NavSetup { mut ed, _guard, .. } = setup(&file_dir.path().join("main.rs"), tmp.path(), &[]);
    let before = state(&ed);

    run(&mut ed, "goto-next-diagnostic");

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
    let NavSetup { mut ed, _guard, .. } = setup(
        &file_dir.path().join("main.rs"),
        tmp.path(),
        &[DIAG_A, DIAG_B],
    );

    type_cmd(&mut ed, ":diagnostics");
    ed.settle();

    let rows = drawer_rows(&ed);
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

    ed.handle_key(key_ctrl('d'));
    ed.handle_key(key_enter());
    ed.settle();
    assert_eq!(
        ed.current_selections().primary().head(),
        co(9),
        "selecting row 2 (B) in the drawer must jump to B's start"
    );
}

// ── Live refresh: the drawer follows corrected publishes ────────────────────
// The drawer freezes its rows at open time; the plugin rebuilds them on
// every `on-diagnostics-changed` for its buffer (see
// `runtime/plugins/core/lsp/diagnostics.scm`). These tests drive the whole
// path: scripted publish → store → hook → `update-drawer-list!`.

const DIAG_C: DiagFixture = ((2, 0), (2, 2), 1, "problem C");

/// The reported bug: fixing an error updated the statusline count but left
/// the open drawer showing the stale list.
#[test]
fn drawer_refreshes_rows_when_a_diagnostic_is_fixed() {
    let tmp = safe_tempdir();
    let file_dir = safe_tempdir();
    let file = file_dir.path().join("main.rs");
    let NavSetup {
        mut ed,
        _guard,
        sid,
    } = setup(&file, tmp.path(), &[DIAG_A, DIAG_B]);

    type_cmd(&mut ed, ":diagnostics");
    ed.settle();
    assert_eq!(drawer_rows(&ed).len(), 2, "sanity: both rows listed");

    republish(&mut ed, sid, &file, &[DIAG_B]);

    let rows = drawer_rows(&ed);
    assert_eq!(rows.len(), 1, "the fixed diagnostic must disappear");
    assert!(
        rows[0].contains("problem B"),
        "the surviving diagnostic must remain: {rows:?}"
    );
}

/// Three rows, middle one selected, first one fixed: the selection must
/// follow the surviving diagnostic by identity (message + severity), not by
/// index — a plain index clamp would land on B here instead of C.
#[test]
fn drawer_keeps_selection_on_the_surviving_diagnostic() {
    let tmp = safe_tempdir();
    let file_dir = safe_tempdir();
    let file = file_dir.path().join("main.rs");
    let NavSetup {
        mut ed,
        _guard,
        sid,
    } = setup(&file, tmp.path(), &[DIAG_A, DIAG_C, DIAG_B]);

    type_cmd(&mut ed, ":diagnostics");
    ed.settle();
    ed.handle_key(key_ctrl('d')); // select C (row 1 of [A, C, B])

    republish(&mut ed, sid, &file, &[DIAG_C, DIAG_B]);

    let rows = drawer_rows(&ed);
    assert_eq!(rows.len(), 2);
    assert!(rows[0].contains("problem C"), "C must still lead: {rows:?}");
    assert_eq!(
        ed.state.input.drawer().unwrap().selected,
        0,
        "selection must follow C to its new index, not stay at 1 (B)"
    );
}

/// The selected diagnostic itself fixed: the selection moves to the item now
/// at that position — the next one — rather than tracking a stale index.
#[test]
fn drawer_moves_selection_to_next_when_the_selected_diagnostic_is_fixed() {
    let tmp = safe_tempdir();
    let file_dir = safe_tempdir();
    let file = file_dir.path().join("main.rs");
    let NavSetup {
        mut ed,
        _guard,
        sid,
    } = setup(&file, tmp.path(), &[DIAG_A, DIAG_C, DIAG_B]);

    type_cmd(&mut ed, ":diagnostics");
    ed.settle();
    ed.handle_key(key_ctrl('d')); // select C (row 1 of [A, C, B])

    republish(&mut ed, sid, &file, &[DIAG_A, DIAG_B]);

    let rows = drawer_rows(&ed);
    assert_eq!(rows.len(), 2);
    assert_eq!(
        ed.state.input.drawer().unwrap().selected,
        1,
        "C is gone — selection must sit on B, the next item: {rows:?}"
    );
    assert!(rows[1].contains("problem B"), "row 1 must be B: {rows:?}");
}

/// Fixing the last error auto-closes the drawer instead of leaving an empty
/// (or stale) list behind.
#[test]
fn drawer_closes_when_all_diagnostics_are_fixed() {
    let tmp = safe_tempdir();
    let file_dir = safe_tempdir();
    let file = file_dir.path().join("main.rs");
    let NavSetup {
        mut ed,
        _guard,
        sid,
    } = setup(&file, tmp.path(), &[DIAG_A, DIAG_B]);

    type_cmd(&mut ed, ":diagnostics");
    ed.settle();
    assert!(ed.state.input.drawer().is_some(), "sanity: drawer open");

    republish(&mut ed, sid, &file, &[]);

    assert!(
        ed.state.input.drawer().is_none(),
        "an empty publish must close the drawer"
    );
    assert!(
        ed.state.views.drawer.read().is_none(),
        "the view must follow the closed model"
    );
}

/// A foreign replace must kill tracking: another owner's
/// `show-drawer-list!` fires `#f` to our callback at the current
/// generation, so the next publish for our buffer must leave the foreign
/// rows alone instead of refreshing them.
#[test]
fn foreign_replace_kills_refresh_tracking() {
    let tmp = safe_tempdir();
    let file_dir = safe_tempdir();
    let file = file_dir.path().join("main.rs");
    let NavSetup {
        mut ed,
        _guard,
        sid,
    } = setup(&file, tmp.path(), &[DIAG_A, DIAG_B]);

    type_cmd(&mut ed, ":diagnostics");
    ed.settle();

    // Evaled against the live host, not a fresh one: `run` would replace
    // `ed.scripting` and drop the refresh hook under test itself.
    let mut scripting = ed.scripting.take().expect("setup installs scripting");
    eval_with_real_host(
        &mut ed,
        &mut scripting,
        r#"(define-typed-command! "foreign" "" (lambda ()
             (show-drawer-list! (list "foreign") (lambda (idx) (void)))))"#,
        tmp.path(),
    );
    ed.scripting = Some(scripting);
    type_cmd(&mut ed, ":foreign");
    ed.settle();
    assert_eq!(drawer_rows(&ed), vec!["foreign".to_string()]);

    republish(&mut ed, sid, &file, &[DIAG_B]);

    assert_eq!(
        drawer_rows(&ed),
        vec!["foreign".to_string()],
        "tracking died with the replace — our publish must not refresh foreign rows"
    );
}

/// Re-running `:diagnostics` replaces the drawer — the replace fires `#f`
/// to the outgoing callback, but with a stale generation, so the plugin's
/// open-tracking must survive it and the next publish must still refresh.
#[test]
fn rerunning_diagnostics_keeps_refresh_tracking_alive() {
    let tmp = safe_tempdir();
    let file_dir = safe_tempdir();
    let file = file_dir.path().join("main.rs");
    let NavSetup {
        mut ed,
        _guard,
        sid,
    } = setup(&file, tmp.path(), &[DIAG_A, DIAG_B]);

    type_cmd(&mut ed, ":diagnostics");
    ed.settle();
    type_cmd(&mut ed, ":diagnostics");
    ed.settle();

    republish(&mut ed, sid, &file, &[DIAG_B]);

    let rows = drawer_rows(&ed);
    assert_eq!(
        rows.len(),
        1,
        "the replace's own stale #f must not have killed tracking: {rows:?}"
    );
    assert!(rows[0].contains("problem B"));
}
