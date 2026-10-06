// Diagnostic underlines + extra-highlights wiring: the
// `update_highlight_providers` write side that feeds the new
// `ScopedHighlighter` (Diagnostic/Extra tiers) from the diagnostics store
// and the extra-highlights store.
//
// Every test here renders a pane made by `build_pane` (`Editor::open`'s
// initial pane, or a split's): highlight providers are only registered
// there (see `editor/mod.rs`'s `for_testing` doc comment on why
// `editor_from`'s bare pane has no `PaneHighlights` entry at all).

use hume_grid::Rect;

use super::lsp_rig::LspRig;
use super::*;
use crate::editor::commands::open_pane_in_layout;
use hume_engine::pipeline::{Direction, PaneId, RenderContext};
use hume_engine::providers::HighlightTier;
use hume_scripting::ScriptingHost;

/// `((start_line, start_char), (end_line, end_char), severity)`.
type DiagFixture = ((u32, u32), (u32, u32), i64);

/// Keeps the rig's root alive for the test's duration (dropped at the end
/// of the owning test function).
struct DiagCtx {
    _tmp: tempfile::TempDir,
    ed: Editor,
    pid: PaneId,
}

/// Opens `content` in a file attached to a scripted server, shows it in a
/// split (so `build_pane`'s providers are wired), publishes `diags` against
/// it, and runs one `prepare_frame` so `update_highlight_providers` has
/// populated the pane's Arcs.
fn setup_with_diagnostics(content: &str, diags: &[DiagFixture]) -> DiagCtx {
    let tmp = safe_tempdir();
    let marked = format!("-[{}]>{}", &content[..1], &content[1..]);
    let mut rig = LspRig::rust(tmp.path(), &marked, serde_json::json!({"capabilities": {}}));
    let bare = rig.ed.state.focus.id();
    let pid = open_pane_in_layout(
        &mut rig.ed.state,
        &mut rig.ed.view,
        bare,
        rig.bid,
        Direction::Horizontal,
    )
    .expect("split must succeed");
    rig.ed.state.focus.set_for_test(pid);

    if !diags.is_empty() {
        let diagnostics: Vec<serde_json::Value> = diags
            .iter()
            .map(|&((sl, sc), (el, ec), severity)| {
                serde_json::json!({
                    "range": {"start": {"line": sl, "character": sc}, "end": {"line": el, "character": ec}},
                    "severity": severity,
                    "message": "boom",
                })
            })
            .collect();
        let sid = rig.sid("rust-analyzer");
        rig.publish(sid, serde_json::Value::Array(diagnostics));
    }

    let mut ed = rig.ed;
    let mut ctx = RenderContext::new();
    ed.sync_viewport_dims(80, 25);
    ed.settle();
    ed.prepare_frame(&mut ctx);

    DiagCtx { _tmp: tmp, ed, pid }
}

// ── Diagnostic underlines ────────────────────────────────────────────────────

#[test]
fn single_line_error_diagnostic_gets_the_error_scope() {
    let c = setup_with_diagnostics("abcdefgh\n", &[((0, 2), (0, 5), 1)]);
    let error_scope = scope(&c.ed, "diagnostic.error");
    assert_eq!(
        pane_highlights(&c.ed, c.pid, HighlightTier::Diagnostic),
        vec![(
            hume_rope::line::ContentLine::new(0),
            bc(2),
            bc(5),
            error_scope
        )],
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
        pane_highlights(&c.ed, c.pid, HighlightTier::Diagnostic),
        vec![
            (
                hume_rope::line::ContentLine::new(0),
                bc(0),
                bc(1),
                error_scope
            ),
            (
                hume_rope::line::ContentLine::new(0),
                bc(6),
                bc(7),
                hint_scope
            )
        ],
        "sanity: default floor (Hint) keeps everything"
    );

    let fp = FocusedPane::current(&c.ed.state);
    crate::editor::commands::typed_set(
        &mut c.ed,
        fp,
        Some("global lsp.diagnostics-severity-floor=warning"),
        false,
    )
    .unwrap();
    let mut ctx = RenderContext::new();
    c.ed.sync_viewport_dims(80, 25);
    c.ed.settle();
    c.ed.prepare_frame(&mut ctx);

    assert_eq!(
        pane_highlights(&c.ed, c.pid, HighlightTier::Diagnostic),
        vec![(
            hume_rope::line::ContentLine::new(0),
            bc(0),
            bc(1),
            error_scope
        )],
        "raising the floor to warning must drop the hint but keep the error"
    );
}

