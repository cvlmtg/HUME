use std::sync::Arc;

use hume_core::{OptionalQuery, QueryFile};
use hume_editing::text::BufferText;
use hume_engine::folds::Fold;
use hume_engine::providers::SyntaxSource;
use hume_rope::line::ContentLine;
use hume_rope::offset::InclusiveRange;
use test_fixtures::require_grammars;

use crate::registry::{FileSkip, GrammarBundle, GrammarSnapshot, Skipped};
use crate::syntax::Syntax;
use crate::test_support::{
    assert_skipped_at_pattern_start, bundle_with_fixture_query, fixture_query, make_bundle,
    rust_bundle_with, rust_skips, syntax_with,
};

fn fold(first: usize, last: usize, shows_last: bool) -> Fold {
    Fold::new(ContentLine::new(first), ContentLine::new(last), shows_last)
        .expect("the region hides a line")
}

/// The region starting at `line` that hides the most lines.
fn fold_at(syn: &Syntax, text: &BufferText, line: usize) -> Option<Fold> {
    all(syn, text)
        .into_iter()
        .find(|fold| fold.first() == ContentLine::new(line))
}

/// The first lines of the regions starting in `first ..= last`, ascending.
fn starts(syn: &Syntax, text: &BufferText, first: usize, last: usize) -> Vec<usize> {
    let lines = InclusiveRange::new(ContentLine::new(first), ContentLine::new(last));
    let mut out = Vec::new();
    syn.fold_starts(lines, text.rope(), &mut out);
    let mut out: Vec<usize> = out.iter().map(|line| line.index()).collect();
    out.sort_unstable();
    out.dedup();
    out
}

fn spanning(syn: &Syntax, text: &BufferText, line: usize) -> Vec<Fold> {
    let mut out = Vec::new();
    syn.layers().expect("layers installed").folds_spanning(
        text.rope(),
        ContentLine::new(line),
        &mut out,
    );
    out
}

fn all(syn: &Syntax, text: &BufferText) -> Vec<Fold> {
    syn.layers()
        .expect("layers installed")
        .all_folds(text.rope())
}

/// `name` with `folds` as its `folds.scm` and no other optional query.
fn with_folds(name: &str, symbol: &str, folds: &str) -> Arc<GrammarBundle> {
    require_grammars(&[name]);
    make_bundle(name, symbol, "", &[(OptionalQuery::Folds, folds)])
}

fn rust_blocks() -> Arc<GrammarBundle> {
    with_folds("rust", "tree_sitter_rust", "(block) @fold")
}

#[test]
fn a_closing_brace_line_stays_shown() {
    let source = "fn f() {\n    a();\n    b();\n}\n";
    let (syn, text) = syntax_with(rust_blocks(), source);
    assert_eq!(fold_at(&syn, &text, 0), Some(fold(0, 3, true)));
    assert_eq!(fold_at(&syn, &text, 1), None);
}

#[test]
fn a_brace_before_else_closes_its_branch() {
    let source = "fn f(x: bool) {\n    if x {\n        a();\n        b();\n    } else {\n        c();\n        d();\n    }\n}\n";
    let (syn, text) = syntax_with(rust_blocks(), source);
    assert_eq!(fold_at(&syn, &text, 1), Some(fold(1, 4, true)));
    assert_eq!(fold_at(&syn, &text, 4), Some(fold(4, 7, true)));
}

#[test]
fn a_region_hiding_no_line_is_not_foldable() {
    let one_line = "fn f() { a(); }\n";
    let (syn, text) = syntax_with(rust_blocks(), one_line);
    assert_eq!(fold_at(&syn, &text, 0), None);

    let brace_only = "fn f() {\n}\n";
    let (syn, text) = syntax_with(rust_blocks(), brace_only);
    assert_eq!(fold_at(&syn, &text, 0), None);

    let one_body_line = "fn f() {\n    a();\n}\n";
    let (syn, text) = syntax_with(rust_blocks(), one_body_line);
    assert_eq!(fold_at(&syn, &text, 0), Some(fold(0, 2, true)));
}

#[test]
fn a_region_ending_in_content_hides_its_last_line() {
    let bundle = with_folds(
        "python",
        "tree_sitter_python",
        "(function_definition) @fold (argument_list) @fold",
    );
    let def = "def f():\n    a = 1\n    b = 2\nx = 3\n";
    let (syn, text) = syntax_with(bundle.clone(), def);
    assert_eq!(fold_at(&syn, &text, 0), Some(fold(0, 2, false)));

    let call = "f(\n    1,\n    2,\n)\n";
    let (syn, text) = syntax_with(bundle, call);
    assert_eq!(fold_at(&syn, &text, 0), Some(fold(0, 3, true)));
}

const MARKDOWN_SECTIONS: &str = "# A\n\npara\n\n# B\n\ntext\n";

