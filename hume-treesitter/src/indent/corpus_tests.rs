use std::path::PathBuf;
use std::sync::Arc;

use rustc_hash::FxHashMap;

use hume_editing::text::BufferText;
use hume_rope::line::ContentLine;
use hume_rope::offset::ExclusiveRange;
use test_fixtures::{require_grammars, require_helix_indent_corpus, require_helix_query};

use super::*;
use crate::registry::OptionalQuery;
use crate::syntax::Syntax;
use crate::test_support::make_bundle;

/// A corpus language: its name, the corpus file's extension, its indent
/// unit (also the tab width it is checked with), the 1-based numbers of
/// the lines that start inside a multi-line string, and whether its
/// `injections.scm` is attached, so the file is checked with its injected
/// layers.
struct Corpus {
    name: &'static str,
    ext: &'static str,
    unit: u8,
    opaque_lines: &'static [usize],
    injections: bool,
}

const CORPORA: [Corpus; 4] = [
    Corpus {
        name: "rust",
        ext: "rs",
        unit: 4,
        opaque_lines: &[276, 277, 279, 280],
        injections: true,
    },
    Corpus {
        name: "json",
        ext: "json",
        unit: 2,
        opaque_lines: &[],
        injections: false,
    },
    Corpus {
        name: "python",
        ext: "py",
        unit: 4,
        opaque_lines: &[88, 89, 90],
        injections: false,
    },
    Corpus {
        name: "c",
        ext: "c",
        unit: 2,
        opaque_lines: &[],
        injections: false,
    },
];

/// The line comment tokens `runtime/scheme/languages/<name>.scm` declares.
fn line_comment_tokens(name: &str) -> Vec<String> {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../runtime/scheme/languages")
        .join(format!("{name}.scm"));
    let data = std::fs::read_to_string(&path).expect("read the language data file");
    let Some(record) = data.lines().find_map(|line| {
        line.trim_start_matches(['(', ' '])
            .strip_prefix("comment-tokens ")
    }) else {
        return Vec::new();
    };
    record
        .split('"')
        .skip(1)
        .step_by(2)
        .map(str::to_owned)
        .collect()
}

fn has_missing_node(root: tree_sitter::Node) -> bool {
    let mut cursor = root.walk();
    let mut stack = vec![root];
    while let Some(node) = stack.pop() {
        if node.is_missing() {
            return true;
        }
        stack.extend(node.children(&mut cursor));
    }
    false
}

/// Every indent corpus file re-indents to its own widths, and a line
/// break typed at the end of each line indents the next line at least as
/// far as the file does.
#[test]
fn the_indent_corpus_indents_as_written() {
    let mut failures = Vec::new();
    for corpus in &CORPORA {
        require_grammars(&[corpus.name]);
        let indents = std::fs::read_to_string(require_helix_query(
            corpus.name,
            OptionalQuery::Indents.file_name(),
        ))
        .expect("read helix-indents.scm");
        let injections = corpus.injections.then(|| {
            std::fs::read_to_string(require_helix_query(
                corpus.name,
                OptionalQuery::Injections.file_name(),
            ))
            .expect("read helix-injections.scm")
        });
        let mut queries = vec![(OptionalQuery::Indents, indents.as_str())];
        queries.extend(
            injections
                .as_deref()
                .map(|injections| (OptionalQuery::Injections, injections)),
        );
        let bundle = make_bundle(
            corpus.name,
            &format!("tree_sitter_{}", corpus.name),
            "",
            &queries,
        );
        let langs = Arc::new(FxHashMap::from_iter([(
            corpus.name.to_owned(),
            Arc::clone(&bundle),
        )]));
        let file = format!("{}.{}", corpus.name, corpus.ext);
        let source = std::fs::read_to_string(require_helix_indent_corpus(corpus.name, &file))
            .expect("read the indent corpus");
        let text = BufferText::from(source.as_str());
        let syntax = Syntax::attach_sync(Arc::clone(&bundle), &text, &langs);
        let layers = syntax.layers().expect("the corpus parses");
        if corpus.injections && layers.layers.len() < 2 {
            failures.push(format!("{file}: no injected layer was parsed"));
        }
        if has_missing_node(layers.layers[0].tree.root_node()) {
            failures.push(format!("{file}: the tree has a MISSING node"));
        }

        let tw = corpus.unit;
        let comments = line_comment_tokens(corpus.name);
        let skipped = |line: ContentLine| {
            let start = text.lines().indent_end(line);
            let end = text.lines().newline(line);
            let content = text
                .slice(ExclusiveRange::new(start.offset(), end.offset()))
                .to_string();
            start == end
                || comments
                    .iter()
                    .any(|token| content.starts_with(token.as_str()))
        };
        let width = |line: ContentLine| text.columns().indent(line, tw).1;
        let count = text.content_line_count();
        let mut checked = 0;
        let mut opaque_lines = Vec::new();
        for line in ExclusiveRange::new(ContentLine::new(0), count.end_exclusive()).iter() {
            if skipped(line) {
                continue;
            }
            checked += 1;
            let number = line.number();
            let computed = absolute_for_line(layers, &text, line, tw);
            if is_opaque_interior(layers, &text, line, tw) {
                opaque_lines.push(number);
                if computed.is_some() {
                    failures.push(format!(
                        "{file}:{number}: re-indent inside an opaque node gave {computed:?}"
                    ));
                }
            } else if computed != Some(width(line)) {
                failures.push(format!(
                    "{file}:{number}: re-indent gave {computed:?}, the file has {:?}",
                    width(line)
                ));
            }

            let next = line.advance(1);
            if next >= count.end_exclusive()
                || skipped(next)
                || is_opaque_interior(layers, &text, next, tw)
            {
                continue;
            }
            let typed = absolute_for_new_line(layers, &text, text.lines().newline(line), tw);
            if !typed.is_some_and(|typed| typed >= width(next)) {
                failures.push(format!(
                    "{file}:{number}: a line break after it gave {typed:?}, the next line has {:?}",
                    width(next)
                ));
            }
        }
        assert!(checked > 0, "{file}: no line was checked");
        if opaque_lines != corpus.opaque_lines {
            failures.push(format!(
                "{file}: lines {opaque_lines:?} start inside an opaque node, the file has {:?}",
                corpus.opaque_lines
            ));
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}
