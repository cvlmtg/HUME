// Diagnostic gutter signs, placed by `core:lsp` itself through `set-signs!`
// (source `"lsp-diagnostics"`) rather than a separate Rust-side render
// path — see `runtime/plugins/core/lsp/diagnostics.scm`'s
// `lsp/refresh-diagnostic-decorations`. Shares `setup_diagnostics` with
// `lsp_diagnostics_inline.rs` (hoisted to `tests/unix/mod.rs`), since both
// decorations are driven by the same `on-diagnostics-changed` hook.
//
// Plugin-only signs (no diagnostics involved) are covered by the portable
// `tests/lsp_signs.rs` instead — this file is only for the diagnostic
// source itself and its interaction with an ordinary plugin sign.

use super::*;
use hume_engine::builtins::sign_column::Sign;
use hume_engine::pipeline::{PaneId, RenderContext};

fn pane_signs(ed: &Editor, pid: PaneId) -> rustc_hash::FxHashMap<usize, Vec<Sign>> {
    ed.state.panes.render[pid].signs.read().unwrap().clone()
}

fn sign_column_width(ed: &Editor, pid: PaneId) -> u8 {
    ed.view.panes[pid]
        .providers
        .gutter_columns()
        .next()
        .expect("sign column registered first")
        .width(0)
}

/// `RenderContext::new` + `sync_viewport_dims(80, 25)` + `settle` +
/// `prepare_frame` — `setup_diagnostics` only settles the queued hook that
/// writes the decoration store; this drives the frame that syncs it into
/// the pane's own sign map.
fn render(ed: &mut Editor) {
    let mut ctx = RenderContext::new();
    ed.sync_viewport_dims(80, 25);
    ed.settle();
    ed.prepare_frame(&mut ctx);
}

#[test]
fn error_line_gets_a_sign_with_the_error_scope() {
    let tmp = safe_tempdir();
    let file_dir = safe_tempdir();
    let diag: DiagFixture = ((0, 2), (0, 5), 1, "boom");
    let (mut ed, _guard) = setup_diagnostics(
        "abcdefgh\n",
        &file_dir.path().join("main.rs"),
        tmp.path(),
        &[diag],
    );
    let pid = ed.state.focused_pane_id;
    render(&mut ed);

    let error_scope = ed
        .view
        .registry
        .get("error")
        .expect("interned by the write side");

    let signs = pane_signs(&ed, pid);
    assert_eq!(signs.len(), 1);
    let sign = signs[&0].first().expect("one sign on the error line");
    assert_eq!(sign.text, "●");
    assert_eq!(sign.scope, error_scope);
    assert_eq!(
        sign.slot, 0,
        "diagnostics are this buffer's only sign channel — slot 0"
    );
}

#[test]
fn sign_and_buffer_text_use_different_scopes_for_the_same_severity() {
    // The gutter glyph and the text span it marks are different render
    // surfaces: the editing-area scope (`diagnostic.error`) carries an
    // `underline` modifier meant for the text span, which the sign column
    // must not inherit. Regression guard for the two surfaces ever being
    // pointed at each other's scope.
    let tmp = safe_tempdir();
    let file_dir = safe_tempdir();
    let diag: DiagFixture = ((0, 2), (0, 5), 1, "boom");
    let (mut ed, _guard) = setup_diagnostics(
        "abcdefgh\n",
        &file_dir.path().join("main.rs"),
        tmp.path(),
        &[diag],
    );
    let pid = ed.state.focused_pane_id;
    render(&mut ed);

    let gutter_scope = ed
        .view
        .registry
        .get("error")
        .expect("interned by the sign write side");
    let text_scope = ed
        .view
        .registry
        .get("diagnostic.error")
        .expect("interned by the highlight write side");
    assert_ne!(
        gutter_scope, text_scope,
        "gutter and buffer-text diagnostics must resolve to distinct scopes"
    );

    let signs = pane_signs(&ed, pid);
    let sign = signs[&0].first().expect("one sign on the error line");
    assert_eq!(sign.scope, gutter_scope);

    let highlights = ed.state.panes.render[pid].highlights.diagnostics.clone();
    let highlights = highlights.read().unwrap();
    assert!(
        highlights
            .iter()
            .any(|&(_, _, _, scope)| scope == text_scope),
        "buffer-text diagnostic highlight must carry the editing-area scope"
    );
}

