use std::sync::Arc;

use hume_editing::indent::ElectricTrigger;
use hume_editing::text::BufferText;
use hume_editing::word::WordChars;
use hume_engine::theme::ScopeRegistry;
use hume_rope::cluster::ClusterStart;
use hume_rope::column::BufferLineCol;
use hume_rope::offset::CharOffset;
use test_fixtures::{require_grammars, require_helix_query};
use tree_sitter::QueryErrorKind;

use super::query::IndentCapture;
use super::*;
use crate::registry::{GrammarBundle, OptionalQuery, RegisterError, compile_queries};
use crate::syntax::Syntax;
use crate::test_support::{empty_langs, make_bundle, open_grammar};

const SPLIT: char = '‸';

/// `src` without its one `‸`, and the cluster the `‸` stood before.
fn marked(src: &str) -> (BufferText, ClusterStart) {
    let at = src.find(SPLIT).expect("source marks the split with ‸");
    let text = BufferText::from(src.replacen(SPLIT, "", 1).as_str());
    let split = text.snap(CharOffset::new(src[..at].chars().count()));
    (text, split)
}

fn symbol(name: &str) -> String {
    format!("tree_sitter_{name}")
}

fn bundle(name: &str, indents: &str) -> Arc<GrammarBundle> {
    require_grammars(&[name]);
    make_bundle(
        name,
        &symbol(name),
        "",
        &[(OptionalQuery::Indents, indents)],
    )
}

fn fixture_indents(name: &str) -> String {
    let path = require_helix_query(name, OptionalQuery::Indents.file_name());
    std::fs::read_to_string(path).expect("read helix-indents.scm")
}

fn fixture_bundle(name: &str) -> Arc<GrammarBundle> {
    bundle(name, &fixture_indents(name))
}

/// The absolute width the model gives the line a break at the split
/// starts.
fn width_in(
    bundle: &Arc<GrammarBundle>,
    text: &BufferText,
    split: ClusterStart,
    tw: u8,
) -> Option<u32> {
    width_in_with(bundle, &empty_langs(), text, split, tw)
}

/// [`width_in`], parsing injected layers with `langs`.
fn width_in_with(
    bundle: &Arc<GrammarBundle>,
    langs: &Arc<rustc_hash::FxHashMap<String, Arc<GrammarBundle>>>,
    text: &BufferText,
    split: ClusterStart,
    tw: u8,
) -> Option<u32> {
    let syntax = Syntax::attach_sync(Arc::clone(bundle), text, langs);
    let layers = syntax.layers().expect("the text parses");
    absolute_for_new_line(layers, text, split, tw).map(BufferLineCol::get)
}

fn width(bundle: &Arc<GrammarBundle>, src: &str, tw: u8) -> Option<u32> {
    let (text, split) = marked(src);
    width_in(bundle, &text, split, tw)
}

/// What `indent_for_new_line` gives a break at the split, parsing injected
/// layers with `langs`.
fn break_indent_with(
    bundle: &Arc<GrammarBundle>,
    langs: &Arc<rustc_hash::FxHashMap<String, Arc<GrammarBundle>>>,
    src: &str,
    tw: u8,
) -> Option<BreakIndent> {
    let (text, split) = marked(src);
    let syntax = Syntax::attach_sync(Arc::clone(bundle), &text, langs);
    let layers = syntax.layers().expect("the text parses");
    indent_for_new_line(layers, &text, split, tw)
}

fn break_indent(bundle: &Arc<GrammarBundle>, src: &str, tw: u8) -> Option<BreakIndent> {
    break_indent_with(bundle, &empty_langs(), src, tw)
}

fn hybrid_width(bundle: &Arc<GrammarBundle>, src: &str, tw: u8) -> Option<u32> {
    break_indent(bundle, src, tw).map(|indent| indent.width.get())
}

/// Whether `bundle`'s tree for `src` has a node `pred` accepts.
fn tree_has(
    bundle: &Arc<GrammarBundle>,
    src: &str,
    pred: impl Fn(tree_sitter::Node) -> bool,
) -> bool {
    let (text, _) = marked(src);
    let syntax = Syntax::attach_sync(Arc::clone(bundle), &text, &empty_langs());
    let root = syntax.layers().expect("the text parses").layers[0]
        .tree
        .root_node();
    let mut cursor = root.walk();
    let mut stack = vec![root];
    while let Some(node) = stack.pop() {
        if pred(node) {
            return true;
        }
        stack.extend(node.children(&mut cursor));
    }
    false
}

fn has_kind(bundle: &Arc<GrammarBundle>, src: &str, kind: &str) -> bool {
    tree_has(bundle, src, |node| node.kind() == kind)
}

fn compile(name: &str, src: &str) -> Result<IndentQuery, tree_sitter::QueryError> {
    require_grammars(&[name]);
    let grammar = open_grammar(name, &symbol(name));
    let query = tree_sitter::Query::new(grammar.language(), src).expect("query syntax is valid");
    IndentQuery::new(query, src, grammar.language())
}

// ── IndentQuery ──────────────────────────────────────────────────────────

#[test]
fn the_indents_queries_of_every_fixture_language_compile() {
    for name in ["rust", "json", "python", "c"] {
        assert!(
            fixture_bundle(name).indents.is_some(),
            "{name}'s indents.scm must compile"
        );
    }
}

/// Each query is invalid on its third line (row 2), where its invalid pattern
/// starts.
#[test]
fn an_unsupported_predicate_or_property_fails_the_compile_on_its_pattern_row() {
    require_grammars(&["rust"]);
    let grammar = open_grammar("rust", "tree_sitter_rust");
    let invalid = [
        r#"(block (#is? "local"))"#,
        r#"(block (#is-not? "local"))"#,
        r#"(block (#set! "scope" "tail"))"#,
        r#"(block (#set! "kind" "header"))"#,
        r#"((block) @b (#kind-eq? @b "block"))"#,
        r#"((block) @b (#not-kind-eq? "block" @b))"#,
        r#"((block) @b (#same-line? @b))"#,
        r#"((block) @b (#not-same-line? @b "x"))"#,
        r#"((block) @b (#one-line? "x"))"#,
        r#"((block) @b (#not-one-line? @b @b))"#,
    ];
    for pattern in invalid {
        let src = format!("(block) @indent\n\n{pattern}\n");
        let error = match compile_queries(
            grammar.language(),
            "",
            &[(OptionalQuery::Indents, &src)],
            &mut ScopeRegistry::new(),
        ) {
            Err(RegisterError::OptionalQueryBuild {
                kind: OptionalQuery::Indents,
                error,
            }) => error,
            Err(other) => panic!("{pattern}: expected an indents build error, got {other}"),
            Ok(_) => panic!("{pattern}: must not compile"),
        };
        assert_eq!(error.kind, QueryErrorKind::Predicate, "{pattern}");
        assert_eq!((error.row, error.column), (2, 0), "{pattern}");
        assert_eq!(
            error.offset,
            src.find(pattern).expect("pattern in source"),
            "{pattern}"
        );
    }
}

