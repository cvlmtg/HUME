use hume_core::OptionalQuery;
use hume_editing::changeset::ChangeSetBuilder;
use hume_engine::theme::ScopeRegistry;
use hume_rope::offset::CharOffset;
use proptest::prelude::*;
use tree_sitter::Query;

use super::*;
use crate::edited_bytes::EditedBytes;
use crate::edits::{ChangeEdits, input_edits_from_changeset};
use crate::injections::InjectionSites;
use crate::test_support::{
    bundle_with_fixture_query, fixture_query, open_grammar, parse, whole_text_prior,
};

const SCOPES: &str = "(function_item) @local.scope\n(block) @local.scope\n";
const DEFINITIONS: &str = "(parameter pattern: (identifier) @local.definition.variable.parameter)
(let_declaration pattern: (identifier) @local.definition.variable)\n";
const REFERENCES: &str = "(identifier) @local.reference\n";

/// The locals `source`, a rust `locals.scm`, finds in `text`, and the
/// registry its highlights were interned in.
fn locals(source: &str, text: &str) -> (LayerLocals, ScopeRegistry) {
    test_fixtures::require_grammars(&["rust"]);
    let grammar = open_grammar("rust", "tree_sitter_rust");
    let mut registry = ScopeRegistry::new();
    let mut skipped = Vec::new();
    let query = LocalsQuery::new(
        Query::new(grammar.language(), source).expect("the query compiles"),
        source,
        &mut registry,
        &mut skipped,
    )
    .expect("the query names locals");
    assert!(skipped.is_empty(), "{skipped:?}");
    let (_parser, tree) = parse(grammar.language(), text);
    let rope = ropey::Rope::from_str(text);
    let mut locals = LayerLocals::default();
    locals.rebuild(&query, &mut QueryCursor::new(), &tree, &rope, None);
    (locals, registry)
}

/// Each resolved reference with a highlight, as `(byte offset, highlight)`.
fn references(locals: &LayerLocals) -> Vec<(usize, ScopeId)> {
    locals
        .references_in(0, usize::MAX)
        .map(|(range, highlight)| (range.start, highlight))
        .collect()
}

fn all() -> String {
    format!("{SCOPES}{DEFINITIONS}{REFERENCES}")
}

#[test]
fn a_parameter_resolves_in_its_function_and_not_in_another() {
    let text = "fn f(a: u8) -> u8 { a }\nfn g() -> u8 { a }\n";
    let (locals, mut registry) = locals(&all(), text);
    let parameter = registry.intern("variable.parameter");
    let use_in_f = text.find("{ a }").unwrap() + 2;
    assert_eq!(references(&locals), [(use_in_f, parameter)]);
    let use_in_g = text.rfind("{ a }").unwrap() + 2;
    assert!(locals.is_local(use_in_f, use_in_f + 1));
    assert!(!locals.is_local(use_in_g, use_in_g + 1));
}

#[test]
fn a_definition_is_local_but_no_reference_to_itself() {
    let text = "fn f(a: u8) {}\n";
    let (locals, _) = locals(&all(), text);
    let definition = text.find('a').unwrap();
    assert!(locals.is_local(definition, definition + 1));
    assert!(references(&locals).is_empty());
}

#[test]
fn a_scope_that_does_not_inherit_sees_none_of_its_parents_definitions() {
    let text = "fn f() { let x = 1; fn g() -> i32 { x } }\n";
    let inner_use = text.rfind('x').unwrap();
    let inheriting = all();
    let (locals_in, _) = locals(&inheriting, text);
    assert!(locals_in.is_local(inner_use, inner_use + 1));
    let closed = format!(
        "((function_item) @local.scope (#set! local.scope-inherits false))\n(block) @local.scope\n{DEFINITIONS}{REFERENCES}"
    );
    let (locals_out, _) = locals(&closed, text);
    assert!(!locals_out.is_local(inner_use, inner_use + 1));
}

#[test]
fn a_reference_sees_only_the_definitions_before_it() {
    let text = "fn f(x: u8) { x; let x = 1; x; }\n";
    let (locals, mut registry) = locals(&all(), text);
    let parameter = registry.intern("variable.parameter");
    let variable = registry.intern("variable");
    let uses: Vec<usize> = text.match_indices("x;").map(|(i, _)| i).collect();
    assert_eq!(
        references(&locals),
        [(uses[0], parameter), (uses[1], variable)]
    );
}

