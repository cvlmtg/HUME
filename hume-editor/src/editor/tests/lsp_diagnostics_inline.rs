// Diagnostics end-of-line inline summary (`set-inline-diagnostics!`, wired
// from `on-diagnostics-changed` in `diagnostics.scm`) and the gn/gp
// dismiss-on-next-key overlay (`show-popup! #:dismiss-on-key`). Same harness
// shape as `lsp_diagnostics_nav.rs`; not on Windows for the same reason
// (Scheme require strings embed OS paths).

use std::path::Path;

use super::*;
use crate::editor::lsp::LspState;
use crate::editor::scripting_setup::make_init_host;
use hume_lsp::backend::LspBackend;
use hume_lsp::client::LspClient;
use hume_lsp::inline::InlineLspBackend;
use hume_scripting::ScriptingHost;

#[cfg(not(windows))]
fn eval_with_real_host(ed: &mut Editor, host: &mut ScriptingHost, source: &str, tmp: &Path) {
    let init_path = tmp.join("init.scm");
    std::fs::write(&init_path, source).unwrap();
    let mut ih = make_init_host(&mut ed.state, &mut ed.view);
    host.eval_init(&init_path, 10_000, &mut ih, Default::default())
        .expect("eval_init");
}

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
/// line1 'bb' = 3..5, line2 'cc' = 6..8, line3 'dd' = 9..11.
///
/// Plugin load happens *before* `drain_lsp()` (unlike
/// `lsp_diagnostics_nav.rs`'s otherwise-identical `setup`) — the inline
/// summary is driven by `on-diagnostics-changed`, which is a queued hook
/// (`fire_hook_silent` → `pending_hooks`, actually invoked by
/// `drain_hooks()`): the handler must be registered by `(load-plugin
/// "core:lsp")` before that queued hook is drained, or the first batch's
/// summary never renders. Nav-only tests don't need this ordering since
/// `goto-next-diagnostic`/`:diagnostics` pull `diagnostics-for-buffer`
/// fresh at call time, independent of the hook.
#[cfg(not(windows))]
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

    let mut host = ScriptingHost::new();
    eval_with_real_host(
        &mut ed,
        &mut host,
        r#"(load-plugin "core:stdlib") (load-plugin "core:lsp")"#,
        tmp,
    );
    ed.scripting = Some(host);

    ed.drain_lsp();
    ed.drain_hooks();

    (ed, guard)
}

#[cfg(not(windows))]
fn run(ed: &mut Editor, cmd: &str) {
    type_cmd(ed, cmd);
    ed.drain_hooks();
    ed.drain_pending_steel_calls();
}

// ── End-of-line inline summary ──────────────────────────────────────────────

#[test]
#[cfg(not(windows))]
fn single_diagnostic_on_a_line_shows_a_bare_message() {
    let tmp = safe_tempdir();
    let file_dir = safe_tempdir();
    // Severity 1 = error, per the LSP DiagnosticSeverity enum.
    let diag: DiagFixture = ((1, 0), (1, 2), 1, "problem A");
    let (ed, _guard) = setup(&file_dir.path().join("main.rs"), tmp.path(), &[diag]);
    let bid = ed.focused_buffer_id();

    let entries = ed.state.decorations.inline_diagnostics_for(bid);
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].line, 1);
    assert_eq!(
        entries[0].text, "problem A",
        "a single diagnostic must not get a '[1]' count prefix"
    );
    assert_eq!(entries[0].scope, "diagnostic.error");
}

#[test]
#[cfg(not(windows))]
fn two_diagnostics_on_the_same_line_show_count_and_leftmost_message() {
    let tmp = safe_tempdir();
    let file_dir = safe_tempdir();
    // Both on line1 ("bb", chars 3..5): D1 at col0 (char3), D2 at col1
    // (char4) — diagnostics-for-buffer is start-ascending, so D1 (leftmost)
    // supplies the message.
    let d1: DiagFixture = ((1, 0), (1, 1), 2, "warn near start");
    let d2: DiagFixture = ((1, 1), (1, 2), 1, "error further right");
    let (ed, _guard) = setup(&file_dir.path().join("main.rs"), tmp.path(), &[d1, d2]);
    let bid = ed.focused_buffer_id();

    let entries = ed.state.decorations.inline_diagnostics_for(bid);
    assert_eq!(entries.len(), 1, "both diagnostics collapse into one entry");
    assert_eq!(entries[0].line, 1);
    assert_eq!(
        entries[0].text, "[2] warn near start",
        "count prefix plus the leftmost (D1) diagnostic's message"
    );
}

