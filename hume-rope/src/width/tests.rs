use super::*;
use pretty_assertions::assert_eq;

// ── tab_advance ──────────────────────────────────────────────────────────

#[test]
fn tab_advance_from_display_col_zero_reaches_full_width() {
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
    // display_col = u32::MAX as usize, tw = 4: u32::MAX % 4 == 3, so the
    // distance to the next stop is 4 - 3 = 1. The modulo-based formula
    // never computes a "next stop" value that could itself overflow —
    // relevant since `hume-engine` casts its `u32` document display column
    // straight into this `usize` parameter.
    assert_eq!(tab_advance(u32::MAX as usize, 4), 1);
}

// ── prev_tab_stop ────────────────────────────────────────────────────────

#[test]
fn prev_tab_stop_within_first_stop_is_zero() {
    assert_eq!(prev_tab_stop(0, 4), 0);
    assert_eq!(prev_tab_stop(1, 4), 0);
    assert_eq!(prev_tab_stop(3, 4), 0);
}

#[test]
fn prev_tab_stop_sitting_on_a_stop_steps_back_to_the_previous_one() {
    // A tab stop already sitting exactly on a stop still steps back, never
    // a no-op — the mirror of tab_advance never advancing by zero.
    assert_eq!(prev_tab_stop(4, 4), 0);
    assert_eq!(prev_tab_stop(8, 4), 4);
}

#[test]
fn prev_tab_stop_mid_stop_steps_to_the_stop_before_it() {
    assert_eq!(prev_tab_stop(5, 4), 4);
    assert_eq!(prev_tab_stop(9, 4), 8);
}

#[test]
fn prev_tab_stop_zero_width_clamps_to_one() {
    assert_eq!(prev_tab_stop(3, 0), 2);
}

#[test]
fn prev_tab_stop_and_tab_advance_are_a_fixed_point_pair() {
    // For any k >= 1, stepping back from exactly k tab stops lands on
    // (k-1) stops — the same relationship tab_advance has going forward.
    for k in 1usize..10 {
        assert_eq!(prev_tab_stop(k * 4, 4), (k - 1) * 4);
    }
}

// ── grapheme_width ───────────────────────────────────────────────────────

#[test]
fn grapheme_width_ascii_is_one_display_column() {
    assert_eq!(grapheme_width("a", 0, 4), 1);
    assert_eq!(grapheme_width("a", 7, 4), 1); // display-column-independent
}

#[test]
fn grapheme_width_wide_cjk_is_two_display_columns() {
    // U+6F22 (漢) is East Asian Wide.
    assert_eq!(grapheme_width("\u{6F22}", 0, 4), 2);
}

#[test]
fn grapheme_width_tab_advances_to_next_stop() {
    assert_eq!(grapheme_width("\t", 0, 4), 4);
    assert_eq!(grapheme_width("\t", 2, 4), 2);
}

#[test]
fn grapheme_width_decomposed_e_acute_is_one_display_column() {
    // "e" + U+0301 (combining acute accent) is ONE grapheme cluster
    // (unicode-segmentation merges them). The base 'e' contributes 1
    // display column; the combining mark contributes 0 — total 1, matching
    // how the character actually renders on screen.
    assert_eq!(grapheme_width("e\u{0301}", 0, 4), 1);
}

#[test]
fn grapheme_width_lone_combining_mark_clamps_to_one() {
    // A combining mark with no base character is a genuinely zero-width
    // cluster (measures 0 via unicode-width). Clamped up to 1 so it still
    // occupies an addressable cell.
    assert_eq!(grapheme_width("\u{0301}", 0, 4), 1);
}

#[test]
fn no_grapheme_cluster_exceeds_the_upper_clamp() {
    // `grapheme_width`'s upper clamp is defensive, not load-bearing: the
    // pinned `unicode-width` already measures every one of these multi-code-
    // point clusters as 2, so nothing reaches the clamp to be cut down. This
    // test exists to notice if that ever stops being true — a `unicode-width`
    // bump that started summing a ZWJ sequence's parts (5 code points, 3 of
    // them emoji) would silently turn one glyph into a 6-column cell, and the
    // clamp would start doing real work with no other test to say so.
    for cluster in [
        "\u{1F468}\u{200D}\u{1F469}\u{200D}\u{1F467}", // ZWJ family
        "\u{1F1EC}\u{1F1E7}",                          // regional-indicator flag
        "1\u{FE0F}\u{20E3}",                           // keycap
        "\u{1F44D}\u{1F3FD}",                          // emoji + skin-tone modifier
        "\u{1F3F4}\u{E0067}\u{E0062}\u{E0073}\u{E0063}\u{E0074}\u{E007F}", // tag flag
    ] {
        assert_eq!(
            cluster.graphemes(true).count(),
            1,
            "fixture must be a single cluster: {cluster:?}"
        );
        assert_eq!(
            unicode_width::UnicodeWidthStr::width(cluster),
            2,
            "unicode-width no longer caps this cluster at 2: {cluster:?}"
        );
        assert_eq!(grapheme_width(cluster, 0, 4), 2);
    }
}

// ── is_zero_width ────────────────────────────────────────────────────────