#[test]
fn error_beats_warning_on_the_same_line() {
    let tmp = safe_tempdir();
    let file_dir = safe_tempdir();
    let warning: DiagFixture = ((0, 0), (0, 1), 2, "warn");
    let error: DiagFixture = ((0, 4), (0, 5), 1, "err");
    let (mut ed, _guard) = setup_diagnostics(
        "abcdefgh\n",
        &file_dir.path().join("main.rs"),
        tmp.path(),
        &[warning, error],
    );
    let pid = ed.state.focused_pane_id;
    render(&mut ed);
    let error_scope = ed.view.registry.get("error").unwrap();

    let signs = pane_signs(&ed, pid);
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
    let tmp = safe_tempdir();
    let file_dir = safe_tempdir();
    let diag: DiagFixture = ((0, 2), (1, 3), 1, "boom");
    let (mut ed, _guard) = setup_diagnostics(
        "abc\ndef\n",
        &file_dir.path().join("main.rs"),
        tmp.path(),
        &[diag],
    );
    let pid = ed.state.focused_pane_id;
    render(&mut ed);
    let signs = pane_signs(&ed, pid);
    assert_eq!(
        signs.len(),
        2,
        "both lines the diagnostic touches get a sign"
    );
    assert!(signs.contains_key(&0));
    assert!(signs.contains_key(&1));
}

/// `diagnostics-for-buffer`'s `"end-line"` field itself (D9), independent of
/// its sign-placement consequence above: equal to `"line"` for a diagnostic
/// that stays on one line, greater for one that crosses a line boundary —
/// the span `lsp/diagnostic-signs` expands over.
#[test]
fn end_line_equals_line_for_single_line_and_diverges_for_multiline() {
    let tmp = safe_tempdir();
    let file_dir = safe_tempdir();
    let single: DiagFixture = ((0, 2), (0, 5), 1, "single-line");
    let multi: DiagFixture = ((0, 2), (1, 3), 1, "multi-line");
    let (ed, _guard) = setup_diagnostics(
        "abc\ndef\n",
        &file_dir.path().join("main.rs"),
        tmp.path(),
        &[single, multi],
    );
    let bid = ed.focused_buffer_id();

    let entries =
        crate::editor::lsp::introspect::diagnostics_for_buffer(&ed.state, &ed.lsp, bid, None, None)
            .expect("diagnostics-for-buffer must not error");
    let find = |msg: &str| {
        entries
            .iter()
            .find(|e| e["message"] == msg)
            .unwrap_or_else(|| panic!("missing diagnostic {msg:?}"))
    };

    let single_entry = find("single-line");
    assert_eq!(
        single_entry["line"], single_entry["end-line"],
        "a diagnostic that stays on one line has end-line == line"
    );

    let multi_entry = find("multi-line");
    assert_eq!(multi_entry["line"], 0);
    assert_eq!(
        multi_entry["end-line"], 1,
        "a diagnostic crossing a line boundary has end-line > line"
    );
}

#[test]
fn zero_diagnostics_produce_no_signs() {
    let tmp = safe_tempdir();
    let file_dir = safe_tempdir();
    let (mut ed, _guard) = setup_diagnostics(
        "abcdefgh\n",
        &file_dir.path().join("main.rs"),
        tmp.path(),
        &[],
    );
    let pid = ed.state.focused_pane_id;
    render(&mut ed);
    assert!(pane_signs(&ed, pid).is_empty());
}

#[test]
fn gutter_width_is_the_default_when_a_diagnostic_exists() {
    let tmp = safe_tempdir();
    let file_dir = safe_tempdir();
    let diag: DiagFixture = ((0, 0), (0, 1), 1, "boom");
    let (mut ed, _guard) = setup_diagnostics(
        "abcdefgh\n",
        &file_dir.path().join("main.rs"),
        tmp.path(),
        &[diag],
    );
    let pid = ed.state.focused_pane_id;
    render(&mut ed);
    assert_eq!(
        sign_column_width(&ed, pid),
        2,
        "a diagnostic exists — column shows at the default width"
    );
}

#[test]
fn gutter_width_auto_2_expands_when_signs_exist() {
    let tmp = safe_tempdir();
    let file_dir = safe_tempdir();
    let diag: DiagFixture = ((0, 0), (0, 1), 1, "boom");
    let (mut ed, _guard) = setup_diagnostics(
        "abcdefgh\n",
        &file_dir.path().join("main.rs"),
        tmp.path(),
        &[diag],
    );
    let bid = ed.focused_buffer_id();
    ed.state.buffers.get_mut(bid).overrides.signcolumn = Some("auto:2".parse().unwrap());
    let pid = ed.state.focused_pane_id;
    render(&mut ed);
    assert_eq!(
        sign_column_width(&ed, pid),
        3,
        "auto:2 with signs = 2 sign slots + 1 padding = 3 cells"
    );
}

