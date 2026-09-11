use super::*;
use hume_ui::width::{ELLIPSIS, text_width};

fn entry(label: &str) -> TabEntry {
    TabEntry::new(TabId::default(), label.to_string())
}

// ── tab_extents ──────────────────────────────────────────────────────────────

#[test]
fn every_tab_fits_and_reserves_no_arrow_column() {
    // " a " = 3 cells, " bb " = 4 cells, one 1-cell separator between them.
    let tabs = [entry("a"), entry("bb")];

    let extents = tab_extents(&tabs, 0, 0, 8);

    assert_eq!(extents.ranges, vec![(0, 3), (4, 8)]);
    assert!(!extents.indicator_left);
    assert!(!extents.clipped_tail, "everything fit — no arrow needed");
}

#[test]
fn overflow_reserves_one_column_so_the_arrow_never_lands_on_a_tab() {
    // Same two tabs, but one column too narrow for both — tab_extents must
    // shrink to the tab(s) that fit in `width - 1`, not `width`, so the `›`
    // painted at the row's last column never falls inside a packed extent.
    let tabs = [entry("a"), entry("bb")];

    let extents = tab_extents(&tabs, 0, 0, 7);

    assert_eq!(
        extents.ranges,
        vec![(0, 3)],
        "only the first tab fits once the arrow column is reserved"
    );
    assert!(extents.clipped_tail);
    for &(_, end_x) in &extents.ranges {
        assert!(
            end_x < 7,
            "extent must end before the row's last column (the arrow's own column)"
        );
    }
}

#[test]
fn a_tab_wider_than_the_whole_row_still_packs_one_clamped_extent() {
    // A single tab whose padded label alone is wider than the row must not
    // leave the bar empty — it packs anyway, clamped to the available width.
    let tabs = [entry("a very long generated buffer name.rs")];

    let extents = tab_extents(&tabs, 0, 0, 10);

    assert_eq!(
        extents.ranges.len(),
        1,
        "the bar must never render with zero tabs when one exists"
    );
    assert_eq!(extents.ranges[0].0, 0);
    assert!(extents.ranges[0].1 <= 10);
    assert!(extents.clipped_tail);
}

#[test]
fn empty_tab_list_yields_no_ranges() {
    let extents = tab_extents(&[], 0, 0, 40);
    assert!(extents.ranges.is_empty());
    assert!(!extents.clipped_tail);
}

#[test]
fn scroll_past_zero_reserves_the_leading_indicator_column() {
    let tabs = [entry("a"), entry("bb")];

    let extents = tab_extents(&tabs, 1, 0, 8);

    assert!(extents.indicator_left);
    // Packing starts one column later (x=1) to leave room for `‹`.
    assert_eq!(extents.ranges, vec![(1, 5)]);
}

// ── fit_label ────────────────────────────────────────────────────────────────

#[test]
fn fit_label_pads_normally_when_the_label_fits_its_extent() {
    assert_eq!(fit_label("main.rs", 9), " main.rs ");
}

#[test]
fn fit_label_truncates_with_an_ellipsis_when_it_does_not_fit() {
    let label = fit_label("a_very_long_buffer_name.rs", 10);
    assert_eq!(text_width(&label), 10);
    assert!(label.ends_with(&format!("{ELLIPSIS} ")));
    assert!(label.starts_with(' '));
}