#[test]
fn a_reference_before_any_definition_stays_unresolved() {
    let text = "fn f() { x; let x = 1; x; }\n";
    let (locals, _) = locals(&all(), text);
    let first = text.find("x;").unwrap();
    let last = text.rfind("x;").unwrap();
    assert!(!locals.is_local(first, first + 1));
    assert!(locals.is_local(last, last + 1));
}

#[test]
fn a_bare_definition_makes_its_references_local_without_a_highlight() {
    let source =
        format!("{SCOPES}(let_declaration pattern: (identifier) @local.definition)\n{REFERENCES}");
    let text = "fn f() { let x = 1; x; }\n";
    let (locals, _) = locals(&source, text);
    let reference = text.rfind('x').unwrap();
    assert!(locals.is_local(reference, reference + 1));
    assert!(references(&locals).is_empty());
}

#[test]
fn a_node_another_capture_names_is_no_reference_whatever_the_pattern_order() {
    let text = "fn f(a: u8) { a; a.max(a); }\n";
    let discard = "(field_expression value: (identifier) @_)\n";
    for source in [format!("{}{discard}", all()), format!("{discard}{}", all())] {
        let (locals, _) = locals(&source, text);
        let receiver = text.find("a.").unwrap();
        let (first, argument) = (text.find("a;").unwrap(), text.find("a)").unwrap());
        assert!(!locals.is_local(receiver, receiver + 1), "{source}");
        assert!(locals.is_local(first, first + 1), "{source}");
        assert!(locals.is_local(argument, argument + 1), "{source}");
    }
}

#[test]
fn the_rust_fixture_leaves_lifetimes_labels_and_paths_out_of_references() {
    test_fixtures::require_grammars(&["rust"]);
    let source = fixture_query("rust", OptionalQuery::Locals);
    let text = "fn f<'a>(a: &'a str, len: usize, outer: u8) -> &'a str { 'outer: loop { m::len(); break 'outer; } a }\n";
    let (locals, mut registry) = locals(&source, text);
    let parameter = registry.intern("variable.parameter");
    let body = text.rfind(" a }").unwrap() + 1;
    assert_eq!(references(&locals), [(body, parameter)]);
    let lifetimes = text.match_indices("'a").map(|(at, _)| at + 1);
    let labels = text.match_indices("'outer").map(|(at, _)| at + 1);
    let path = text.find("::len").unwrap() + 2;
    for (at, len) in lifetimes
        .map(|at| (at, 1))
        .chain(labels.map(|at| (at, 5)))
        .chain([(path, 3)])
    {
        assert!(!locals.is_local(at, at + len), "{:?}", &text[at..at + len]);
    }
}

#[test]
fn an_edit_moves_every_range() {
    let text = "fn f(a: u8) -> u8 { a }\n";
    let (mut locals, _) = locals(&all(), text);
    let before = references(&locals);
    locals.edit(&change(text, &[(0, 0, "//")]).0.byte_map());
    let after: Vec<_> = before.iter().map(|&(at, h)| (at + 2, h)).collect();
    assert_eq!(references(&locals), after);
    let definition = text.find('a').unwrap() + 2;
    assert!(locals.is_local(definition, definition + 1));
}

// ── Rebuilding after an edit ───────────────────────────────────────────────

const RUST: &str = "fn f(alpha: u32) -> u32 {\n    let beta = alpha + 1;\n    beta * alpha\n}\n\nfn g(gamma: u32) -> u32 {\n    gamma\n}\n";
const PYTHON: &str = "def f(alpha):\n    beta = alpha + 1\n    return beta * alpha\n\ndef g(gamma):\n    return gamma\n";

/// `name`'s fixture grammar with its fixture `locals.scm` and no other query.
fn fixture_bundle(name: &str) -> Arc<GrammarBundle> {
    bundle_with_fixture_query(name, &format!("tree_sitter_{name}"), OptionalQuery::Locals)
}

/// One replacement of a change: `removed` bytes at `at` replaced by
/// `inserted`, in an ASCII text.
type Replace<'a> = (usize, usize, &'a str);

