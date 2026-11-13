use super::*;

fn line(n: usize) -> ContentLine {
    ContentLine::new(n)
}

fn span(start: usize, end: usize) -> InclusiveRange<ContentLine> {
    InclusiveRange::new(line(start), line(end))
}

fn set(ranges: &[(usize, usize)]) -> LineSet {
    LineSet::from_ranges(ranges.iter().map(|&(start, end)| span(start, end)))
}

/// The lines `0..20` that `set` contains.
fn contained(set: &LineSet) -> Vec<usize> {
    (0..20).filter(|&n| set.contains(line(n))).collect()
}

#[test]
fn from_ranges_merges_nested_overlapping_and_adjacent_ranges() {
    let nested = set(&[(2, 9), (4, 5)]);
    assert_eq!(nested, set(&[(2, 9)]));
    assert_eq!(contained(&nested), [2, 3, 4, 5, 6, 7, 8, 9]);

    let overlapping = set(&[(5, 8), (2, 6)]);
    assert_eq!(overlapping, set(&[(2, 8)]));
    assert_eq!(contained(&overlapping), [2, 3, 4, 5, 6, 7, 8]);

    let adjacent = set(&[(5, 8), (2, 4)]);
    assert_eq!(adjacent, set(&[(2, 8)]));
    assert_eq!(contained(&adjacent), [2, 3, 4, 5, 6, 7, 8]);

    let apart = set(&[(5, 5), (2, 3)]);
    assert_eq!(apart, set(&[(2, 3), (5, 5)]));
    assert_eq!(contained(&apart), [2, 3, 5]);
}

#[test]
fn containing_and_ending_at_or_after() {
    let lines = set(&[(2, 3), (6, 8)]);
    let containing: Vec<Option<InclusiveRange<ContentLine>>> =
        (0..10).map(|n| lines.containing(line(n))).collect();
    assert_eq!(
        containing,
        [
            None,
            None,
            Some(span(2, 3)),
            Some(span(2, 3)),
            None,
            None,
            Some(span(6, 8)),
            Some(span(6, 8)),
            Some(span(6, 8)),
            None,
        ]
    );

    assert_eq!(lines.ending_at_or_after(line(0)), [span(2, 3), span(6, 8)]);
    assert_eq!(lines.ending_at_or_after(line(3)), [span(2, 3), span(6, 8)]);
    assert_eq!(lines.ending_at_or_after(line(4)), [span(6, 8)]);
    assert_eq!(lines.ending_at_or_after(line(8)), [span(6, 8)]);
    assert_eq!(lines.ending_at_or_after(line(9)), []);
}

#[test]
fn intersects_a_span_touching_a_range_and_not_one_in_a_gap() {
    let lines = set(&[(2, 3), (6, 8)]);
    assert!(
        lines.intersects(span(3, 4)),
        "touches the first range's end"
    );
    assert!(
        lines.intersects(span(5, 6)),
        "touches the second range's start"
    );
    assert!(lines.intersects(span(0, 20)), "covers both ranges");
    assert!(lines.intersects(span(7, 7)), "inside a range");
    assert!(!lines.intersects(span(4, 5)), "the gap between the ranges");
    assert!(!lines.intersects(span(0, 1)), "before every range");
    assert!(!lines.intersects(span(9, 12)), "after every range");
    assert!(!LineSet::default().intersects(span(0, 20)));
    assert!(LineSet::default().is_empty());
    assert!(!lines.is_empty());
}
