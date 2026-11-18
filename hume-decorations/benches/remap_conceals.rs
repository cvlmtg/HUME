//! `remap_through` at the store for the three stores a rendered Markdown
//! buffer holds (conceals, extra highlights, line backgrounds), one
//! keystroke-sized change on a 5,000-line text, at two store sizes: the
//! entries of the first viewport's lines, and of every line.

use std::hint::black_box;
use std::ops::Range;

use criterion::{BatchSize, Criterion, criterion_group, criterion_main};
use hume_core::BufferId;
use hume_decorations::decorations::{
    ConcealEntry, ConcealEntrySpan, DecorationStores, ExtraHighlightEntry, LineBgEntry,
};
use hume_editing::changeset::{ChangeSet, ChangeSetBuilder};
use hume_editing::edit::TextChange;
use hume_editing::text::BufferText;
use hume_engine::pipeline::EngineView;
use hume_engine::providers::ConcealText;
use hume_engine::theme::Theme;
use hume_engine::types::ScopeId;
use hume_rope::line::ContentLine;
use hume_rope::offset::{CharOffset, ExclusiveRange};

const LINES: usize = 5_000;
const LINE: &str = "**bold** words and *it* here\n";

/// Lines of two conceals each that match the 86 conceals `core:markdown`
/// sets for an 80x25 pane at the top of `hume-editor`'s Markdown bench sample.
const VIEWPORT_LINES: usize = 43;

fn buffer_id() -> BufferId {
    let (mut ev, _) = EngineView::new(Theme::default(), Default::default());
    ev.buffers.insert(())
}

fn at(line: usize, col: usize) -> CharOffset {
    CharOffset::new(line * LINE.len() + col)
}

/// The two `**` of each line in `lines`, hidden, with the line's reveal span
/// on every tenth line.
fn conceals(lines: Range<usize>) -> Vec<ConcealEntry> {
    let hide = |line, start, end, reveal| ConcealEntry {
        span: ConcealEntrySpan::Hidden {
            chars: ExclusiveRange::new(at(line, start), at(line, end)),
            text: Some(ConcealText {
                text: "·".into(),
                scope: None,
            }),
        },
        reveal,
    };
    lines
        .flat_map(|line| {
            let reveal =
                (line % 10 == 0).then(|| ExclusiveRange::new(at(line, 0), at(line + 1, 0)));
            [hide(line, 0, 2, reveal), hide(line, 6, 8, reveal)]
        })
        .collect()
}

/// The bold run of each line in `lines`.
fn extra_highlights(lines: Range<usize>) -> Vec<ExtraHighlightEntry> {
    lines
        .map(|line| ExtraHighlightEntry {
            start: at(line, 0),
            end: at(line, 8),
            scope: ScopeId(0),
        })
        .collect()
}

/// A tint on every tenth line of `lines`, the headings of the sample.
fn line_backgrounds(lines: Range<usize>) -> Vec<LineBgEntry> {
    lines
        .filter(|line| line % 10 == 0)
        .map(|line| LineBgEntry {
            line: ContentLine::new(line),
            scope: ScopeId(0),
        })
        .collect()
}

/// `inserted` typed at column 10 of `line`.
fn typed(text: &BufferText, line: usize, inserted: &str) -> ChangeSet {
    let mut b = ChangeSetBuilder::new(CharOffset::new(text.len_chars()));
    b.retain_to(at(line, 10));
    b.insert(inserted);
    b.finish()
}

/// One bench: a store holding `lines`' conceals, and with `all_stores` their
/// extra highlights and line backgrounds too, with `inserted` typed in the
/// middle of `lines`.
fn bench(c: &mut Criterion, name: &str, lines: Range<usize>, all_stores: bool, inserted: &str) {
    let before = BufferText::from(LINE.repeat(LINES).as_str());
    let cs = typed(&before, (lines.start + lines.end) / 2, inserted);
    let after = cs.apply(&before).expect("the change starts from `before`");
    let change = TextChange::new(&before, &after, &cs);
    let bid = buffer_id();
    c.bench_function(name, |b| {
        b.iter_batched(
            || {
                let mut store = DecorationStores::default();
                store.set_conceals("md".into(), bid, conceals(lines.clone()));
                if all_stores {
                    store.set_extra_highlights("md".into(), bid, extra_highlights(lines.clone()));
                    store.set_line_backgrounds("md".into(), bid, line_backgrounds(lines.clone()));
                }
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

fn remap_conceals(c: &mut Criterion) {
    for (size, lines) in [("viewport", 0..VIEWPORT_LINES), ("all_lines", 0..LINES)] {
        bench(
            c,
            &format!("{size}_conceals_only"),
            lines.clone(),
            false,
            "x",
        );
        bench(
            c,
            &format!("{size}_all_stores_char"),
            lines.clone(),
            true,
            "x",
        );
        bench(c, &format!("{size}_all_stores_newline"), lines, true, "\n");
    }
}

criterion_group!(benches, remap_conceals);
criterion_main!(benches);
