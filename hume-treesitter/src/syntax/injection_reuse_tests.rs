//! Each time a session's layers are parsed, every installed layer is
//! checked: its sites and its injected layers equal those a fresh
//! resolution finds over its tree, and an injected layer's nodes equal
//! what tree-sitter gives for its ranges against the right prior tree.
//!
//! The right prior tree is the previously installed layer of the same
//! grammar and depth whose ranges, after the session's edits, equal the
//! new layer's, with those edits applied. When no byte the edits changed
//! lies in those ranges, before them on their start rows or right after
//! them, the layer's nodes equal that edited tree's. Otherwise they equal
//! a bare parser's for the ranges against it, or against no tree without a
//! prior. A reparse of an untouched tree is no oracle: markdown.inline can
//! reparse an unedited tree to other nodes. Nor is a fresh parse:
//! tree-sitter's incremental result can differ from a fresh one even when
//! neither has an error. Those differences are recorded for printing, not
//! checked. The root's are recorded too, and a fixture checks there are
//! none.

use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use proptest::prelude::*;
use rustc_hash::FxHashMap;

use hume_editing::changeset::{ChangeSet, ChangeSetBuilder};
use hume_editing::edit::TextChange;
use hume_editing::text::BufferText;
use hume_engine::pipeline::BufferId;
use hume_rope::offset::CharOffset;
use test_fixtures::{require_grammars, require_helix_query, unicode};

use super::{ChainBreak, Syntax};
use crate::edits::input_edits_from_changeset;
use crate::layers::{LayerField, LayerMismatch, LayerShape, SyntaxLayer, first_layer_mismatch};
use crate::parse_worker::{MAX_INJECTION_DEPTH, ParseDone, ParseOutcome, run_parse};
use crate::registry::{GrammarBundle, OptionalQuery};
use crate::test_support::{fresh_bid, make_bundle};

type Langs = Arc<FxHashMap<String, Arc<GrammarBundle>>>;

/// Markdown with the fixture injections query, and what it injects: rust
/// with the fixture injections query (rust into macro bodies, rustdoc
/// comments as one combined markdown layer), json, and markdown.inline.
struct Grammars {
    markdown: Arc<GrammarBundle>,
    json: Arc<GrammarBundle>,
    langs: Langs,
}

fn grammars() -> Grammars {
    require_grammars(&["markdown", "markdown.inline", "rust", "json"]);
    let fixture_injections = |name: &str| {
        let path = require_helix_query(name, OptionalQuery::Injections.file_name());
        std::fs::read_to_string(path).expect("read the helix injections query")
    };
    let injecting = |name: &str, symbol: &str| {
        make_bundle(
            name,
            symbol,
            "",
            &[(OptionalQuery::Injections, &fixture_injections(name))],
        )
    };
    let markdown = injecting("markdown", "tree_sitter_markdown");
    let json = make_bundle("json", "tree_sitter_json", "", &[]);
    let langs = Arc::new(FxHashMap::from_iter([
        ("rust".to_owned(), injecting("rust", "tree_sitter_rust")),
        ("markdown-rustdoc".to_owned(), Arc::clone(&markdown)),
        ("json".to_owned(), Arc::clone(&json)),
        (
            "markdown.inline".to_owned(),
            make_bundle("markdown.inline", "tree_sitter_markdown_inline", "", &[]),
        ),
    ]));
    Grammars {
        markdown,
        json,
        langs,
    }
}

/// Prose, a block quote, a rust block whose doc comments inject markdown
/// and whose nested macros inject rust down to the nesting cap, a json
/// block inside a list item, and a json block.
const DOCUMENT: &str = "# Notes\n\
    \n\
    Some *prose* with `code`.\n\
    \n\
    > A quoted\n\
    > paragraph.\n\
    \n\
    ```rust\n\
    /// Adds *one*.\n\
    /// Then two.\n\
    fn a() {\n    let v = vec![f(g!(1)), vec![2]];\n}\n\
    ```\n\
    \n\
    More prose.\n\
    \n\
    - x\n\
    \n  ```json\n  [1]\n  ```\n\
    \n\
    ```json\n\
    {\"k\": [1, 2]}\n\
    ```\n";

