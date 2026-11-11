use std::collections::BTreeSet;

use super::*;

fn line(n: usize) -> ContentLine {
    ContentLine::new(n)
}

fn fold(first: usize, last: usize, shows_last: bool) -> Fold {
    Fold::new(line(first), line(last), shows_last).expect("fold hides at least one line")
}

fn ranges(hidden: &HiddenLines) -> Vec<(usize, usize)> {
    hidden
        .0
        .iter()
        .map(|r| (r.start.index(), r.end.index()))
        .collect()
}

/// Every hidden line of `folds`, listed one by one.
fn hidden_set(folds: &[Fold]) -> BTreeSet<usize> {
    folds
        .iter()
        .flat_map(|f| f.hidden().iter().map(ContentLine::index))
        .collect()
}

#[test]
fn a_fold_hides_the_lines_after_its_first() {
    assert_eq!(
        fold(3, 6, false).hidden(),
        InclusiveRange::new(line(4), line(6))
    );
    assert_eq!(
        fold(3, 6, true).hidden(),
        InclusiveRange::new(line(4), line(5))
    );
}

#[test]
fn a_fold_hiding_nothing_is_not_a_fold() {
    assert_eq!(Fold::new(line(3), line(3), false), None);
    assert_eq!(Fold::new(line(3), line(4), true), None);
    assert_eq!(Fold::new(line(3), line(3), true), None);
    assert!(Fold::new(line(3), line(4), false).is_some());
}

#[test]
fn a_fold_spans_its_first_through_its_last_line() {
    let f = fold(3, 6, true);
    let spanned: Vec<usize> = (0..10).filter(|&n| f.spans(line(n))).collect();
    assert_eq!(spanned, [3, 4, 5, 6]);
}

#[test]
fn nested_overlapping_and_adjacent_folds_merge() {
    let nested = HiddenLines::from_folds([fold(1, 9, false), fold(3, 5, false)]);
    assert_eq!(ranges(&nested), [(2, 9)]);

    let adjacent = HiddenLines::from_folds([fold(4, 8, false), fold(1, 4, false)]);
    assert_eq!(ranges(&adjacent), [(2, 8)]);

    let apart = HiddenLines::from_folds([fold(1, 3, false), fold(4, 6, true)]);
    assert_eq!(ranges(&apart), [(2, 3), (5, 5)]);
}

#[test]
fn queries_match_the_listed_hidden_lines() {
    let folds = [
        fold(0, 2, false),
        fold(4, 8, true),
        fold(5, 7, false),
        fold(10, 12, false),
        fold(13, 15, true),
    ];
    let hidden = HiddenLines::from_folds(folds);
    let set = hidden_set(&folds);
    let shown = |n: usize| !set.contains(&n);
    let end = 20;

    for n in 0..end {
        assert_eq!(
            hidden.is_hidden(line(n)),
            set.contains(&n),
            "is_hidden({n})"
        );
        assert_eq!(
            hidden.closed_at(line(n)),
            shown(n) && set.contains(&(n + 1)),
            "closed_at({n})"
        );
        let at_or_before = (0..=n).rev().find(|&m| shown(m)).expect("line 0 is shown");
        assert_eq!(
            hidden.shown_at_or_before(line(n)).index(),
            at_or_before,
            "shown_at_or_before({n})"
        );
        let prev = (0..n).rev().find(|&m| shown(m));
        assert_eq!(
            hidden.prev_shown(line(n)).map(ContentLine::index),
            prev,
            "prev_shown({n})"
        );
        for steps in 0..6 {
            let expected = match steps {
                0 => n,
                _ => (n + 1..)
                    .filter(|&m| shown(m))
                    .nth(steps - 1)
                    .expect("lines past the folds are shown"),
            };
            assert_eq!(
                hidden.line_after_shown(line(n), steps).index(),
                expected,
                "line_after_shown({n}, {steps})"
            );
        }
        assert_eq!(
            hidden.next_shown(line(n)).index(),
            (n + 1..)
                .find(|&m| shown(m))
                .expect("lines past the folds are shown"),
            "next_shown({n})"
        );
        for to in n..end {
            assert_eq!(
                hidden.shown_between(line(n), line(to)),
                (n..to).filter(|&m| shown(m)).count(),
                "shown_between({n}, {to})"
            );
            let runs: Vec<(usize, usize)> = hidden
                .shown_runs(InclusiveRange::new(line(n), line(to)))
                .map(|r| (r.start.index(), r.end.index()))
                .collect();
            assert_eq!(runs, shown_runs_of(&set, n, to), "shown_runs({n}, {to})");
        }
    }
}

/// The runs of lines in `first ..= last` not in `hidden`.
fn shown_runs_of(hidden: &BTreeSet<usize>, first: usize, last: usize) -> Vec<(usize, usize)> {
    let mut runs: Vec<(usize, usize)> = Vec::new();
    for n in (first..=last).filter(|n| !hidden.contains(n)) {
        match runs.last_mut() {
            Some(run) if run.1 + 1 == n => run.1 = n,
            _ => runs.push((n, n)),
        }
    }
    runs
}

#[test]
fn no_folds_hide_nothing() {
    let hidden = HiddenLines::default();
    assert!(hidden.is_empty());
    assert!(!hidden.is_hidden(line(0)));
    assert_eq!(hidden.next_shown(line(4)), line(5));
    assert_eq!(hidden.shown_between(line(2), line(7)), 5);
}
