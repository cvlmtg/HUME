use super::*;
use crate::test_support::rope;

#[test]
fn ropey_line_clamped_pulls_a_past_end_index_back_to_the_last_ropey_line() {
    let r = rope("a\nb\n"); // ropey lines: "a\n", "b\n", "" (phantom) — last is 2
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
fn ropey_line_down_and_up() {
    assert_eq!(RopeyLine::new(2).down(3), RopeyLine::new(5));
    assert_eq!(RopeyLine::new(2).up(3), RopeyLine::new(0)); // saturates
    assert_eq!(RopeyLine::new(5).up(3), RopeyLine::new(2));
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
