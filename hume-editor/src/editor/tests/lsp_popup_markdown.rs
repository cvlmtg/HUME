// Markdown highlighting for the LSP hover popup (`show-popup! #:lang
// "markdown"`): grammar-optional — highlighted through the real tree-sitter
// pipeline when a `markdown` grammar is registered, plain text otherwise
// (both when `#:lang` isn't set, and when it is but no such grammar exists).
//
// Requires scripts/fetch-test-grammars.sh (markdown) for the two tests that
// need a real grammar; the fallback test needs no fixture.

use std::path::Path;

use super::*;
use hume_engine::pipeline::RenderContext;
use hume_scripting::ScriptingHost;
use hume_test_fixtures::skip_unless_grammars;

fn run(ed: &mut Editor, tmp: &Path, source: &str) {
    let mut host = ScriptingHost::new();
    eval_with_real_host(ed, &mut host, source, tmp);
    ed.scripting = Some(host);
}

/// Attach the real `markdown` grammar fixture, no injections — these tests
/// only check that top-level spans reach the popup, not fenced-code
/// injection (already covered end-to-end for buffers by `injections_editor.rs`).
fn register_markdown(ed: &mut Editor) {
    let parser_path = grammar_parser_path("markdown");
    let hl_path = grammar_query_path("markdown");
    ed.state
        .config
        .languages
        .register_identity("markdown", &["md"], &[], &[])
        .unwrap();
    ed.state
        .config
        .languages
        .attach_grammar(
            "markdown",
            &parser_path,
            "tree_sitter_markdown",
            &hl_path,
            None,
            &mut ed.view.registry,
        )
        .unwrap_or_else(|e| panic!("attach markdown: {e}"));
}

fn styled_rows(ed: &Editor) -> Option<Vec<crate::ui::popup::StyledRow>> {
    ed.state
        .popup_view
        .read()
        .unwrap()
        .as_ref()
        .and_then(|s| s.styled_rows.as_deref().cloned())
}

/// The docked (`#:anchor 'bottom`) counterpart of [`styled_rows`] — reads
/// `popup_band_view`, not `popup_view` (empty for a docked popup).
fn band_styled_rows(ed: &Editor) -> Option<Vec<crate::ui::popup::StyledRow>> {
    ed.state
        .popup_band_view
        .read()
        .unwrap()
        .as_ref()
        .and_then(|s| s.styled_rows.as_deref().cloned())
}

#[test]
fn markdown_popup_highlights_when_the_grammar_is_registered() {
    if skip_unless_grammars(&["markdown"]) {
        return;
    }
    let tmp = safe_tempdir();
    let mut ed = editor_from("-[x]>abcdefgh\n");
    register_markdown(&mut ed);
    run(
        &mut ed,
        tmp.path(),
        r##"(define-command! "go" "" (lambda () (show-popup! "# heading" #:lang "markdown")))"##,
    );
    type_cmd(&mut ed, ":go");

    assert!(
        ed.state.config.popup.as_ref().unwrap().syntax.is_some(),
        "a registered markdown grammar must attach synchronous syntax to the popup"
    );

    let mut ctx = RenderContext::new();
    ed.prepare_frame(80, 25, &mut ctx);

    let rows = styled_rows(&ed).expect("markdown popup must resolve styled rows");
    assert_eq!(rows.len(), 1, "single-line content wraps to one row");
    let runs = &rows[0];
    // The ATX heading marker ("#") and the heading text carry different
    // scopes in the real markdown grammar's highlights.scm, so a correctly
    // highlighted line must coalesce into at least two distinct-style runs.
    // Asserting against the popup's own *base* style (not `Style::default`,
    // which the base style already isn't) is the real oracle here — every
    // run trivially differs from `Style::default` regardless of whether any
    // tree-sitter span was ever applied, which would make this assertion
    // pass even if highlighting were completely broken.
    assert!(
        runs.len() > 1,
        "a heading line highlighted by a real grammar must not come back as \
         a single run — that would mean nothing was actually highlighted, \
         got {runs:?}"
    );
    let distinct_styles: std::collections::HashSet<_> = runs.iter().map(|(_, s)| *s).collect();
    assert!(
        distinct_styles.len() > 1,
        "the heading marker and the heading text must carry different \
         styles, got {runs:?}"
    );
    // Highlighting must never drop or reorder characters — the runs must
    // still concatenate to exactly the source line.
    let flattened: String = runs.iter().map(|(s, _)| s.as_str()).collect();
    assert_eq!(flattened, "# heading");
}

#[test]
fn markdown_popup_paints_per_run_styles() {
    // Appearance lock for the `draw_menu_box` styled-runs branch (the
    // data-level test above only checks `PopupState.styled_rows`, not that
    // painting actually applies per-run styles to the terminal buffer).
    if skip_unless_grammars(&["markdown"]) {
        return;
    }
    let tmp = safe_tempdir();
    let mut ed = Editor::open(None, std::sync::Arc::new(|| {})).unwrap();
    ed.view.theme = crate::ui::theme::build_dark_theme_for_snapshot_tests();
    register_markdown(&mut ed);
    run(
        &mut ed,
        tmp.path(),
        r##"(define-command! "go" "" (lambda () (show-popup! "# heading\n\nplain text" #:lang "markdown")))"##,
    );
    type_cmd(&mut ed, ":go");

    use ratatui::layout::Rect;
    let rect = Rect::new(0, 0, 30, 10);
    let snap = render_snapshot::render_to_styled_string(&mut ed, rect);
    insta::assert_snapshot!(snap);
}