/// Diagnostic signs are placed through the same `set-signs!` path as any
/// other plugin sign now (source `"lsp-diagnostics"`) — this proves a
/// diagnostic sign and an unrelated plugin sign on the *same* line both
/// survive into one render, in priority order, sharing the one per-pane
/// sign map rather than two.
#[test]
fn diagnostic_and_plugin_sign_share_a_line_and_both_survive_the_merge() {
    let tmp = safe_tempdir();
    let file_dir = safe_tempdir();
    let diag: DiagFixture = ((0, 0), (0, 1), 1, "boom");
    let (mut ed, _guard) = setup_diagnostics(
        "abcdefgh\n",
        &file_dir.path().join("main.rs"),
        tmp.path(),
        &[diag],
    );

    let bid = ed.focused_buffer_id();
    ed.state.buffers.get_mut(bid).overrides.signcolumn = Some("always:2".parse().unwrap());

    run(
        &mut ed,
        tmp.path(),
        r#"(define-command! "arm" "" (lambda ()
             (set-signs! "linter" (current-buffer) (list (list 0 "!" "warn-scope" 20)))))"#,
    );
    type_cmd(&mut ed, ":arm");

    let pid = ed.state.focused_pane_id;
    render(&mut ed);

    let rope = ed.state.buffers.get(bid).text().rope().clone();
    let gutter_ctx = hume_engine::providers::GutterRowCtx {
        mode: hume_engine::types::EditorMode::Normal,
        primary_head_line: 0,
        rope: &rope,
    };
    let col = ed.view.panes[pid]
        .providers
        .gutter_columns()
        .next()
        .expect("sign column registered first");
    let cells = col.render_row_cells(
        hume_engine::types::RowKind::LineStart { line_idx: 0 },
        &gutter_ctx,
    );
    assert_eq!(
        cells.len(),
        2,
        "always:2 keeps 2 slots; both the diagnostic sign and the plugin sign must survive"
    );
    assert_eq!(
        cells[0].as_str(),
        "!",
        "plugin sign priority 20 outranks the diagnostic's fixed priority 10 — slot 0"
    );
    assert_eq!(
        cells[1].as_str(),
        "●",
        "diagnostic's fixed priority 10 resolves to slot 1"
    );
}

/// The sign-priority ladder is built from the whole buffer, not the current
/// viewport — a diagnostic scrolled out of view still reserves its
/// priority's slot, so a lower-priority plugin sign on a visible line
/// doesn't slide into slot 0 just because the diagnostic isn't sharing this
/// particular frame with it.
#[test]
fn ladder_is_buffer_wide_not_viewport_restricted() {
    let tmp = safe_tempdir();
    let file_dir = safe_tempdir();
    let content: String = "line\n".repeat(60);
    let diag: DiagFixture = ((50, 0), (50, 1), 1, "boom");
    let (mut ed, _guard) = setup_diagnostics(
        &content,
        &file_dir.path().join("main.rs"),
        tmp.path(),
        &[diag],
    );

    run(
        &mut ed,
        tmp.path(),
        r#"(define-command! "arm" "" (lambda ()
             (set-signs! "git-diff" (current-buffer) (list (list 0 "+" "diff.plus.gutter" 0)))))"#,
    );
    type_cmd(&mut ed, ":arm");

    let pid = ed.state.focused_pane_id;
    render(&mut ed);

    let signs = pane_signs(&ed, pid);
    assert!(
        !signs.contains_key(&50),
        "the diagnostic on line 50 is scrolled out of the 25-row viewport"
    );
    let sign = signs[&0].first().expect("plugin sign on line 0");
    assert_eq!(
        sign.slot, 1,
        "the off-screen diagnostic (priority 10) still reserves slot 0 on the \
         buffer-wide ladder — the visible priority-0 git sign is pushed to slot 1"
    );
}

/// Regression: a stored diagnostic computed against the pre-reload text can
/// end up with offsets past the new (shorter) content. `:e!` must clear
/// diagnostics for the reloaded buffer and re-fire `on-diagnostics-changed`
/// so `core:lsp` clears the now-stale sign along with the EOL summary —
/// this proves the sign side of that hook does the same, not just the
/// text-highlight side `introspect::diagnostics_for_buffer` already guards
/// via its own end-of-file clamp.
#[test]
fn reload_to_shorter_text_clears_stale_diagnostics_and_does_not_panic() {
    let tmp = safe_tempdir();
    let file_dir = safe_tempdir();
    let file = file_dir.path().join("main.rs");
    // Wire end character 27 — the original (longer) content's length, far
    // past the much shorter reloaded content's 4 chars below.
    let diag: DiagFixture = ((0, 0), (0, 27), 1, "boom");
    let (mut ed, _guard) =
        setup_diagnostics("one two three four five six\n", &file, tmp.path(), &[diag]);
    let bid = ed.focused_buffer_id();
    assert_eq!(
        ed.lsp.diagnostic_counts_for_test(bid),
        (1, 0),
        "seed diagnostic must land before the reload"
    );

    std::fs::write(&file, "one\n").unwrap();
    ed.execute_typed("e!", None).unwrap();
    ed.settle();

    assert_eq!(
        ed.lsp.diagnostic_counts_for_test(bid),
        (0, 0),
        "reload must clear diagnostics computed against the pre-reload text"
    );

    let pid = ed.state.focused_pane_id;
    render(&mut ed); // must not panic

    let signs = pane_signs(&ed, pid);
    assert!(
        signs.is_empty(),
        "no stale sign should remain after reload clears diagnostics"
    );
}
