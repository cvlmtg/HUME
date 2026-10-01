use std::hint::black_box;

use criterion::{BatchSize, BenchmarkId, Criterion, criterion_group, criterion_main};
use hume_editing::selection::Selection;
use hume_editing::state::EditState;
use hume_editing::text::BufferText;
use hume_ops::edit::insert_char;
use hume_rope::line::ContentLine;

const CURSOR_COUNTS: [usize; 4] = [1, 10, 100, 1000];

/// `cursors` lines of code-like text with a cursor at the start of each.
fn state_with_cursors(cursors: usize) -> EditState {
    let text: String = (0..cursors)
        .map(|n| format!("let value_{n} = compute({n});\n"))
        .collect();
    let text = BufferText::from(text.as_str());
    let selections = (0..cursors)
        .map(|line| Selection::cursor(text.lines().start(ContentLine::new(line))))
        .collect();
    EditState::at_text_start(text).with_selections(selections, 0)
}

fn bench_insert_char(c: &mut Criterion) {
    let mut group = c.benchmark_group("insert_char");
    for cursors in CURSOR_COUNTS {
        let state = state_with_cursors(cursors);
        group.bench_with_input(BenchmarkId::from_parameter(cursors), &state, |b, state| {
            b.iter_batched(
                || state.clone(),
                |state| black_box(insert_char(state, 'x')),
                BatchSize::SmallInput,
            );
        });
    }
    group.finish();
}

criterion_group!(benches, bench_insert_char);
criterion_main!(benches);