/// Byte offsets are hand-computed from the known ASCII
/// content, not derived by calling the code under test (matches the
/// multiline search-match test's convention in `multi_pane.rs`).
#[test]
fn multiline_diagnostic_splits_into_per_line_spans() {
    // "abc\ndef\n": a diagnostic covering char 2 ('c') through char 6 ('f'),
    // crossing the line-0/line-1 boundary at the '\n' (char 3).
    let c = setup_with_diagnostics("abc\ndef\n", &[((0, 2), (1, 3), 1)]);
    let error_scope = scope(&c.ed, "diagnostic.error");
    assert_eq!(
        pane_highlights(&c.ed, c.pid, HighlightTier::Diagnostic),
        vec![
            (
                hume_rope::line::ContentLine::new(0),
                bc(2),
                bc(3),
                error_scope
            ),
            (
                hume_rope::line::ContentLine::new(1),
                bc(0),
                bc(3),
                error_scope
            )
        ],
        "line 0 gets 'c' clipped before its own '\\n' (byte 2..3); \
         line 1 gets 'def' from its own start (byte 0..3)"
    );
}

#[test]
fn zero_diagnostics_produce_empty_provider_output() {
    let c = setup_with_diagnostics("abcdefgh\n", &[]);
    assert!(
        pane_highlights(&c.ed, c.pid, HighlightTier::Diagnostic).is_empty(),
        "no diagnostics published; the diagnostics Arc must stay empty"
    );
}

fn diagnostic_lines(c: &DiagCtx) -> Vec<usize> {
    pane_highlights(&c.ed, c.pid, HighlightTier::Diagnostic)
        .iter()
        .map(|&(line, ..)| line.index())
        .collect()
}

#[test]
fn diagnostic_underline_is_hidden_on_the_insert_cursor_line_and_returns_on_esc() {
    let mut c = setup_with_diagnostics("abc\ndef\n", &[((0, 0), (0, 1), 1), ((1, 0), (1, 1), 1)]);
    assert_eq!(
        diagnostic_lines(&c),
        vec![0, 1],
        "sanity: both lines in Normal mode"
    );

    c.ed.feed_key(key('i'));
    render(&mut c.ed);
    assert_eq!(
        diagnostic_lines(&c),
        vec![1],
        "cursor line 0 hidden while typing"
    );

    c.ed.feed_key(key_esc());
    render(&mut c.ed);
    assert_eq!(diagnostic_lines(&c), vec![0, 1], "back in Normal mode");
}

#[test]
fn diagnostic_underline_stays_in_insert_with_the_option_on() {
    let mut c = setup_with_diagnostics("abc\ndef\n", &[((0, 0), (0, 1), 1), ((1, 0), (1, 1), 1)]);
    let fp = FocusedPane::current(&c.ed.state);
    crate::editor::commands::typed_set(
        &mut c.ed,
        fp,
        Some("global lsp.diagnostics-on-insert-line=true"),
        false,
    )
    .unwrap();

    c.ed.feed_key(key('i'));
    render(&mut c.ed);
    assert_eq!(diagnostic_lines(&c), vec![0, 1]);
}

#[test]
fn moving_to_another_line_in_insert_restores_the_previous_line_underline() {
    let mut c = setup_with_diagnostics("abc\ndef\n", &[((0, 0), (0, 1), 1), ((1, 0), (1, 1), 1)]);
    c.ed.feed_key(key('i'));
    c.ed.feed_key(key_down());
    render(&mut c.ed);
    assert_eq!(diagnostic_lines(&c), vec![0], "line 1 now holds the cursor");
}

