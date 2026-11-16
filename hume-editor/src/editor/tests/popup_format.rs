// `format-text/<lang>` commands applied to popups: what a formatter's
// conceals, highlights and line backgrounds draw in a `show-popup! #:lang`
// popup, and when the editor calls one.

use super::*;
use hume_engine::theme::Theme;
use hume_engine::types::{Modifiers, ResolvedStyle};
use hume_grid::Rgb;
use hume_ui::popup::StyledRow;
use test_fixtures::require_grammars;

const WHITE: Rgb = Rgb(255, 255, 255);
const BLACK: Rgb = Rgb(0, 0, 0);
const RED: Rgb = Rgb(255, 0, 0);
const BLUE: Rgb = Rgb(0, 0, 255);
const YELLOW: Rgb = Rgb(255, 255, 0);

fn style(fg: Option<Rgb>, bg: Option<Rgb>) -> ResolvedStyle {
    ResolvedStyle {
        fg,
        bg,
        ..Default::default()
    }
}

/// `ui.popup` in this file's theme.
fn base() -> ResolvedStyle {
    style(Some(WHITE), Some(BLACK))
}

/// A heading's marker and text in this file's theme: the same, so the style
/// under a heading's first char does not depend on which capture wins.
fn heading() -> ResolvedStyle {
    style(None, Some(RED))
}

/// A heading's char as the popup draws it: its style over `ui.popup`.
fn heading_cell() -> ResolvedStyle {
    base().layer(heading())
}

fn theme() -> Theme {
    let styles = std::collections::HashMap::from([
        ("ui.popup", base()),
        ("markup.heading.marker", heading()),
        ("markup.heading.1", heading()),
        ("glyph", style(Some(BLUE), None)),
        (
            "hl",
            ResolvedStyle {
                modifiers: Modifiers::BOLD,
                ..Default::default()
            },
        ),
        (
            "band",
            ResolvedStyle {
                fg: Some(RED),
                bg: Some(YELLOW),
                modifiers: Modifiers::REVERSED,
                ..Default::default()
            },
        ),
    ]);
    Theme::new(styles)
}

/// An editor with the `markdown` grammar and this file's theme.
fn markdown_editor() -> Editor {
    require_grammars(&["markdown"]);
    let mut ed = editor_from("-[x]>abcdefgh\n");
    ed.view.theme = theme();
    register_markdown(&mut ed);
    ed
}

