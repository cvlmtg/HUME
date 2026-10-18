use std::hint::black_box;

use criterion::{BatchSize, BenchmarkId, Criterion, criterion_group, criterion_main};
use hume_editing::edit::Edited;
use hume_editing::selection::Selection;
use hume_editing::state::EditState;
use hume_editing::text::BufferText;
use hume_editing::word::WordChars;
use hume_ops::edit::{Stepping, apply_increment_plan, plan_increment};
use hume_rope::offset::CharOffset;

const RECORD: &str = r#"{"id":1234,"v":-56},"#;

/// One minified JSON line of `records` records, with a cursor on the first
/// char of each of the first `cursors` records.
fn minified_line(records: usize, cursors: usize) -> EditState {
    let mut line = String::from("[");
    line.push_str(&RECORD.repeat(records));
    line.push_str("]\n");
    let text = BufferText::from(line.as_str());
    let selections = (0..cursors)
        .map(|n| Selection::cursor(text.snap(CharOffset::new(1 + n * RECORD.len()))))
        .collect();
    EditState::at_text_start(text).with_selections(selections, 0)
}

fn step(state: EditState, stepping: Stepping) -> Edited {
    let plan = plan_increment(state.view(), 1, stepping, WordChars::default())
        .expect("every cursor reaches a number");
    apply_increment_plan(state, plan)
}

/// One cursor at the start of a single line of growing length.
fn bench_one_cursor_long_line(c: &mut Criterion) {
    let mut group = c.benchmark_group("increment_one_cursor_long_line");
    group.sample_size(10);
    for bytes in [10_000, 100_000, 1_000_000, 10_000_000] {
        let state = minified_line(bytes / RECORD.len(), 1);
        group.bench_with_input(BenchmarkId::from_parameter(bytes), &state, |b, state| {
            b.iter_batched(
                || state.clone(),
                |state| black_box(step(state, Stepping::Uniform)),
                BatchSize::LargeInput,
            );
        });
    }
    group.finish();
}

/// A growing number of cursors on one 10 000-record line.
fn bench_sequence_on_one_line(c: &mut Criterion) {
    let mut group = c.benchmark_group("increment_sequence_on_one_line");
    group.sample_size(10);
    for cursors in [10, 100, 1000, 10_000] {
        let state = minified_line(10_000, cursors);
        group.bench_with_input(BenchmarkId::from_parameter(cursors), &state, |b, state| {
            b.iter_batched(
                || state.clone(),
                |state| black_box(step(state, Stepping::Sequence)),
                BatchSize::LargeInput,
            );
        });
    }
    group.finish();
}

criterion_group!(
    benches,
    bench_one_cursor_long_line,
    bench_sequence_on_one_line
);
criterion_main!(benches);
