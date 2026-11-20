//! What `core:markdown`'s list renumbering costs: Enter at the end of an
//! ordered list's first and last item, and the undo of the first, each with
//! every settle the run loop gives the key. The `floor` rows press the same
//! keys with `'continue-lists` off, the editor's own Enter and undo.
//! Lists of 50 and 200 items give the cost per item.

use std::time::{Duration, Instant};

use criterion::{Criterion, criterion_group, criterion_main};
use hume_editor::testing::{Keys, ListRenumberBench, ListSample};

const SIZES: [usize; 2] = [50, 200];

/// Keys that put the cursor at the end of the first or the last item, in
/// Insert mode.
const AT_TOP: &str = "ggA";
const AT_END: &str = "geA";

/// `iters` rounds of `before`, then `timed`, then `after`, timing only
/// `timed`.
fn timed_between(
    bench: &mut ListRenumberBench,
    iters: u64,
    [before, timed, after]: [&Keys; 3],
) -> Duration {
    let mut total = Duration::ZERO;
    for _ in 0..iters {
        bench.press(before);
        let start = Instant::now();
        bench.press(timed);
        total += start.elapsed();
        bench.press(after);
    }
    total
}

/// The list's lines once Enter has added an item after `index` and the list
/// has been renumbered.
fn entered_lines(sample: ListSample, index: usize) -> Vec<String> {
    let n = sample.items();
    let ordered = matches!(sample, ListSample::Ordered(_));
    let mut lines = Vec::with_capacity(n + 1);
    for k in 0..n {
        let shifted = ordered && k > index;
        lines.push(format!(
            "{}. item {k}",
            sample.number(k) + usize::from(shifted)
        ));
        if k == index {
            lines.push(format!("{}. ", sample.number(k) + usize::from(ordered)));
        }
    }
    lines
}

/// Checks once that `at` then Enter renumbers as the list's numbers say,
/// and that the undo gives back the list.
fn assert_renumbers(bench: &mut ListRenumberBench, sample: ListSample, at: &str, index: usize) {
    let original = bench.text();
    bench.press(&Keys::parse(&format!("{at}<ret>")));
    let text = bench.text();
    assert_eq!(
        text.lines().collect::<Vec<_>>(),
        entered_lines(sample, index)
    );
    bench.press(&Keys::parse("<esc>u"));
    assert_eq!(
        bench.text(),
        original,
        "the undo did not give back the list"
    );
}

fn bench_markdown_lists(c: &mut Criterion) {
    test_fixtures::require_grammars(&["markdown", "markdown.inline"]);
    let root = tempfile::tempdir().expect("a temp dir");
    for name in ["markdown", "markdown.inline"] {
        test_fixtures::install_fixture_grammar(&root.path().join("data/grammars"), name, "1");
    }
    let enter = Keys::parse("<ret>");
    let leave_undo = Keys::parse("<esc>u");
    let undo = Keys::parse("u");
    let none = Keys::parse("");
    let mut group = c.benchmark_group("markdown_lists");

    for n in SIZES {
        for (sample_name, sample) in [
            ("ordered", ListSample::Ordered(n)),
            ("uniform", ListSample::Uniform(n)),
        ] {
            let id = format!("{sample_name}/{n}");
            let mut bench = ListRenumberBench::new(root.path(), sample, true);
            assert_renumbers(&mut bench, sample, AT_TOP, 0);
            assert_renumbers(&mut bench, sample, AT_END, n - 1);

            for (row, at) in [("top", AT_TOP), ("end", AT_END)] {
                let at = Keys::parse(at);
                group.bench_function(format!("enter/{row}/{id}"), |b| {
                    b.iter_custom(|iters| {
                        timed_between(&mut bench, iters, [&at, &enter, &leave_undo])
                    });
                });
            }

            let entered = Keys::parse(&format!("{AT_TOP}<ret><esc>"));
            group.bench_function(format!("undo/top/{id}"), |b| {
                b.iter_custom(|iters| timed_between(&mut bench, iters, [&entered, &undo, &none]));
            });
        }

        let mut bench = ListRenumberBench::new(root.path(), ListSample::Ordered(n), false);
        let at = Keys::parse(AT_TOP);
        group.bench_function(format!("floor/enter/top/ordered/{n}"), |b| {
            b.iter_custom(|iters| timed_between(&mut bench, iters, [&at, &enter, &leave_undo]));
        });
        let entered = Keys::parse(&format!("{AT_TOP}<ret><esc>"));
        group.bench_function(format!("floor/undo/top/ordered/{n}"), |b| {
            b.iter_custom(|iters| timed_between(&mut bench, iters, [&entered, &undo, &none]));
        });
    }

    group.finish();
}

criterion_group!(benches, bench_markdown_lists);
criterion_main!(benches);
