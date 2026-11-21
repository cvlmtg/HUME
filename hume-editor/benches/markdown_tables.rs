//! What `core:markdown`'s table selection costs: `m |` on a column of 200
//! body rows, and of 1000 as a reported row only, and `m i |` with one cursor
//! in each of two tables 3000 lines apart. Each sample first checks that the
//! keys select what they should.

use std::time::{Duration, Instant};

use criterion::{Criterion, criterion_group, criterion_main};
use hume_editor::testing::{Keys, MarkdownBench};

const ROWS: [usize; 2] = [200, 1000];

/// Prose lines between the two tables of the far sample.
const GAP: usize = 3000;

/// A table of `rows` body rows and three columns.
fn table(rows: usize, marker: &str) -> String {
    let mut text = String::from("| head | col | end |\n|------|-----|-----|\n");
    for k in 0..rows {
        text.push_str(&format!("| row {k} | {marker} | x |\n"));
    }
    text
}

/// `iters` rounds of `before`, then `timed`, timing only `timed`.
fn timed_after(bench: &mut MarkdownBench, iters: u64, before: &Keys, timed: &Keys) -> Duration {
    let mut total = Duration::ZERO;
    for _ in 0..iters {
        bench.press(before);
        let start = Instant::now();
        bench.press(timed);
        total += start.elapsed();
    }
    total
}

fn bench_markdown_tables(c: &mut Criterion) {
    test_fixtures::require_grammars(&["markdown", "markdown.inline"]);
    let root = tempfile::tempdir().expect("a temp dir");
    for name in ["markdown", "markdown.inline"] {
        test_fixtures::install_fixture_grammar(&root.path().join("data/grammars"), name, "1");
    }
    let mut group = c.benchmark_group("markdown_tables");

    let on_first_body_row = Keys::parse(";ggjj");
    let column = Keys::parse("m|");
    for rows in ROWS {
        let mut bench = MarkdownBench::new(root.path(), &table(rows, "c"), true);
        bench.press(&on_first_body_row);
        bench.press(&column);
        assert_eq!(
            bench.selection_count(),
            rows + 1,
            "m | did not select the column"
        );
        group.bench_function(format!("column/{rows}"), |b| {
            b.iter_custom(|iters| timed_after(&mut bench, iters, &on_first_body_row, &column));
        });
    }

    let far = format!(
        "{}{}{}",
        table(3, "far"),
        "prose\n".repeat(GAP),
        table(3, "far")
    );
    let mut bench = MarkdownBench::new(root.path(), &far, true);
    let on_both = Keys::parse(";gg/far<ret>m/");
    let cell = Keys::parse("mi|");
    bench.press(&on_both);
    assert_eq!(
        bench.selection_count(),
        6,
        "the search did not find every cell"
    );
    bench.press(&cell);
    assert_eq!(bench.selection_count(), 6, "m i | did not keep every cell");
    group.bench_function(format!("cell/far/{GAP}"), |b| {
        b.iter_custom(|iters| timed_after(&mut bench, iters, &on_both, &cell));
    });

    group.finish();
}

criterion_group!(benches, bench_markdown_tables);
criterion_main!(benches);
