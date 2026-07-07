// U2 (docs/lsp/step-3.md) — diagnostic gutter signs: the `update_sign_providers`
// write side that feeds `SharedSignSource` (diagnostics + plugin `set-signs!`)
// from C9's diagnostics store and B5's signs store, plus the sign column's
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

fn diag_signs(ed: &Editor, pid: PaneId) -> std::collections::HashMap<usize, Sign> {
    ed.state.panes.signs[pid]
        .diagnostics
        .read()
        .unwrap()
        .clone()
}

fn plugin_signs(ed: &Editor, pid: PaneId) -> std::collections::HashMap<usize, Sign> {
    ed.state.panes.signs[pid].plugin.read().unwrap().clone()
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
    let sign = &signs[&0];
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
    assert_eq!(
        signs[&0].scope, error_scope,
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

// ── Gutter width auto-collapse ────────────────────────────────────────────────

#[test]
fn gutter_width_collapses_to_zero_with_no_signs() {
    let c = setup_with_diagnostics("abcdefgh\n", &[]);
    assert_eq!(
        sign_column_width(&c.ed, c.pid),
        0,
        "no diagnostics, no plugin signs — column collapses"
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
    let sign = &signs[&0];
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
fn two_plugin_sources_on_the_same_line_keep_the_higher_priority() {
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
    assert_eq!(signs.len(), 1, "one line, one merged winner across sources");
    assert_eq!(
        signs[&0].text, "+",
        "priority 9 (vcs) beats priority 3 (linter)"
    );
}
