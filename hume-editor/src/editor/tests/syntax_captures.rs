// `syntax-captures` through a real editor: a markdown buffer with its
// injected inline layers, queried from a typed command.

use super::*;
use hume_core::OptionalQuery;
use test_fixtures::{
    grammar_parser_path, grammar_query_path, require_grammars, require_helix_query,
};

/// An editor on `source` as a markdown buffer with markdown's injections,
/// the initial parse drained.
fn markdown_editor(source: &str) -> Editor {
    require_helix_query("markdown", OptionalQuery::Injections.file_name());
    markdown_editor_with(source, &[OptionalQuery::Injections])
}

/// [`markdown_editor`] where markdown gets the Helix queries `queries` lists.
fn markdown_editor_with(source: &str, queries: &[OptionalQuery]) -> Editor {
    require_grammars(&["markdown", "markdown.inline", "rust"]);
    super::injections_editor::markdown_editor_with_queries(source, &Dirs::none(), &[], queries, &[])
        .0
}

/// `captures-text` renders a `syntax-captures` result as `[name start..end;…]`.
const CAPTURES_TEXT: &str = r#"
(define (captures-text caps)
  (string-append
    "["
    (apply string-append
      (map (lambda (c)
             (string-append (list-ref c 0) " "
                            (number->string (list-ref c 1)) ".."
                            (number->string (list-ref c 2)) ";"))
           caps))
    "]"))"#;

