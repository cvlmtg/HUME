//! Edits and selection-history walks with a full selection history: one
//! pane on a 2,000-line buffer, 50 cursors, and 20 saved entries of 50
//! selections each.

use std::hint::black_box;

use criterion::{Criterion, criterion_group, criterion_main};
use hume_editor::testing::SelectionHistoryBench;

const LINES: usize = 2_000;
const CURSORS: usize = 50;

fn fixture(dir: &tempfile::TempDir) -> SelectionHistoryBench {
    let file = dir.path().join("lines.txt");
    let text: String = (0..LINES)
        .map(|n| format!("let value_{n} = compute({n});\n"))
        .collect();
    std::fs::write(&file, text).expect("the bench file is written");
    let bench = SelectionHistoryBench::new(&file, CURSORS);
    assert_eq!(bench.selection_count(), CURSORS);
    bench
}

fn bench_selection_history(c: &mut Criterion) {
    let dir = tempfile::tempdir().expect("a temp dir");
    let mut group = c.benchmark_group("selection_history");
    group.sample_size(10);

    let mut bench = fixture(&dir);
    let edit_pair = SelectionHistoryBench::keys("du");
    group.bench_function("edit_pair", |b| {
        b.iter(|| bench.press(black_box(&edit_pair)));
    });

    let mut bench = fixture(&dir);
    bench.press(&SelectionHistoryBench::keys("i"));
    let keystroke = SelectionHistoryBench::keys("x<backspace>");
    group.bench_function("insert_keystroke", |b| {
        b.iter(|| bench.press(black_box(&keystroke)));
    });

    let mut bench = fixture(&dir);
    let burst = SelectionHistoryBench::keys(&format!("{}mumU", "du".repeat(10)));
    group.bench_function("edit_burst_then_walk", |b| {
        b.iter(|| bench.press(black_box(&burst)));
    });

    group.finish();
}

criterion_group!(benches, bench_selection_history);
criterion_main!(benches);
