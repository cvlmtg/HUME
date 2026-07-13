// Diagnostic gutter signs: the `update_sign_providers`
// write side that feeds `SharedSignSource` (diagnostics + plugin `set-signs!`)
// from the diagnostics store and the signs store, plus the sign column's
// auto-collapsing width.
//
// Every test here goes through `Editor::open(None)` (not `editor_from`'s bare
// `Pane::new`) — sign providers are only registered by `build_pane`, same
// reasoning as `lsp_render.rs`.

use std::path::Path;

use super::*;
use crate::editor::lsp::LspState;
use crate::editor::scripting_setup::make_init_host;
use hume_engine::builtins::sign_column::Sign;
use hume_engine::pipeline::{PaneId, RenderContext};
use hume_lsp::backend::LspBackend;
use hume_lsp::client::LspClient;
use hume_lsp::inline::InlineLspBackend;
use hume_scripting::ScriptingHost;

fn eval_with_real_host(ed: &mut Editor, host: &mut ScriptingHost, source: &str, tmp: &Path) {
    let init_path = tmp.join("init.scm");
    std::fs::write(&init_path, source).unwrap();
    let mut ih = make_init_host(&mut ed.state, &mut ed.view);
    host.eval_init(&init_path, 10_000, &mut ih, Default::default())
        .expect("eval_init");
}

/// `((start_line, start_char), (end_line, end_char), severity)`.
type DiagFixture = ((u32, u32), (u32, u32), i64);

fn publish_diagnostics_notification(
    uri: &str,
    ranges_and_severity: &[DiagFixture],
) -> hume_lsp::codec::Message {
    let diagnostics: Vec<serde_json::Value> = ranges_and_severity
        .iter()
        .map(|&((sl, sc), (el, ec), severity)| {
            serde_json::json!({
                "range": {"start": {"line": sl, "character": sc}, "end": {"line": el, "character": ec}},
                "severity": severity,
                "message": "boom",
            })
        })
        .collect();
    hume_lsp::codec::Message::Notification {
        method: "textDocument/publishDiagnostics".to_string(),
        params: serde_json::json!({"uri": uri, "diagnostics": diagnostics}),
    }
}

struct DiagCtx {
    _file_dir: tempfile::TempDir,
    ed: Editor,
    pid: PaneId,
}

fn setup_with_diagnostics(content: &str, diags: &[DiagFixture]) -> DiagCtx {
    let file_dir = tempfile::tempdir().unwrap();
    let file = file_dir.path().join("main.rs");
    std::fs::write(&file, content).unwrap();
    let canonical = std::fs::canonicalize(&file).unwrap();
    let uri = hume_lsp::uri::path_to_uri(&canonical).unwrap();

    let mut backend = InlineLspBackend::new();
    let sid = backend.start("rust-analyzer", &[], Path::new(".")).unwrap();
    if !diags.is_empty() {
        backend.push_from_server(sid, publish_diagnostics_notification(uri.as_str(), diags));
    }

    let mut ed = Editor::open(None).unwrap();
    ed.lsp = LspState::from_backend_for_test(Box::new(backend));
    ed.lsp
        .insert_client_for_test(LspClient::new(sid, std::path::PathBuf::from(".")));
    ed.execute_typed("e", Some(file.to_str().unwrap())).unwrap();
    let bid = ed.focused_buffer_id();
    ed.state.buffers.get_mut(bid).lsp_server = Some(sid);
    ed.drain_lsp();

    let pid = ed.state.focused_pane_id;
    let mut ctx = RenderContext::new();
    ed.prepare_frame(80, 25, &mut ctx);

    DiagCtx {
        _file_dir: file_dir,
        ed,
        pid,
    }
}

fn diag_signs(ed: &Editor, pid: PaneId) -> std::collections::HashMap<usize, Vec<Sign>> {
    ed.state.panes.render[pid]
        .signs
        .diagnostics
        .read()
        .unwrap()
        .clone()
}

fn plugin_signs(ed: &Editor, pid: PaneId) -> std::collections::HashMap<usize, Vec<Sign>> {
    ed.state.panes.render[pid]
        .signs
        .plugin
        .read()
        .unwrap()
        .clone()
}

