use std::sync::Arc;

use hume_editing::text::BufferText;
use hume_rope::line::ContentLine;
use hume_rope::lines::content_line_count;
use hume_rope::offset::ExclusiveRange;
use test_fixtures::require_grammars;
use tree_sitter::QueryErrorKind;

use super::Capture;
use crate::registry::{GrammarBundle, GrammarSnapshot};
use crate::syntax::Syntax;
use crate::test_support::{fixture_markdown, make_bundle, syntax_with};

/// Per match `source` yields over `syntax`'s layers, compiled by `bundle`,
/// restricted to the line ranges `lines` or over every line when `None`,
/// the `(name, start char, end char)` of each of its captures.
fn captures(
    bundle: &GrammarBundle,
    source: &str,
    lines: Option<&[ExclusiveRange<ContentLine>]>,
    syntax: &Syntax,
    text: &BufferText,
) -> Vec<Vec<(String, usize, usize)>> {
    let query = bundle.capture_query(source).expect("query compiles");
    let mut out = Vec::new();
    let every_line = [ExclusiveRange::new(
        ContentLine::default(),
        content_line_count(text.rope()).end_exclusive(),
    )];
    query.collect(
        syntax.layers().expect("a committed tree"),
        text.rope(),
        lines.unwrap_or(&every_line),
        &mut out,
    );
    out.iter()
        .map(|captures| {
            captures
                .iter()
                .map(|Capture { name, range }| {
                    (
                        query.names()[*name as usize].to_string(),
                        range.start.index(),
                        range.end.index(),
                    )
                })
                .collect()
        })
        .collect()
}

fn capture(name: &str, start: usize, end: usize) -> (String, usize, usize) {
    (name.to_owned(), start, end)
}

/// Lines `first` through `last`.
fn lines(first: usize, last: usize) -> ExclusiveRange<ContentLine> {
    ExclusiveRange::new(ContentLine::new(first), ContentLine::new(last + 1))
}

/// `source` parsed by the markdown fixture with `markdown.inline` loaded
/// for its injections, and that inline bundle.
fn markdown_with_inline(source: &str) -> (Syntax, BufferText, Arc<GrammarBundle>) {
    require_grammars(&["markdown", "markdown.inline"]);
    let inline = make_bundle("markdown.inline", "tree_sitter_markdown_inline", "", &[]);
    let langs: Arc<GrammarSnapshot> = Arc::new(
        [("markdown.inline".to_owned(), Arc::clone(&inline))]
            .into_iter()
            .collect(),
    );
    let text = BufferText::from(source);
    let syntax = Syntax::attach_sync(fixture_markdown(), &text, &langs);
    (syntax, text, inline)
}

#[test]
fn root_captures_are_sorted_by_start_then_end_desc() {
    require_grammars(&["markdown"]);
    let bundle = fixture_markdown();
    // "# A\n" 0..4, "\n", "## B\n" 5..10, "\n", "some text\n" 11..21.
    let (syntax, text) = syntax_with(Arc::clone(&bundle), "# A\n\n## B\n\nsome text\n");
    let got = captures(
        &bundle,
        "(paragraph) @p (atx_heading) @h (section) @s",
        None,
        &syntax,
        &text,
    );
    assert_eq!(
        got,
        [
            [capture("s", 0, 21)],
            [capture("h", 0, 4)],
            [capture("s", 5, 21)],
            [capture("h", 5, 10)],
            [capture("p", 11, 21)],
        ]
    );
}

#[test]
fn injected_inline_layer_captures_are_included() {
    // "é" is two bytes and one char: "é *x*" puts the emphasis at chars 2..5.
    let (syntax, text, inline) = markdown_with_inline("é *x* y\n");
    assert_eq!(
        captures(&inline, "(emphasis) @e", None, &syntax, &text),
        [[capture("e", 2, 5)]]
    );
}

#[test]
fn a_range_returns_only_the_inline_layers_it_covers() {
    // Line 0 is chars 0..10, the blank line 10..11, line 2 starts at 11.
    let (syntax, text, inline) = markdown_with_inline("a *one* b\n\nc *two* d\n");
    assert_eq!(
        captures(
            &inline,
            "(emphasis) @e",
            Some(&[lines(2, 2)]),
            &syntax,
            &text
        ),
        [[capture("e", 13, 18)]]
    );
}

#[test]
fn eq_predicate_filters_matches() {
    require_grammars(&["markdown"]);
    let bundle = fixture_markdown();
    let (syntax, text) = syntax_with(Arc::clone(&bundle), "# A\n\n## B\n\n# C\n");
    let got = captures(
        &bundle,
        r##"((atx_heading [(atx_h1_marker) (atx_h2_marker)] @m) (#eq? @m "#"))"##,
        None,
        &syntax,
        &text,
    );
    assert_eq!(got, [[capture("m", 0, 1)], [capture("m", 11, 12)]]);
}