/// An injected layer installed by the last parse, its tree carried
/// through every edit since.
struct PreviousLayer {
    bundle: Arc<GrammarBundle>,
    depth: u8,
    tree: tree_sitter::Tree,
}

/// The injected layers the last parse installed, and which bytes of the
/// current text the edits since then changed.
struct Previous {
    layers: Vec<PreviousLayer>,
    /// One entry per byte of the text, and one for its end: whether an
    /// edit inserted the byte, or it follows an insertion or a deletion.
    edited: Vec<bool>,
}

impl Previous {
    fn edit(&mut self, edit: &tree_sitter::InputEdit) {
        for layer in &mut self.layers {
            layer.tree.edit(edit);
        }
        self.edited.splice(
            edit.start_byte..edit.old_end_byte,
            std::iter::repeat_n(true, edit.new_end_byte - edit.start_byte),
        );
        self.edited[edit.new_end_byte] = true;
    }

    /// Whether an edited byte lies in one of `ranges`, before it on its
    /// start row, or right after it.
    fn touched(&self, ranges: &[tree_sitter::Range]) -> bool {
        ranges.iter().any(|range| {
            let row_start = range.start_byte - range.start_point.column;
            self.edited[row_start..=range.end_byte].contains(&true)
        })
    }
}

/// A buffer's text and syntax, edited and brought up to date the way the
/// editor does it. Every time the layers are brought up to date they are
/// checked as the module docs describe.
struct Session {
    grammars: Grammars,
    /// The buffer's grammar.
    root: Arc<GrammarBundle>,
    bid: BufferId,
    syntax: Syntax,
    text: BufferText,
    /// Each time the installed root differed from a fresh parse.
    root_differences: Vec<String>,
    /// Each time an injected layer differed from a fresh parse.
    fresh_differences: Vec<String>,
    /// How many injected layers of each grammar had their nodes compared
    /// with a bare parser's.
    nodes_compared: Vec<(Arc<GrammarBundle>, usize)>,
    /// The injected layers the last parse installed, edited since. `None`
    /// before the first parse and after a change the syntax never heard
    /// of, after which every layer parses afresh.
    previous: Option<Previous>,
}

/// One replacement in a change: `deleted` chars at `at`, then `inserted`.
struct Replace<'a> {
    at: CharOffset,
    deleted: usize,
    inserted: &'a str,
}

impl Session {
    /// A markdown buffer holding `source`.
    fn open(source: &str) -> Self {
        let grammars = grammars();
        let root = Arc::clone(&grammars.markdown);
        Self::open_in(grammars, root, source)
    }

    /// A buffer of `root`'s grammar holding `source`.
    fn open_in(grammars: Grammars, root: Arc<GrammarBundle>, source: &str) -> Self {
        let text = BufferText::from(source);
        let bid = fresh_bid();
        let (syntax, _full_parse) = Syntax::attach(Arc::clone(&root), bid, &text, &grammars.langs);
        let mut session = Self {
            grammars,
            root,
            bid,
            syntax,
            text,
            root_differences: Vec::new(),
            fresh_differences: Vec::new(),
            nodes_compared: Vec::new(),
            previous: None,
        };
        assert!(session.ensure_current().is_none());
        session
    }

    /// The char offset of the `nth` occurrence of `needle`, counting from 0.
    fn find(&self, needle: &str, nth: usize) -> CharOffset {
        let source = self.text.to_string();
        let (byte, _) = source
            .match_indices(needle)
            .nth(nth)
            .unwrap_or_else(|| panic!("{needle:?} occurs {} times", nth + 1));
        CharOffset::new(self.text.rope().byte_to_char(byte))
    }