fn sign_column_width(ed: &Editor, pid: PaneId) -> u8 {
    ed.view.panes[pid]
        .providers
        .gutter_columns()
        .next()
        .expect("sign column registered first")
        .width(0)
}

// ── Diagnostic signs ──────────────────────────────────────────────────────────

#[test]
fn error_line_gets_a_sign_with_the_error_scope() {
    let c = setup_with_diagnostics("abcdefgh\n", &[((0, 2), (0, 5), 1)]);
    let error_scope =
        c.ed.view
            .registry
            .get("diagnostic.error")
            .expect("interned by the write side");

    let signs = diag_signs(&c.ed, c.pid);
    assert_eq!(signs.len(), 1);
    let sign = signs[&0].first().expect("one sign on the error line");
    assert_eq!(sign.text, "●");
    assert_eq!(sign.scope, error_scope);
    assert_eq!(sign.priority, 10);
}

#[test]
fn error_beats_warning_on_the_same_line() {
    let c = setup_with_diagnostics(
        "abcdefgh\n",
        &[((0, 0), (0, 1), 2), ((0, 4), (0, 5), 1)], // warning then error, same line
    );
    let error_scope = c.ed.view.registry.get("diagnostic.error").unwrap();

    let signs = diag_signs(&c.ed, c.pid);
    assert_eq!(signs.len(), 1, "one line, one merged sign");
    let sign = signs[&0].first().expect("one sign on the line");
    assert_eq!(
        sign.scope, error_scope,
        "error must win over warning on the same line regardless of publish order"
    );
}

#[test]
fn multiline_diagnostic_marks_every_line_it_touches() {
    // "abc\ndef\n" — a diagnostic covering char 2 ('c') through char 6 ('f'),
    // crossing the line-0/line-1 boundary.
    let c = setup_with_diagnostics("abc\ndef\n", &[((0, 2), (1, 3), 1)]);
    let signs = diag_signs(&c.ed, c.pid);
    assert_eq!(
        signs.len(),
        2,
        "both lines the diagnostic touches get a sign"
    );
    assert!(signs.contains_key(&0));
    assert!(signs.contains_key(&1));
}

#[test]
fn zero_diagnostics_produce_no_signs() {
    let c = setup_with_diagnostics("abcdefgh\n", &[]);
    assert!(diag_signs(&c.ed, c.pid).is_empty());
}

// ── Gutter width ──────────────────────────────────────────────────────────────

#[test]
fn gutter_width_stays_at_default_with_no_signs_under_always_mode() {
    let c = setup_with_diagnostics("abcdefgh\n", &[]);
    assert_eq!(
        sign_column_width(&c.ed, c.pid),
        2,
        "default is `always` — column stays visible even with no signs"
    );
}

#[test]
fn gutter_width_collapses_under_auto_mode_with_no_signs() {
    let mut c = setup_with_diagnostics("abcdefgh\n", &[]);
    let bid = c.ed.focused_buffer_id();
    c.ed.state.buffers.get_mut(bid).overrides.signcolumn =
        Some("auto".parse().unwrap());
    let mut ctx = RenderContext::new();
    c.ed.prepare_frame(80, 25, &mut ctx);
    assert_eq!(
        sign_column_width(&c.ed, c.pid),
        0,
        "auto mode with no signs — column collapses"
    );
}

#[test]
fn gutter_width_is_the_default_when_a_diagnostic_exists() {
    let c = setup_with_diagnostics("abcdefgh\n", &[((0, 0), (0, 1), 1)]);
    assert_eq!(
        sign_column_width(&c.ed, c.pid),
        2,
        "a diagnostic exists — column shows at the default width"
    );
}

#[test]
fn gutter_width_always_2_is_3_cells_wide() {
    let mut c = setup_with_diagnostics("abcdefgh\n", &[]);
    let bid = c.ed.focused_buffer_id();
    c.ed.state.buffers.get_mut(bid).overrides.signcolumn =
        Some("always:2".parse().unwrap());
    let mut ctx = RenderContext::new();
    c.ed.prepare_frame(80, 25, &mut ctx);
    assert_eq!(
        sign_column_width(&c.ed, c.pid),
        3,
        "always:2 = 2 sign slots + 1 padding = 3 cells"
    );
}

