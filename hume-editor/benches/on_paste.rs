//! What a terminal paste and its undo cost with `on-paste` handlers: none,
//! handlers that change nothing, and `core:markdown`'s link paste, at one
//! selection and at 200, for a short value and a 10 KB one pasted over every
//! selection. The undo puts the buffer back for the next sample.

use std::hint::black_box;

use criterion::{Criterion, criterion_group, criterion_main};
use hume_editor::testing::{OnPasteBench, PasteHandlers};

const SELECTIONS: [usize; 2] = [1, 200];

const SHORT: &str = "pasted";

const URL: &str = "https://example.com";

fn fixture(handlers: PasteHandlers, selections: usize) -> (OnPasteBench, tempfile::TempDir) {
    let root = tempfile::tempdir().expect("a temp dir");
    for name in ["markdown", "markdown.inline"] {
        test_fixtures::install_fixture_grammar(&root.path().join("data/grammars"), name, "1");
    }
    let mut bench = OnPasteBench::new(root.path(), handlers, selections);
    assert_eq!(bench.selection_count(), selections);
    let expected = match handlers {
        PasteHandlers::Markdown => format!("[word0]({URL}) and the rest of line 0"),
        _ => format!("{URL} and the rest of line 0"),
    };
    assert_eq!(bench.first_line_pasting(URL), expected);
    (bench, root)
}

fn bench_on_paste(c: &mut Criterion) {
    test_fixtures::require_grammars(&["markdown", "markdown.inline"]);
    let long = "x".repeat(10 * 1024);
    let cases = [
        ("none/short", PasteHandlers::None, SHORT),
        ("one_unchanged/short", PasteHandlers::Unchanged(1), SHORT),
        ("three_unchanged/short", PasteHandlers::Unchanged(3), SHORT),
        ("markdown/url", PasteHandlers::Markdown, URL),
        ("none/10k", PasteHandlers::None, long.as_str()),
        (
            "one_unchanged/10k",
            PasteHandlers::Unchanged(1),
            long.as_str(),
        ),
    ];
    let mut group = c.benchmark_group("on_paste");
    group.sample_size(20);
    for selections in SELECTIONS {
        for (name, handlers, text) in cases {
            let (mut bench, _root) = fixture(handlers, selections);
            group.bench_function(format!("{name}/{selections}"), |b| {
                b.iter(|| bench.paste_then_undo(black_box(text)));
            });
        }
    }
    group.finish();
}

criterion_group!(benches, bench_on_paste);
criterion_main!(benches);
