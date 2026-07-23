// Markdown highlighting for the drawer (`show-drawer-list! items on-select
// #:lang "markdown"`) — the drawer counterpart of `lsp_popup_markdown.rs`.
// Grammar-optional: highlighted through the real tree-sitter pipeline (one
// row = one parsed line, `MarkupSyntax::styled_row`) when a `markdown`
// grammar is registered, plain text otherwise (both when `#:lang` isn't
// set, and when it is but no such grammar exists).
//
// Requires scripts/fetch-test-grammars.sh (markdown) for the tests that need
// a real grammar; the fallback tests need no fixture.

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

/// Attach the real `markdown` grammar fixture, no injections — mirrors
/// `lsp_popup_markdown.rs`'s helper of the same shape.
fn register_markdown(ed: &mut Editor) {
    let parser_path = grammar_parser_path("markdown");
    let hl_path = grammar_query_path("markdown");
    ed.state
        .languages
        .register_identity("markdown", &["md"], &[], &[])
        .unwrap();
    ed.state
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

#[test]
fn drawer_attaches_syntax_when_the_grammar_is_registered() {
    if skip_unless_grammars(&["markdown"]) {
        return;
    }
    let tmp = tempfile::tempdir().unwrap();
    let mut ed = editor_from("-[x]>abcdefgh\n");
    register_markdown(&mut ed);
    run(
        &mut ed,
        tmp.path(),
        r##"(define-command! "go" "" (lambda ()
             (show-drawer-list! (list "# heading" "plain text") (lambda (idx) (void))
               #:lang "markdown")))"##,
    );
    type_cmd(&mut ed, ":go");

    assert!(
        ed.state.drawer.as_ref().unwrap().syntax.is_some(),
        "a registered markdown grammar must attach synchronous syntax to the drawer"
    );
}

#[test]
fn drawer_without_lang_stays_plain_even_with_the_grammar_registered() {
    if skip_unless_grammars(&["markdown"]) {
        return;
    }
    let tmp = tempfile::tempdir().unwrap();
    let mut ed = editor_from("-[x]>abcdefgh\n");
    register_markdown(&mut ed);
    run(
        &mut ed,
        tmp.path(),
        r##"(define-command! "go" "" (lambda ()
             (show-drawer-list! (list "# heading") (lambda (idx) (void)))))"##,
    );
    type_cmd(&mut ed, ":go");

    assert!(
        ed.state.drawer.as_ref().unwrap().syntax.is_none(),
        "without #:lang, the drawer must stay plain even when a markdown \
         grammar is registered"
    );
}

#[test]
fn lang_without_a_registered_grammar_falls_back_to_plain() {
    // No grammar registered at all — this test needs no fixture.
    let tmp = tempfile::tempdir().unwrap();
    let mut ed = editor_from("-[x]>abcdefgh\n");
    run(
        &mut ed,
        tmp.path(),
        r##"(define-command! "go" "" (lambda ()
             (show-drawer-list! (list "# heading") (lambda (idx) (void))
               #:lang "markdown")))"##,
    );
    type_cmd(&mut ed, ":go");

    assert!(
        ed.state.drawer.as_ref().unwrap().syntax.is_none(),
        "#:lang \"markdown\" with no markdown grammar registered must fall \
         back to plain, not error"
    );
    let rows = ed
        .state
        .drawer_view
        .read()
        .unwrap()
        .as_ref()
        .unwrap()
        .rows
        .clone();
    assert_eq!(
        rows,
        vec!["# heading".to_string()],
        "plain fallback still renders the row text itself"
    );
}

#[test]
fn drawer_paints_per_run_styles() {
    // Appearance lock for the drawer's styled-row render branch (the
    // data-level test above only checks `DrawerModel.syntax`, not that
    // painting actually applies per-run styles to the terminal buffer).
    //
    // "plain text" comes first so it lands on row 0, the default selection —
    // the *heading* row must be the non-selected one, since a selected row
    // always paints in the flat `selected_style` regardless of `syntax`
    // (drawer.rs: "highlight bar always wins"), which would otherwise mask
    // the very highlighting this test exists to check.
    if skip_unless_grammars(&["markdown"]) {
        return;
    }
    let tmp = tempfile::tempdir().unwrap();
    let mut ed = Editor::open(None, std::sync::Arc::new(|| {})).unwrap();
    ed.view.theme = crate::ui::theme::build_dark_theme_for_snapshot_tests();
    register_markdown(&mut ed);
    run(
        &mut ed,
        tmp.path(),
        r##"(define-command! "go" "" (lambda ()
             (show-drawer-list! (list "plain text" "# heading") (lambda (idx) (void))
               #:lang "markdown")))"##,
    );
    type_cmd(&mut ed, ":go");

    let mut ctx = RenderContext::new();
    ed.prepare_frame(40, 10, &mut ctx);

    use ratatui::layout::Rect;
    let rect = Rect::new(0, 0, 40, 10);
    let snap = render_snapshot::render_to_styled_string(&mut ed, rect);
    insta::assert_snapshot!(snap);
}