#[test]
fn multi_line_diagnostic_hides_only_the_cursor_line_segment() {
    let mut c = setup_with_diagnostics("abc\ndef\n", &[((0, 2), (1, 3), 1)]);
    c.ed.feed_key(key('i'));
    render(&mut c.ed);
    assert_eq!(diagnostic_lines(&c), vec![1]);
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
        r#"(define-typed-command! "arm" "" (lambda (bid)
             (set-extra-highlights! "linter" bid (list (hash 'start 1 'end 4 'scope "unused")))))"#,
        tmp.path(),
    );
    ed.scripting = Some(host);
    type_cmd(&mut ed, ":arm");

    let pid = ed.state.focus.id();
    let mut ctx = RenderContext::new();
    ed.sync_viewport_dims(80, 25);
    ed.settle();
    ed.prepare_frame(&mut ctx);

    let unused_scope = scope(&ed, "unused");
    assert_eq!(
        pane_highlights(&ed, pid, HighlightTier::Extra),
        vec![(
            hume_rope::line::ContentLine::new(0),
            bc(1),
            bc(4),
            unused_scope
        )],
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
        r#"(define-typed-command! "arm-a" "" (lambda (bid)
             (set-extra-highlights! "a" bid (list (hash 'start 0 'end 1 'scope "shared")))))
           (define-typed-command! "arm-b" "" (lambda (bid)
             (set-extra-highlights! "b" bid (list (hash 'start 2 'end 3 'scope "shared")))))"#,
        tmp.path(),
    );
    ed.scripting = Some(host);
    type_cmd(&mut ed, ":arm-a");
    type_cmd(&mut ed, ":arm-b");

    let pid = ed.state.focus.id();
    let mut ctx = RenderContext::new();
    ed.sync_viewport_dims(80, 25);
    ed.settle();
    ed.prepare_frame(&mut ctx);

    let spans = pane_highlights(&ed, pid, HighlightTier::Extra);
    assert_eq!(spans.len(), 2);
    assert_eq!(
        spans[0].3, spans[1].3,
        "two sources using the same scope name must resolve to the same ScopeId"
    );
}

/// Two sources' extra highlights overlapping the same range must resolve
/// the tie in alphabetical source-name order, not whichever source called
/// `set-extra-highlights!` first. `SourceStore::set` keeps a buffer's
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
        r#"(define-typed-command! "arm-zzz" "" (lambda (bid)
             (set-extra-highlights! "zzz" bid (list (hash 'start 1 'end 4 'scope "zzz-scope")))))
           (define-typed-command! "arm-aaa" "" (lambda (bid)
             (set-extra-highlights! "aaa" bid (list (hash 'start 1 'end 4 'scope "aaa-scope")))))"#,
        tmp.path(),
    );
    ed.scripting = Some(host);
    // "zzz" armed first: if the tie-break followed call order this would win.
    type_cmd(&mut ed, ":arm-zzz");
    type_cmd(&mut ed, ":arm-aaa");

    let pid = ed.state.focus.id();
    let mut ctx = RenderContext::new();
    ed.sync_viewport_dims(80, 25);
    ed.settle();
    ed.prepare_frame(&mut ctx);

    let aaa_scope = scope(&ed, "aaa-scope");
    assert_eq!(
        pane_highlights(&ed, pid, HighlightTier::Extra),
        vec![(
            hume_rope::line::ContentLine::new(0),
            bc(1),
            bc(4),
            aaa_scope
        )],
        "the alphabetically first source (\"aaa\") must win the overlap \
         regardless of which source called set-extra-highlights! first"
    );
}

/// Reproduces the same-frame scope-intern-then-resolve race: a scope name
/// that has never been interned before must render its real style on the
/// very first frame it appears in, not a stale/default style (or panic).
/// `render_to_buf`'s internal `prepare_frame` is the ONLY frame here, with no
/// warm-up frame, unlike most tests in this file, since a warm-up frame is
/// exactly what would paper over the bug this asserts against. Uses a
/// dot-notation sub-key of an existing scope ("diagnostic.warning") so the
/// name itself is new (freshly interned by `update_highlight_providers`)
/// while still resolving to a real, non-default style via fallback.
#[test]
fn extra_highlight_style_resolves_correctly_on_the_frame_it_is_first_interned() {
    let tmp = safe_tempdir();
    let mut ed = Editor::open(None, std::sync::Arc::new(|| {})).unwrap();
    ed.view.theme = crate::testing::build_snapshot_theme();
    type_text(&mut ed, "abcdefgh");
    let mut host = ScriptingHost::new();
    eval_with_real_host(
        &mut ed,
        &mut host,
        r#"(define-typed-command! "arm" "" (lambda (bid)
             (set-extra-highlights! "linter" bid
               (list (hash 'start 0 'end 8 'scope "diagnostic.warning.qa-regression-marker")))))"#,
        tmp.path(),
    );
    ed.scripting = Some(host);
    type_cmd(&mut ed, ":arm");

    let rect = Rect::new(0, 0, 20, 3);
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
/// `Extra`) in an overlapping region. The engine's per-tier `HighlightStack`
/// composes this automatically; this snapshot proves the two new registrations
/// in `build_pane` actually feed it, not just that the Arcs are populated.
#[test]
fn search_match_beats_extra_highlight_in_overlapping_region() {
    let tmp = safe_tempdir();
    let mut ed = Editor::open(None, std::sync::Arc::new(|| {})).unwrap();
    ed.view.theme = crate::testing::build_snapshot_theme();
    type_text(&mut ed, "abcdefgh");
    let mut host = ScriptingHost::new();
    // Reuses the theme's "diagnostic.warning" name as the extra highlight's
    // scope purely so the span has a *visible* style to prove the tier
    // ordering with; extra highlights don't otherwise care what string a
    // plugin passes.
    eval_with_real_host(
        &mut ed,
        &mut host,
        r#"(define-typed-command! "arm" "" (lambda (bid)
             (set-extra-highlights! "linter" bid (list (hash 'start 0 'end 8 'scope "diagnostic.warning")))))"#,
        tmp.path(),
    );
    ed.scripting = Some(host);
    type_cmd(&mut ed, ":arm");
    ed = ed.with_search_regex("cde");
    let rect = Rect::new(0, 0, 20, 3);
    let snap = render_snapshot::render_to_styled_string(&mut ed, rect);
    insta::assert_snapshot!(snap);
}
