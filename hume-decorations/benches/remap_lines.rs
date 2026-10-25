use std::hint::black_box;

use criterion::{BatchSize, Criterion, criterion_group, criterion_main};
use hume_decorations::decorations::{DecorationStores, SignEntry};
use hume_editing::changeset::{ChangeSet, ChangeSetBuilder};
use hume_editing::edit::TextChange;
use hume_editing::text::BufferText;
use hume_engine::pipeline::{BufferId, EngineView};
use hume_engine::theme::Theme;
use hume_engine::types::ScopeId;
use hume_rope::line::ContentLine;
use hume_rope::offset::CharOffset;

const LINES: usize = 50_000;
const LINE: &str = "    let value = compute(left, right, 42);\n";

fn buffer_id() -> BufferId {
    let (mut ev, _) = EngineView::new(Theme::default(), Default::default());
    ev.buffers.insert(())
}

/// A sign on every `every`-th line.
fn signs(every: usize) -> Vec<SignEntry> {
    (0..LINES)
        .step_by(every)
        .map(|line| SignEntry {
            line: ContentLine::new(line),
            text: "+".into(),
            scope: ScopeId(0),
        })
        .collect()
}

/// `inserted` typed at column 4 of each of `lines`, in ascending order.
fn typed_at(
    text: &BufferText,
    lines: impl IntoIterator<Item = usize>,
    inserted: &str,
) -> ChangeSet {
    let mut b = ChangeSetBuilder::new(CharOffset::new(text.len_chars()));
    for line in lines {
        b.retain_to(CharOffset::new(line * LINE.len() + 4));
        b.insert(inserted);
    }
    b.finish()
}

fn bench(c: &mut Criterion, name: &str, every: usize, cs: impl Fn(&BufferText) -> ChangeSet) {
    let before = BufferText::from(LINE.repeat(LINES).as_str());
    let cs = cs(&before);
    let after = cs.apply(&before).expect("the change starts from `before`");
    let change = TextChange::new(&before, &after, &cs);
    let bid = buffer_id();
    c.bench_function(name, |b| {
        b.iter_batched(
            || {
                let mut store = DecorationStores::default();
                store.set_signs("git".into(), bid, signs(every));
                store
            },
            |mut store| {
                store.remap_through(bid, &change);
                black_box(store)
            },
            BatchSize::LargeInput,
        );
    });
}

fn remap_lines(c: &mut Criterion) {
    bench(c, "insert_char_10k", 5, |t| typed_at(t, [LINES / 2], "x"));
    bench(c, "insert_newline_10k", 5, |t| {
        typed_at(t, [LINES / 2], "\n")
    });
    bench(c, "insert_char_100", 500, |t| typed_at(t, [LINES / 2], "x"));
    bench(c, "multi_cursor_100_10k", 5, |t| {
        typed_at(t, (0..LINES).step_by(LINES / 100), "x")
    });
}

criterion_group!(benches, remap_lines);
criterion_main!(benches);
