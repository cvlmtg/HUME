use super::*;

// `DisplayLineCol`/`BufferLineCol` share their entire behavior (the
// `display_col_methods!` macro in the parent module) — one macro-generated
// test body exercised against both, rather than two hand-copies that could
// silently drift apart, mirrors how the implementation itself is shared.
macro_rules! display_col_tests {
    ($ty:ident, $mod_name:ident) => {
        mod $mod_name {
            use super::*;

            #[test]
            fn get_returns_the_minted_value() {
                assert_eq!($ty::new(7).get(), 7);
            }

            #[test]
            fn default_is_column_zero() {
                assert_eq!($ty::default(), $ty::new(0));
            }

            #[test]
            fn advance_adds_and_saturates() {
                assert_eq!($ty::new(3).advance(4), $ty::new(7));
                assert_eq!($ty::new(u32::MAX).advance(1), $ty::new(u32::MAX));
            }

            #[test]
            fn cells_since_measures_forward_distance() {
                assert_eq!($ty::new(7).cells_since($ty::new(2)), 5);
                assert_eq!($ty::new(2).cells_since($ty::new(2)), 0);
            }

            #[test]
            #[should_panic(expected = "cells_since measures forward only")]
            fn cells_since_panics_on_inversion() {
                let _ = $ty::new(2).cells_since($ty::new(5));
            }

            #[test]
            fn cells_since_saturating_measures_forward_distance() {
                assert_eq!($ty::new(7).cells_since_saturating($ty::new(2)), 5);
                assert_eq!($ty::new(2).cells_since_saturating($ty::new(2)), 0);
            }

            #[test]
            fn cells_since_saturating_clamps_to_zero_on_inversion() {
                assert_eq!($ty::new(2).cells_since_saturating($ty::new(5)), 0);
            }

            #[test]
            fn abs_diff_ignores_direction() {
                assert_eq!($ty::new(2).abs_diff($ty::new(5)), 3);
                assert_eq!($ty::new(5).abs_diff($ty::new(2)), 3);
                assert_eq!($ty::new(5).abs_diff($ty::new(5)), 0);
            }

            #[test]
            fn shift_moves_forward_and_backward_and_saturates_at_zero() {
                assert_eq!($ty::new(5).shift(3), $ty::new(8));
                assert_eq!($ty::new(5).shift(-3), $ty::new(2));
                assert_eq!($ty::new(2).shift(-5), $ty::new(0));
            }

            #[test]
            fn ord_matches_the_underlying_column() {
                assert!($ty::new(2) < $ty::new(5));
                assert_eq!($ty::new(2).min($ty::new(5)), $ty::new(2));
                assert_eq!($ty::new(2).max($ty::new(5)), $ty::new(5));
            }
        }
    };
}

display_col_tests!(DisplayLineCol, display_line_col);
display_col_tests!(BufferLineCol, buffer_line_col);

#[test]
fn as_display_line_unwrapped_carries_the_same_number() {
    assert_eq!(
        BufferLineCol::new(9).as_display_line_unwrapped(),
        DisplayLineCol::new(9)
    );
}

#[test]
fn char_col_round_trips_through_index() {
    assert_eq!(CharCol::new(4).index(), 4);
}

#[test]
fn grapheme_col_number_is_one_based() {
    assert_eq!(GraphemeCol::new(0).number(), 1);
    assert_eq!(GraphemeCol::new(4).number(), 5);
}

#[test]
fn grapheme_col_from_number_rejects_zero() {
    assert_eq!(GraphemeCol::from_number(0), None);
    assert_eq!(GraphemeCol::from_number(1), Some(GraphemeCol::new(0)));
    assert_eq!(GraphemeCol::from_number(5), Some(GraphemeCol::new(4)));
}

#[test]
fn byte_col_round_trips_through_index() {
    assert_eq!(ByteCol::new(12).index(), 12);
}
