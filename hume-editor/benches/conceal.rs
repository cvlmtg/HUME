//! What a concealed Markdown buffer adds to a keystroke: the conceal mirror
//! rebuild, the layout key over many selections, a reveal-all `j` and the
//! formatter, on a 5,000-line sample with soft wrap. Each runs with the
//! conceals of the first viewport and with those of every line; the layout
//! key, `j` and the formatter run once more with no conceals, so the
//! difference is what the conceals cost.

use criterion::{Criterion, criterion_group, criterion_main};
use hume_editor::testing::{ConcealBench, StoreSize};

const LINES: usize = 5_000;

const SIZES: [(&str, Option<StoreSize>); 3] = [
    ("viewport", Some(StoreSize::Viewport)),
    ("all_lines", Some(StoreSize::AllLines)),
    ("no_conceals", None),
];

/// The layout key's rows: its id suffix, the pattern whose matches are
/// selected (the one cursor of a fresh editor when `None`) and the fewest
/// selections that selects. The last is a stress case.
const LAYOUT_KEY_SELECTIONS: [(&str, Option<&str>, usize); 3] = [
    ("1_selection", None, 1),
    ("100_selections", Some(r"Heading (0|\d*[05]0)\b"), 100),
    ("10k_selections", Some(r"\d+"), 10_000),
];

fn fixture(dir: &tempfile::TempDir, conceals: Option<StoreSize>) -> ConcealBench {
    ConcealBench::new(&dir.path().join("sample.md"), LINES, conceals)
}

fn bench_conceal(c: &mut Criterion) {
    let dir = tempfile::tempdir().expect("a temp dir");
    let mut group = c.benchmark_group("conceal");
    group.sample_size(10);

    for (name, size) in SIZES {
        if size.is_some() {
            let mut bench = fixture(&dir, size);
            group.bench_function(format!("mirror_rebuild/{name}"), |b| {
                b.iter(|| bench.mirror_rebuild());
            });
        }

        for (count, pattern, expected) in LAYOUT_KEY_SELECTIONS {
            let mut bench = fixture(&dir, size);
            if let Some(pattern) = pattern {
                let selections = bench.select_matches(pattern);
                assert!(selections >= expected, "{selections} selections");
            }
            group.bench_function(format!("layout_key_with_{count}/{name}"), |b| {
                b.iter(|| bench.layout_key());
            });
        }

        let mut bench = fixture(&dir, size);
        group.bench_function(format!("reveal_all_j_then_settle/{name}"), |b| {
            b.iter(|| bench.reveal_all_j_then_settle());
        });

        let mut bench = fixture(&dir, size);
        group.bench_function(format!("format_every_line/{name}"), |b| {
            b.iter(|| bench.format_every_line());
        });
    }

    group.finish();
}

criterion_group!(benches, bench_conceal);
criterion_main!(benches);