#[test]
fn every_supported_predicate_and_property_compiles() {
    let query = compile(
        "rust",
        r#"((block) @a (#not-kind-eq? @a "block"))
           ((block) @a (block) @b (#same-line? @a @b))
           ((block) @a (block) @b (#not-same-line? @a @b))
           ((block) @a (#one-line? @a))
           ((block) @a (#not-one-line? @a))
           ((block) @a (#set! "scope" "header"))
           ((block) @a (#eq? @a "{}"))"#,
    );
    assert!(query.is_ok(), "{:?}", query.err());
}

#[test]
fn the_header_flag_is_set_only_on_patterns_that_set_the_header_scope() {
    let query = compile(
        "rust",
        r#"(block) @indent
           (if_expression consequence: (_) @indent (#set! "scope" "header"))
           (arguments) @indent"#,
    )
    .expect("compiles");
    assert_eq!(
        (0..3)
            .map(|pattern| query.is_header(pattern))
            .collect::<Vec<_>>(),
        [false, true, false]
    );
}

#[test]
fn helper_captures_compile_and_name_no_indent_role() {
    let query = compile(
        "rust",
        r#"((let_declaration "let" @expr-start value: (_) @indent) @extend
            (#not-same-line? @indent @expr-start))
           (return_expression) @extend.prevent-once
           ((block) @_helper @outdent.always)"#,
    )
    .expect("helper captures are accepted");
    let role = |name: &str| {
        let index = query
            .query
            .capture_index_for_name(name)
            .expect("capture in query");
        query.capture(index)
    };
    for helper in ["expr-start", "_helper"] {
        assert_eq!(role(helper), None, "{helper}");
    }
    assert_eq!(role("indent"), Some(IndentCapture::Indent));
    assert_eq!(role("outdent.always"), Some(IndentCapture::OutdentAlways));
    assert_eq!(role("extend"), Some(IndentCapture::Extend));
    assert_eq!(
        role("extend.prevent-once"),
        Some(IndentCapture::ExtendPreventOnce)
    );
}

// ── Containment ──────────────────────────────────────────────────────────

const BLOCK_QUERY: &str = r#"(block) @indent "}" @outdent"#;

#[test]
fn a_new_line_inside_a_block_is_one_level_deeper() {
    let rust = bundle("rust", BLOCK_QUERY);
    assert_eq!(width(&rust, "fn f() {\n    a();‸\n}\n", 4), Some(4));
}

/// Enter before `}`: the `}` opens the new line, which the block contains,
/// so the outdent cancels the block's level.
#[test]
fn a_closer_split_onto_its_own_line_returns_to_the_block_level() {
    let rust = bundle("rust", BLOCK_QUERY);
    assert_eq!(width(&rust, "fn f() {\n    a();‸}\n", 4), Some(0));
}

#[test]
fn no_layer_with_an_indents_query_gives_none() {
    require_grammars(&["rust"]);
    let rust = make_bundle("rust", "tree_sitter_rust", "", &[]);
    assert_eq!(width(&rust, "fn f() {\n    a();‸\n}\n", 4), None);
}

/// Both argument lists open on line 1 and count once; on separate lines
/// they count once each.
#[test]
fn scopes_opening_on_one_line_count_once() {
    let rust = bundle("rust", "(arguments) @indent");
    assert_eq!(width(&rust, "fn f() {\n    g(h(‸a));\n}\n", 4), Some(4));
    assert_eq!(
        width(&rust, "fn f() {\n    g(\n        h(‸a));\n}\n", 4),
        Some(8)
    );
}

#[test]
fn indent_always_scopes_opening_on_one_line_each_count() {
    let rust = bundle("rust", "(arguments) @indent.always");
    assert_eq!(width(&rust, "fn f() {\n    g(h(‸a));\n}\n", 4), Some(8));
}

/// Three blocks open on lines 0 to 2 and contain the new line. `a.b()`
/// opens it, and its call and its field expression each outdent it: 3 − 2
/// levels with both captured, 3 − 1 with the call alone.
#[test]
fn outdent_always_scopes_opening_the_line_each_count() {
    let src =
        "fn f() {\n    if a {\n        if b {\n            x();‸a.b();\n        }\n    }\n}\n";
    let both = bundle(
        "rust",
        "(block) @indent (call_expression) @outdent.always (field_expression) @outdent.always",
    );
    assert_eq!(width(&both, src, 4), Some(4));
    let call = bundle("rust", "(block) @indent (call_expression) @outdent.always");
    assert_eq!(width(&call, src, 4), Some(8));
}

/// The `else` keyword and its clause both outdent the `else:` line and
/// count once: the `if` level minus one leaves that baseline at net 0 and
/// column 0. The new line inside the `if` is one level deeper: 4.
#[test]
fn plain_outdents_opening_one_line_count_once() {
    let python = bundle(
        "python",
        &(fixture_indents("python") + "\n(else_clause) @outdent\n"),
    );
    assert_eq!(
        hybrid_width(&python, "if a:\n    b\nelse:‸\n    c\n", 4),
        Some(4)
    );
}

/// The block (line 0) adds one level; the argument list (line 1) is both
/// indent and outdent, so it adds none.
#[test]
fn a_node_captured_as_indent_and_outdent_adds_nothing() {
    let rust = bundle("rust", "(block) @indent (arguments) @indent @outdent");
    assert_eq!(width(&rust, "fn f() {\n    g(‸a);\n}\n", 4), Some(4));
}

/// The brace-less consequence opens its scope on the `if` line, so the
/// new line holding it is contained: one level for the function body and
/// one for the `if`, at C's unit of 2.
#[test]
fn a_header_scoped_body_split_onto_its_own_line_is_indented() {
    let c = fixture_bundle("c");
    assert_eq!(width(&c, "void f() {\n  if (c)‸return 1;\n}\n", 2), Some(4));
}

// ── Start node ───────────────────────────────────────────────────────────

/// The method and the class both end on the split line, so both extend
/// over the new line: one level each.
#[test]
fn extended_scopes_ending_on_the_split_line_contain_the_new_line() {
    let python = fixture_bundle("python");
    assert_eq!(
        width(&python, "class A:\n    def f(self):\n        x = 1‸\n", 4),
        Some(8)
    );
}

/// `return` prevents the method's extension, so only the class extends
/// over the new line.
#[test]
fn prevent_once_stops_the_innermost_extension() {
    let python = fixture_bundle("python");
    assert_eq!(
        width(
            &python,
            "class A:\n    def f(self):\n        return 1‸\n",
            4
        ),
        Some(4)
    );
}

/// The body after `while (c)` is the candidate, and its header-scoped
/// `@indent` opens on the `while` line: one level at C's unit of 2.
#[test]
fn a_new_line_before_a_brace_less_body_descends_into_it() {
    let c = fixture_bundle("c");
    assert_eq!(width(&c, "while (c)‸\nx();\n", 2), Some(2));
}

/// The body is the candidate and its header pattern captures only
/// `@outdent`. Starting from the body, its outdent opens the new line and
/// cancels the function body's level.
#[test]
fn a_header_pattern_capturing_only_an_outdent_still_descends() {
    let c = bundle(
        "c",
        r#"(compound_statement) @indent
           (while_statement body: (_) @outdent (#set! "scope" "header"))"#,
    );
    assert_eq!(width(&c, "void f() {\n  while (c)‸ x();\n}\n", 2), Some(0));
}

/// The function ends on line 1, but the split line's 4 spaces are one level
/// deeper than the `def` line, so the function extends over the new line.
#[test]
fn an_extend_node_reaches_a_more_indented_split_line() {
    let python = fixture_bundle("python");
    assert_eq!(width(&python, "def f():\n    x = 1\n    ‸\n", 4), Some(4));
}

/// The split line is no deeper than the `def` line and the function ends
/// before it, so nothing extends.
#[test]
fn an_extend_node_does_not_reach_a_split_line_at_its_own_level() {
    let python = fixture_bundle("python");
    assert_eq!(width(&python, "def f():\n    x = 1\n‸\n", 4), Some(0));
}

/// The header pattern's only stored capture is an anchored `@align`, so a
/// new line before the body still descends into it, where the body's own
/// `@outdent` opens the new line and cancels the function body's level.
#[test]
fn a_header_pattern_capturing_only_an_alignment_still_descends() {
    let c = bundle(
        "c",
        r#"(compound_statement) @indent
           (expression_statement) @outdent
           (while_statement
             condition: (_) @anchor
             body: (_) @align
             (#set! "scope" "header"))"#,
    );
    assert_eq!(width(&c, "void f() {\n  while (c)‸ x();\n}\n", 2), Some(0));
}

// ── Alignment ────────────────────────────────────────────────────────────

/// Splitting before `b` puts the closure's parameters on two lines, which
/// `#not-same-line?` needs to see to align `b` under `a` at column 9.
#[test]
fn predicates_compare_lines_as_they_read_after_the_break() {
    let rust = fixture_bundle("rust");
    assert_eq!(width(&rust, "let f = |a, ‸b| a;\n", 4), Some(9));
}

/// A statement whose call arguments must lie on one line, as the text
/// reads after the break, indents the new line it contains. The arguments
/// of `g(a)‸;` end at the break and stay on their line; those of `g(a‸);`
/// move down with their `)`.
#[test]
fn one_line_reads_an_end_at_the_break_as_staying_on_its_line() {
    let query = |predicate: &str| {
        bundle(
            "rust",
            &format!(
                "((expression_statement (call_expression (arguments) @args)) @indent \
                 ({predicate} @args))"
            ),
        )
    };
    let (one, not_one) = (query("#one-line?"), query("#not-one-line?"));
    let at_end = "fn f() {\n    g(a)‸;\n}\n";
    let inside = "fn f() {\n    g(a‸);\n}\n";
    assert_eq!(width(&one, at_end, 4), Some(4));
    assert_eq!(width(&one, inside, 4), Some(0));
    assert_eq!(width(&not_one, at_end, 4), Some(0));
    assert_eq!(width(&not_one, inside, 4), Some(4));
}

/// A statement whose callee and arguments must start on one line, as the
/// text reads after the break, indents the new line it contains. The
/// arguments of `g‸(a);` start at the break and move down; those of
/// `g(‸a);` stay.
#[test]
fn same_line_reads_a_start_at_the_break_as_moving_down() {
    let query = |predicate: &str| {
        bundle(
            "rust",
            &format!(
                "((expression_statement \
                   (call_expression function: (_) @callee arguments: (arguments) @args)) \
                 @indent ({predicate} @callee @args))"
            ),
        )
    };
    let (same, not_same) = (query("#same-line?"), query("#not-same-line?"));
    let at_start = "fn f() {\n    g‸(a);\n}\n";
    let inside = "fn f() {\n    g(‸a);\n}\n";
    assert_eq!(width(&same, at_start, 4), Some(0));
    assert_eq!(width(&same, inside, 4), Some(4));
    assert_eq!(width(&not_same, at_start, 4), Some(4));
    assert_eq!(width(&not_same, inside, 4), Some(0));
}

/// `a` is at column 4. Without a closer on the new line the continuation
/// line aligns under `a`, which at tab width 2 is two levels where the
/// argument list's own level is one; with the `)` moved onto it, the `)`
/// outdents one level from that alignment.
#[test]
fn python_call_arguments_align_under_the_first_argument() {
    let python = fixture_bundle("python");
    assert!(has_kind(&python, "foo(a,‸\n)\n", "argument_list"));
    assert_eq!(width(&python, "foo(a,‸\n)\n", 4), Some(4));
    assert_eq!(width(&python, "foo(a,‸\n)\n", 2), Some(4));
    assert!(has_kind(&python, "foo(a,‸)\n", "argument_list"));
    assert_eq!(width(&python, "foo(a,‸)\n", 4), Some(0));
}

/// `x = "` is 5 columns, `é` 1, `漢` 2, `" + foo(` 8: `a` is at 16.
#[test]
fn an_alignment_measures_wide_and_combining_text_in_display_columns() {
    let python = fixture_bundle("python");
    let src = "x = \"e\u{301}漢\" + foo(a,‸\n)\n";
    assert!(has_kind(&python, src, "argument_list"));
    assert_eq!(width(&python, src, 4), Some(16));
}

/// The tab reaches column 4 and `foo(` 8. The argument list's own level
/// alone would give 4.
#[test]
fn an_alignment_measures_a_tab_at_the_tab_width() {
    let python = fixture_bundle("python");
    let src = "\tfoo(a,‸\n)\n";
    assert!(has_kind(&python, src, "argument_list"));
    assert_eq!(width(&python, src, 4), Some(8));
}

#[test]
fn an_align_capture_without_an_anchor_is_ignored() {
    let rust = bundle("rust", "(block) @indent (arguments) @align");
    assert_eq!(width(&rust, "fn f() {\n    g(a,‸b);\n}\n", 4), Some(4));
    let anchored = bundle("rust", "(block) @indent (arguments . (_) @anchor) @align");
    assert_eq!(width(&anchored, "fn f() {\n    g(a,‸b);\n}\n", 4), Some(6));
}

/// The call's arguments align at `a`, column 4. The list opens on line 1,
/// below the call's line 0, so a line it contains is one level past the
/// alignment. The baseline `[1,` or `[` has the same alignment and no
/// level of its own: 4 + 4.
#[test]
fn a_scope_opening_below_an_aligned_node_adds_its_level() {
    let python = fixture_bundle("python");
    assert_eq!(
        hybrid_width(&python, "foo(a,\n    [1,‸\n     2])\n", 4),
        Some(8)
    );
    assert_eq!(
        hybrid_width(&python, "foo(a,\n    [‸\n    ])\n", 4),
        Some(8)
    );
}

/// The list opens on the call's own line, so its level is part of the
/// alignment column 4.
#[test]
fn a_scope_opening_on_the_aligned_nodes_line_adds_nothing() {
    let python = fixture_bundle("python");
    assert_eq!(hybrid_width(&python, "foo(a, [‸\n])\n", 4), Some(4));
}

/// The list's level and the `]`'s outdent cancel, leaving the alignment
/// column 4, the width of the baseline `[1,`, which has the same alignment
/// and no level.
#[test]
fn a_closer_typed_under_an_alignment_dedents_to_its_opener() {
    let python = fixture_bundle("python");
    assert_eq!(
        electric(
            &python,
            ElectricTrigger::Typed,
            "foo(a,\n    [1,\n        ]‸)\n",
            4
        ),
        Some(4)
    );
}

/// The baseline closer line `},` or `],` has its scope's level and its
/// outdent: net 0 at 4. The new line after `},` is net 0 too: 4. The
/// call's `)` outdents once from the alignment at `first`: 0.
#[test]
fn a_closer_line_under_an_alignment_is_a_baseline_at_net_zero() {
    let python = fixture_bundle("python");
    assert_eq!(
        hybrid_width(
            &python,
            "foo(a,\n    {\n        'k': 1,\n    },‸\n    b)\n",
            4
        ),
        Some(4)
    );
    assert_eq!(
        electric(
            &python,
            ElectricTrigger::Typed,
            "x = f(\n    first,\n    [\n        1,\n    ],\n    )‸\n",
            4
        ),
        Some(0)
    );
}

// ── Opaque and MISSING ───────────────────────────────────────────────────

#[test]
fn a_split_inside_a_string_begun_on_an_earlier_line_gives_none() {
    let rust = fixture_bundle("rust");
    assert_eq!(
        width(&rust, "fn f() {\n    let s = \"ab\ncd‸ef\";\n}\n", 4),
        None
    );
}

#[test]
fn a_split_inside_a_string_begun_on_its_own_line_is_indented() {
    let rust = fixture_bundle("rust");
    assert_eq!(
        width(&rust, "fn f() {\n    let s = \"ab‸cd\";\n}\n", 4),
        Some(4)
    );
}

/// The function body's `}` closes the block, and the module's `}` is a
/// MISSING node after it: the new line is two levels deep.
#[test]
fn a_missing_closer_does_not_outdent() {
    let rust = bundle(
        "rust",
        r#"[(block) (declaration_list)] @indent "}" @outdent"#,
    );
    let src = "mod m {\n    fn f() {\n        a();‸\n}\n";
    assert!(tree_has(&rust, src, |node| node.is_missing() && node.kind() == "}"));
    assert_eq!(width(&rust, src, 4), Some(8));
}

// ── Byte and char positions ──────────────────────────────────────────────

/// The comment's wide clusters make its byte length differ from its char
/// length, so a split read at the wrong unit lands on another line.
#[test]
fn wide_text_before_the_split_does_not_shift_it() {
    let rust = fixture_bundle("rust");
    assert_eq!(
        width(&rust, "fn f() {\n    // 漢漢😀\n    a();‸}\n", 4),
        Some(0)
    );
}

#[test]
fn a_split_on_a_rope_chunk_boundary_reads_the_tree_at_the_split() {
    let rust = fixture_bundle("rust");
    let (text, split) = (0..4096)
        .map(|n| {
            marked(&format!(
                "fn f() {{\n    // {}\n    a();‸}}\n// {}\n",
                "x".repeat(n),
                "y".repeat(4096)
            ))
        })
        .find(|(text, split)| {
            let byte = text.char_to_byte(split.offset());
            let mut chunk_ends = text.rope().chunks().scan(0, |end, chunk| {
                *end += chunk.len();
                Some(*end)
            });
            chunk_ends.any(|end| end == byte)
        })
        .expect("some comment length puts a chunk boundary at the split");
    assert_eq!(width_in(&rust, &text, split, 4), Some(0));
}

// ── Hybrid ───────────────────────────────────────────────────────────────

/// A rust query that indents blocks but not module bodies, so a line inside
/// `mod m` is wider than its absolute width.
const ALIGNED_BLOCK_QUERY: &str =
    r#"(block) @indent "}" @outdent (arguments . (_) @anchor) @align"#;

/// The model gives the new line one level more than the `fn` line, whose
/// tab is 4 columns: 8. The absolute width is 4, since `mod m`'s body has
/// no indent scope in this query.
#[test]
fn a_new_line_is_a_level_deeper_than_its_tab_indented_baseline() {
    let rust = bundle("rust", ALIGNED_BLOCK_QUERY);
    assert_eq!(
        hybrid_width(&rust, "mod m {\n\tfn f() {‸\n\t}\n}\n", 4),
        Some(8)
    );
}

/// The split line `b);` is aligned and the new line is not, so the
/// baseline is `g(a,`: 8 columns, the same level as the new line.
#[test]
fn a_baseline_with_another_alignment_is_passed_over() {
    let rust = bundle("rust", ALIGNED_BLOCK_QUERY);
    assert_eq!(
        hybrid_width(
            &rust,
            "mod m {\n\tfn f() {\n\t\tx();\n\t\tg(a,\n\t\t  b);‸\n\t}\n}\n",
            4
        ),
        Some(8)
    );
}

/// The four aligned argument lines use up the attempts, so `g(a,`, the
/// fifth non-blank baseline, is not tried and the absolute width 4 is used.
#[test]
fn four_mismatched_baselines_fall_back_to_the_absolute_width() {
    let rust = bundle("rust", ALIGNED_BLOCK_QUERY);
    assert_eq!(
        hybrid_width(
            &rust,
            "mod m {\n\tfn f() {\n\t\tg(a,\n\t\t  c,\n\t\t  d,\n\t\t  e,\n\t\t  f);‸\n\t}\n}\n",
            4
        ),
        Some(4)
    );
}

/// Three aligned argument lines precede `g(a,`, the fourth non-blank
/// baseline: it is still tried and gives 8, one level in from `mod m`'s
/// unindented scope with its own 8 columns.
#[test]
fn a_fourth_baseline_is_still_tried() {
    let rust = bundle("rust", ALIGNED_BLOCK_QUERY);
    assert_eq!(
        hybrid_width(
            &rust,
            "mod m {\n\tfn f() {\n\t\tg(a,\n\t\t  c,\n\t\t  d,\n\t\t  e);‸\n\t}\n}\n",
            4
        ),
        Some(8)
    );
}

/// Two aligned lines and two blank lines precede `g(a,`; the blank lines
/// are not attempts, so `g(a,` is the third and gives 8.
#[test]
fn blank_lines_do_not_use_up_baseline_attempts() {
    let rust = bundle("rust", ALIGNED_BLOCK_QUERY);
    assert_eq!(
        hybrid_width(
            &rust,
            "mod m {\n\tfn f() {\n\t\tg(a,\n\n\t\t  b,\n\n\t\t  c);‸\n\t}\n}\n",
            4
        ),
        Some(8)
    );
}

/// The closer line is two levels deep and its baseline `x();` three, but
/// `x();` has no indent: 0 minus one level stops at 0. The absolute width
/// is 8.
#[test]
fn a_baseline_narrower_than_the_level_drop_gives_zero() {
    let rust = bundle("rust", ALIGNED_BLOCK_QUERY);
    assert_eq!(
        hybrid_width(&rust, "fn f() {\n{\n{\nx();‸}}}\n", 4),
        Some(0)
    );
}

/// Each fence is its own rust layer. The new line aligns under `c` at
/// column 2; its own line `h(c, d);` is not aligned, and the two markdown
/// lines above have no indents query. The fourth baseline, `      b);`, is
/// aligned at column 2 too but in the first fence's layer, so it is passed
/// over and the alignment column 2 is used. Taking it would give its own
/// width, 6.
#[test]
fn a_baseline_in_another_layer_is_passed_over() {
    let (markdown, langs) = markdown_with_rust(ALIGNED_BLOCK_QUERY);
    let src = "```rust\ng(a,\n      b);\n```\n\n```rust\nh(c,‸ d);\n```\n";
    let (text, _) = marked(src);
    let syntax = Syntax::attach_sync(Arc::clone(&markdown), &text, &langs);
    assert_eq!(syntax.layers().expect("the text parses").layers.len(), 3);
    assert_eq!(
        break_indent_with(&markdown, &langs, src, 4).map(|indent| indent.width.get()),
        Some(2)
    );
}

/// U+3000 is a blank 2 columns wide: one level past it is 6.
#[test]
fn a_wide_blank_baseline_is_measured_in_display_columns() {
    let rust = bundle("rust", ALIGNED_BLOCK_QUERY);
    assert_eq!(
        hybrid_width(&rust, "mod m {\n\u{3000}fn f() {‸\n}\n}\n", 4),
        Some(6)
    );
}

// ── Injections ───────────────────────────────────────────────────────────

/// A markdown bundle injecting fenced code, and `langs` mapping `rust` to
/// a rust bundle with `indents`.
fn markdown_with_rust(
    indents: &str,
) -> (
    Arc<GrammarBundle>,
    Arc<rustc_hash::FxHashMap<String, Arc<GrammarBundle>>>,
) {
    require_grammars(&["markdown"]);
    let injections = std::fs::read_to_string(require_helix_query(
        "markdown",
        OptionalQuery::Injections.file_name(),
    ))
    .expect("read helix-injections.scm");
    let markdown = make_bundle(
        "markdown",
        "tree_sitter_markdown",
        "",
        &[(OptionalQuery::Injections, &injections)],
    );
    let langs = Arc::new(rustc_hash::FxHashMap::from_iter([(
        "rust".to_owned(),
        bundle("rust", indents),
    )]));
    (markdown, langs)
}

/// A rust bundle with the fixture injections and indents queries, and `langs`
/// mapping `rust` to it, so a macro's arguments parse as a nested rust
/// layer.
fn rust_with_macro_layers() -> (
    Arc<GrammarBundle>,
    Arc<rustc_hash::FxHashMap<String, Arc<GrammarBundle>>>,
) {
    require_grammars(&["rust"]);
    let injections = std::fs::read_to_string(require_helix_query(
        "rust",
        OptionalQuery::Injections.file_name(),
    ))
    .expect("read helix-injections.scm");
    let rust = make_bundle(
        "rust",
        "tree_sitter_rust",
        "",
        &[
            (OptionalQuery::Injections, &injections),
            (OptionalQuery::Indents, &fixture_indents("rust")),
        ],
    );
    let langs = Arc::new(rustc_hash::FxHashMap::from_iter([(
        "rust".to_owned(),
        Arc::clone(&rust),
    )]));
    (rust, langs)
}

/// The fence's first code line, `  fn f() {`, starts the injected layer's
/// range and is 2 wide: that is the layer's base, and the new line in the
/// function body is one level past it.
#[test]
fn an_injected_layer_is_indented_from_its_first_line() {
    let (markdown, langs) = markdown_with_rust(BLOCK_QUERY);
    let (text, split) = marked("- item\n\n  ```rust\n  fn f() {‸\n  }\n  ```\n");
    assert_eq!(width_in_with(&markdown, &langs, &text, split, 4), Some(6));
}

/// `a` is at column 8 of the buffer line, the 2-column base included, so
/// the aligned new line is 8 wide and the base is not added again.
#[test]
fn an_aligned_line_in_an_injected_layer_gets_the_base_once() {
    let (markdown, langs) = markdown_with_rust(ALIGNED_BLOCK_QUERY);
    let (text, split) = marked("- item\n\n  ```rust\n  fn f() {\n      g(a,‸ b);\n  }\n  ```\n");
    assert_eq!(width_in_with(&markdown, &langs, &text, split, 4), Some(8));
}

/// The macro's arguments are a rust layer inside the root rust layer, so
/// the model merges out to the root: the token tree (line 1) and the
/// function body (line 0) each add a level. The `x` line's own 2 columns
/// do not count.
#[test]
fn a_same_language_layer_merges_into_the_one_enclosing_it() {
    let (rust, langs) = rust_with_macro_layers();
    let src = "fn f() {\n    foo! {\n  x‸\n    }\n}\n";
    let (text, split) = marked(src);
    let syntax = Syntax::attach_sync(Arc::clone(&rust), &text, &langs);
    assert_eq!(syntax.layers().expect("the text parses").layers.len(), 2);
    assert_eq!(width_in_with(&rust, &langs, &text, split, 4), Some(8));
}

/// The macro's empty arguments are a rust layer merged into the root, so
/// both the new line and its baseline `  foo! {` are modelled by the root
/// layer: the token tree and the function body give the new line 2 levels,
/// the baseline's function body 1, so 2 columns plus one level.
#[test]
fn hybrid_baselines_are_compared_in_the_merged_layer() {
    let (rust, langs) = rust_with_macro_layers();
    let src = "fn f() {\n  foo! {‸\n  }\n}\n";
    let (text, _) = marked(src);
    let syntax = Syntax::attach_sync(Arc::clone(&rust), &text, &langs);
    assert_eq!(syntax.layers().expect("the text parses").layers.len(), 2);
    assert_eq!(
        break_indent_with(&rust, &langs, src, 4).map(|indent| indent.width.get()),
        Some(6)
    );
}

/// A rust bundle injecting every string body as C, and `langs` mapping
/// `c` to a bundle with the fixture C indents query.
fn rust_with_c_strings() -> (
    Arc<GrammarBundle>,
    Arc<rustc_hash::FxHashMap<String, Arc<GrammarBundle>>>,
) {
    require_grammars(&["rust"]);
    let rust = make_bundle(
        "rust",
        "tree_sitter_rust",
        "",
        &[(
            OptionalQuery::Injections,
            r#"((string_content) @injection.content (#set! injection.language "c"))"#,
        )],
    );
    let langs = Arc::new(rustc_hash::FxHashMap::from_iter([(
        "c".to_owned(),
        fixture_bundle("c"),
    )]));
    (rust, langs)
}

/// The C range starts at `/*`, after the `let` line's first non-blank
/// character, so the base line is the next one, `      {`: 6 columns. The
/// new line in the compound statement is one level past it.
#[test]
fn an_injected_range_starting_mid_line_takes_its_base_from_the_next_line() {
    let (rust, langs) = rust_with_c_strings();
    let (text, split) = marked("fn f() {\n    let s = r\"/* c */\n      {‸\n      }\n\";\n}\n");
    assert_eq!(width_in_with(&rust, &langs, &text, split, 4), Some(10));
}

/// The C range starts and ends on the `let` line, after its first
/// non-blank character, so no line of it gives a base: the compound
/// statement's level is counted from column 0.
#[test]
fn an_injected_range_within_one_line_has_no_base_line() {
    let (rust, langs) = rust_with_c_strings();
    let (text, split) = marked("fn f() {\n    let s = r\"{ x;‸ }\";\n        g();\n}\n");
    assert_eq!(width_in_with(&rust, &langs, &text, split, 4), Some(4));
}

/// The fence is a rust layer, and the macro's arguments a rust layer
/// inside it, so the model merges out to the fence and stops at markdown:
/// the fence's first line gives the 2-column base, and the function body
/// and the token tree each add a level.
#[test]
fn a_macro_in_a_fence_merges_into_the_fence_layer() {
    let (rust, _) = rust_with_macro_layers();
    let (markdown, _) = markdown_with_rust(BLOCK_QUERY);
    let langs = Arc::new(rustc_hash::FxHashMap::from_iter([(
        "rust".to_owned(),
        rust,
    )]));
    let (text, split) =
        marked("- item\n\n  ```rust\n  fn f() {\n      foo! {\n    x‸\n      }\n  }\n  ```\n");
    let syntax = Syntax::attach_sync(Arc::clone(&markdown), &text, &langs);
    let mut depths: Vec<u8> = syntax
        .layers()
        .expect("the text parses")
        .layers
        .iter()
        .map(|layer| layer.depth)
        .collect();
    depths.sort_unstable();
    assert_eq!(depths, [0, 1, 2]);
    assert_eq!(width_in_with(&markdown, &langs, &text, split, 4), Some(10));
}

/// The string's C layer has no indents query, so the rust layer around
/// it answers: the block holds the new line, one level.
#[test]
fn a_layer_without_an_indents_query_leaves_the_line_to_the_layer_around_it() {
    require_grammars(&["rust", "c"]);
    let rust = make_bundle(
        "rust",
        "tree_sitter_rust",
        "",
        &[
            (
                OptionalQuery::Injections,
                r#"((string_content) @injection.content (#set! injection.language "c"))"#,
            ),
            (OptionalQuery::Indents, BLOCK_QUERY),
        ],
    );
    let langs = Arc::new(rustc_hash::FxHashMap::from_iter([(
        "c".to_owned(),
        make_bundle("c", "tree_sitter_c", "", &[]),
    )]));
    let (text, split) = marked("fn f() {\n    let s = \"a‸b\";\n}\n");
    let syntax = Syntax::attach_sync(Arc::clone(&rust), &text, &langs);
    assert_eq!(syntax.layers().expect("the text parses").layers.len(), 2);
    assert_eq!(width_in_with(&rust, &langs, &text, split, 4), Some(4));
}

// ── after_token ──────────────────────────────────────────────────────────

#[test]
fn an_opening_brace_before_the_split_is_a_token() {
    let rust = bundle("rust", BLOCK_QUERY);
    assert_eq!(
        break_indent(&rust, "fn f() {‸}\n", 4).map(|indent| indent.after_token),
        Some(true)
    );
}

/// `é` is 2 bytes and `漢` 3, so the `{` before the split starts 3 bytes
/// past its char offset; it is still the leaf `{` covering that cluster.
#[test]
fn a_token_after_multi_byte_text_is_found_at_its_byte_offset() {
    let rust = bundle("rust", BLOCK_QUERY);
    assert_eq!(
        break_indent(&rust, "fn f() { let s = \"é漢\"; if x {‸} }\n", 4)
            .map(|indent| indent.after_token),
        Some(true)
    );
}

#[test]
fn a_brace_inside_a_string_is_not_a_token() {
    let rust = bundle("rust", BLOCK_QUERY);
    assert_eq!(
        break_indent(&rust, "fn f() {\n    let s = \"{‸}\";\n}\n", 4)
            .map(|indent| indent.after_token),
        Some(false)
    );
}

#[test]
fn a_split_at_the_buffer_start_follows_no_token() {
    let rust = bundle("rust", BLOCK_QUERY);
    assert_eq!(
        break_indent(&rust, "‸fn f() {}\n", 4).map(|indent| indent.after_token),
        Some(false)
    );
}

// ── Electric dedent ──────────────────────────────────────────────────────

/// What `electric_indent` gives the line holding the `‸`, with the cursor
/// at the `‸` and `trigger` the input landing there.
fn electric(
    bundle: &Arc<GrammarBundle>,
    trigger: ElectricTrigger,
    src: &str,
    tw: u8,
) -> Option<u32> {
    electric_with(bundle, trigger, WordChars::default(), src, tw)
}

/// [`electric`] with `word_chars` as the buffer's extra word characters.
fn electric_with(
    bundle: &Arc<GrammarBundle>,
    trigger: ElectricTrigger,
    word_chars: WordChars<'_>,
    src: &str,
    tw: u8,
) -> Option<u32> {
    let (text, cursor) = marked(src);
    let syntax = Syntax::attach_sync(Arc::clone(bundle), &text, &empty_langs());
    let layers = syntax.layers().expect("the text parses");
    let line = text.char_to_line(cursor.offset());
    electric_indent(layers, &text, line, cursor, trigger, word_chars, tw).map(BufferLineCol::get)
}

/// The block holds the `}` line and the `}` opens it: 0 levels, one fewer
/// than the baseline `a();` at 4.
#[test]
fn a_closing_brace_typed_on_a_fresh_line_dedents_it() {
    let rust = fixture_bundle("rust");
    assert_eq!(
        electric(
            &rust,
            ElectricTrigger::Typed,
            "fn f() {\n    a();\n    }‸\n",
            4
        ),
        Some(0)
    );
}

/// The model would put the line at 4, but nothing on it closes a scope.
#[test]
fn a_line_without_an_outdent_token_is_left_alone() {
    let rust = fixture_bundle("rust");
    assert_eq!(
        electric(
            &rust,
            ElectricTrigger::Typed,
            "fn f() {\n    a();\n        let x = 1;‸\n}\n",
            4
        ),
        None
    );
}

/// The `if` statement holds the clause line and its keyword opens it: 0
/// levels, one fewer than the baseline `x` at 4.
#[test]
fn a_python_elif_clause_dedents_to_its_if() {
    let python = fixture_bundle("python");
    assert_eq!(
        electric(
            &python,
            ElectricTrigger::Typed,
            "if a:\n    x\n    elif c:‸\n",
            4
        ),
        Some(0)
    );
}

#[test]
fn a_python_except_clause_dedents_to_its_try() {
    let python = fixture_bundle("python");
    assert_eq!(
        electric(
            &python,
            ElectricTrigger::Typed,
            "try:\n    x\n    except E:‸\n",
            4
        ),
        Some(0)
    );
}

/// The baseline `break;` sits two compound statements deep at 6; the
/// label's own outdent puts it one level shallower: 4, under `case 0:`.
#[test]
fn a_c_case_or_default_label_dedents_to_the_previous_label() {
    let c = fixture_bundle("c");
    let switch = "void f(int a) {\n  switch (a) {\n    case 0:\n      break;\n      ";
    for label in ["case 1:", "default:"] {
        assert_eq!(
            electric(
                &c,
                ElectricTrigger::Typed,
                &format!("{switch}{label}‸\n"),
                2
            ),
            Some(4),
            "{label}"
        );
    }
}

/// Python's grammar parses a bare `else:`, `except:` or `finally:` typed
/// one level too deep as an error with no clause node, so no outdent
/// capture names its keyword.
#[test]
fn bare_python_else_except_and_finally_are_not_dedented() {
    let python = fixture_bundle("python");
    for src in [
        "if a:\n    x\n    else:‸\n",
        "try:\n    x\n    except:‸\n",
        "try:\n    x\n    finally:‸\n",
    ] {
        assert_eq!(
            electric(&python, ElectricTrigger::Typed, src, 4),
            None,
            "{src:?}"
        );
    }
}

/// A python query making the bare keyword `else` an outdent. Typed on its
/// own the parser keeps it as a token inside the `if` body's block; under
/// the cursor it could still grow into an identifier, so it waits at every
/// prefix of `elsewhere`. Once a space follows it, it is complete: the
/// block holds its line and `else` opens it, 0 levels, the same as the
/// baseline `x`, whose block starts on its own line: 4.
#[test]
fn a_word_starting_with_an_outdent_keyword_never_dedents_while_typed() {
    let python = bundle("python", r#"(block) @indent "else" @outdent"#);
    let keyword = "if a:\n    x\n    else‸\n";
    assert!(
        has_kind(&python, keyword, "else"),
        "the bare keyword is a token of the tree"
    );
    assert_eq!(
        electric(
            &python,
            ElectricTrigger::Typed,
            "if a:\n    x\n    else ‸\n",
            4
        ),
        Some(4)
    );
    let word = "elsewhere";
    for end in 1..=word.len() {
        let typed = &word[..end];
        assert_eq!(
            electric(
                &python,
                ElectricTrigger::Typed,
                &format!("if a:\n    x\n    {typed}‸\n"),
                4
            ),
            None,
            "{typed}"
        );
    }
}

/// C's `"default" @outdent` names the bare keyword, which the parser keeps
/// as a token while the label lacks its `:`. Under the cursor it could
/// still grow into an identifier, so it waits.
#[test]
fn a_c_identifier_starting_with_default_never_dedents_while_typed() {
    let c = fixture_bundle("c");
    let switch = "void f(int a) {\n  switch (a) {\n    case 0:\n      break;\n      ";
    let keyword = format!("{switch}default‸\n");
    assert!(
        has_kind(&c, &keyword, "default"),
        "the label without its `:` keeps the `default` token"
    );
    let word = "defaultValue";
    for end in 1..=word.len() {
        let typed = &word[..end];
        assert_eq!(
            electric(
                &c,
                ElectricTrigger::Typed,
                &format!("{switch}{typed}‸\n"),
                2
            ),
            None,
            "{typed}"
        );
    }
}

/// A rust query making `.` an outdent: under the cursor it could still
/// grow into `..`, so it waits; once a character follows it, it is
/// complete and the line dedents from the baseline `x` at 4 to 0.
#[test]
fn a_token_that_prefixes_another_waits_for_the_next_character() {
    let rust = bundle("rust", r#"(block) @indent "}" @outdent "." @outdent"#);
    let dot = "fn f() {\n    x\n    .‸\n}\n";
    assert!(has_kind(&rust, dot, "."), "the `.` is a token of the tree");
    assert_eq!(electric(&rust, ElectricTrigger::Typed, dot, 4), None);
    assert_eq!(
        electric(
            &rust,
            ElectricTrigger::Typed,
            "fn f() {\n    x\n    .y‸\n}\n",
            4
        ),
        Some(0)
    );
}

/// A C query making `end--` an outdent. Typed, it ends at the cursor: with
/// `-` a plain character it is complete, and the line dedents from the
/// baseline `a();` at 4 to 0; with `-` a word character it could still
/// grow, so it waits.
#[test]
fn a_token_ending_in_a_buffer_word_character_waits_while_typed() {
    let c = bundle(
        "c",
        r#"(compound_statement) @indent "}" @outdent (update_expression) @outdent"#,
    );
    let src = "void f() {\n    a();\n    end--‸\n}\n";
    assert!(
        has_kind(&c, src, "update_expression"),
        "`end--` is an update expression"
    );
    assert_eq!(electric(&c, ElectricTrigger::Typed, src, 4), Some(0));
    assert_eq!(
        electric_with(&c, ElectricTrigger::Typed, WordChars::new("-"), src, 4),
        None
    );
}

/// A python query making only the bare keyword `else` an outdent.
const ELSE_QUERY: &str = r#""else" @outdent"#;

/// Under the cursor `else` could still grow into an identifier; a line
/// break cannot continue it. `else` opens its line, -1 level, and the
/// baseline `x` has none, so the line moves from 4 to 0.
#[test]
fn a_line_break_after_an_outdent_keyword_completes_it() {
    let python = bundle("python", ELSE_QUERY);
    let src = "if a:\n    x\n    else‸\n";
    assert!(
        has_kind(&python, src, "else"),
        "`else` is a token of the tree"
    );
    assert_eq!(electric(&python, ElectricTrigger::Typed, src, 4), None);
    assert_eq!(
        electric(&python, ElectricTrigger::LineBreak, src, 4),
        Some(0)
    );
}

/// A break inside `else` or before `}` carries the token's rest to the
/// next line, so the line it leaves does not start with that token.
/// `elsewh` is one identifier, which no capture names.
#[test]
fn a_line_break_before_an_outdent_tokens_end_does_not_complete_it() {
    let python = bundle("python", ELSE_QUERY);
    let rust = bundle("rust", BLOCK_QUERY);
    for src in ["if a:\n    x\n    els‸e\n", "if a:\n    x\n    elsewh‸\n"] {
        assert_eq!(
            electric(&python, ElectricTrigger::LineBreak, src, 4),
            None,
            "{src:?}"
        );
    }
    assert_eq!(
        electric(
            &rust,
            ElectricTrigger::LineBreak,
            "fn f() {\n    a();\n    ‸}\n",
            4
        ),
        None
    );
}

#[test]
fn the_token_prefixes_come_from_the_grammars_anonymous_tokens() {
    let rust = compile("rust", BLOCK_QUERY).expect("compiles");
    assert!(rust.is_token_prefix("."), "`.` prefixes `..`");
    assert!(!rust.is_token_prefix("}"), "no rust token extends `}}`");
    let c = compile("c", "(compound_statement) @indent").expect("compiles");
    assert!(c.is_token_prefix("]"), "`]` prefixes `]]`");
}
