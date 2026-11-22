//! What one `core:markdown` render costs on the main thread: the
//! `on-viewport-change` hook its viewport debounce fires, alone and with the
//! frame drawn after it. Each runs on prose and on a table taller than the
//! pane, which a render that shows one of its rows draws whole, at a small
//! and a full-screen pane.

use criterion::{Criterion, criterion_group, criterion_main};
use hume_editor::testing::{PreviewSample, markdown_preview_bench};

const SAMPLES: [(&str, PreviewSample); 2] = [
    ("prose", PreviewSample::Prose),
    ("tall_table", PreviewSample::TallTable),
];

const SIZES: [(u16, u16); 2] = [(80, 25), (200, 60)];

fn bench_markdown_preview(c: &mut Criterion) {
    let root = tempfile::tempdir().expect("a temp dir");
    test_fixtures::install_fixture_grammars(
        &root.path().join("data/grammars"),
        &["markdown", "markdown.inline"],
    );
    let mut group = c.benchmark_group("markdown_preview");

    for (sample_name, sample) in SAMPLES {
        for (width, height) in SIZES {
            let id = format!("{sample_name}/{width}x{height}");

            let mut bench = markdown_preview_bench(root.path(), sample, width, height);
            group.bench_function(format!("render/{id}"), |b| b.iter(|| bench.render()));

            let mut bench = markdown_preview_bench(root.path(), sample, width, height);
            group.bench_function(format!("render_then_frame/{id}"), |b| {
                b.iter(|| bench.render_then_frame());
            });
        }
    }

    group.finish();
}

criterion_group!(benches, bench_markdown_preview);
criterion_main!(benches);
