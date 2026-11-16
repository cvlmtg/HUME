//! What one `core:markdown-preview` render costs on the main thread: the
//! `on-viewport-change` hook its viewport debounce fires, alone and with the
//! frame drawn after it. Each runs on prose and on a table taller than the
//! pane, which a render that shows one of its rows draws whole, at a small
//! and a full-screen pane.

use criterion::{Criterion, criterion_group, criterion_main};
use hume_editor::testing::{MarkdownPreviewBench, PreviewSample, TALL_TABLE_ROWS};

const SAMPLES: [(&str, PreviewSample); 2] = [
    ("prose", PreviewSample::Prose),
    ("tall_table", PreviewSample::TallTable),
];

const SIZES: [(u16, u16); 2] = [(80, 25), (200, 60)];

/// Pipes on each row of the tall table, each one conceal.
const PIPES_PER_ROW: usize = 4;

fn fixture(
    root: &std::path::Path,
    sample: PreviewSample,
    size: (u16, u16),
) -> MarkdownPreviewBench {
    let bench = MarkdownPreviewBench::new(root, sample, size.0, size.1);
    if let PreviewSample::TallTable = sample {
        assert!(
            bench.conceal_count() >= PIPES_PER_ROW * TALL_TABLE_ROWS,
            "the render drew {} conceals, not the whole table",
            bench.conceal_count()
        );
    }
    bench
}

fn bench_markdown_preview(c: &mut Criterion) {
    test_fixtures::require_grammars(&["markdown", "markdown.inline"]);
    let root = tempfile::tempdir().expect("a temp dir");
    for name in ["markdown", "markdown.inline"] {
        test_fixtures::install_fixture_grammar(&root.path().join("data/grammars"), name, "1");
    }
    let mut group = c.benchmark_group("markdown_preview");

    for (sample_name, sample) in SAMPLES {
        for size in SIZES {
            let id = format!("{sample_name}/{}x{}", size.0, size.1);

            let mut bench = fixture(root.path(), sample, size);
            group.bench_function(format!("render/{id}"), |b| b.iter(|| bench.render()));

            let mut bench = fixture(root.path(), sample, size);
            group.bench_function(format!("render_then_frame/{id}"), |b| {
                b.iter(|| bench.render_then_frame());
            });
        }
    }

    group.finish();
}

criterion_group!(benches, bench_markdown_preview);
criterion_main!(benches);
