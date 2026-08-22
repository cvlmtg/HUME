use super::*;
use pretty_assertions::assert_eq;

// ── tab_advance ──────────────────────────────────────────────────────────

#[test]
fn tab_advance_from_col_zero_reaches_full_width() {
    assert_eq!(tab_advance(0, 4), 4);
}

#[test]
fn tab_advance_mid_stop_reaches_next_stop() {
    assert_eq!(tab_advance(1, 4), 3);
    assert_eq!(tab_advance(2, 4), 2);
    assert_eq!(tab_advance(3, 4), 1);
}

#[test]
fn tab_advance_already_on_stop_advances_full_width() {
    // A tab sitting exactly on a stop still advances a full tw, not 0 —
    // tabs never collapse to a no-op.
    assert_eq!(tab_advance(4, 4), 4);
    assert_eq!(tab_advance(8, 4), 4);
}

#[test]
fn tab_advance_zero_width_clamps_to_one() {
    assert_eq!(tab_advance(0, 0), 1);
    assert_eq!(tab_advance(3, 0), 1);
}

#[test]
fn tab_advance_no_overflow_near_u32_max() {
    // col = u32::MAX as usize, tw = 4: u32::MAX % 4 == 3, so the distance
    // to the next stop is 4 - 3 = 1. The modulo-based formula never
    // computes a "next stop" value that could itself overflow — relevant
    // since `hume-engine` casts its `u32` document column straight into
    // this `usize` parameter.
    assert_eq!(tab_advance(u32::MAX as usize, 4), 1);
}

// ── grapheme_width ───────────────────────────────────────────────────────

#[test]
fn grapheme_width_ascii_is_one_column() {
    assert_eq!(grapheme_width("a", 0, 4), 1);
    assert_eq!(grapheme_width("a", 7, 4), 1); // column-independent
}

#[test]
fn grapheme_width_wide_cjk_is_two_columns() {
    // U+6F22 (漢) is East Asian Wide.
    assert_eq!(grapheme_width("\u{6F22}", 0, 4), 2);
}

#[test]
fn grapheme_width_tab_advances_to_next_stop() {
    assert_eq!(grapheme_width("\t", 0, 4), 4);
    assert_eq!(grapheme_width("\t", 2, 4), 2);
}

#[test]
fn grapheme_width_decomposed_e_acute_is_one_column() {
    // "e" + U+0301 (combining acute accent) is ONE grapheme cluster
    // (unicode-segmentation merges them). The base 'e' contributes 1
    // column; the combining mark contributes 0 — total 1, matching how
    // the character actually renders on screen.
    assert_eq!(grapheme_width("e\u{0301}", 0, 4), 1);
}

#[test]
fn grapheme_width_lone_combining_mark_clamps_to_one() {
    // A combining mark with no base character is a genuinely zero-width
    // cluster (measures 0 via unicode-width). Clamped up to 1 so it still
    // occupies an addressable cell.
    assert_eq!(grapheme_width("\u{0301}", 0, 4), 1);
}

// ── str_width ────────────────────────────────────────────────────────────

#[test]
fn str_width_no_tabs_sums_grapheme_widths() {
    // "ab" = 1 + 1.
    assert_eq!(str_width("ab", 0, 4), 2);
}

#[test]
fn str_width_tab_uses_running_column() {
    // "ab\tcd" at tw=4: a(1)->1, b(1)->2, tab from col 2 advances 2 (to
    // col 4), c(1)->5, d(1)->6. Total width = 6.
    assert_eq!(str_width("ab\tcd", 0, 4), 6);
}

#[test]
fn str_width_wide_char_before_tab_shifts_the_stop() {
    // This is the case the git-diff plugin got wrong when it counted one
    // Steel char (not one display column) per preceding character: a wide
    // CJK char occupies 2 columns, so the tab after it lands one column
    // later than a naive char-count would predict.
    //
    // "\u{6F22}\tx" at tw=4: 漢(2)->2, tab from col 2 advances 2 (to col
    // 4), x(1)->5. Total width = 5 — not 4, which a char-counting (not
    // column-counting) walk would have produced.
    assert_eq!(str_width("\u{6F22}\tx", 0, 4), 5);
}

#[test]
fn str_width_honors_nonzero_start_col() {
    // Starting at column 2, a tab (tw=4) advances 2 to reach column 4.
    assert_eq!(str_width("\t", 2, 4), 2);
}