#[test]
fn gutter_width_auto_2_expands_when_signs_exist() {
    let mut c = setup_with_diagnostics("abcdefgh\n", &[((0, 0), (0, 1), 1)]);
    let bid = c.ed.focused_buffer_id();
    c.ed.state.buffers.get_mut(bid).overrides.signcolumn =
        Some("auto:2".parse().unwrap());
    let mut ctx = RenderContext::new();
    c.ed.prepare_frame(80, 25, &mut ctx);
    assert_eq!(
        sign_column_width(&c.ed, c.pid),
        3,
        "auto:2 with signs = 2 sign slots + 1 padding = 3 cells"
    );
}

// ── Plugin signs ──────────────────────────────────────────────────────────────

#[test]
fn plugin_sign_via_set_signs_appears_in_the_plugin_map() {
    let tmp = tempfile::tempdir().unwrap();
    let mut ed = Editor::open(None).unwrap();
    ed.feed_key(key('i'));
    for ch in "abcdefgh".chars() {
        ed.feed_key(key(ch));
    }
    ed.feed_key(key_esc());
    let mut host = ScriptingHost::new();
    eval_with_real_host(
        &mut ed,
        &mut host,
        r#"(define-command! "arm" "" (lambda ()
             (set-signs! "linter" (current-buffer) (list (list 0 "!" "warn-scope" 7)))))"#,
        tmp.path(),
    );
    ed.scripting = Some(host);
    type_cmd(&mut ed, ":arm");

    let pid = ed.state.focused_pane_id;
    let mut ctx = RenderContext::new();
    ed.prepare_frame(80, 25, &mut ctx);

    let signs = plugin_signs(&ed, pid);
    assert_eq!(signs.len(), 1);
    let sign = signs[&0].first().expect("one sign on the line");
    assert_eq!(sign.text, "!");
    assert_eq!(sign.priority, 7);
    let warn_scope = ed.view.registry.get("warn-scope").unwrap();
    assert_eq!(sign.scope, warn_scope);

    assert_eq!(
        sign_column_width(&ed, pid),
        2,
        "a plugin sign alone must also expand the gutter"
    );
}

#[test]
fn two_plugin_sources_on_the_same_line_keep_the_higher_priority_first() {
    let tmp = tempfile::tempdir().unwrap();
    let mut ed = Editor::open(None).unwrap();
    ed.feed_key(key('i'));
    for ch in "abcdefgh".chars() {
        ed.feed_key(key(ch));
    }
    ed.feed_key(key_esc());
    let mut host = ScriptingHost::new();
    eval_with_real_host(
        &mut ed,
        &mut host,
        r#"(define-command! "arm" "" (lambda ()
             (set-signs! "linter" (current-buffer) (list (list 0 "!" "a" 3)))
             (set-signs! "vcs" (current-buffer) (list (list 0 "+" "b" 9)))))"#,
        tmp.path(),
    );
    ed.scripting = Some(host);
    type_cmd(&mut ed, ":arm");

    let pid = ed.state.focused_pane_id;
    let mut ctx = RenderContext::new();
    ed.prepare_frame(80, 25, &mut ctx);

    let signs = plugin_signs(&ed, pid);
    assert_eq!(signs.len(), 1, "one line, one merged entry across sources");
    let line_signs = &signs[&0];
    assert_eq!(
        line_signs.len(),
        1,
        "default `signcolumn` columns=1 keeps only the winner"
    );
    assert_eq!(
        line_signs[0].text, "+",
        "priority 9 (vcs) beats priority 3 (linter)"
    );
}