/// The edits of the change making `replaces`, ascending, of ASCII `before`,
/// and the text it makes.
fn change(before: &str, replaces: &[Replace<'_>]) -> (ChangeEdits, String) {
    let mut builder = ChangeSetBuilder::new(CharOffset::new(before.len()));
    let mut after = String::new();
    let mut kept = 0;
    for &(at, removed, inserted) in replaces {
        builder.retain_to(CharOffset::new(at));
        builder.delete_to(CharOffset::new(at + removed));
        builder.insert(inserted);
        after.push_str(&before[kept..at]);
        after.push_str(inserted);
        kept = at + removed;
    }
    after.push_str(&before[kept..]);
    let edits = input_edits_from_changeset(&builder.finish(), &ropey::Rope::from_str(before));
    (edits, after)
}

/// The locals of `before` after the change making `replaces`, updated from
/// `before`'s locals as the parse worker updates them, after `tamper`
/// changes those, and built afresh.
fn updated_and_fresh(
    bundle: &Arc<GrammarBundle>,
    before: &str,
    replaces: &[Replace<'_>],
    tamper: impl FnOnce(&mut LayerLocals),
) -> (Arc<LayerLocals>, Arc<LayerLocals>) {
    let (mut parser, mut old_tree) = parse(bundle.grammar.language(), before);
    let mut prior = LayerLocals::clone(&layer_locals(
        bundle,
        &old_tree,
        &ropey::Rope::from_str(before),
        None,
    ));
    tamper(&mut prior);
    let (edits, after) = change(before, replaces);
    let mut edited = EditedBytes::default();
    for edit in edits.iter() {
        old_tree.edit(edit);
        edited.record(edit);
    }
    prior.edit(&edits.byte_map());
    let tree = parser
        .parse(&after, Some(&old_tree))
        .expect("the text parses");
    let rope = ropey::Rope::from_str(&after);
    let prior = Arc::new(prior);
    let sites = InjectionSites::default();
    let layer_prior = whole_text_prior(&old_tree, &sites, &prior, &edited, true);
    (
        layer_locals(bundle, &tree, &rope, Some(&layer_prior.against(&tree))),
        layer_locals(bundle, &tree, &rope, None),
    )
}

/// One edit of `source`: `removed` bytes at the first `needle`, offset by
/// `skip`, replaced by `inserted`.
fn edit_at(
    source: &str,
    needle: &str,
    skip: usize,
    removed: usize,
    inserted: &'static str,
) -> Replace<'static> {
    (
        source.find(needle).expect("the needle is in the source") + skip,
        removed,
        inserted,
    )
}

/// Whether an entry planted in the prior locals over `RUST`'s use of
/// `gamma` in `g` survives the edit: true when the update did not rebuild
/// the scope it is in.
fn keeps_a_marker_in_g(at: usize, removed: usize, inserted: &str) -> bool {
    let bundle = fixture_bundle("rust");
    let in_g = RUST.rfind("gamma\n").unwrap();
    let marker = ExclusiveRange::new(in_g, in_g + "gamma".len());
    let (updated, _) = updated_and_fresh(&bundle, RUST, &[(at, removed, inserted)], |prior| {
        prior.resolved.retain(|(range, _)| *range != marker);
        let place = prior
            .resolved
            .partition_point(|(r, _)| key(r) < key(&marker));
        prior.resolved.insert(place, (marker, None));
    });
    let moved = if at <= in_g {
        in_g + inserted.len() - removed
    } else {
        in_g
    };
    updated
        .resolved
        .iter()
        .any(|(range, highlight)| range.start == moved && highlight.is_none())
}

#[test]
fn an_edit_in_one_function_leaves_the_others_locals_as_they_were() {
    let (at, removed, inserted) = edit_at(RUST, "beta * alpha", 0, 4, "delta");
    assert!(keeps_a_marker_in_g(at, removed, inserted));
}

#[test]
fn a_top_level_edit_rebuilds_the_layer() {
    assert!(!keeps_a_marker_in_g(0, 0, "fn h() {}\n"));
}

#[test]
fn removing_a_functions_closing_brace_rebuilds_what_it_closed_over() {
    let (at, removed, inserted) = edit_at(RUST, "alpha\n}", 6, 1, "");
    assert!(!keeps_a_marker_in_g(at, removed, inserted));
}

#[test]
fn an_incremental_update_equals_a_fresh_build() {
    for (name, source, edits) in [
        (
            "rust",
            RUST,
            vec![
                edit_at(RUST, "beta * alpha", 0, 4, "delta"),
                edit_at(RUST, "let beta", 4, 4, "bet"),
                edit_at(RUST, "alpha: u32", 0, 10, ""),
                edit_at(RUST, "gamma\n}", 5, 0, ";\n    let gamma = 2;\n    gamma"),
                edit_at(RUST, "alpha\n}", 6, 1, ""),
                edit_at(RUST, "-> u32 {\n    let", 7, 1, ""),
                edit_at(RUST, "}\n\nfn g", 0, 7, ""),
                (0, 0, "fn h() {}\n"),
                (RUST.len(), 0, "fn k(a: u8) -> u8 { a }\n"),
            ],
        ),
        (
            "python",
            PYTHON,
            vec![
                edit_at(PYTHON, "beta * alpha", 0, 4, "delta"),
                edit_at(PYTHON, "beta = ", 0, 4, "bet"),
                edit_at(PYTHON, "(gamma)", 1, 5, "delta"),
                edit_at(PYTHON, "return gamma", 0, 0, "gamma = 2\n    "),
                (0, 0, "x = 1\n"),
                (PYTHON.len(), 0, "def k(a):\n    return a\n"),
            ],
        ),
    ] {
        let bundle = fixture_bundle(name);
        for (at, removed, inserted) in edits {
            let (updated, fresh) =
                updated_and_fresh(&bundle, source, &[(at, removed, inserted)], |_| {});
            assert_eq!(
                updated, fresh,
                "{name}: {removed} bytes at {at} replaced by {inserted:?}"
            );
        }
    }
}

#[test]
fn a_change_at_several_places_updates_to_a_fresh_build() {
    let bundle = fixture_bundle("rust");
    let replaces = [
        edit_at(RUST, "let beta", 4, 4, "bet"),
        edit_at(RUST, "beta * alpha", 0, 4, "delta"),
        edit_at(RUST, "gamma\n}", 0, 5, "gamma + gamma"),
        (RUST.len(), 0, "fn k(a: u8) -> u8 { a }\n"),
    ];
    let (updated, fresh) = updated_and_fresh(&bundle, RUST, &replaces, |_| {});
    assert_eq!(updated, fresh);
}

#[test]
fn a_change_in_several_functions_changing_their_counts_updates_to_a_fresh_build() {
    const FIVE: &str = "fn a(alpha: u32) -> u32 {\n    alpha + alpha\n}\n\nfn b(beta: u32) -> u32 {\n    beta\n}\n\nfn c(gamma: u32) -> u32 {\n    gamma\n}\n\nfn d(delta: u32) -> u32 {\n    delta\n}\n\nfn e(eps: u32) -> u32 {\n    eps\n}\n";
    let bundle = fixture_bundle("rust");
    let replaces = [
        edit_at(FIVE, "(alpha:", 6, 0, "x"),
        edit_at(FIVE, "    gamma\n", 4, 0, "let mut z = 1;\n    z;\n    "),
        edit_at(FIVE, "    delta\n", 4, 0, "let f = |q: u32| q;\n    "),
    ];
    let (updated, fresh) = updated_and_fresh(&bundle, FIVE, &replaces, |_| {});
    assert_eq!(updated, fresh);
}

#[test]
fn changes_in_two_blocks_of_one_function_update_to_a_fresh_build() {
    const BLOCKS: &str = "fn f(alpha: u32) -> u32 {\n    { alpha; }\n    let mut beta = 1;\n    { beta; }\n    beta\n}\n";
    let bundle = fixture_bundle("rust");
    let replaces = [
        edit_at(BLOCKS, "{ alpha; }", 2, 0, "alpha; "),
        edit_at(BLOCKS, "{ beta; }", 5, 1, ""),
    ];
    let (updated, fresh) = updated_and_fresh(&bundle, BLOCKS, &replaces, |_| {});
    assert_eq!(updated, fresh);
}

/// The texts the edit proptests insert.
fn inserted_texts() -> impl Strategy<Value = &'static str> {
    prop::sample::select(vec![
        "",
        "a",
        "{",
        "}",
        "(",
        ")",
        "\n",
        "let x = 1;",
        "fn z() {}",
        "alpha",
    ])
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(64))]

    #[test]
    fn any_two_edits_update_to_a_fresh_build(
        ats in (0..=RUST.len(), 0..=RUST.len()),
        removed in (0usize..12, 0usize..12),
        inserted in (inserted_texts(), inserted_texts()),
    ) {
        let (first, second) = (ats.0.min(ats.1), ats.0.max(ats.1));
        let replaces = [
            (first, removed.0.min(second - first), inserted.0),
            (second, removed.1.min(RUST.len() - second), inserted.1),
        ];
        let bundle = fixture_bundle("rust");
        let (updated, fresh) = updated_and_fresh(&bundle, RUST, &replaces, |_| {});
        prop_assert_eq!(updated, fresh);
    }

    #[test]
    fn any_single_edit_updates_to_a_fresh_build(
        at in 0..=RUST.len(),
        removed in 0usize..12,
        inserted in inserted_texts(),
    ) {
        let removed = removed.min(RUST.len() - at);
        let bundle = fixture_bundle("rust");
        let (updated, fresh) =
            updated_and_fresh(&bundle, RUST, &[(at, removed, inserted)], |_| {});
        prop_assert_eq!(updated, fresh);
    }
}
