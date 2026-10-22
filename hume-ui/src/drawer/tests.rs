use super::{band_height, band_visible_rows};

#[test]
fn band_height_is_capped_by_the_ceiling() {
    // 50 items and the top frame row want 51 rows; a 20-row terminal's 35%
    // ceiling is 7.
    assert_eq!(band_height(50, 20), 7);
}

/// The overflow-safety guard on the shared arithmetic itself lives in
/// `menu_box::tests` (`band_capacity_clamps_instead_of_overflowing_u16`).
/// This pins the drawer's own 1-row top frame against it instead of
/// re-testing the arithmetic.
#[test]
fn band_visible_rows_reserves_the_top_frame_row_below_the_cap() {
    assert_eq!(
        band_visible_rows(3, 20),
        3,
        "well under the cap: all 3 items fit"
    );
}