    /// The text `replaces`, in ascending position, make of the current one.
    fn replaced(&self, replaces: &[Replace<'_>]) -> (BufferText, ChangeSet) {
        let mut builder = ChangeSetBuilder::new(self.text.end());
        for replace in replaces {
            builder.retain_to(replace.at);
            builder.delete_to(replace.at.shift(replace.deleted as isize));
            builder.insert(replace.inserted);
        }
        let change = builder.finish();
        let next = change.apply(&self.text).expect("the change applies");
        (next, change)
    }

    /// Apply and record `replaces` as one text change.
    fn change(&mut self, replaces: &[Replace<'_>]) {
        let (next, change) = self.replaced(replaces);
        self.syntax
            .record_edit(&TextChange::new(&self.text, &next, &change));
        if let Some(previous) = &mut self.previous {
            for edit in input_edits_from_changeset(&change, self.text.rope()) {
                previous.edit(&edit);
            }
        }
        self.text = next;
    }

    /// Apply `replaces` without recording them, as a mutation the syntax
    /// never hears of.
    fn change_unrecorded(&mut self, replaces: &[Replace<'_>]) {
        self.text = self.replaced(replaces).0;
        self.previous = None;
    }

    /// One frame: bake the recorded edits and drop the posted request.
    fn bake(&mut self) {
        let outcome = self
            .syntax
            .frame_tick(self.bid, &self.text, &self.grammars.langs);
        assert!(outcome.chain_break.is_none());
    }

    /// Install a failed parse for the current text.
    fn fail_parse(&mut self) {
        let generation = self.text.generation();
        self.syntax.install(
            ParseDone {
                bid: self.bid,
                generation,
                bundle: Arc::clone(&self.root),
                outcome: ParseOutcome::ParseFailed,
            },
            generation,
        );
    }

    /// Bring the layers up to date and check them, unless they already
    /// were: layers that are current were checked when installed, and
    /// nothing is parsed.
    fn ensure_current(&mut self) -> Option<ChainBreak> {
        let parses = !self.syntax.is_current(self.text.generation());
        let chain_break = self
            .syntax
            .ensure_current(self.bid, &self.text, &self.grammars.langs);
        if parses {
            self.check_layers();
        }
        chain_break
    }

    /// Check every installed layer as the module docs describe, then keep
    /// the injected layers as the previous ones.
    fn check_layers(&mut self) {
        let rope = self.text.rope();
        let layers = self
            .syntax
            .current_layers(self.text.generation())
            .expect("the layers describe the text");
        let installed = layers.shapes();
        let mut children_seen = 0;
        for (index, layer) in layers.layers.iter().enumerate() {
            let (sites, injected) = layer.injections_resolved_afresh(rope, &self.grammars.langs);
            assert_eq!(
                layer.sites,
                sites,
                "sites of layer {index} in {:?}",
                self.text.to_string()
            );
            let expected: Vec<LayerShape> = injected
                .iter()
                .filter(|shape| shape.depth == layer.depth + 1)
                .map(identity)
                .collect();
            let actual = installed_children(&installed, index);
            children_seen += actual.len();
            assert_eq!(
                first_layer_mismatch(&expected, &actual),
                None,
                "injections of layer {index} in {:?}",
                self.text.to_string()
            );

            let nodes = |tree: &tree_sitter::Tree| {
                LayerShape::of(tree, &layer.bundle, layer.depth, &layer.ranges).nodes
            };
            let fresh_difference =
                (nodes(&parse_ranges(layer, rope, None)) != installed[index].nodes).then(|| {
                    format!(
                        "layer {index} differs from a fresh parse in {:?}",
                        self.text.to_string()
                    )
                });
            if index == 0 {
                self.root_differences.extend(fresh_difference);
                continue;
            }
            self.fresh_differences.extend(fresh_difference);

            let previous = self.previous.as_ref();
            let prior = previous.and_then(|previous| {
                previous.layers.iter().find(|prior| {
                    Arc::ptr_eq(&prior.bundle, &layer.bundle)
                        && prior.depth == layer.depth
                        && prior.tree.included_ranges() == layer.ranges
                })
            });
            let (expected, against) = match (previous, prior) {
                (Some(previous), Some(prior)) if !previous.touched(&layer.ranges) => {
                    (nodes(&prior.tree), "its edited prior tree")
                }
                (_, Some(prior)) => (
                    nodes(&parse_ranges(layer, rope, Some(&prior.tree))),
                    "a parse against its prior tree",
                ),
                (_, None) => (nodes(&parse_ranges(layer, rope, None)), "a fresh parse"),
            };
            assert!(
                expected == installed[index].nodes,
                "layer {index} differs from {against} in {:?}",
                self.text.to_string()
            );
            match self
                .nodes_compared
                .iter_mut()
                .find(|(bundle, _)| Arc::ptr_eq(bundle, &layer.bundle))
            {
                Some((_, count)) => *count += 1,
                None => self.nodes_compared.push((Arc::clone(&layer.bundle), 1)),
            }
        }
        assert_eq!(
            children_seen,
            installed.len() - 1,
            "every injected layer is installed as the injection of one layer"
        );
        self.previous = Some(Previous {
            layers: layers.layers[1..]
                .iter()
                .map(|layer| PreviousLayer {
                    bundle: Arc::clone(&layer.bundle),
                    depth: layer.depth,
                    tree: layer.tree.clone(),
                })
                .collect(),
            edited: vec![false; self.text.len_bytes() + 1],
        });
    }
}

impl Session {
    /// How many injected layers of `bundle` had their nodes compared.
    fn compared(&self, bundle: &Arc<GrammarBundle>) -> usize {
        self.nodes_compared
            .iter()
            .find(|(compared, _)| Arc::ptr_eq(compared, bundle))
            .map_or(0, |&(_, count)| count)
    }

    /// Panics unless an injected layer of every injected language had its
    /// nodes compared with a bare parser's.
    fn assert_every_language_compared(&self) {
        for (name, bundle) in self.grammars.langs.iter() {
            assert!(self.compared(bundle) > 0, "no {name} layer was compared");
        }
    }

    fn print_fresh_differences(&self) {
        for difference in &self.fresh_differences {
            eprintln!("fresh parse: {difference}");
        }
    }
}

/// `shape` without its nodes, for comparing where and what a layer is.
fn identity(shape: &LayerShape) -> LayerShape {
    LayerShape {
        nodes: Vec::new(),
        ..shape.clone()
    }
}

/// The identities of the layers installed as direct injections of layer
/// `parent`. Each injected layer's own injections are installed right
/// before it, so they are the run of deeper layers ending there; the
/// root's are every layer at depth 1.
fn installed_children(installed: &[LayerShape], parent: usize) -> Vec<LayerShape> {
    let depth = installed[parent].depth;
    let descendants = if parent == 0 {
        &installed[1..]
    } else {
        let first = installed[..parent]
            .iter()
            .rposition(|shape| shape.depth <= depth)
            .map_or(0, |before| before + 1);
        &installed[first..parent]
    };
    descendants
        .iter()
        .filter(|shape| shape.depth == depth + 1)
        .map(identity)
        .collect()
}

/// `layer`'s ranges parsed with its grammar against `old_tree`.
fn parse_ranges(
    layer: &SyntaxLayer,
    rope: &ropey::Rope,
    old_tree: Option<&tree_sitter::Tree>,
) -> tree_sitter::Tree {
    let mut parser = tree_sitter::Parser::new();
    parser
        .set_language(layer.bundle.grammar.language())
        .expect("the grammar loaded");
    parser
        .set_included_ranges(&layer.ranges)
        .expect("installed ranges are sorted and disjoint");
    run_parse(&mut parser, rope, old_tree, &AtomicBool::new(false))
        .expect("a parse that is never cancelled finishes")
}

/// Whether the ranges of a json layer hold `at`.
fn json_layer_at(session: &Session, at: CharOffset) -> bool {
    let byte = session.text.rope().char_to_byte(at.index());
    session
        .syntax
        .layers()
        .expect("installed")
        .layers
        .iter()
        .any(|layer| {
            Arc::ptr_eq(&layer.bundle, &session.grammars.json)
                && layer
                    .ranges
                    .iter()
                    .any(|range| range.start_byte <= byte && byte < range.end_byte)
        })
}

// ── Fixtures ─────────────────────────────────────────────────────────────

fn edit_inside_a_fence(session: &mut Session) {
    let at = session.find("    let v", 0);
    session.change(&[Replace {
        at,
        deleted: 0,
        inserted: "    b();\n",
    }]);
    session.ensure_current();
}

fn edit_before_a_fence(session: &mut Session) {
    let at = session.find("Some", 0);
    session.change(&[Replace {
        at,
        deleted: 0,
        inserted: "Two\nnew lines.\n\n",
    }]);
    session.ensure_current();
}

fn edit_in_prose(session: &mut Session) {
    let at = session.find("prose*", 0);
    session.change(&[Replace {
        at,
        deleted: 0,
        inserted: "plain ",
    }]);
    session.ensure_current();
}

fn rename_an_info_string(session: &mut Session) {
    let at = session.find("rust", 0);
    session.change(&[Replace {
        at,
        deleted: "rust".len(),
        inserted: "json",
    }]);
    session.ensure_current();
    let body = session.find("fn a()", 0);
    assert!(
        json_layer_at(session, body),
        "the renamed block is parsed as json"
    );
}

fn add_and_remove_a_fence(session: &mut Session) {
    const FENCE: &str = "```rust\nfn z() { z!(1) }\n```\n\n";
    let at = session.find("More prose", 0);
    session.change(&[Replace {
        at,
        deleted: 0,
        inserted: FENCE,
    }]);
    session.ensure_current();
    session.change(&[Replace {
        at,
        deleted: FENCE.len(),
        inserted: "",
    }]);
    session.ensure_current();
}

fn open_a_stray_fence(session: &mut Session) {
    let at = session.find("More prose", 0);
    session.change(&[Replace {
        at,
        deleted: 0,
        inserted: "```\n",
    }]);
    session.ensure_current();
}

fn delete_a_closing_fence(session: &mut Session) {
    let at = session.find("```\n\nMore", 0);
    session.change(&[Replace {
        at,
        deleted: "```\n".len(),
        inserted: "",
    }]);
    session.ensure_current();
}

fn change_two_fences_at_once(session: &mut Session) {
    let in_rust = session.find("vec![2]", 0);
    let in_json = session.find("2]}", 0);
    session.change(&[
        Replace {
            at: in_rust,
            deleted: "vec".len(),
            inserted: "foo",
        },
        Replace {
            at: in_json,
            deleted: 0,
            inserted: "3, ",
        },
    ]);
    session.ensure_current();
}

fn bake_several_times_before_parsing(session: &mut Session) {
    for (needle, inserted) in [("let v", "x"), ("More", "\n\n"), ("\"k\"", "[")] {
        let at = session.find(needle, 0);
        session.change(&[Replace {
            at,
            deleted: 0,
            inserted,
        }]);
        session.bake();
    }
    session.ensure_current();
}

fn parse_again_after_a_failed_parse(session: &mut Session) {
    let at = session.find("    let v", 0);
    session.change(&[Replace {
        at,
        deleted: 0,
        inserted: "    b();\n",
    }]);
    session.bake();
    session.fail_parse();
    session.ensure_current();
}

fn retry_after_a_failed_parse(session: &mut Session) {
    let at = session.find("g!(1)", 0);
    session.change(&[Replace {
        at,
        deleted: 0,
        inserted: "1 + ",
    }]);
    session.bake();
    session.fail_parse();
    let at = session.find("More", 0);
    session.change(&[Replace {
        at,
        deleted: 0,
        inserted: "Much ",
    }]);
    session.ensure_current();
}

fn break_the_edit_chain(session: &mut Session) {
    let at = session.find("fn a", 0);
    session.change_unrecorded(&[Replace {
        at,
        deleted: 0,
        inserted: "struct S;\n",
    }]);
    let at = session.find("More", 0);
    session.change(&[Replace {
        at,
        deleted: 0,
        inserted: "Even ",
    }]);
    assert!(
        session.ensure_current().is_some(),
        "an unrecorded mutation breaks the chain"
    );
}

/// Edits a session from [`DOCUMENT`] and checks it along the way, and
/// that every root it installed equals a fresh parse.
type Fixture = fn(&mut Session);

fn run(fixture: Fixture) {
    let mut session = Session::open(DOCUMENT);
    fixture(&mut session);
    session.assert_every_language_compared();
    session.print_fresh_differences();
    assert_eq!(
        session.root_differences,
        Vec::<String>::new(),
        "every installed root equals a fresh parse"
    );
}

#[test]
fn injected_layers_hold_after_an_edit_inside_a_fence() {
    run(edit_inside_a_fence);
}

#[test]
fn injected_layers_hold_after_an_edit_shifts_a_fence_down() {
    run(edit_before_a_fence);
}

#[test]
fn injected_layers_hold_after_an_edit_in_prose() {
    run(edit_in_prose);
}

#[test]
fn injected_layers_hold_after_an_info_string_names_another_language() {
    run(rename_an_info_string);
}

#[test]
fn injected_layers_hold_after_a_fence_is_added_and_removed() {
    run(add_and_remove_a_fence);
}

#[test]
fn injected_layers_hold_after_a_stray_fence_opener() {
    run(open_a_stray_fence);
}

#[test]
fn injected_layers_hold_after_a_closing_fence_is_deleted() {
    run(delete_a_closing_fence);
}

#[test]
fn injected_layers_hold_after_one_change_edits_two_fences() {
    run(change_two_fences_at_once);
}

#[test]
fn injected_layers_hold_after_several_bakes_whose_requests_were_dropped() {
    run(bake_several_times_before_parsing);
}

#[test]
fn injected_layers_hold_after_a_failed_parse_is_retried() {
    run(retry_after_a_failed_parse);
}

#[test]
fn injected_layers_hold_after_a_broken_edit_chain() {
    run(break_the_edit_chain);
}

#[test]
fn injected_layers_hold_after_a_failed_parse_and_a_parse_of_the_same_text() {
    run(parse_again_after_a_failed_parse);
}

#[test]
fn the_document_injects_down_to_the_nesting_cap() {
    // The rust block (depth 1) holds `vec![..]`, whose body holds the
    // call `f(g!(1))` (depth 2), whose macro body holds `1` (depth 3).
    let session = Session::open(DOCUMENT);
    let depths: Vec<u8> = session
        .syntax
        .layers()
        .expect("installed")
        .layers
        .iter()
        .map(|layer| layer.depth)
        .collect();
    assert_eq!(
        depths.iter().max(),
        Some(&MAX_INJECTION_DEPTH),
        "{depths:?}"
    );
}

#[test]
fn the_document_injects_combined_split_and_indented_layers() {
    let session = Session::open(DOCUMENT);
    let layers: Vec<(&str, u8, Vec<&str>, usize)> = session
        .syntax
        .layers()
        .expect("installed")
        .layers
        .iter()
        .map(|layer| {
            let name = session
                .grammars
                .langs
                .iter()
                .find(|(_, bundle)| Arc::ptr_eq(bundle, &layer.bundle))
                .map_or("", |(name, _)| name.as_str());
            let texts = layer
                .ranges
                .iter()
                .map(|range| &DOCUMENT[range.start_byte..range.end_byte])
                .collect();
            let column = layer
                .ranges
                .first()
                .map_or(0, |range| range.start_point.column);
            (name, layer.depth, texts, column)
        })
        .collect();
    // The two doc comments, after their `///` markers, form one layer.
    assert!(
        layers.contains(&(
            "markdown-rustdoc",
            2,
            vec![" Adds *one*.\n", " Then two.\n"],
            3
        )),
        "{layers:?}"
    );
    // The `[`, `,`, `!` and `]` of `vec![f(g!(1)), vec![2]]` split its body.
    assert!(
        layers.contains(&("rust", 2, vec!["f(g!(1))", " vec", "[2]"], 17)),
        "{layers:?}"
    );
    // The quoted paragraph starts after `> `.
    assert!(
        layers.contains(&("markdown.inline", 1, vec!["A quoted\n> paragraph."], 2)),
        "{layers:?}"
    );
}

#[test]
fn layer_comparison_names_the_first_layer_and_field_that_differ() {
    // An equal-length replacement keeps every range in place, so only the
    // json block's nodes differ, and that block is the first injection.
    let mut session = Session::open("```json\n{\"k\": [1, 2]}\n```\n\nText.\n");
    let shapes = |session: &Session| {
        session
            .syntax
            .injected_shapes_resolved_afresh(&session.text, &session.grammars.langs)
    };
    let array = shapes(&session);
    let at = session.find("[1, 2]", 0);
    session.change(&[Replace {
        at,
        deleted: "[1, 2]".len(),
        inserted: "\"1, 2\"",
    }]);
    session.ensure_current();
    assert_eq!(
        first_layer_mismatch(&array, &shapes(&session)),
        Some(LayerMismatch {
            index: 0,
            field: LayerField::Nodes,
        })
    );
}

// ── Reuse of baked layers ────────────────────────────────────────────────

/// The layers of `bundle` installed in `session`.
fn layers_of<'s>(
    session: &'s Session,
    bundle: &Arc<GrammarBundle>,
) -> impl Iterator<Item = &'s SyntaxLayer> {
    session
        .syntax
        .layers()
        .expect("installed")
        .layers
        .iter()
        .filter(move |layer| Arc::ptr_eq(&layer.bundle, bundle))
}

#[test]
fn an_edit_in_a_fence_reparses_it_against_its_baked_tree() {
    let mut session = Session::open("```rust\nfn a() {}\nfn b() {}\n```\n");
    let rust = Arc::clone(&session.grammars.langs["rust"]);
    let first_item = |session: &Session| {
        let layer = layers_of(session, &rust).next().expect("a rust layer");
        let item = layer
            .tree
            .root_node()
            .named_child(0)
            .expect("the fence holds items");
        assert_eq!(item.kind(), "function_item");
        item.id()
    };
    let before = first_item(&session);
    let at = session.find("b()", 0);
    session.change(&[Replace {
        at,
        deleted: 0,
        inserted: "b",
    }]);
    session.ensure_current();
    assert_eq!(
        first_item(&session),
        before,
        "`fn a` is reused from the baked tree"
    );
}

#[test]
fn an_untouched_fence_keeps_its_baked_tree() {
    let mut session = Session::open("Text.\n\n```rust\nfn a() {}\n```\n");
    let rust = Arc::clone(&session.grammars.langs["rust"]);
    let at = session.find("Text", 0);
    session.change(&[Replace {
        at,
        deleted: 0,
        inserted: "More\n\n",
    }]);
    session.ensure_current();
    let fence = layers_of(&session, &rust).next().expect("a rust layer");
    // An edit before a tree marks it changed until a parse clears that.
    assert!(
        fence.tree.root_node().has_changes(),
        "the fence's tree is the baked one, not reparsed"
    );
}

/// The doc comments around `fn f` form one combined layer whose ranges an
/// edit inside `fn f` leaves untouched, though it lies between them: the
/// layer keeps its baked tree, and its sites are queried again over the
/// edited bytes, which the session checks against a fresh resolution.
#[test]
fn a_combined_layer_keeps_its_tree_after_an_edit_between_its_ranges() {
    let mut session = Session::open("```rust\n/// One.\nfn f() {}\n/// Two.\nfn g() {}\n```\n");
    let rustdoc = Arc::clone(&session.grammars.langs["markdown-rustdoc"]);
    let at = session.find("f()", 0);
    session.change(&[Replace {
        at,
        deleted: 0,
        inserted: "x",
    }]);
    session.ensure_current();
    let layer = layers_of(&session, &rustdoc)
        .find(|layer| layer.depth == 2)
        .expect("the doc comments' layer");
    assert_eq!(layer.ranges.len(), 2, "both doc comments");
    // An edit inside a tree's span marks it changed until a parse clears that.
    assert!(
        layer.tree.root_node().has_changes(),
        "the layer's tree is the baked one, not reparsed"
    );
}

#[test]
fn a_rebound_language_name_parses_the_fence_fresh() {
    let mut session = Session::open("```rust\nfn a() {}\n```\n\nText.\n");
    let mut langs = (*session.grammars.langs).clone();
    langs.insert("rust".to_owned(), Arc::clone(&session.grammars.json));
    session.grammars.langs = Arc::new(langs);
    // After the fence, so the fence's ranges stay those of its baked layer.
    let at = session.find("Text", 0);
    session.change(&[Replace {
        at,
        deleted: 0,
        inserted: "More ",
    }]);
    session.ensure_current();
    let body = session.find("fn a()", 0);
    assert!(
        json_layer_at(&session, body),
        "the fence is parsed with the grammar its name now gives"
    );
}

#[test]
fn a_combined_injection_gaining_a_range_parses_it_fresh() {
    let grammars = grammars();
    let markdown = Arc::clone(&grammars.markdown);
    let doc_comments = make_bundle(
        "rust",
        "tree_sitter_rust",
        "",
        &[(
            OptionalQuery::Injections,
            r#"((line_comment doc: (_)) @injection.content
                (#set! injection.language "markdown-rustdoc")
                (#set! injection.combined))"#,
        )],
    );
    let mut session = Session::open_in(
        grammars,
        doc_comments,
        "impl S {\n    /// One.\n    // Two.\n    fn f() {}\n}\n",
    );
    let ranges = |session: &Session| {
        layers_of(session, &markdown)
            .map(|layer| layer.ranges.len())
            .collect::<Vec<_>>()
    };
    assert_eq!(ranges(&session), vec![1]);
    let at = session.find("// Two", 0);
    session.change(&[Replace {
        at,
        deleted: 0,
        inserted: "/",
    }]);
    session.ensure_current();
    assert_eq!(
        ranges(&session),
        vec![2],
        "both doc comments form one layer"
    );
}

// ── Random edits ─────────────────────────────────────────────────────────

/// Text that opens, closes or fills a code block, a doc comment, a block
/// quote or a list item, inserted besides the shared unicode samples.
const CODE_ATOMS: [&str; 7] = ["}", "```rust\n", "\n", "fn x() {}", "/// ", "> ", "- "];

fn atom(index: usize) -> &'static str {
    unicode::ALL
        .get(index)
        .copied()
        .unwrap_or_else(|| CODE_ATOMS[index - unicode::ALL.len()])
}

/// An edit at a cluster boundary: `deleted` clusters, at most up to the
/// structural final newline, replaced by the atom `inserted`. `settle`
/// brings the layers up to date after it; otherwise the edit is only
/// baked.
#[derive(Clone, Debug)]
struct RandomEdit {
    at: usize,
    deleted: usize,
    inserted: Option<usize>,
    settle: bool,
}

fn random_edit() -> impl Strategy<Value = RandomEdit> {
    (
        any::<usize>(),
        0..=3usize,
        proptest::option::of(0..unicode::ALL.len() + CODE_ATOMS.len()),
        any::<bool>(),
    )
        .prop_map(|(at, deleted, inserted, settle)| RandomEdit {
            at,
            deleted,
            inserted,
            settle,
        })
}

impl Session {
    fn random_edit(&mut self, edit: &RandomEdit) {
        let text = &self.text;
        let final_newline = text.last_char();
        let start = text.snap(CharOffset::new(edit.at % (final_newline.index() + 1)));
        let mut end = start;
        for _ in 0..edit.deleted {
            if end.offset() < final_newline {
                end = text
                    .clusters()
                    .next(end)
                    .expect("a cluster before the final newline has a successor");
            }
        }
        let deleted = end.offset().chars_since(start.offset());
        let inserted = edit.inserted.map_or("", atom);
        if deleted == 0 && inserted.is_empty() {
            return;
        }
        self.change(&[Replace {
            at: start.offset(),
            deleted,
            inserted,
        }]);
        if edit.settle {
            self.ensure_current();
        } else {
            self.bake();
        }
    }
}

/// The last edit types after `*.` in a doc comment, where tree-sitter
/// can reparse the markdown.inline layer to other nodes than a fresh parse
/// of its ranges gives. The parse after it checks that layer against its
/// prior tree.
#[test]
fn injected_layers_hold_after_typing_after_emphasis_in_a_doc_comment() {
    let edits = [
        RandomEdit {
            at: 6675352138154465184,
            deleted: 1,
            inserted: Some(8),
            settle: false,
        },
        RandomEdit {
            at: 4618154635477059019,
            deleted: 3,
            inserted: None,
            settle: false,
        },
        RandomEdit {
            at: 4409006695647663062,
            deleted: 1,
            inserted: None,
            settle: true,
        },
        RandomEdit {
            at: 418560009592890608,
            deleted: 0,
            inserted: Some(0),
            settle: false,
        },
    ];
    let mut session = Session::open(DOCUMENT);
    for edit in &edits {
        session.random_edit(edit);
    }
    let inline = Arc::clone(&session.grammars.langs["markdown.inline"]);
    let before = session.compared(&inline);
    session.ensure_current();
    assert!(
        session.compared(&inline) > before,
        "the last parse checks the markdown.inline layers"
    );
}

proptest! {
    #[test]
    fn injected_layers_hold_after_random_edits(
        edits in proptest::collection::vec(random_edit(), 1..=8),
    ) {
        let mut session = Session::open(DOCUMENT);
        for edit in &edits {
            session.random_edit(edit);
        }
        session.ensure_current();
        session.print_fresh_differences();
    }
}
