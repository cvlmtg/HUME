// Diagnostic underlines + extra-highlights wiring: the
// `update_highlight_providers` write side that feeds the new
// `ScopedHighlighter` (Diagnostic/Extra tiers) from the diagnostics store
// and the extra-highlights store.
//
// Every test here goes through `Editor::open(None, std::sync::Arc::new(|| {}))` (not `editor_from`'s
// bare `Pane::new`) — highlight providers are only registered by
// `build_pane`, which only the real `Editor::open`/`:e` construction path
// runs (see `editor/mod.rs`'s `for_testing` doc comment on why its own
// pane has no `PaneHighlights` entry at all).

use std::path::Path;

use super::*;
use crate::editor::lsp::LspState;
use hume_engine::pipeline::{PaneId, RenderContext};
use hume_engine::types::ScopeId;
use hume_lsp::backend::LspBackend;
use hume_lsp::client::LspClient;
use hume_lsp::inline::InlineLspBackend;
use hume_scripting::ScriptingHost;

fn type_text(ed: &mut Editor, text: &str) {
    ed.feed_key(key('i'));
    for ch in text.chars() {
        if ch == '\n' {
            ed.feed_key(key_enter());
        } else {
            ed.feed_key(key(ch));
        }
    }
    ed.feed_key(key_esc());
}

/// `((start_line, start_char), (end_line, end_char), severity)` — same shape
/// as `lsp_diagnostics.rs`'s fixture (kept independent per this codebase's
/// one-file-owns-its-fixtures convention).
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

/// Keeps the temp file alive for the test's duration (dropped at the end of
/// the owning test function, same lifetime shape as `lsp_diagnostics.rs`'s
/// inline `tempfile::tempdir()` locals).
struct DiagCtx {
    _file_dir: tempfile::TempDir,
    ed: Editor,
    pid: PaneId,
}

/// Opens a real file (via `Editor::open` + `:e`, so `build_pane`'s providers
/// are wired), publishes `diags` against it through a scripted server,
/// drains, and runs one `prepare_frame` so `update_highlight_providers` has
/// populated the pane's Arcs.
fn setup_with_diagnostics(content: &str, diags: &[DiagFixture]) -> DiagCtx {
    let file_dir = safe_tempdir();
    let file = file_dir.path().join("main.rs");
    std::fs::write(&file, content).unwrap();
    let canonical = std::fs::canonicalize(&file).unwrap();
    let uri = hume_lsp::uri::path_to_uri(&canonical).unwrap();

    let mut backend = InlineLspBackend::new();
    let sid = backend
        .start("rust-analyzer", &[], Path::new("."), &[])
        .unwrap();
    if !diags.is_empty() {
        backend.push_from_server(sid, publish_diagnostics_notification(uri.as_str(), diags));
    }

    let mut ed = Editor::open(None, std::sync::Arc::new(|| {})).unwrap();
    ed.lsp = LspState::from_backend_for_test(Box::new(backend));
    ed.lsp
        .insert_client_for_test(LspClient::new(sid, std::path::PathBuf::from(".")));
    ed.execute_typed("e", Some(file.to_str().unwrap())).unwrap();
    let bid = ed.focused_buffer_id();
    ed.state.buffers.get_mut(bid).lsp_server = Some(sid);
    ed.drain_lsp();

    let pid = ed.state.focused_pane_id;
    let mut ctx = RenderContext::new();
    ed.sync_viewport_dims(80, 25);
    ed.settle();
    ed.prepare_frame(&mut ctx);

    DiagCtx {
        _file_dir: file_dir,
        ed,
        pid,
    }
}

fn diagnostics_arc(ed: &Editor, pid: PaneId) -> Vec<(usize, usize, usize, ScopeId)> {
    ed.state.panes.render[pid]
        .highlights
        .diagnostics
        .read()
        .unwrap()
        .clone()
}

fn extra_arc(ed: &Editor, pid: PaneId) -> Vec<(usize, usize, usize, ScopeId)> {
    ed.state.panes.render[pid]
        .highlights
        .extra
        .read()
        .unwrap()
        .clone()
}