/// Define `:check`, which logs what `call` returns, run with the command's
/// pane bound to `bid`, as `captures-text` renders it.
fn define_check(ed: &mut Editor, tmp: &std::path::Path, call: &str) {
    run(
        ed,
        tmp,
        &format!(
            r#"{CAPTURES_TEXT}
               (define-typed-command! "check" "" (lambda (bid) (log! 'info (captures-text {call}))))"#
        ),
    );
}

/// Run `:check` and read what it logged.
fn check(ed: &mut Editor) -> String {
    type_cmd(ed, ":check");
    status(ed)
}

/// [`define_check`] for `call`, then [`check`].
fn captures(ed: &mut Editor, tmp: &std::path::Path, call: &str) -> String {
    define_check(ed, tmp, call);
    check(ed)
}

/// What `expr` evaluates to, run from a typed command whose pane is `bid`,
/// as `to-string` renders it.
fn value_of(ed: &mut Editor, tmp: &std::path::Path, expr: &str) -> String {
    run(
        ed,
        tmp,
        &format!(
            r#"(define-typed-command! "value" "" (lambda (bid) (log! 'info (to-string {expr}))))"#
        ),
    );
    type_cmd(ed, ":value");
    status(ed)
}

/// The errors logged by running `call` from a typed command whose pane is
/// `bid`.
fn errors_of(ed: &mut Editor, tmp: &std::path::Path, call: &str) -> Vec<String> {
    run(
        ed,
        tmp,
        &format!(r#"(define-typed-command! "go" "" (lambda (bid) {call}))"#),
    );
    type_cmd(ed, ":go");
    error_log(ed)
}

const HEADINGS: &str = r#"(syntax-captures bid "markdown" "(atx_heading) @h")"#;

/// `call` after inserting `text`, the body of a Steel string literal, at the
/// start of the buffer, in one expression.
fn insert_then(text: &str, call: &str) -> String {
    format!(
        r#"(begin (edit-text! bid (list (hash 'start-char 0 'end-char 0 'text "{text}"))) {call})"#
    )
}

/// A `# Z` heading and a blank line, for [`insert_then`].
const HEADING_Z: &str = r"# Z\n\n";

/// Headings on lines 0, 2 and 4: chars 0..4, 5..9 and 10..14.
const THREE_HEADINGS: &str = "# A\n\n# B\n\n# C\n";

const EMPHASIS: &str = r#"(syntax-captures bid "markdown.inline" "(emphasis) @e")"#;

#[test]
fn multibyte_text_yields_char_offsets() {
    let tmp = tempfile::tempdir().unwrap();
    let mut ed = markdown_editor("漢字\n\n# A\n");
    // The heading starts at char 4 (byte 8) and ends after its line break.
    assert_eq!(captures(&mut ed, tmp.path(), HEADINGS), "[h 4..8;]");
}

#[test]
fn edit_then_query_in_one_command_reads_the_edited_tree() {
    let tmp = tempfile::tempdir().unwrap();
    let mut ed = markdown_editor("# A\n");
    let call = insert_then(HEADING_Z, HEADINGS);
    assert_eq!(captures(&mut ed, tmp.path(), &call), "[h 0..4;h 5..9;]");
}

/// The root is not `markdown.inline`: its injections query is what lets the
/// edited tree's inline layers be parsed before the query.
#[test]
fn edit_then_inline_query_in_one_command_reads_the_edited_tree() {
    let tmp = tempfile::tempdir().unwrap();
    let mut ed = markdown_editor("a\n");
    let call = insert_then("*x* ", EMPHASIS);
    assert_eq!(captures(&mut ed, tmp.path(), &call), "[e 0..3;]");
}

/// Markdown without an injections query: the root being the queried
/// language is what lets the edited tree be parsed before the query.
#[test]
fn edit_then_root_query_without_injections_reads_the_edited_tree() {
    let tmp = tempfile::tempdir().unwrap();
    let mut ed = markdown_editor_with("# A\n", &[]);
    let call = insert_then(HEADING_Z, HEADINGS);
    assert_eq!(captures(&mut ed, tmp.path(), &call), "[h 0..4;h 5..9;]");
}

#[test]
fn injected_inline_captures_are_included() {
    let tmp = tempfile::tempdir().unwrap();
    let mut ed = markdown_editor("é *x* y\n");
    assert_eq!(captures(&mut ed, tmp.path(), EMPHASIS), "[e 2..5;]");
}

#[test]
fn unknown_language_raises_naming_it() {
    let tmp = tempfile::tempdir().unwrap();
    let mut ed = markdown_editor("# A\n");
    let errors = errors_of(
        &mut ed,
        tmp.path(),
        r#"(syntax-captures bid "klingon" "(x) @x")"#,
    );
    assert!(
        errors
            .iter()
            .any(|e| e.contains("syntax-captures: no grammar for language 'klingon'")),
        "{errors:?}"
    );
}

#[test]
fn language_without_a_grammar_raises_naming_it() {
    let tmp = tempfile::tempdir().unwrap();
    let mut ed = markdown_editor("# A\n");
    ed.state
        .config
        .languages
        .register_identity("toml", &["toml"], &[], &[])
        .unwrap();
    let errors = errors_of(
        &mut ed,
        tmp.path(),
        r#"(syntax-captures bid "toml" "(x) @x")"#,
    );
    assert!(
        errors
            .iter()
            .any(|e| e.contains("syntax-captures: no grammar for language 'toml'")),
        "{errors:?}"
    );
}

#[test]
fn query_that_does_not_compile_raises_with_its_position() {
    let tmp = tempfile::tempdir().unwrap();
    let mut ed = markdown_editor("# A\n");
    let errors = errors_of(
        &mut ed,
        tmp.path(),
        r#"(syntax-captures bid "markdown" "(atx_heading) @h\n  (no_such_node) @n")"#,
    );
    // Row 2, column 4, 1-based: the node name on the second line.
    assert!(
        errors.iter().any(|e| e.contains(
            "syntax-captures: 'markdown' query: Query error at 2:4. Invalid node type \"no_such_node\""
        )),
        "{errors:?}"
    );
}

#[test]
fn reattached_grammar_answers_from_its_new_bundles_tree() {
    let tmp = tempfile::tempdir().unwrap();
    let mut ed = markdown_editor("# A\n");
    assert_eq!(captures(&mut ed, tmp.path(), HEADINGS), "[h 0..4;]");
    run(
        &mut ed,
        tmp.path(),
        &format!(
            r#"(define-typed-command! "reattach" "" (lambda (bid)
                 (%register-grammar! "markdown" {:?} "tree_sitter_markdown" {:?} #f)))"#,
            grammar_parser_path("markdown"),
            grammar_query_path("markdown"),
        ),
    );
    type_cmd(&mut ed, ":reattach");
    assert_eq!(error_log(&ed), Vec::<String>::new());
    // The new attachment has no committed tree until its first parse lands.
    assert_eq!(check(&mut ed), "[]");
    ed.settle();
    assert_eq!(check(&mut ed), "[h 0..4;]");
}

#[test]
fn over_cap_buffer_returns_the_empty_list() {
    let tmp = tempfile::tempdir().unwrap();
    let mut ed = markdown_editor("# A\n");
    define_check(&mut ed, tmp.path(), &insert_then(HEADING_Z, HEADINGS));
    // Below the buffer's size, so the edit leaves the tree stale.
    ed.state.settings.syntax_highlight_max_bytes = 1;
    assert_eq!(check(&mut ed), "[]");
}

#[test]
fn buffer_with_no_syntax_returns_the_empty_list() {
    let tmp = tempfile::tempdir().unwrap();
    let mut ed = markdown_editor("# A\n");
    open_other_buffer(&mut ed, "# B\n");
    assert_eq!(captures(&mut ed, tmp.path(), HEADINGS), "[]");
}

#[test]
fn range_keyword_returns_the_captures_of_the_lines_it_names() {
    let tmp = tempfile::tempdir().unwrap();
    let mut ed = markdown_editor(THREE_HEADINGS);
    let call = r#"(syntax-captures bid "markdown" "(atx_heading) @h" #:range (hash 'start-line 2 'end-line 3))"#;
    assert_eq!(captures(&mut ed, tmp.path(), call), "[h 5..9;]");
}

#[test]
fn range_covering_the_buffer_returns_every_capture() {
    let tmp = tempfile::tempdir().unwrap();
    let mut ed = markdown_editor(THREE_HEADINGS);
    let call = r#"(syntax-captures bid "markdown" "(atx_heading) @h" #:range (hash 'start-line 0 'end-line 5))"#;
    assert_eq!(
        captures(&mut ed, tmp.path(), call),
        "[h 0..4;h 5..9;h 10..14;]"
    );
}

#[test]
fn empty_range_returns_the_empty_list() {
    let tmp = tempfile::tempdir().unwrap();
    let mut ed = markdown_editor(THREE_HEADINGS);
    let call = r#"(syntax-captures bid "markdown" "(atx_heading) @h" #:range (hash 'start-line 0 'end-line 0))"#;
    assert_eq!(captures(&mut ed, tmp.path(), call), "[]");
}

#[test]
fn range_past_the_line_count_raises() {
    let tmp = tempfile::tempdir().unwrap();
    let mut ed = markdown_editor(THREE_HEADINGS);
    let errors = errors_of(
        &mut ed,
        tmp.path(),
        r#"(syntax-captures bid "markdown" "(atx_heading) @h" #:range (hash 'start-line 0 'end-line 6))"#,
    );
    assert!(
        errors
            .iter()
            .any(|e| e.contains("syntax-captures: range 0..6 out of bounds for a 5-line buffer")),
        "{errors:?}"
    );
}

#[test]
fn reversed_range_raises() {
    let tmp = tempfile::tempdir().unwrap();
    let mut ed = markdown_editor(THREE_HEADINGS);
    let errors = errors_of(
        &mut ed,
        tmp.path(),
        r#"(syntax-captures bid "markdown" "(atx_heading) @h" #:range (hash 'start-line 3 'end-line 1))"#,
    );
    assert!(
        errors
            .iter()
            .any(|e| e.contains("syntax-captures: range 3..1 out of bounds for a 5-line buffer")),
        "{errors:?}"
    );
}

#[test]
fn range_without_an_end_raises() {
    let tmp = tempfile::tempdir().unwrap();
    let mut ed = markdown_editor(THREE_HEADINGS);
    let errors = errors_of(
        &mut ed,
        tmp.path(),
        r#"(syntax-captures bid "markdown" "(atx_heading) @h" #:range (hash 'start-line 1))"#,
    );
    assert!(
        errors
            .iter()
            .any(|e| e.contains("syntax-captures: #:range")),
        "{errors:?}"
    );
}

/// An editor on a document of nested sections with its two-row pane scrolled
/// to the last line, so the viewport starts below every heading.
fn scrolled_to_the_tail(tmp: &std::path::Path) -> Editor {
    // Lines: 0 `# A`, 4 `## B`, 8 `### C`, then 2, 6 and the rest plain text.
    // Chars: `# A` 0..4, `## B` 11..16, `### C` 23..29.
    let mut ed =
        markdown_editor("# A\n\ntext\n\n## B\n\nmore\n\n### C\n\nend 1\n\nend 2\n\nend 3\n");
    ed.resize_terminal(80, 4);
    type_cmd(&mut ed, ":goto 15");
    ed.settle();
    let top = value_of(&mut ed, tmp, "(hash-ref (viewport-range bid) 'start-line)");
    assert!(top.parse::<usize>().unwrap() > 8, "viewport top {top}");
    ed
}

/// The viewport range is the call's `#:range` as it stands: a section-rooted
/// pattern returns the headings of the sections the visible tail sits in,
/// which start above it.
#[test]
fn viewport_range_feeds_syntax_captures_unchanged() {
    let tmp = tempfile::tempdir().unwrap();
    let mut ed = scrolled_to_the_tail(tmp.path());
    let call = r#"(syntax-captures bid "markdown" "(section (atx_heading) @h)" #:range (viewport-range bid))"#;
    assert_eq!(
        captures(&mut ed, tmp.path(), call),
        "[h 0..4;h 11..16;h 23..29;]"
    );
}

/// A heading-rooted pattern over the same range returns the visible headings
/// only: the widening is the query's, not the call's.
#[test]
fn heading_rooted_pattern_over_the_viewport_range_returns_no_hidden_heading() {
    let tmp = tempfile::tempdir().unwrap();
    let mut ed = scrolled_to_the_tail(tmp.path());
    let call =
        r#"(syntax-captures bid "markdown" "(atx_heading) @h" #:range (viewport-range bid))"#;
    assert_eq!(captures(&mut ed, tmp.path(), call), "[]");
}

/// Entries of one capture name carry one string, so building the list costs
/// the per-capture work of two integers, not a string.
#[test]
fn entries_of_one_capture_name_share_one_string() {
    let tmp = tempfile::tempdir().unwrap();
    let mut ed = markdown_editor(THREE_HEADINGS);
    // Per heading, `h` then `m` (the marker): indexes 0 and 2 are `h`.
    let name = |i: usize| format!("(list-ref (list-ref caps {i}) 0)");
    let expr = format!(
        r#"(let ((caps (syntax-captures bid "markdown" "(atx_heading) @h (atx_h1_marker) @m")))
             (list (eq? {} {}) (eq? {} {})))"#,
        name(0),
        name(2),
        name(0),
        name(1)
    );
    assert_eq!(value_of(&mut ed, tmp.path(), &expr), "(#true #false)");
}