#[test]
fn a_layer_of_another_bundle_is_skipped() {
    require_grammars(&["markdown"]);
    let parsed_with = fixture_markdown();
    let reregistered = fixture_markdown();
    let (syntax, text) = syntax_with(Arc::clone(&parsed_with), "# A\n");
    assert_eq!(
        captures(&parsed_with, "(atx_heading) @h", None, &syntax, &text),
        [[capture("h", 0, 4)]]
    );
    assert!(captures(&reregistered, "(atx_heading) @h", None, &syntax, &text).is_empty());
}

/// A table between two paragraphs. Lines 0 to 7, by char offset: `intro`
/// 0..6, blank, header 7..17, delimiter 17..27, row c/d 27..37, row e/f
/// 37..47, blank, `outer` 48..54. A row node stops before its line break and a cell
/// node takes the space after its text.
const TABLE: &str = "intro\n\n| a | b |\n|---|---|\n| c | d |\n| e | f |\n\nouter\n";

#[test]
fn range_inside_a_table_returns_every_row_for_a_table_rooted_pattern() {
    require_grammars(&["markdown"]);
    let bundle = fixture_markdown();
    let (syntax, text) = syntax_with(Arc::clone(&bundle), TABLE);
    // The last row.
    let got = captures(
        &bundle,
        "(pipe_table (pipe_table_row) @row)",
        Some(&[lines(5, 5)]),
        &syntax,
        &text,
    );
    assert_eq!(got, [[capture("row", 27, 36)], [capture("row", 37, 46)]]);
}

#[test]
fn range_inside_a_table_returns_only_intersecting_cells_for_a_cell_rooted_pattern() {
    require_grammars(&["markdown"]);
    let bundle = fixture_markdown();
    let (syntax, text) = syntax_with(Arc::clone(&bundle), TABLE);
    // The last row, whose cells `e` and `f` take 39..41 and 43..45.
    let got = captures(
        &bundle,
        "(pipe_table_cell) @cell",
        Some(&[lines(5, 5)]),
        &syntax,
        &text,
    );
    assert_eq!(got, [[capture("cell", 39, 41)], [capture("cell", 43, 45)]]);
}

#[test]
fn range_on_a_nested_section_returns_the_ancestor_headings() {
    require_grammars(&["markdown"]);
    let bundle = fixture_markdown();
    // Headings: `# A` 0..4, `## B` 11..16, `### C` 23..29; `end` on line 10.
    let (syntax, text) = syntax_with(
        Arc::clone(&bundle),
        "# A\n\ntext\n\n## B\n\nmore\n\n### C\n\nend\n",
    );
    let got = captures(
        &bundle,
        "(section (atx_heading) @h)",
        Some(&[lines(10, 10)]),
        &syntax,
        &text,
    );
    assert_eq!(
        got,
        [
            [capture("h", 0, 4)],
            [capture("h", 11, 16)],
            [capture("h", 23, 29)]
        ]
    );
}

#[test]
fn a_node_spanning_two_ranges_is_captured_once() {
    require_grammars(&["markdown"]);
    let bundle = fixture_markdown();
    let (syntax, text) = syntax_with(Arc::clone(&bundle), TABLE);
    // The header and the last row: both reach the one table.
    let got = captures(
        &bundle,
        "(pipe_table (pipe_table_row) @row)",
        Some(&[lines(2, 2), lines(5, 5)]),
        &syntax,
        &text,
    );
    assert_eq!(got, [[capture("row", 27, 36)], [capture("row", 37, 46)]]);
}

#[test]
fn lines_between_two_ranges_are_not_captured() {
    require_grammars(&["markdown"]);
    let bundle = fixture_markdown();
    // Headings `# A` 0..4, `# B` 5..9 and `# C` 10..14 on lines 0, 2 and 4.
    let (syntax, text) = syntax_with(Arc::clone(&bundle), "# A\n\n# B\n\n# C\n");
    let got = captures(
        &bundle,
        "(atx_heading) @h",
        Some(&[lines(4, 4), lines(0, 0)]),
        &syntax,
        &text,
    );
    assert_eq!(got, [[capture("h", 0, 4)], [capture("h", 10, 14)]]);
}

#[test]
fn overlapping_ranges_capture_each_match_once() {
    require_grammars(&["markdown"]);
    let bundle = fixture_markdown();
    let (syntax, text) = syntax_with(Arc::clone(&bundle), TABLE);
    // The rows c/d and e/f, whose cells take 29..31, 33..35, 39..41 and 43..45.
    let got = captures(
        &bundle,
        "(pipe_table_cell) @cell",
        Some(&[lines(4, 5), lines(5, 5)]),
        &syntax,
        &text,
    );
    assert_eq!(
        got,
        [
            [capture("cell", 29, 31)],
            [capture("cell", 33, 35)],
            [capture("cell", 39, 41)],
            [capture("cell", 43, 45)]
        ]
    );
}

#[test]
fn an_identical_match_from_two_patterns_comes_once() {
    require_grammars(&["markdown"]);
    let bundle = fixture_markdown();
    let (syntax, text) = syntax_with(Arc::clone(&bundle), "# A\n");
    let got = captures(
        &bundle,
        "(atx_heading) @h\n(section (atx_heading) @h)",
        None,
        &syntax,
        &text,
    );
    assert_eq!(got, [[capture("h", 0, 4)]]);
}