#[test]
#[cfg(not(windows))]
fn inline_color_follows_the_highest_severity_on_the_line_not_the_leftmost() {
    let tmp = safe_tempdir();
    let file_dir = safe_tempdir();
    // D1 (leftmost, col0) is a warning; D2 (col1) is an error. The message
    // must still come from D1, but the color must reflect D2's higher
    // severity.
    let d1: DiagFixture = ((1, 0), (1, 1), 2, "warn near start");
    let d2: DiagFixture = ((1, 1), (1, 2), 1, "error further right");
    let (ed, _guard) = setup(&file_dir.path().join("main.rs"), tmp.path(), &[d1, d2]);
    let bid = ed.focused_buffer_id();

    let entries = ed.state.decorations.inline_diagnostics_for(bid);
    assert_eq!(entries.len(), 1);
    assert_eq!(
        entries[0].scope, "diagnostic.error",
        "an error anywhere on the line must win the color, even when the \
         leftmost (message-supplying) diagnostic is only a warning"
    );
}

#[test]
#[cfg(not(windows))]
fn diagnostics_on_different_lines_get_independent_entries() {
    let tmp = safe_tempdir();
    let file_dir = safe_tempdir();
    let diag_a: DiagFixture = ((1, 0), (1, 2), 1, "problem A");
    let diag_b: DiagFixture = ((3, 0), (3, 2), 2, "problem B");
    let (ed, _guard) = setup(
        &file_dir.path().join("main.rs"),
        tmp.path(),
        &[diag_a, diag_b],
    );
    let bid = ed.focused_buffer_id();

    let mut entries: Vec<(usize, String, String)> = ed
        .state
        .decorations
        .inline_diagnostics_for(bid)
        .iter()
        .map(|e| (e.line, e.text.clone(), e.scope.clone()))
        .collect();
    entries.sort_by_key(|(line, _, _)| *line);
    assert_eq!(
        entries,
        vec![
            (1, "problem A".to_string(), "diagnostic.error".to_string()),
            (3, "problem B".to_string(), "diagnostic.warning".to_string()),
        ]
    );
}

// ── gn/gp full-message overlay ───────────────────────────────────────────────

#[test]
#[cfg(not(windows))]
fn goto_next_diagnostic_opens_a_dismiss_on_key_popup_with_the_full_message() {
    let tmp = safe_tempdir();
    let file_dir = safe_tempdir();
    let diag: DiagFixture = ((1, 0), (1, 2), 1, "problem A\nsecond line of detail");
    let (mut ed, _guard) = setup(&file_dir.path().join("main.rs"), tmp.path(), &[diag]);

    // The real `g n` keybinding, not `:goto-next-diagnostic` — invoking via
    // the command line round-trips Command -> Normal mode, firing
    // hover.scm's unconditional `on-mode-change` -> `close-popup!` and
    // wiping the popup this same command just set. `g n` stays in Normal
    // mode throughout, matching real usage.
    ed.feed_key(key('g'));
    ed.feed_key(key('n'));

    let popup = ed.state.popup.as_ref().expect("popup must be shown");
    assert_eq!(
        popup.text, "problem A\nsecond line of detail",
        "the overlay must show the FULL message, not just its first line \
         (unlike the inline summary and the :diagnostics drawer row)"
    );
    assert!(
        popup.dismiss_on_key,
        "the gn/gp overlay must be dismiss-on-next-key, unlike hover"
    );
}

#[test]
#[cfg(not(windows))]
fn the_next_key_after_gn_dismisses_the_popup_but_still_executes() {
    let tmp = safe_tempdir();
    let file_dir = safe_tempdir();
    let diag_a: DiagFixture = ((1, 0), (1, 2), 1, "problem A");
    let diag_b: DiagFixture = ((3, 0), (3, 2), 1, "problem B");
    let (mut ed, _guard) = setup(
        &file_dir.path().join("main.rs"),
        tmp.path(),
        &[diag_a, diag_b],
    );

    ed.feed_key(key('g'));
    ed.feed_key(key('n'));
    assert!(ed.state.popup.is_some(), "popup must be open after gn");
    let line_before = ed.current_selections().primary().head();

    ed.feed_key(key('j')); // an ordinary Normal-mode motion, not a special dismiss key

    assert!(
        ed.state.popup.is_none(),
        "any key press must dismiss the overlay"
    );
    assert_ne!(
        ed.current_selections().primary().head(),
        line_before,
        "the dismissing key must still perform its own action (passive \
         dismiss, not swallowed)"
    );
}

#[test]
#[cfg(not(windows))]
fn diagnostics_drawer_selection_does_not_open_a_popup() {
    let tmp = safe_tempdir();
    let file_dir = safe_tempdir();
    let diag_a: DiagFixture = ((1, 0), (1, 2), 1, "problem A");
    let diag_b: DiagFixture = ((3, 0), (3, 2), 2, "problem B");
    let (mut ed, _guard) = setup(
        &file_dir.path().join("main.rs"),
        tmp.path(),
        &[diag_a, diag_b],
    );

    run(&mut ed, ":diagnostics");
    assert!(
        ed.state.popup.is_none(),
        "opening the drawer itself must not show a popup"
    );

    ed.handle_key(key('j'));
    ed.handle_key(key_enter());
    ed.drain_pending_steel_calls();

    assert!(
        ed.state.popup.is_none(),
        "selecting a row in the :diagnostics drawer must jump without \
         opening the gn/gp overlay — only gn/gp show it"
    );
}
