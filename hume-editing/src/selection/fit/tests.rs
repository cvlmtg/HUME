use hume_rope::offset::CharOffset;
use pretty_assertions::assert_eq;

use super::*;
use crate::marked::{parse, render};
use crate::selection::EditView;
use crate::selection::Selection;
use crate::text::BufferText;
use hume_rope::cluster::ClusterStart;

fn co(n: usize) -> CharOffset {
    CharOffset::new(n)
}

/// A text wide enough that every char below 60 starts a cluster: the text a
/// set under test was computed for, before it meets another one.
fn earlier() -> BufferText {
    BufferText::from(" ".repeat(60).as_str())
}

fn at(text: &BufferText, n: usize) -> ClusterStart {
    let pos = text.snap(co(n));
    assert_eq!(pos.offset(), co(n));
    pos
}

/// `selections` of `earlier()`, tagged for it.
fn set_of(selections: impl FnOnce(&BufferText) -> Vec<Selection>, primary: usize) -> SelectionSet {
    let text = earlier();
    SelectionSet::from_parts(selections(&text), primary, text.version())
}

#[test]
fn a_set_tagged_for_its_text_fits() {
    let state = parse("a-[b]>c\n");
    let set = state.clone().into_selections();
    assert_fits(state.text(), &set);
}

#[test]
fn refitting_floors_each_end_to_its_cluster_and_clamps_it_into_the_text() {
    let state = parse("-[a]>e\u{301}b\n");
    let raw = set_of(|t| vec![Selection::new(at(t, 2), at(t, 40))], 0);
    let fitted = refit_parts(state.text(), raw.selections(), raw.primary_pos());
    let view = EditView::bind(state.text(), &fitted);
    assert_eq!(view.check(), Ok(()));
    assert_eq!(render(view), "a-[e\u{301}b\n]>");
}

#[test]
fn refitting_merges_selections_that_land_on_one_cluster() {
    let state = parse("-[a]>e\u{301}b\n");
    let raw = set_of(
        |t| vec![Selection::cursor(at(t, 1)), Selection::cursor(at(t, 2))],
        1,
    );
    let fitted = refit_parts(state.text(), raw.selections(), raw.primary_pos());
    assert_eq!(
        render(EditView::bind(state.text(), &fitted)),
        "a-[e\u{301}]>b\n"
    );
}

#[test]
#[should_panic(expected = "was paired with another text")]
fn pairing_a_set_with_another_text_panics() {
    let one = parse("-[a]>\n");
    let other = parse("-[a]>\n");
    let set = one.into_selections();
    assert_fits(other.text(), &set);
}

#[test]
fn check_positions_reports_a_split_cluster() {
    let state = parse("-[a]>e\u{301}b\n");
    let raw = set_of(|t| vec![Selection::cursor(at(t, 2))], 0);
    assert_eq!(
        check_positions(state.text(), &raw),
        Err(InvariantViolation::SplitsCluster { index: 0 })
    );
}

#[test]
fn check_positions_reports_an_end_past_the_text() {
    let state = parse("-[a]>\n");
    let raw = set_of(|t| vec![Selection::cursor(at(t, 2))], 0);
    assert_eq!(
        check_positions(state.text(), &raw),
        Err(InvariantViolation::OutOfBounds { index: 0 })
    );
}

#[test]
fn check_positions_reports_overlap() {
    let state = parse("-[a]>bc\n");
    let text = earlier();
    let raw = SelectionSet::from_parts_unchecked(
        vec![
            Selection::new(at(&text, 0), at(&text, 1)),
            Selection::cursor(at(&text, 1)),
        ],
        0,
        text.version(),
    );
    assert_eq!(
        check_positions(state.text(), &raw),
        Err(InvariantViolation::Overlapping { index: 1 })
    );
}

#[test]
fn check_reports_a_set_tagged_for_another_text() {
    let state = parse("-[a]>\n");
    let raw = set_of(|t| vec![Selection::cursor(at(t, 0))], 0);
    let view = EditView::fitted(state.text(), &raw);
    assert_eq!(view.check(), Err(InvariantViolation::VersionMismatch));
}
