use super::*;
use crate::lines::content_line_count;
use crate::test_support::rope;

#[test]
fn ropey_line_clamped_pulls_a_past_end_index_back_to_the_last_ropey_line() {
    let r = rope("a\nb\n"); // ropey lines: "a\n", "b\n", "" (phantom), last is 2
    assert_eq!(RopeyLine::clamped(&r, 2), RopeyLine::new(2));
    assert_eq!(RopeyLine::clamped(&r, 99), RopeyLine::new(2));
}

#[test]
fn ropey_line_to_content_is_none_on_the_phantom_line() {
    let r = rope("a\nb\n");
    assert_eq!(RopeyLine::new(0).to_content(&r), Some(ContentLine::new(0)));
    assert_eq!(RopeyLine::new(1).to_content(&r), Some(ContentLine::new(1)));
    assert_eq!(RopeyLine::new(2).to_content(&r), None); // the phantom line
}

#[test]
fn ropey_line_advance() {
    assert_eq!(RopeyLine::new(2).advance(3), RopeyLine::new(5));
}

#[test]
fn content_line_advance_and_retreat_saturating() {
    assert_eq!(ContentLine::new(2).advance(3), ContentLine::new(5));
    assert_eq!(
        ContentLine::new(2).retreat_saturating(3),
        ContentLine::new(0)
    );
    assert_eq!(
        ContentLine::new(5).retreat_saturating(3),
        ContentLine::new(2)
    );
}

#[test]
fn default_is_line_zero() {
    assert_eq!(ContentLine::default(), ContentLine::new(0));
}

#[test]
fn content_line_checked_rejects_out_of_range() {
    let r = rope("a\nb\n"); // 2 content lines: 0, 1
    assert_eq!(ContentLine::checked(&r, 0), Some(ContentLine::new(0)));
    assert_eq!(ContentLine::checked(&r, 1), Some(ContentLine::new(1)));
    assert_eq!(ContentLine::checked(&r, 2), None);
}

#[test]
fn content_line_clamped_pulls_a_past_end_index_back_to_the_last_content_line() {
    let r = rope("a\nb\n");
    assert_eq!(ContentLine::clamped(&r, 1), ContentLine::new(1));
    assert_eq!(ContentLine::clamped(&r, 99), ContentLine::new(1));
}

#[test]
fn content_line_from_number_decodes_1_based_input() {
    assert_eq!(ContentLine::from_number(1), Some(ContentLine::new(0)));
    assert_eq!(ContentLine::from_number(5), Some(ContentLine::new(4)));
    assert_eq!(ContentLine::from_number(0), None); // no line 0
}

#[test]
fn content_line_number_is_the_inverse_of_from_number() {
    for n in 1..10 {
        assert_eq!(ContentLine::from_number(n).unwrap().number(), n);
    }
}

#[test]
fn content_line_from_ropey_line_widening_is_an_identity_on_the_index() {
    let line = ContentLine::new(3);
    let widened: RopeyLine = line.into();
    assert_eq!(widened.index(), line.index());
}

#[test]
fn content_line_count_end_exclusive_is_one_past_the_last_content_line() {
    let r = rope("a\nb\n"); // 2 content lines: 0, 1
    assert_eq!(content_line_count(&r).end_exclusive(), ContentLine::new(2));
}

#[test]
fn content_line_abs_diff_ignores_direction() {
    assert_eq!(
        ContentLine::new(5).abs_diff(ContentLine::new(2)),
        ContentLine::new(2).abs_diff(ContentLine::new(5))
    );
    assert_eq!(ContentLine::new(5).abs_diff(ContentLine::new(2)), 3);
    assert_eq!(ContentLine::new(5).abs_diff(ContentLine::new(5)), 0);
}

#[test]
fn content_line_lines_since_measures_forward_distance() {
    assert_eq!(ContentLine::new(5).lines_since(ContentLine::new(2)), 3);
    assert_eq!(ContentLine::new(5).lines_since(ContentLine::new(5)), 0);
}

#[test]
#[should_panic(expected = "lines_since measures forward only")]
fn content_line_lines_since_panics_on_inversion() {
    let _ = ContentLine::new(2).lines_since(ContentLine::new(5));
}

fn lines_of(range: impl Iterator<Item = ContentLine>) -> Vec<usize> {
    range.map(ContentLine::index).collect()
}

#[test]
fn inclusive_line_range_iterates_both_ends() {
    let range = InclusiveRange::new(ContentLine::new(2), ContentLine::new(4));
    assert_eq!(lines_of(range.iter()), [2, 3, 4]);
}

#[test]
fn inclusive_line_range_of_one_line_yields_that_line() {
    let range = InclusiveRange::new(ContentLine::new(3), ContentLine::new(3));
    assert_eq!(lines_of(range.iter()), [3]);
}

#[test]
fn exclusive_line_range_stops_before_its_end() {
    let range = ExclusiveRange::new(ContentLine::new(2), ContentLine::new(5));
    assert_eq!(lines_of(range.iter()), [2, 3, 4]);
}

#[test]
fn exclusive_line_range_that_starts_at_or_past_its_end_is_empty() {
    let at = ExclusiveRange::new(ContentLine::new(3), ContentLine::new(3));
    let past = ExclusiveRange::new(ContentLine::new(4), ContentLine::new(3));
    assert_eq!(lines_of(at.iter()), Vec::<usize>::new());
    assert_eq!(lines_of(past.iter()), Vec::<usize>::new());
}

fn lines(start: usize, end: usize) -> ExclusiveRange<ContentLine> {
    ExclusiveRange::new(ContentLine::new(start), ContentLine::new(end))
}

fn joined(start: usize, last: usize) -> InclusiveRange<ContentLine> {
    InclusiveRange::new(ContentLine::new(start), ContentLine::new(last))
}

#[test]
fn join_line_ranges_keeps_ranges_apart_sorted() {
    assert_eq!(
        join_line_ranges([lines(50, 60), lines(0, 10)]),
        [joined(0, 9), joined(50, 59)]
    );
}

#[test]
fn join_line_ranges_joins_overlapping_touching_and_contained_ranges() {
    assert_eq!(
        join_line_ranges([lines(0, 10), lines(5, 20)]),
        [joined(0, 19)]
    );
    assert_eq!(
        join_line_ranges([lines(10, 20), lines(0, 10)]),
        [joined(0, 19)]
    );
    assert_eq!(
        join_line_ranges([lines(0, 20), lines(5, 10)]),
        [joined(0, 19)]
    );
}

#[test]
fn join_line_ranges_drops_empty_ranges() {
    assert_eq!(
        join_line_ranges([lines(3, 3), lines(20, 30), lines(40, 40)]),
        [joined(20, 29)]
    );
}

#[test]
fn join_line_ranges_gives_a_one_line_range_as_that_line() {
    assert_eq!(join_line_ranges([lines(3, 4)]), [joined(3, 3)]);
}

#[test]
fn to_exclusive_ends_on_the_line_after_the_last() {
    assert_eq!(joined(3, 6).to_exclusive(), lines(3, 7));
    assert_eq!(joined(3, 3).to_exclusive(), lines(3, 4));
}