fn scope(ed: &Editor, name: &str) -> ScopeId {
    ed.view
        .registry
        .get(name)
        .unwrap_or_else(|| panic!("scope '{name}' must already be interned"))
}

// ── Diagnostic underlines ────────────────────────────────────────────────────

#[test]
fn single_line_error_diagnostic_gets_the_error_scope() {
    let c = setup_with_diagnostics("abcdefgh\n", &[((0, 2), (0, 5), 1)]);
    let error_scope = scope(&c.ed, "diagnostic.error");
    assert_eq!(
        diagnostics_arc(&c.ed, c.pid),
        vec![(0, 2, 5, error_scope)],
        "single-line ASCII diagnostic: byte offsets equal char offsets"
    );
}

#[test]
fn severity_floor_hides_less_severe_diagnostics() {
    let mut c = setup_with_diagnostics(
        "abcdefgh\n",
        &[((0, 0), (0, 1), 1), ((0, 6), (0, 7), 4)], // error + hint
    );
    let error_scope = scope(&c.ed, "diagnostic.error");
    let hint_scope = scope(&c.ed, "diagnostic.hint");
    assert_eq!(
        diagnostics_arc(&c.ed, c.pid),
        vec![(0, 0, 1, error_scope), (0, 6, 7, hint_scope)],
        "sanity: default floor (Hint) keeps everything"
    );

    crate::editor::commands::typed_set(
        &mut c.ed,
        Some("global lsp.diagnostics-severity-floor=warning"),
        false,
    )
    .unwrap();
    let mut ctx = RenderContext::new();
    c.ed.sync_viewport_dims(80, 25);
    c.ed.settle();
    c.ed.prepare_frame(&mut ctx);

    assert_eq!(
        diagnostics_arc(&c.ed, c.pid),
        vec![(0, 0, 1, error_scope)],
        "raising the floor to warning must drop the hint but keep the error"
    );
}

/// Independent oracle: byte offsets are hand-computed from the known ASCII
/// content, not derived by calling the code under test (matches the
/// multiline search-match test's convention in `multi_pane.rs`).
#[test]
fn multiline_diagnostic_splits_into_per_line_spans() {
    // "abc\ndef\n" — a diagnostic covering char 2 ('c') through char 6 ('f'),
    // crossing the line-0/line-1 boundary at the '\n' (char 3).
    let c = setup_with_diagnostics("abc\ndef\n", &[((0, 2), (1, 3), 1)]);
    let error_scope = scope(&c.ed, "diagnostic.error");
    assert_eq!(
        diagnostics_arc(&c.ed, c.pid),
        vec![(0, 2, 3, error_scope), (1, 0, 3, error_scope)],
        "line 0 gets 'c' clipped before its own '\\n' (byte 2..3); \
         line 1 gets 'def' from its own start (byte 0..3)"
    );
}

#[test]
fn zero_diagnostics_produce_empty_provider_output() {
    let c = setup_with_diagnostics("abcdefgh\n", &[]);
    assert!(
        diagnostics_arc(&c.ed, c.pid).is_empty(),
        "no diagnostics published — the diagnostics Arc must stay empty"
    );
}

/// Unlike search/bracket-match highlights, diagnostics stay visible while
/// typing — an error squiggle is exactly as relevant mid-edit as it is in
/// Normal mode.
#[test]
fn diagnostics_stay_visible_in_insert_mode() {
    let mut c = setup_with_diagnostics("abcdefgh\n", &[((0, 0), (0, 1), 1)]);
    assert!(
        !diagnostics_arc(&c.ed, c.pid).is_empty(),
        "sanity: visible in Normal mode"
    );

    c.ed.feed_key(key('i'));
    let mut ctx = RenderContext::new();
    c.ed.sync_viewport_dims(80, 25);
    c.ed.settle();
    c.ed.prepare_frame(&mut ctx);
    assert!(
        !diagnostics_arc(&c.ed, c.pid).is_empty(),
        "diagnostics must stay visible in Insert mode"
    );

    c.ed.feed_key(key_esc());
    c.ed.sync_viewport_dims(80, 25);
    c.ed.settle();
    c.ed.prepare_frame(&mut ctx);
    assert!(
        !diagnostics_arc(&c.ed, c.pid).is_empty(),
        "still visible back in Normal mode"
    );
}