#[test]
fn a_trimmed_region_leaves_out_its_trailing_blank_lines() {
    let markdown =
        bundle_with_fixture_query("markdown", "tree_sitter_markdown", OptionalQuery::Folds);
    let (syn, text) = syntax_with(markdown, MARKDOWN_SECTIONS);
    assert_eq!(fold_at(&syn, &text, 0), Some(fold(0, 2, false)));
}

#[test]
fn an_untrimmed_region_keeps_its_trailing_blank_lines() {
    let markdown = with_folds("markdown", "tree_sitter_markdown", "(section) @fold");
    let (syn, text) = syntax_with(markdown, MARKDOWN_SECTIONS);
    assert_eq!(fold_at(&syn, &text, 0), Some(fold(0, 3, false)));
}

#[test]
fn a_not_has_parent_match_under_those_parents_does_not_fold() {
    let bundle = with_folds(
        "rust",
        "tree_sitter_rust",
        "((block) @b (#not-has-parent? @b function_item)) @fold",
    );
    let source = "fn f() {\n    if x {\n        a();\n    }\n}\n";
    let (syn, text) = syntax_with(bundle, source);
    assert_eq!(fold_at(&syn, &text, 0), None);
    assert_eq!(fold_at(&syn, &text, 1), Some(fold(1, 3, true)));
}

#[test]
fn an_unsupported_fold_directive_skips_its_pattern() {
    assert_skipped_at_pattern_start(
        OptionalQuery::Folds,
        "(block) @fold",
        &[
            r#"((block) @fold (#frobnicate! @fold))"#,
            r#"((block) @fold (#set! "a" "b"))"#,
            r#"((block) @b (#trim! @b)) @fold"#,
            r#"((block) @fold (#trim! @fold 1 1 1 1))"#,
            r#"((block) @b (#not-has-parent? @b)) @fold"#,
            r#"((block) @b (#not-has-parent? "x" @b)) @fold"#,
        ],
    );
}

#[test]
fn a_folds_scm_without_a_fold_capture_reports_its_skipped_pattern_too() {
    let skipped = rust_skips(
        "",
        &[(OptionalQuery::Folds, "((block) @b (#frobnicate! @b))\n")],
    );
    assert!(
        matches!(
            skipped.as_slice(),
            [
                Skipped::Pattern {
                    file: QueryFile::Optional(OptionalQuery::Folds),
                    ..
                },
                Skipped::File {
                    kind: OptionalQuery::Folds,
                    reason: FileSkip::NamesNothing(_),
                },
            ]
        ),
        "{skipped:?}"
    );
}

/// The end moves from after the closing `}` to column 0 of its line, so the
/// region ends on the line before.
#[test]
fn an_offset_function_object_ends_its_fallback_fold_on_the_line_before() {
    let source = "fn f() {\n    a();\n    b();\n}\n";
    let offset = rust_bundle_with(
        OptionalQuery::TextObjects,
        "((function_item) @function.around (#offset! @function.around 0 0 0 -1))",
    );
    let (syn, text) = syntax_with(offset, source);
    assert_eq!(fold_at(&syn, &text, 0), Some(fold(0, 2, false)));
    let plain = rust_bundle_with(
        OptionalQuery::TextObjects,
        "(function_item) @function.around",
    );
    let (syn, text) = syntax_with(plain, source);
    assert_eq!(fold_at(&syn, &text, 0), Some(fold(0, 3, true)));
}