/// Defines `format-text/markdown` as `(lambda (text parsed) body)` and
/// `:go`, which shows `text` as a markdown popup.
fn define(ed: &mut Editor, tmp: &std::path::Path, text: &str, body: &str) {
    run(
        ed,
        tmp,
        &format!(
            r#"(define-command! "format-text/markdown" "" (lambda (text parsed) {body}))
               (define-typed-command! "go" "" (lambda (pane) (show-popup! pane {text:?} #:lang "markdown")))"#
        ),
    );
}

/// The popup's rows after `:go` and a frame.
fn shown_rows(ed: &mut Editor) -> Vec<StyledRow> {
    type_cmd(ed, ":go");
    frame(ed, 80, 24);
    ed.state
        .views
        .popup
        .read()
        .as_ref()
        .and_then(|s| s.rows.lines.styled().map(<[_]>::to_vec))
        .expect("a markdown popup shows styled rows")
}

/// `text` shown as a markdown popup that `body` formats: its rows.
fn formatted(text: &str, body: &str) -> Vec<StyledRow> {
    let tmp = tempfile::tempdir().unwrap();
    let mut ed = markdown_editor();
    define(&mut ed, tmp.path(), text, body);
    let rows = shown_rows(&mut ed);
    assert_eq!(error_log(&ed), Vec::<String>::new());
    rows
}

fn texts(rows: &[StyledRow]) -> Vec<String> {
    rows.iter()
        .map(|row| row.runs.iter().map(|(text, _)| text.as_str()).collect())
        .collect()
}

fn runs(text: &str, style: ResolvedStyle) -> (String, ResolvedStyle) {
    (text.to_owned(), style)
}

// ── What a format draws ──────────────────────────────────────────────────

#[test]
fn a_hidden_range_drops_its_chars() {
    let rows = formatted(
        "ab cd",
        "(hash 'conceals (list (hash 'start-char 2 'end-char 3)))",
    );
    assert_eq!(rows[0].runs, [runs("abcd", base())]);
}

#[test]
fn offsets_count_a_crlf_break_as_one_char() {
    // The formatter gets "ab\ncd ef\n": char 5 is the space after "cd".
    let rows = formatted(
        "ab\r\ncd ef",
        "(hash 'conceals (list (hash 'start-char 5 'end-char 6)))",
    );
    assert_eq!(texts(&rows), ["ab", "cdef"]);
}

#[test]
fn a_tab_draws_as_one_blank_column() {
    let rows = formatted("a\tb", "(hash)");
    assert_eq!(texts(&rows), ["a b"]);
}

#[test]
fn a_wide_char_draws_once() {
    let rows = formatted("a中b", "(hash)");
    assert_eq!(texts(&rows), ["a中b"]);
}

#[test]
fn a_control_char_draws_as_its_placeholder() {
    let rows = formatted("a\u{7}b", "(hash)");
    insta::assert_snapshot!(texts(&rows).concat(), @"a<7>b");
}

#[test]
fn a_replacement_draws_in_its_scope_over_the_syntax_style() {
    let rows = formatted(
        "# H",
        r#"(hash 'conceals (list (hash 'start-char 0 'end-char 2 'text "1 " 'scope "glyph")))"#,
    );
    // Wrapping draws the space between two words in the second word's style.
    assert_eq!(
        rows[0].runs,
        [
            runs("1", style(Some(BLUE), Some(RED))),
            runs(" H", heading_cell())
        ]
    );
}

#[test]
fn a_replacement_without_a_scope_keeps_the_syntax_style() {
    let rows = formatted(
        "# H",
        r#"(hash 'conceals (list (hash 'start-char 0 'end-char 2 'text "1 ")))"#,
    );
    assert_eq!(rows[0].runs, [runs("1 H", heading_cell())]);
}

#[test]
fn an_insert_draws_before_its_char() {
    let rows = formatted(
        "ab",
        r#"(hash 'conceals (list (hash 'start-char 1 'end-char 1 'text "X" 'scope "glyph")))"#,
    );
    assert_eq!(
        rows[0].runs,
        [
            runs("a", base()),
            runs("X", style(Some(BLUE), Some(BLACK))),
            runs("b", base())
        ]
    );
}

#[test]
fn an_insert_at_a_lines_end_draws_after_its_text() {
    let rows = formatted(
        "ab\ncd",
        r#"(hash 'conceals (list (hash 'start-char 2 'end-char 2 'text "X")))"#,
    );
    assert_eq!(texts(&rows), ["abX", "cd"]);
}

#[test]
fn a_highlight_layers_over_the_syntax_style() {
    let rows = formatted(
        "# H",
        r#"(hash 'highlights (list (hash 'start-char 2 'end-char 3 'scope "hl")))"#,
    );
    let bold = ResolvedStyle {
        modifiers: Modifiers::BOLD,
        ..heading_cell()
    };
    // Wrapping draws the space between two words in the second word's style.
    assert_eq!(rows[0].runs, [runs("#", heading_cell()), runs(" H", bold)]);
}

#[test]
fn of_two_overlapping_highlights_the_earlier_start_draws() {
    let rows = formatted(
        "abc",
        r#"(hash 'highlights (list (hash 'start-char 1 'end-char 2 'scope "glyph")
                                  (hash 'start-char 0 'end-char 3 'scope "hl")))"#,
    );
    let bold = ResolvedStyle {
        modifiers: Modifiers::BOLD,
        ..base()
    };
    assert_eq!(rows[0].runs, [runs("abc", bold)]);
}

#[test]
fn a_line_background_fills_its_lines_rows_with_its_background_only() {
    let rows = formatted(
        "a\nb",
        r#"(hash 'line-backgrounds (list (hash 'line 1 'scope "band")))"#,
    );
    assert_eq!(rows[0].fill, None);
    assert_eq!(rows[0].runs, [runs("a", base())]);
    assert_eq!(rows[1].fill, Some(YELLOW));
    assert_eq!(rows[1].runs, [runs("b", style(Some(WHITE), Some(YELLOW)))]);
}

#[test]
fn a_concealed_line_wraps_at_its_drawn_width() {
    // Twelve chars of source draw as six, which fit the box's 16-column
    // text area on one row.
    let tmp = tempfile::tempdir().unwrap();
    let mut ed = markdown_editor();
    define(
        &mut ed,
        tmp.path(),
        "aaa bbbbbbb ccc",
        "(hash 'conceals (list (hash 'start-char 3 'end-char 12)))",
    );
    type_cmd(&mut ed, ":go");
    frame(&mut ed, 20, 24);
    let rows = ed
        .state
        .views
        .popup
        .read()
        .as_ref()
        .map(|s| s.rows.lines.texts())
        .expect("the popup shows");
    assert_eq!(rows, ["aaaccc"]);
}

// ── When a formatter runs ────────────────────────────────────────────────

#[test]
fn a_popup_without_a_formatter_stays_highlighted_source() {
    let tmp = tempfile::tempdir().unwrap();
    let mut ed = markdown_editor();
    run(
        &mut ed,
        tmp.path(),
        r##"(define-typed-command! "go" "" (lambda (pane) (show-popup! pane "# H" #:lang "markdown")))"##,
    );
    let rows = shown_rows(&mut ed);
    assert_eq!(texts(&rows), ["# H"]);
    assert_eq!(error_log(&ed), Vec::<String>::new());
}

#[test]
fn each_language_calls_its_own_formatter() {
    require_grammars(&["rust"]);
    let tmp = tempfile::tempdir().unwrap();
    let mut ed = markdown_editor();
    ed.state
        .config
        .languages
        .register_identity("rust", &["rs"], &[], &[])
        .unwrap();
    attach_fixture_grammar(&mut ed, "rust", "tree_sitter_rust");
    run(
        &mut ed,
        tmp.path(),
        r#"(define-command! "format-text/markdown" "" (lambda (text parsed)
             (hash 'conceals (list (hash 'start-char 0 'end-char 1 'text "M")))))
           (define-command! "format-text/rust" "" (lambda (text parsed)
             (hash 'conceals (list (hash 'start-char 0 'end-char 1 'text "R")))))
           (define-typed-command! "go" "" (lambda (pane) (show-popup! pane "ab" #:lang "rust")))"#,
    );
    assert_eq!(texts(&shown_rows(&mut ed)), ["Rb"]);
}

#[test]
fn a_plain_popup_calls_no_formatter() {
    let tmp = tempfile::tempdir().unwrap();
    let mut ed = markdown_editor();
    run(
        &mut ed,
        tmp.path(),
        r#"(define calls 0)
           (define-command! "format-text/markdown" "" (lambda (text parsed)
             (set! calls (+ calls 1))
             (hash)))
           (define-typed-command! "go" "" (lambda (pane) (show-popup! pane "ab")))
           (define-typed-command! "calls" "" (lambda (pane) (log! 'info (number->string calls))))"#,
    );
    type_cmd(&mut ed, ":go");
    frame(&mut ed, 80, 24);
    type_cmd(&mut ed, ":calls");
    assert_eq!(status(&ed), "0");
}

#[test]
fn a_popup_activates_the_lazy_plugin_that_declares_its_formatter() {
    let tmp = tempfile::tempdir().unwrap();
    let mut ed = markdown_editor();
    let plugin_dir = tmp.path().join("plugins").join("user").join("fmt");
    std::fs::create_dir_all(&plugin_dir).unwrap();
    std::fs::write(
        plugin_dir.join("plugin.scm"),
        r#"(define-command! "format-text/markdown" "" (lambda (text parsed)
             (hash 'conceals (list (hash 'start-char 0 'end-char 1)))))"#,
    )
    .unwrap();
    let host = hume_scripting::ScriptingHost::for_testing(&hume_platform::dirs::Dirs {
        data: Some(tmp.path().to_path_buf()),
        ..hume_platform::dirs::Dirs::none()
    });
    install_source(
        &mut ed,
        host,
        r#"(declare-plugin! "user/fmt" #:commands '("format-text/markdown"))
           (define-typed-command! "go" "" (lambda (pane) (show-popup! pane "ab" #:lang "markdown")))"#,
        tmp.path(),
    );
    assert!(
        ed.state
            .config
            .registry
            .lazy_owner("format-text/markdown")
            .is_some(),
        "the plugin waits for its command"
    );
    assert_eq!(texts(&shown_rows(&mut ed)), ["b"]);
    assert_eq!(error_log(&ed), Vec::<String>::new());
}

/// The errors logged after `:go` shows "ab" through a formatter whose body
/// is `body`, and the popup's text.
fn failing(body: &str) -> (Vec<String>, Vec<String>) {
    let tmp = tempfile::tempdir().unwrap();
    let mut ed = markdown_editor();
    define(&mut ed, tmp.path(), "ab", body);
    let rows = shown_rows(&mut ed);
    (error_log(&ed), texts(&rows))
}

#[test]
fn a_formatter_that_raises_is_reported_and_leaves_the_source() {
    let (errors, rows) = failing(r#"(error "boom")"#);
    assert_eq!(rows, ["ab"]);
    assert_eq!(errors.len(), 1, "{errors:?}");
    assert!(
        errors[0].contains("format-text/markdown") && errors[0].contains("boom"),
        "{errors:?}"
    );
}

#[test]
fn a_malformed_result_is_reported_and_leaves_the_source() {
    let (errors, rows) = failing("42");
    assert_eq!(rows, ["ab"]);
    assert_eq!(errors.len(), 1, "{errors:?}");
    assert!(errors[0].contains("format-text/markdown"), "{errors:?}");
}

#[test]
fn a_result_outside_the_text_is_reported_and_leaves_the_source() {
    let (errors, rows) =
        failing("(hash 'highlights (list (hash 'start-char 0 'end-char 9 'scope \"hl\")))");
    assert_eq!(rows, ["ab"]);
    assert_eq!(errors.len(), 1, "{errors:?}");
    assert!(errors[0].contains("'highlights"), "{errors:?}");
}

#[test]
fn a_formatter_that_changes_the_editor_is_refused() {
    let (errors, rows) = failing(
        r#"(begin
             (with-handler (lambda (e) (begin))
                           (set-buffer-option! (focused-pane) "indent-width" 8))
             (hash 'conceals (list (hash 'start-char 0 'end-char 1))))"#,
    );
    assert_eq!(rows, ["ab"]);
    assert_eq!(errors.len(), 1, "{errors:?}");
    assert!(
        errors[0].contains("cannot be changed while rendering"),
        "{errors:?}"
    );
}

#[test]
fn a_theme_change_keeps_the_format_without_calling_the_formatter_again() {
    let tmp = tempfile::tempdir().unwrap();
    let mut ed = markdown_editor();
    ed.state.dirs = repo_runtime_dirs();
    run(
        &mut ed,
        tmp.path(),
        r#"(define calls 0)
           (define-command! "format-text/markdown" "" (lambda (text parsed)
             (set! calls (+ calls 1))
             (hash 'conceals (list (hash 'start-char 0 'end-char 1)))))
           (define-typed-command! "go" "" (lambda (pane) (show-popup! pane "ab" #:lang "markdown")))
           (define-typed-command! "calls" "" (lambda (pane) (log! 'info (number->string calls))))"#,
    );
    assert_eq!(texts(&shown_rows(&mut ed)), ["b"]);

    let fp = FocusedPane::current(&ed.view);
    crate::editor::commands::typed_theme(&mut ed, fp, Some("gruvbox"))
        .expect(":theme gruvbox must succeed");
    frame(&mut ed, 80, 24);
    let rows = ed
        .state
        .views
        .popup
        .read()
        .as_ref()
        .map(|s| s.rows.lines.texts())
        .expect("the popup outlives a theme change");
    assert_eq!(rows, ["b"]);

    // `:calls` closes the popup, so it runs last.
    type_cmd(&mut ed, ":calls");
    assert_eq!(status(&ed), "1");
}

#[test]
fn a_theme_change_keeps_the_documentation_boxs_format_without_calling_the_formatter_again() {
    let tmp = tempfile::tempdir().unwrap();
    let mut ed = markdown_editor();
    ed.state.dirs = repo_runtime_dirs();
    let source = completion_source(
        "test",
        r#"(list (hash "label" "foo" "documentation" (hash "kind" "markdown" "value" "ab")))"#,
        "",
    );
    run(
        &mut ed,
        tmp.path(),
        &format!(
            r#"(define calls 0)
               (define-command! "format-text/markdown" "" (lambda (text parsed)
                 (set! calls (+ calls 1))
                 (hash 'conceals (list (hash 'start-char 0 'end-char 1)))))
               (define-typed-command! "calls" "" (lambda (pane) (log! 'info (number->string calls))))
               {source}"#
        ),
    );
    let doc_rows = |ed: &Editor| {
        ed.state
            .views
            .completion_doc
            .read()
            .as_ref()
            .map(|state| state.rows.lines.texts())
            .expect("the documentation box shows")
    };
    ed.feed_key(key('i'));
    ed.feed_key(key_ctrl(' '));
    ed.settle();
    frame(&mut ed, 80, 24);
    assert_eq!(doc_rows(&ed), ["b"]);

    let fp = FocusedPane::current(&ed.view);
    crate::editor::commands::typed_theme(&mut ed, fp, Some("gruvbox"))
        .expect(":theme gruvbox must succeed");
    ed.settle();
    frame(&mut ed, 80, 24);
    assert_eq!(doc_rows(&ed), ["b"]);

    // The first Esc closes the menu, the second leaves Insert mode.
    ed.feed_key(key_esc());
    ed.feed_key(key_esc());
    type_cmd(&mut ed, ":calls");
    assert_eq!(status(&ed), "1");
}