// ── Extra highlights ──────────────────────────────────────────────────────────

#[test]
fn extra_highlight_gets_its_runtime_interned_scope() {
    let tmp = safe_tempdir();
    let mut ed = Editor::open(None, std::sync::Arc::new(|| {})).unwrap();
    type_text(&mut ed, "abcdefgh");
    let mut host = ScriptingHost::new();
    eval_with_real_host(
        &mut ed,
        &mut host,
        r#"(define-command! "arm" "" (lambda ()
             (set-extra-highlights! "linter" (current-buffer) (list (list 1 4 "unused")))))"#,
        tmp.path(),
    );
    ed.scripting = Some(host);
    type_cmd(&mut ed, ":arm");

    let pid = ed.state.focused_pane_id;
    let mut ctx = RenderContext::new();
    ed.sync_viewport_dims(80, 25);
    ed.settle();
    ed.prepare_frame(&mut ctx);

    let unused_scope = scope(&ed, "unused");
    assert_eq!(
        extra_arc(&ed, pid),
        vec![(0, 1, 4, unused_scope)],
        "the plugin's 'unused' scope string must be interned and used verbatim"
    );
}

#[test]
fn extra_highlight_scope_is_cached_not_reinterned() {
    let tmp = safe_tempdir();
    let mut ed = Editor::open(None, std::sync::Arc::new(|| {})).unwrap();
    type_text(&mut ed, "abcdefgh");
    let mut host = ScriptingHost::new();
    eval_with_real_host(
        &mut ed,
        &mut host,
        r#"(define-command! "arm-a" "" (lambda ()
             (set-extra-highlights! "a" (current-buffer) (list (list 0 1 "shared")))))
           (define-command! "arm-b" "" (lambda ()
             (set-extra-highlights! "b" (current-buffer) (list (list 2 3 "shared")))))"#,
        tmp.path(),
    );
    ed.scripting = Some(host);
    type_cmd(&mut ed, ":arm-a");
    type_cmd(&mut ed, ":arm-b");

    let pid = ed.state.focused_pane_id;
    let mut ctx = RenderContext::new();
    ed.sync_viewport_dims(80, 25);
    ed.settle();
    ed.prepare_frame(&mut ctx);

    let spans = extra_arc(&ed, pid);
    assert_eq!(spans.len(), 2);
    assert_eq!(
        spans[0].3, spans[1].3,
        "two sources using the same scope name must resolve to the same ScopeId"
    );
}

/// Two sources' extra highlights overlapping the same range must resolve
/// the tie in alphabetical source-name order, not whichever source called
/// `set-extra-highlights!` first — `SourceStore::set` keeps a buffer's
/// sources sorted ascending by name, and `flatten_priority_overlaps`
/// resolves same-priority ties by push order, so "zzz" set before "aaa"
/// must still lose the overlap to "aaa".
#[test]
fn overlapping_extra_highlights_from_two_sources_resolve_alphabetically() {
    let tmp = safe_tempdir();
    let mut ed = Editor::open(None, std::sync::Arc::new(|| {})).unwrap();
    type_text(&mut ed, "abcdefgh");
    let mut host = ScriptingHost::new();
    eval_with_real_host(
        &mut ed,
        &mut host,
        r#"(define-command! "arm-zzz" "" (lambda ()
             (set-extra-highlights! "zzz" (current-buffer) (list (list 1 4 "zzz-scope")))))
           (define-command! "arm-aaa" "" (lambda ()
             (set-extra-highlights! "aaa" (current-buffer) (list (list 1 4 "aaa-scope")))))"#,
        tmp.path(),
    );
    ed.scripting = Some(host);
    // "zzz" armed first — if the tie-break followed call order this would win.
    type_cmd(&mut ed, ":arm-zzz");
    type_cmd(&mut ed, ":arm-aaa");

    let pid = ed.state.focused_pane_id;
    let mut ctx = RenderContext::new();
    ed.sync_viewport_dims(80, 25);
    ed.settle();
    ed.prepare_frame(&mut ctx);

    let aaa_scope = scope(&ed, "aaa-scope");
    assert_eq!(
        extra_arc(&ed, pid),
        vec![(0, 1, 4, aaa_scope)],
        "the alphabetically first source (\"aaa\") must win the overlap \
         regardless of which source called set-extra-highlights! first"
    );
}

