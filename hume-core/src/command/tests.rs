use super::*;

#[test]
fn typed_zero_counts_as_one() {
    assert_eq!(Count::typed(0).get(), 1);
}

#[test]
fn typed_clamps_to_ten_thousand() {
    assert_eq!(Count::typed(usize::MAX).get(), 10_000);
}

#[test]
fn untyped_runs_once_and_reads_as_untyped() {
    assert_eq!(Count::UNTYPED.get(), 1);
    assert!(!Count::UNTYPED.is_typed());
    assert!(Count::typed(1).is_typed());
}

#[test]
fn default_is_untyped() {
    assert_eq!(Count::default(), Count::UNTYPED);
}

#[test]
fn digits_append_in_typing_order() {
    assert_eq!(Count::UNTYPED.append_digit(1).append_digit(2).get(), 12);
}

#[test]
fn appended_digits_saturate_at_ten_thousand() {
    let count = (0..25).fold(Count::UNTYPED, |count, _| count.append_digit(9));
    assert_eq!(count.get(), 10_000);
}
