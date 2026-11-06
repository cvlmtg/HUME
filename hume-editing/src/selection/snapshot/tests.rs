use pretty_assertions::assert_eq;

use super::*;
use crate::marked::{parse, render};
use crate::selection::EditView;

/// `input`'s selections taken as a snapshot and rebuilt on its own text,
/// written back in marker notation.
fn round_trip(input: &str) -> String {
    let state = parse(input);
    let text = state.text().clone();
    let snapshot = SelectionsSnapshot::of(&state.into_selections());
    let rebuilt = snapshot.on(&text).expect("the snapshot fits its own text");
    render(EditView::bind(&text, &rebuilt))
}

fn on_text(text: &str, ranges: Vec<(usize, usize)>, primary: usize) -> Option<SelectionSet> {
    SelectionsSnapshot::from_ranges(ranges, primary).on(&BufferText::from(text))
}

#[test]
fn selections_rebuild_on_the_text_they_were_taken_from() {
    for input in [
        "a-[b]>c\n",
        "<[hello]- w-{or}>ld\n",
        "-[a]>b<{cd}-\n",
        "x-[e\u{301}]>y\n",
        "one\n-[two\n]>\n",
    ] {
        assert_eq!(round_trip(input), input);
    }
}

#[test]
fn a_primary_past_the_selections_rebuilds_nothing() {
    assert_eq!(on_text("abc\n", vec![(0, 1)], 1), None);
    assert_eq!(on_text("abc\n", vec![], 0), None);
}

#[test]
fn an_end_at_or_past_the_text_end_rebuilds_nothing() {
    assert_eq!(on_text("abc\n", vec![(0, 4)], 0), None);
    assert_eq!(on_text("abc\n", vec![(9, 9)], 0), None);
}

#[test]
fn an_end_inside_a_cluster_rebuilds_nothing() {
    // `e` then a combining acute accent: offset 1 is inside the cluster.
    assert_eq!(on_text("e\u{301}x\n", vec![(1, 1)], 0), None);
}