#[test]
fn compile_error_reports_row_column_and_kind() {
    require_grammars(&["markdown"]);
    let Err(error) = fixture_markdown().capture_query("(atx_heading) @h\n  (no_such_node) @n")
    else {
        panic!("an unknown node type does not compile");
    };
    assert_eq!(
        (error.row, error.column, error.kind),
        (1, 3, QueryErrorKind::NodeType)
    );
}

#[test]
fn a_pattern_using_a_directive_hume_does_not_apply_fails_at_its_start() {
    require_grammars(&["markdown"]);
    for (pattern, operator) in [
        (r#"((paragraph) @p (#set! kind "text"))"#, "#set!"),
        ("((paragraph) @p (#is? local))", "#is?"),
        ("((paragraph) @p (#frobnicate! @p))", "#frobnicate!"),
    ] {
        let Err(error) = fixture_markdown().capture_query(&format!("(atx_heading) @h\n{pattern}"))
        else {
            panic!("{pattern}: a pattern HUME cannot apply does not compile");
        };
        assert_eq!(
            (error.row, error.column, error.kind),
            (1, 0, QueryErrorKind::Predicate),
            "{pattern}"
        );
        assert_eq!(
            error.message,
            format!("{operator} is not supported"),
            "{pattern}"
        );
    }
}

#[test]
fn capture_query_compiles_once_per_bundle_and_source() {
    require_grammars(&["markdown"]);
    let bundle = fixture_markdown();
    let first = bundle.capture_query("(atx_heading) @h").expect("compiles");
    let again = bundle.capture_query("(atx_heading) @h").expect("compiles");
    let other = bundle.capture_query("(paragraph) @p").expect("compiles");
    assert!(Arc::ptr_eq(&first, &again));
    assert!(!Arc::ptr_eq(&first, &other));
    let elsewhere = fixture_markdown()
        .capture_query("(atx_heading) @h")
        .expect("compiles");
    assert!(!Arc::ptr_eq(&first, &elsewhere));
}

#[test]
fn a_match_holds_each_marker_with_its_list() {
    require_grammars(&["markdown"]);
    let bundle = fixture_markdown();
    // The list takes 0..8, its markers `- ` 0..2 and 4..6.
    let (syntax, text) = syntax_with(Arc::clone(&bundle), "- a\n- b\n");
    let got = captures(
        &bundle,
        "(list (list_item (list_marker_minus) @m)) @l",
        None,
        &syntax,
        &text,
    );
    assert_eq!(
        got,
        [
            [capture("l", 0, 8), capture("m", 0, 2)],
            [capture("l", 0, 8), capture("m", 4, 6)]
        ]
    );
}

#[test]
fn a_match_reached_by_two_ranges_comes_once() {
    require_grammars(&["markdown"]);
    let bundle = fixture_markdown();
    let (syntax, text) = syntax_with(Arc::clone(&bundle), TABLE);
    // The header and the last row: both reach the one table, 7..47.
    let got = captures(
        &bundle,
        "(pipe_table (pipe_table_row) @row) @table",
        Some(&[lines(2, 2), lines(5, 5)]),
        &syntax,
        &text,
    );
    assert_eq!(
        got,
        [
            [capture("table", 7, 47), capture("row", 27, 36)],
            [capture("table", 7, 47), capture("row", 37, 46)]
        ]
    );
}

#[test]
fn a_pattern_capturing_nothing_yields_no_match() {
    require_grammars(&["markdown"]);
    let bundle = fixture_markdown();
    let (syntax, text) = syntax_with(Arc::clone(&bundle), "# A\n\nsome text\n");
    let got = captures(
        &bundle,
        "(atx_heading)\n(paragraph) @p",
        None,
        &syntax,
        &text,
    );
    assert_eq!(got, [[capture("p", 5, 15)]]);
}

#[test]
fn a_match_sorts_before_a_longer_one_it_starts() {
    require_grammars(&["markdown"]);
    let bundle = fixture_markdown();
    let (syntax, text) = syntax_with(Arc::clone(&bundle), "# A\n");
    let got = captures(
        &bundle,
        "(atx_heading (atx_h1_marker) @m) @h\n(atx_heading) @h",
        None,
        &syntax,
        &text,
    );
    assert_eq!(
        got,
        [
            vec![capture("h", 0, 4)],
            vec![capture("h", 0, 4), capture("m", 0, 1)]
        ]
    );
}

#[test]
fn a_parent_with_the_range_of_its_child_comes_first() {
    require_grammars(&["markdown"]);
    let bundle = fixture_markdown();
    // The document and its one section both take 0..4.
    let (syntax, text) = syntax_with(Arc::clone(&bundle), "# A\n");
    let got = captures(&bundle, "(document (section) @s) @d", None, &syntax, &text);
    assert_eq!(got, [[capture("d", 0, 4), capture("s", 0, 4)]]);
}
