use super::*;
use crate::test_support::rope;

#[test]
fn checked_accepts_every_position_up_to_and_including_len_chars() {
    let r = rope("ab\n"); // len_chars == 3
    assert_eq!(CharOffset::checked(&r, 0), Some(CharOffset::new(0)));
    assert_eq!(CharOffset::checked(&r, 3), Some(CharOffset::new(3)));
    assert_eq!(CharOffset::checked(&r, 4), None);
}

#[test]
fn clamped_pulls_a_past_end_index_back_to_len_chars() {
    let r = rope("ab\n"); // len_chars == 3
    assert_eq!(CharOffset::clamped(&r, 3), CharOffset::new(3));
    assert_eq!(CharOffset::clamped(&r, 99), CharOffset::new(3));
}

#[test]
fn snapped_is_identity_on_a_boundary_and_floors_mid_cluster() {
    // "e\u{0301}\n" — 'e' + combining acute accent, one grapheme cluster of
    // 2 chars, then the structural trailing '\n'.
    let r = rope("e\u{0301}\n");
    assert_eq!(CharOffset::snapped(r.slice(..), 0), CharOffset::new(0));
    assert_eq!(CharOffset::snapped(r.slice(..), 1), CharOffset::new(0));
    assert_eq!(CharOffset::snapped(r.slice(..), 2), CharOffset::new(2));
}

#[test]
fn chars_since_measures_backward_distance() {
    assert_eq!(CharOffset::new(5).chars_since(CharOffset::new(2)), 3);
    assert_eq!(CharOffset::new(5).chars_since(CharOffset::new(5)), 0);
}

#[test]
#[should_panic(expected = "chars_since measures backward only")]
fn chars_since_panics_on_inversion() {
    let _ = CharOffset::new(2).chars_since(CharOffset::new(5));
}

#[test]
fn shift_moves_forward_and_backward() {
    assert_eq!(CharOffset::new(5).shift(3), CharOffset::new(8));
    assert_eq!(CharOffset::new(5).shift(-3), CharOffset::new(2));
    assert_eq!(CharOffset::new(5).shift(0), CharOffset::new(5));
}

#[test]
#[should_panic(expected = "result would be negative")]
fn shift_panics_on_negative_result() {
    let _ = CharOffset::new(2).shift(-3);
}

#[test]
fn default_is_offset_zero() {
    assert_eq!(CharOffset::default(), CharOffset::new(0));
}

#[test]
fn ord_matches_the_underlying_index() {
    assert!(CharOffset::new(2) < CharOffset::new(5));
    assert_eq!(
        CharOffset::new(2).min(CharOffset::new(5)),
        CharOffset::new(2)
    );
    assert_eq!(
        CharOffset::new(2).max(CharOffset::new(5)),
        CharOffset::new(5)
    );
}

#[test]
fn exclusive_range_excludes_its_own_end() {
    let r = ExclusiveRange::new(CharOffset::new(2), CharOffset::new(5));
    assert!(!r.contains(CharOffset::new(1)));
    assert!(r.contains(CharOffset::new(2)));
    assert!(r.contains(CharOffset::new(4)));
    assert!(!r.contains(CharOffset::new(5)));
}

#[test]
fn exclusive_range_is_empty_when_start_equals_end() {
    let r = ExclusiveRange::new(CharOffset::new(3), CharOffset::new(3));
    assert!(r.is_empty());
    assert!(!ExclusiveRange::new(CharOffset::new(3), CharOffset::new(4)).is_empty());
}

#[test]
fn exclusive_range_converts_to_a_plain_usize_range() {
    let r = ExclusiveRange::new(CharOffset::new(2), CharOffset::new(5));
    let plain: std::ops::Range<usize> = r.into();
    assert_eq!(plain, 2..5);
}

#[test]
fn inclusive_range_covers_its_own_end() {
    let r = InclusiveRange::new(CharOffset::new(2), CharOffset::new(5));
    assert!(!r.contains(CharOffset::new(1)));
    assert!(r.contains(CharOffset::new(2)));
    assert!(r.contains(CharOffset::new(5)));
    assert!(!r.contains(CharOffset::new(6)));
}