/// The region runs from `f` on row 0 through the `)` alone on row 3, which
/// stays shown.
#[test]
fn a_made_context_scope_folds_through_its_end_capture() {
    let bundle = rust_bundle_with(
        OptionalQuery::Context,
        r#"((function_item name: (identifier) @_s parameters: (parameters) @_e)
            (#make-range! "context" @_s @_e))"#,
    );
    let source = "fn f(\n    a: u32,\n    b: u32,\n) {\n    body();\n}\n";
    let (syn, text) = syntax_with(bundle, source);
    assert_eq!(fold_at(&syn, &text, 0), Some(fold(0, 3, true)));
}

#[test]
fn without_folds_scm_scopes_fold() {
    let impl_fn = "impl Foo {\n    fn bar(&self) {\n        a();\n        b();\n    }\n}\n";
    let rust = bundle_with_fixture_query("rust", "tree_sitter_rust", OptionalQuery::Context);
    let (syn, text) = syntax_with(rust, impl_fn);
    assert_eq!(fold_at(&syn, &text, 0), Some(fold(0, 5, true)));
    assert_eq!(fold_at(&syn, &text, 1), Some(fold(1, 4, true)));

    let def = "def f():\n    a = 1\n    b = 2\nx = 3\n";
    let python = bundle_with_fixture_query("python", "tree_sitter_python", OptionalQuery::Context);
    let (syn, text) = syntax_with(python, def);
    assert_eq!(fold_at(&syn, &text, 0), Some(fold(0, 2, false)));

    let sections = "# A\n\npara\n\n# B\n\ntext\n";
    let markdown =
        bundle_with_fixture_query("markdown", "tree_sitter_markdown", OptionalQuery::Context);
    let (syn, text) = syntax_with(markdown, sections);
    assert_eq!(fold_at(&syn, &text, 0), Some(fold(0, 3, false)));
    assert_eq!(fold_at(&syn, &text, 4), Some(fold(4, 6, false)));
}

#[test]
fn without_a_scope_query_function_objects_fold_from_the_item() {
    let source = "#[test]\nfn t() {\n    a();\n    b();\n}\n";
    let rust = bundle_with_fixture_query("rust", "tree_sitter_rust", OptionalQuery::TextObjects);
    let (syn, text) = syntax_with(rust, source);
    assert_eq!(fold_at(&syn, &text, 0), None);
    assert_eq!(fold_at(&syn, &text, 1), Some(fold(1, 4, true)));
}

#[test]
fn folds_scm_wins_over_the_scope_query() {
    require_grammars(&["rust"]);
    let context = fixture_query("rust", OptionalQuery::Context);
    let source = "fn f() {\n    let v = [\n        1,\n    ];\n}\n";
    let only_arrays = make_bundle(
        "rust",
        "tree_sitter_rust",
        "",
        &[
            (OptionalQuery::Context, &context),
            (OptionalQuery::Folds, "(array_expression) @fold"),
        ],
    );
    let (syn, text) = syntax_with(only_arrays, source);
    assert_eq!(fold_at(&syn, &text, 0), None);
    assert_eq!(fold_at(&syn, &text, 1), Some(fold(1, 3, true)));
}

/// The `markdown` grammar with its injections, and a snapshot holding
/// [`rust_blocks`] for the code blocks it injects.
fn markdown_injecting_rust() -> (Arc<GrammarBundle>, Arc<GrammarSnapshot>) {
    require_grammars(&["markdown", "rust"]);
    let injections = fixture_query("markdown", OptionalQuery::Injections);
    let markdown = make_bundle(
        "markdown",
        "tree_sitter_markdown",
        "",
        &[(OptionalQuery::Injections, &injections)],
    );
    let langs = Arc::new(GrammarSnapshot::from_iter([(
        "rust".to_owned(),
        rust_blocks(),
    )]));
    (markdown, langs)
}

#[test]
fn an_injected_layer_folds_its_own_scopes() {
    let (markdown, langs) = markdown_injecting_rust();
    let source = "## Usage\n\n```rust\nfn demo() {\n    let x = 1;\n    let y = 2;\n}\n```\n";
    let text = BufferText::from(source);
    let syn = Syntax::attach_sync(markdown, &text, &langs);
    assert_eq!(fold_at(&syn, &text, 3), Some(fold(3, 6, true)));
}

#[test]
fn a_root_without_regions_can_fold_through_an_injected_layer() {
    let (markdown, langs) = markdown_injecting_rust();
    let prose = BufferText::from("## Usage\n\nText.\n");
    let code = BufferText::from("## Usage\n\n```rust\nfn demo() {\n    x();\n}\n```\n");
    assert!(!Syntax::attach_sync(Arc::clone(&markdown), &prose, &langs).can_fold());
    assert!(Syntax::attach_sync(markdown, &code, &langs).can_fold());
}

#[test]
fn spanning_folds_list_innermost_first() {
    let source = "fn f() {\n    if x {\n        a();\n        b();\n    }\n}\n";
    let (syn, text) = syntax_with(rust_blocks(), source);
    assert_eq!(
        spanning(&syn, &text, 2),
        [fold(1, 4, true), fold(0, 5, true)]
    );
    assert_eq!(spanning(&syn, &text, 0), [fold(0, 5, true)]);
}

#[test]
fn all_folds_keep_the_largest_per_first_line() {
    let source = "fn main() {\n    foo(bar(\n        1,\n        2,\n    ));\n    c();\n}\n";
    let bundle = with_folds(
        "rust",
        "tree_sitter_rust",
        "(block) @fold (arguments) @fold",
    );
    let (syn, text) = syntax_with(bundle, source);
    assert_eq!(all(&syn, &text), [fold(0, 6, true), fold(1, 4, false)]);
}

#[test]
fn fold_starts_lists_the_regions_starting_in_the_lines_only() {
    let source =
        "impl S {\n    fn a() {\n        x();\n    }\n    fn b() {\n        y();\n    }\n}\n";
    let rust = bundle_with_fixture_query("rust", "tree_sitter_rust", OptionalQuery::Context);
    let (syn, text) = syntax_with(rust, source);
    assert_eq!(starts(&syn, &text, 1, 5), [1, 4]);
    assert_eq!(starts(&syn, &text, 0, 7), [0, 1, 4]);
    assert_eq!(starts(&syn, &text, 2, 3), Vec::<usize>::new());
}