/// With `signcolumn=always:2` the plugin merge keeps the top 2 signs per line
/// (sorted by priority desc), so both sources survive to the render stage —
/// the `SignColumn` then lays them out left-to-right in the 2-slot gutter.
#[test]
fn wider_signcolumn_keeps_multiple_signs_per_line() {
    let tmp = tempfile::tempdir().unwrap();
    let mut ed = Editor::open(None).unwrap();
    ed.feed_key(key('i'));
    for ch in "abcdefgh".chars() {
        ed.feed_key(key(ch));
    }
    ed.feed_key(key_esc());
    let bid = ed.focused_buffer_id();
    ed.state.buffers.get_mut(bid).overrides.signcolumn =
        Some("always:2".parse().unwrap());
    let mut host = ScriptingHost::new();
    eval_with_real_host(
        &mut ed,
        &mut host,
        r#"(define-command! "arm" "" (lambda ()
             (set-signs! "linter" (current-buffer) (list (list 0 "!" "a" 3)))
             (set-signs! "vcs" (current-buffer) (list (list 0 "+" "b" 9)))))"#,
        tmp.path(),
    );
    ed.scripting = Some(host);
    type_cmd(&mut ed, ":arm");

    let pid = ed.state.focused_pane_id;
    let mut ctx = RenderContext::new();
    ed.prepare_frame(80, 25, &mut ctx);

    let signs = plugin_signs(&ed, pid);
    let line_signs = &signs[&0];
    assert_eq!(
        line_signs.len(),
        2,
        "signcolumn=always:2 keeps both signs on the line"
    );
    assert_eq!(line_signs[0].text, "+", "priority 9 first");
    assert_eq!(line_signs[1].text, "!", "priority 3 second");
    assert_eq!(
        sign_column_width(&ed, pid),
        3,
        "always:2 = 2 sign slots + 1 padding = 3 cells"
    );
}

/// Regression: a stored diagnostic computed against the pre-reload text can
/// end up with offsets past the new (shorter) content — before the fix,
/// `update_sign_providers` fed that straight into `char_to_line`, which
/// panics on an out-of-bounds char index. `:e!` must clear diagnostics for
/// the reloaded buffer, so this never even reaches the panicking path.
#[test]
fn reload_to_shorter_text_clears_stale_diagnostics_and_does_not_panic() {
    let file_dir = tempfile::tempdir().unwrap();
    let file = file_dir.path().join("main.rs");
    std::fs::write(&file, "one two three four five six\n").unwrap();

    let mut backend = InlineLspBackend::new();
    let sid = backend.start("rust-analyzer", &[], Path::new(".")).unwrap();

    let mut ed = Editor::open(None).unwrap();
    ed.lsp = LspState::from_backend_for_test(Box::new(backend));
    ed.lsp
        .insert_client_for_test(LspClient::new(sid, std::path::PathBuf::from(".")));
    ed.execute_typed("e", Some(file.to_str().unwrap())).unwrap();
    let bid = ed.focused_buffer_id();
    ed.state.buffers.get_mut(bid).lsp_server = Some(sid);

    let canonical = std::fs::canonicalize(&file).unwrap();
    let uri = hume_lsp::uri::path_to_uri(&canonical).unwrap();
    // Starts at 0 (so it still overlaps the post-reload visible range and
    // isn't filtered out by `for_range` before ever reaching the
    // panic-prone `char_to_line` call) but ends at 27 — the original
    // (longer) content's length, far past the much shorter reloaded
    // content's 4 chars below.
    let params = serde_json::json!({
        "uri": uri.as_str(),
        "diagnostics": [{
            "range": {"start": {"line": 0, "character": 0}, "end": {"line": 0, "character": 27}},
            "severity": 1,
            "message": "boom",
        }],
    });
    ed.ingest_publish_diagnostics(sid, params);
    assert_eq!(
        ed.lsp.diagnostic_counts_for_test(bid),
        (1, 0),
        "seed diagnostic must land before the reload"
    );

    std::fs::write(&file, "one\n").unwrap();
    ed.execute_typed("e!", None).unwrap();

    assert_eq!(
        ed.lsp.diagnostic_counts_for_test(bid),
        (0, 0),
        "reload must clear diagnostics computed against the pre-reload text"
    );

    let pid = ed.state.focused_pane_id;
    let mut ctx = RenderContext::new();
    ed.prepare_frame(80, 25, &mut ctx); // must not panic

    let signs = diag_signs(&ed, pid);
    assert!(
        signs.is_empty(),
        "no stale sign should remain after reload clears diagnostics"
    );
}