#[test]
fn is_zero_width_separates_undrawable_clusters_from_narrow_ones() {
    // The clusters a writer must substitute a placeholder for: each measures
    // 0, so writing its own glyph into the reserved cell would advance the
    // terminal by nothing.
    assert!(is_zero_width("\u{200B}")); // zero-width space
    assert!(is_zero_width("\u{200D}")); // zero-width joiner
    assert!(is_zero_width("\u{0301}")); // combining acute, no base

    // Ordinary clusters, including a base with a combining mark attached —
    // that pair draws as one visible glyph and must not be substituted.
    assert!(!is_zero_width("a"));
    assert!(!is_zero_width("e\u{0301}"));
    assert!(!is_zero_width("\u{6F22}"));

    // Control characters are *not* covered by this predicate: the pinned
    // `unicode-width` reports them as 1, so a writer has to test for them
    // separately rather than assuming a zero measure catches them.
    assert!(!is_zero_width("\t"));
    assert!(!is_zero_width("\n"));
}

// ── str_width ────────────────────────────────────────────────────────────

#[test]
fn str_width_no_tabs_sums_grapheme_widths() {
    // "ab" = 1 + 1.
    assert_eq!(str_width("ab", 0, 4), 2);
}

#[test]
fn str_width_tab_uses_running_display_col() {
    // "ab\tcd" at tw=4: a(1)->1, b(1)->2, tab from display col 2 advances 2
    // (to display col 4), c(1)->5, d(1)->6. Total width = 6.
    assert_eq!(str_width("ab\tcd", 0, 4), 6);
}

#[test]
fn str_width_wide_char_before_tab_shifts_the_stop() {
    // This is the case the git-diff plugin got wrong when it counted one
    // Steel char (not one display column) per preceding character: a wide
    // CJK char occupies 2 display columns, so the tab after it lands one
    // display column later than a naive char-count would predict.
    //
    // "\u{6F22}\tx" at tw=4: 漢(2)->2, tab from display col 2 advances 2 (to
    // display col 4), x(1)->5. Total width = 5 — not 4, which a
    // char-counting (not display-column-counting) walk would have produced.
    assert_eq!(str_width("\u{6F22}\tx", 0, 4), 5);
}

#[test]
fn str_width_honors_nonzero_start_display_col() {
    // Starting at display column 2, a tab (tw=4) advances 2 to reach
    // display column 4.
    assert_eq!(str_width("\t", 2, 4), 2);
}

// ── indent_depth ─────────────────────────────────────────────────────────

#[test]
fn indent_depth_two_spaces() {
    assert_eq!(indent_depth("  foo", 2), 1);
    assert_eq!(indent_depth("    foo", 2), 2);
    assert_eq!(indent_depth("foo", 2), 0);
}

#[test]
fn indent_depth_with_tabs() {
    // Two tabs with tab_width=4 => 2 indent levels.
    assert_eq!(indent_depth("\t\tfoo", 4), 2);
    // Mixed: tab (0→4) then space (4→5), depth = 5/4 = 1.
    assert_eq!(indent_depth("\t foo", 4), 1);
}

#[test]
fn indent_depth_zero_tab_width_no_panic() {
    // tab_width=0 should be clamped to 1 internally.
    let depth = indent_depth("  foo", 0);
    assert_eq!(depth, 2); // tw=1, col=2, depth=2
}

// ── truncate_to_width ────────────────────────────────────────────────────

#[test]
fn truncate_to_width_whole_string_fits() {
    assert_eq!(truncate_to_width("ab", 5, 4), ("ab", 2));
}

#[test]
fn truncate_to_width_exact_budget() {
    assert_eq!(truncate_to_width("abcd", 4, 4), ("abcd", 4));
}

#[test]
fn truncate_to_width_drops_a_whole_wide_cluster_that_would_overshoot() {
    // Each 漢 is 2 columns; budget 3 can only fit the first one (2 cols),
    // not half of the second.
    assert_eq!(truncate_to_width("\u{6F22}\u{6F22}", 3, 4), ("\u{6F22}", 2));
}

#[test]
fn truncate_to_width_tab_expands_against_real_stops() {
    // "a\tb" at tw=4: a(1)->1, tab from col 1 advances 3 (to col 4) -> "a\t"
    // is 4 columns, exactly the budget; 'b' would overshoot to 5.
    assert_eq!(truncate_to_width("a\tb", 4, 4), ("a\t", 4));
}

#[test]
fn truncate_to_width_zero_budget_is_empty() {
    assert_eq!(truncate_to_width("abc", 0, 4), ("", 0));
}

#[test]
fn truncate_to_width_never_splits_a_decomposed_cluster() {
    // "e" + combining acute is one grapheme cluster measuring 1 column;
    // budget 0 drops it whole rather than keeping the bare 'e'.
    assert_eq!(truncate_to_width("e\u{0301}", 0, 4), ("", 0));
    assert_eq!(truncate_to_width("e\u{0301}", 1, 4), ("e\u{0301}", 1));
}

// ── truncate_suffix_to_width ─────────────────────────────────────────────

#[test]
fn truncate_suffix_to_width_whole_string_fits() {
    assert_eq!(truncate_suffix_to_width("ab", 5, 1), ("ab", 2));
}

#[test]
fn truncate_suffix_to_width_keeps_the_tail() {
    assert_eq!(truncate_suffix_to_width("abc", 2, 1), ("bc", 2));
}

#[test]
fn truncate_suffix_to_width_drops_a_whole_wide_cluster_that_would_overshoot() {
    assert_eq!(
        truncate_suffix_to_width("\u{6F22}\u{6F22}", 3, 1),
        ("\u{6F22}", 2)
    );
}

#[test]
fn truncate_suffix_to_width_zero_budget_is_empty() {
    assert_eq!(truncate_suffix_to_width("abc", 0, 1), ("", 0));
}