/// Reproduces the same-frame scope-intern-then-resolve race: a scope name
/// that has never been interned before must render its real style on the
/// very first frame it appears in, not a stale/default style (or panic).
/// `render_to_buf`'s internal `prepare_frame` is the ONLY frame here — no
/// warm-up frame, unlike most tests in this file, since a warm-up frame is
/// exactly what would paper over the bug this asserts against. Uses a
/// dot-notation sub-key of an existing scope ("diagnostic.warning") so the
/// name itself is new (freshly interned by `update_highlight_providers`)
/// while still resolving to a real, non-default style via fallback.
#[test]
fn extra_highlight_style_resolves_correctly_on_the_frame_it_is_first_interned() {
    let tmp = safe_tempdir();
    let mut ed = Editor::open(None, std::sync::Arc::new(|| {})).unwrap();
    ed.view.theme = crate::ui::theme::build_dark_theme_for_snapshot_tests();
    type_text(&mut ed, "abcdefgh");
    let mut host = ScriptingHost::new();
    eval_with_real_host(
        &mut ed,
        &mut host,
        r#"(define-command! "arm" "" (lambda ()
             (set-extra-highlights! "linter" (current-buffer)
               (list (list 0 8 "diagnostic.warning.qa-regression-marker")))))"#,
        tmp.path(),
    );
    ed.scripting = Some(host);
    type_cmd(&mut ed, ":arm");

    let rect = ratatui::layout::Rect::new(0, 0, 20, 3);
    let buf = ed.render_to_buf(rect);

    let scope_id = ed
        .view
        .registry
        .get("diagnostic.warning.qa-regression-marker")
        .expect("set-extra-highlights! must have interned the scope");
    let resolved = ed.view.theme.resolve(scope_id);
    assert!(
        resolved.fg.is_some(),
        "sanity: the dot-notation fallback to \"diagnostic.warning\" must resolve to a real color"
    );

    let fg_colors: Vec<_> = (rect.left()..rect.right())
        .flat_map(|x| (rect.top()..rect.bottom()).map(move |y| (x, y)))
        .map(|(x, y)| buf[(x, y)].style().fg)
        .collect();
    assert!(
        fg_colors.contains(&resolved.fg),
        "the newly-interned scope's real color must appear on the frame it was \
         first interned, not the default the bake-before-intern race would produce"
    );
}

// ── Cross-tier layering (engine-level, confirms end-to-end wiring) ──────────

/// Search matches (tier `SearchMatch`) must beat extra highlights (tier
/// `Extra`) in an overlapping region — the engine's per-tier `HighlightStack`
/// composes this automatically; this snapshot proves the two new registrations
/// in `build_pane` actually feed it, not just that the Arcs are populated.
#[test]
fn search_match_beats_extra_highlight_in_overlapping_region() {
    let tmp = safe_tempdir();
    let mut ed = Editor::open(None, std::sync::Arc::new(|| {})).unwrap();
    ed.view.theme = crate::ui::theme::build_dark_theme_for_snapshot_tests();
    type_text(&mut ed, "abcdefgh");
    let mut host = ScriptingHost::new();
    // Reuses the theme's "diagnostic.warning" name as the extra highlight's
    // scope purely so the span has a *visible* style to prove the tier
    // ordering with — extra highlights don't otherwise care what string a
    // plugin passes.
    eval_with_real_host(
        &mut ed,
        &mut host,
        r#"(define-command! "arm" "" (lambda ()
             (set-extra-highlights! "linter" (current-buffer) (list (list 0 8 "diagnostic.warning")))))"#,
        tmp.path(),
    );
    ed.scripting = Some(host);
    type_cmd(&mut ed, ":arm");
    ed = ed.with_search_regex("cde");

    use ratatui::layout::Rect;
    let rect = Rect::new(0, 0, 20, 3);
    let snap = render_snapshot::render_to_styled_string(&mut ed, rect);
    insta::assert_snapshot!(snap);
}