#[test]
fn docked_popup_highlights_when_the_grammar_is_registered() {
    // Same syntax-build path as the cursor popup (`#:lang` is layout-
    // independent) but resolved into `popup_band_view`, not `popup_view` —
    // the docked layout hover overflow actually uses (`#:anchor 'bottom`).
    if skip_unless_grammars(&["markdown"]) {
        return;
    }
    let tmp = safe_tempdir();
    let mut ed = editor_from("-[x]>abcdefgh\n");
    register_markdown(&mut ed);
    run(
        &mut ed,
        tmp.path(),
        r##"(define-command! "go" "" (lambda ()
             (show-popup! "# heading" #:lang "markdown" #:anchor 'bottom)))"##,
    );
    type_cmd(&mut ed, ":go");

    assert!(
        ed.state.config.popup.as_ref().unwrap().syntax.is_some(),
        "a registered markdown grammar must attach synchronous syntax to a docked popup"
    );

    let mut ctx = RenderContext::new();
    ed.prepare_frame(80, 25, &mut ctx);

    let rows = band_styled_rows(&ed).expect("docked markdown popup must resolve styled rows");
    assert_eq!(rows.len(), 1, "single-line content wraps to one row");
    let runs = &rows[0];
    assert!(
        runs.len() > 1,
        "a heading line highlighted by a real grammar must not come back as \
         a single run, got {runs:?}"
    );
    let flattened: String = runs.iter().map(|(s, _)| s.as_str()).collect();
    assert_eq!(flattened, "# heading");
}

#[test]
fn docked_popup_survives_a_multiline_capture_node() {
    // Regression for the `z k` panic: `(fenced_code_block) @text.literal`
    // captures the whole block (opening fence through closing fence), not
    // just its own line — `collect_line_spans` used to emit that node's
    // absolute end byte relative to *this* line's start with no clamp, so
    // on the (short) opening-fence line the span end ran past the line's
    // own length and `MarkupSyntax::styled_row`'s `&line[start..end]` slice
    // panicked. Single-line grammars never produced an over-long span, so
    // this only ever surfaced through markdown. `styled_row` is shared by
    // every caller (cursor popup, docked popup) — exercised here through
    // the docked layout, hover's actual long-content path.
    if skip_unless_grammars(&["markdown"]) {
        return;
    }
    let tmp = safe_tempdir();
    let mut ed = Editor::open(None, std::sync::Arc::new(|| {})).unwrap();
    ed.view.theme = crate::ui::theme::build_dark_theme_for_snapshot_tests();
    register_markdown(&mut ed);
    run(
        &mut ed,
        tmp.path(),
        r##"(define-command! "go" "" (lambda ()
             (show-popup! "```rust\nfn foo() -> i32\n```\ndoc text" #:lang "markdown" #:anchor 'bottom)))"##,
    );
    type_cmd(&mut ed, ":go");

    let mut ctx = RenderContext::new();
    ed.prepare_frame(40, 10, &mut ctx);

    use ratatui::layout::Rect;
    let rect = Rect::new(0, 0, 40, 10);
    let snap = render_snapshot::render_to_styled_string(&mut ed, rect);
    insta::assert_snapshot!(snap);
}

#[test]
fn popup_without_markdown_flag_stays_plain_even_with_the_grammar_registered() {
    if skip_unless_grammars(&["markdown"]) {
        return;
    }
    let tmp = safe_tempdir();
    let mut ed = editor_from("-[x]>abcdefgh\n");
    register_markdown(&mut ed);
    run(
        &mut ed,
        tmp.path(),
        r##"(define-command! "go" "" (lambda () (show-popup! "# heading")))"##,
    );
    type_cmd(&mut ed, ":go");

    assert!(
        ed.state.config.popup.as_ref().unwrap().syntax.is_none(),
        "without #:lang, the popup must stay plain even when a markdown \
         grammar is registered"
    );

    let mut ctx = RenderContext::new();
    ed.prepare_frame(80, 25, &mut ctx);
    assert!(
        styled_rows(&ed).is_none(),
        "plain popup must not populate styled_rows"
    );
}

#[test]
fn markdown_flag_without_a_registered_grammar_falls_back_to_plain() {
    // No grammar registered at all — this test needs no fixture.
    let tmp = safe_tempdir();
    let mut ed = editor_from("-[x]>abcdefgh\n");
    run(
        &mut ed,
        tmp.path(),
        r##"(define-command! "go" "" (lambda () (show-popup! "# heading" #:lang "markdown")))"##,
    );
    type_cmd(&mut ed, ":go");

    assert!(
        ed.state.config.popup.as_ref().unwrap().syntax.is_none(),
        "#:lang \"markdown\" with no markdown grammar registered must fall \
         back to plain, not error"
    );

    let mut ctx = RenderContext::new();
    ed.prepare_frame(80, 25, &mut ctx);
    assert!(
        styled_rows(&ed).is_none(),
        "grammar-absent fallback must not populate styled_rows"
    );
    let lines = (*ed.state.popup_view.read().unwrap().as_ref().unwrap().lines).clone();
    assert_eq!(
        lines,
        vec!["# heading".to_string()],
        "plain fallback still renders the text itself"
    );
}
