use super::*;
use crate::column::ByteCol;
use crate::spanned::SpannedText;
use hume_core::TruncateEnd;
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
    // A tab sitting exactly on a stop still advances a full tw, not 0:
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
    // never computes a "next stop" value that could itself overflow,
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
    // a no-op: the mirror of tab_advance never advancing by zero.
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
    // (k-1) stops, the same relationship tab_advance has going forward.
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
    // display column; the combining mark contributes 0. Total 1, matching
    // how the character actually renders on screen.
    assert_eq!(grapheme_width("e\u{0301}", 0, 4), 1);
}

#[test]
fn grapheme_width_of_an_unrenderable_cluster_is_its_placeholder() {
    // A combining mark with no base character measures 0 and cannot be
    // drawn as itself, so it occupies its `<301>` placeholder: five cells,
    // not the one cell a clamp would have given it.
    assert_eq!(grapheme_width("\u{0301}", 0, 4), 5);
    assert_eq!(placeholder("\u{0301}").as_str(), "<301>");

    // A control character is the same case even though it measures 1.
    assert_eq!(grapheme_width("\u{1b}", 0, 4), 4);
    assert_eq!(placeholder("\u{1b}").as_str(), "<1b>");
}

#[test]
fn no_grapheme_cluster_exceeds_the_upper_cap() {
    // `grapheme_width`'s upper cap is defensive: the
    // pinned `unicode-width` already measures every one of these multi-code-
    // point clusters as 2, so nothing reaches the cap to be cut down. This
    // test exists to notice if that ever stops being true: a `unicode-width`
    // bump that started summing a ZWJ sequence's parts (5 code points, 3 of
    // them emoji) would silently turn one glyph into a 6-column cell, and the
    // cap would start doing real work with no other test to say so.
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

// ── needs_placeholder / placeholder ──────────────────────────────────────

#[test]
fn needs_placeholder_covers_both_reasons_a_cluster_cannot_be_drawn() {
    // Measures zero: written as itself the terminal advances nothing and the
    // rest of the row slides left.
    assert!(needs_placeholder("\u{200B}")); // zero-width space
    assert!(needs_placeholder("\u{200D}")); // zero-width joiner
    assert!(needs_placeholder("\u{0301}")); // combining acute, no base

    // Holds a control character: the terminal would act on it. These measure
    // *1*, not 0 (the pinned `unicode-width` gives any character no other
    // rule claims a width of 1), so a zero-measure test alone would let an
    // ESC through to the terminal.
    assert!(needs_placeholder("\t"));
    assert!(needs_placeholder("\n"));
    assert!(needs_placeholder("\u{1b}"));
    assert_eq!(unicode_width::UnicodeWidthStr::width("\u{1b}"), 1);

    // Ordinary clusters, including a base with a combining mark attached:
    // that pair draws as one visible glyph and must not be substituted.
    assert!(!needs_placeholder("a"));
    assert!(!needs_placeholder("e\u{0301}"));
    assert!(!needs_placeholder("\u{6F22}"));
}

#[test]
fn needs_placeholder_covers_the_bidi_overrides() {
    // The Trojan Source characters (CVE-2021-42574) are
    // `Default_Ignorable_Code_Point`s, the same class as a zero-width space,
    // so the same rule catches them. Showing the codepoint rather than a
    // blank is the whole point: an override that rendered like a space would
    // be invisible exactly where it matters.
    for bidi in [
        "\u{202A}", "\u{202B}", "\u{202D}", "\u{202E}", "\u{2066}", "\u{2069}",
    ] {
        assert_eq!(
            unicode_width::UnicodeWidthStr::width(bidi),
            0,
            "expected a zero measure for {bidi:?}"
        );
        assert!(needs_placeholder(bidi));
    }
    assert_eq!(placeholder("\u{202E}").as_str(), "<202e>");
}

#[test]
fn placeholder_is_the_codepoint_in_angle_brackets() {
    // The form Vim and Neovim show. Lowercase hex, no padding, so the width
    // varies with the codepoint, which is why `grapheme_width` reports the
    // placeholder's own length rather than a constant.
    assert_eq!(placeholder("\u{200B}").as_str(), "<200b>");
    assert_eq!(placeholder("\u{7}").as_str(), "<7>");
    // The widest form there is, which is what the inline buffer is sized
    // for: six hex digits and two brackets.
    assert_eq!(placeholder("\u{10FFFF}").as_str(), "<10ffff>");

    // Every placeholder is ASCII, so its byte length is its display width,
    // and that is the width `grapheme_width` reports for a cluster needing
    // one. `\u{10FFFF}` is not in this list: it is unassigned rather than
    // unrenderable, measures 1, and the terminal draws it as tofu. Visible
    // and one cell, so it misaligns nothing and needs no substitution.
    for cluster in ["\u{200B}", "\u{7}", "\u{202E}", "\u{1b}"] {
        assert!(needs_placeholder(cluster));
        let p = placeholder(cluster);
        assert!(p.as_str().is_ascii());
        assert_eq!(grapheme_width(cluster, 0, 4), p.as_str().len());
    }
    assert!(!needs_placeholder("\u{10FFFF}"));
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
    // display col 4), x(1)->5. Total width = 5, not 4, which a
    // char-counting (not display-column-counting) walk would have produced.
    assert_eq!(str_width("\u{6F22}\tx", 0, 4), 5);
}

#[test]
fn str_width_honors_nonzero_start_display_col() {
    // Starting at display column 2, a tab (tw=4) advances 2 to reach
    // display column 4.
    assert_eq!(str_width("\t", 2, 4), 2);
}

#[test]
fn str_width_of_a_placeholder_cluster_is_its_codepoint_length() {
    // A zero-width space (U+200B) measures 0 by unicode-width, but its
    // `<200b>` placeholder occupies 6 cells. `str_width` must report the
    // placeholder's width, not the cluster's own zero.
    assert_eq!(str_width("\u{200b}", 0, 4), 6);
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
fn indent_depth_saturates_at_u8_max() {
    // 300 spaces at tab_width=1 is 300 indent levels, past u8's 255 max.
    // Must saturate, not wrap or panic.
    let indent = " ".repeat(300);
    assert_eq!(indent_depth(&indent, 1), u8::MAX);
}

#[test]
fn indent_depth_zero_tab_width_no_panic() {
    // tab_width=0 should be clamped to 1 internally.
    let depth = indent_depth("  foo", 0);
    assert_eq!(depth, 2); // tw=1, col=2, depth=2
}

// ── indent_stop ──────────────────────────────────────────────────────────

#[test]
fn indent_stop_is_the_inverse_of_indent_depth() {
    // indent_depth("\t\tfoo", 4) == 2; the depth-2 stop is display col 8.
    assert_eq!(indent_stop(2, 4), 8);
    assert_eq!(indent_depth("\t\tfoo", 4), indent_stop(2, 4) as u8 / 4);
}

#[test]
fn indent_stop_zero_k_is_column_zero() {
    assert_eq!(indent_stop(0, 4), 0);
}

#[test]
fn indent_stop_zero_tab_width_clamps_to_one() {
    // Mirrors indent_depth_zero_tab_width_no_panic's clamp, so the two
    // functions agree at tab_width == 0 instead of one silently collapsing
    // to column 0 while the other doesn't.
    assert_eq!(indent_stop(3, 0), 3);
}

#[test]
fn indent_levels_rounds_down_to_whole_stops() {
    assert_eq!(indent_levels(0, 4), 0);
    assert_eq!(indent_levels(7, 4), 1);
    assert_eq!(indent_levels(8, 4), 2);
}

#[test]
fn indent_levels_zero_tab_width_counts_columns() {
    assert_eq!(indent_levels(3, 0), 3);
}

// ── truncate_to_width ────────────────────────────────────────────────────

#[test]
fn truncate_to_width_whole_string_fits() {
    assert_eq!(truncate_to_width("ab", 5), ("ab", 2));
}

#[test]
fn truncate_to_width_exact_budget() {
    assert_eq!(truncate_to_width("abcd", 4), ("abcd", 4));
}

#[test]
fn truncate_to_width_drops_a_whole_wide_cluster_that_would_overshoot() {
    // Each 漢 is 2 columns; budget 3 can only fit the first one (2 cols),
    // not half of the second.
    assert_eq!(truncate_to_width("\u{6F22}\u{6F22}", 3), ("\u{6F22}", 2));
}

#[test]
fn truncate_to_width_counts_a_tab_as_one_cell() {
    // Chrome has no tab stops: a(1) + tab(1) + b(1) is 3 cells.
    assert_eq!(truncate_to_width("a\tb", 4), ("a\tb", 3));
    assert_eq!(truncate_to_width("a\tb", 2), ("a\t", 2));
}

#[test]
fn truncate_to_width_zero_budget_is_empty() {
    assert_eq!(truncate_to_width("abc", 0), ("", 0));
}

#[test]
fn truncate_to_width_drops_a_whole_placeholder_that_would_overshoot() {
    // 'a' (1 col) fits budget 4, but the zero-width space's `<200b>`
    // placeholder needs 6 more (1+6=7 > 4). Dropped whole, same rule a
    // wide grapheme follows, never split into "<20".
    assert_eq!(truncate_to_width("a\u{200b}", 4), ("a", 1));
}

#[test]
fn truncate_to_width_never_splits_a_decomposed_cluster() {
    // "e" + combining acute is one grapheme cluster measuring 1 column;
    // budget 0 drops it whole rather than keeping the bare 'e'.
    assert_eq!(truncate_to_width("e\u{0301}", 0), ("", 0));
    assert_eq!(truncate_to_width("e\u{0301}", 1), ("e\u{0301}", 1));
}

// ── truncate_suffix_to_width ─────────────────────────────────────────────

#[test]
fn truncate_suffix_to_width_whole_string_fits() {
    assert_eq!(truncate_suffix_to_width("ab", 5), ("ab", 2));
}

#[test]
fn truncate_suffix_to_width_keeps_the_tail() {
    assert_eq!(truncate_suffix_to_width("abc", 2), ("bc", 2));
}

#[test]
fn truncate_suffix_to_width_drops_a_whole_wide_cluster_that_would_overshoot() {
    assert_eq!(
        truncate_suffix_to_width("\u{6F22}\u{6F22}", 3),
        ("\u{6F22}", 2)
    );
}

#[test]
fn truncate_suffix_to_width_zero_budget_is_empty() {
    assert_eq!(truncate_suffix_to_width("abc", 0), ("", 0));
}

#[test]
fn indent_depth_counts_non_breaking_and_ideographic_spaces() {
    assert_eq!(indent_depth("\u{a0}\u{a0}\u{a0}\u{a0}x", 4), 1);
    assert_eq!(indent_depth("\u{3000}\u{3000}x", 4), 1);
}

// ── Corpus coverage ───────────────────────────────────────────────────────

#[test]
fn grapheme_width_of_single_cluster_samples() {
    use test_fixtures::unicode::*;
    for (sample, expected) in [
        (PRECOMPOSED, 1),
        (COMBINING, 1),
        (STACKED, 1),
        (CJK, 2),
        (FULLWIDTH, 2),
        (ASTRAL, 2),
        (FLAG_IT, 2),
        (ZWJ_FAMILY, 2),
        (SKIN_TONE, 2),
        (NBSP, 1),
        (IDEO_SPACE, 2),
    ] {
        assert_eq!(grapheme_width(sample, 0, 4), expected, "{sample:?}");
    }
}

#[test]
fn chrome_width_measures_from_column_zero_with_one_cell_tabs() {
    use test_fixtures::unicode::*;
    assert_eq!(chrome_width("a\tb"), 3);
    assert_eq!(chrome_width(CJK), 2);
    assert_eq!(chrome_width(&format!("{ZWJ_FAMILY}{COMBINING}")), 3);
    assert_eq!(chrome_width(""), 0);
}

#[test]
fn chrome_cluster_width_of_single_clusters() {
    use test_fixtures::unicode::*;
    assert_eq!(chrome_cluster_width("\t"), 1);
    assert_eq!(chrome_cluster_width(CJK), 2);
    assert_eq!(chrome_cluster_width(COMBINING), 1);
    assert_eq!(chrome_cluster_width(FLAG_IT), 2);
}

#[test]
fn str_width_sums_clusters_not_chars() {
    use test_fixtures::unicode::*;
    assert_eq!(str_width(&format!("{COMBINING}x"), 0, 4), 2);
    assert_eq!(str_width(&format!("{ZWJ_FAMILY}x"), 0, 4), 3);
    assert_eq!(str_width(&format!("{FLAG_RUN}x"), 0, 4), 5);
    assert_eq!(str_width(&format!("{CJK}{FULLWIDTH}"), 0, 4), 4);
}

#[test]
fn truncate_suffix_to_width_never_splits_a_cluster() {
    use test_fixtures::unicode::*;
    let s = format!("ab{COMBINING}");
    assert_eq!(truncate_suffix_to_width(&s, 1), (COMBINING, 1));
    let s = format!("ab{ZWJ_FAMILY}");
    assert_eq!(truncate_suffix_to_width(&s, 2), (ZWJ_FAMILY, 2));
    assert_eq!(truncate_suffix_to_width(&s, 1), ("", 0));
    let s = format!("x{CJK}");
    assert_eq!(truncate_suffix_to_width(&s, 1), ("", 0));
}

#[test]
fn truncate_to_width_never_splits_a_cluster() {
    use test_fixtures::unicode::*;
    let s = format!("{COMBINING}bc");
    assert_eq!(truncate_to_width(&s, 2), (&s[..s.len() - 1], 2));
    let s = format!("{ZWJ_FAMILY}b");
    assert_eq!(truncate_to_width(&s, 1), ("", 0));
}

/// A space carrying a combining mark is one blank cluster, as
/// `leading_indent` measures it, so the tab after it still counts.
#[test]
fn indent_depth_measures_a_space_with_a_combining_mark_as_one_blank_cluster() {
    assert_eq!(indent_depth(" \u{301}\tx", 4), 1);
}

// ── truncate_marked ─────────────────────────────────────────────────────

#[test]
fn truncate_marked_leaves_short_strings_unchanged() {
    assert_eq!(truncate_marked("hello", 8, TruncateEnd::Head), "hello");
    assert_eq!(
        truncate_marked("hello", 5, TruncateEnd::Head),
        "hello",
        "exact fit needs no marker"
    );
}

#[test]
fn truncate_marked_cut_head_prefixes_ellipsis_and_keeps_tail() {
    let source = "hume-ui/src/picker_panel.rs";
    let out = truncate_marked(source, 12, TruncateEnd::Head);
    assert_eq!(oracle_width(out.as_ref()), 12);
    let kept = out
        .strip_prefix('…')
        .unwrap_or_else(|| panic!("clipped string must lead with …, got {out:?}"));
    assert!(
        source.ends_with(kept),
        "the kept part must be a genuine tail of the source string, got {out:?}"
    );
}

#[test]
fn truncate_marked_cut_tail_appends_ellipsis_and_keeps_head() {
    let source = "src/editor/picker.rs:412:9:  fn push(&mut self)";
    let out = truncate_marked(source, 12, TruncateEnd::Tail);
    assert_eq!(oracle_width(out.as_ref()), 12);
    let kept = out
        .strip_suffix('…')
        .unwrap_or_else(|| panic!("clipped string must trail with …, got {out:?}"));
    assert!(
        source.starts_with(kept),
        "the kept part must be a genuine head of the source string, got {out:?}"
    );
}

#[test]
fn truncate_marked_never_splits_a_grapheme_cluster() {
    use unicode_segmentation::UnicodeSegmentation;

    // "é" here is e + combining acute (U+0065 U+0301). A cut landing inside
    // this cluster would emit a bare combining mark with no base character
    // ahead of it, which re-segmenting the output would catch: a bare mark
    // opens its own (invalid) cluster instead of extending the previous one.
    let source = "cafe\u{0301} bar";
    for cut in [TruncateEnd::Head, TruncateEnd::Tail] {
        // Every clip width up to the source's own, including one that lands
        // mid-cluster if the code were byte-counting instead of grapheme-aware.
        for budget in 0..=8 {
            let out = truncate_marked(source, budget, cut);
            assert!(
                out.graphemes(true).all(|g| g != "\u{0301}"),
                "cut={cut:?} budget={budget}: combining accent split from its base, got {out:?}"
            );
        }
    }
}

#[test]
fn truncate_marked_zero_budget_is_empty() {
    assert_eq!(truncate_marked("anything", 0, TruncateEnd::Head), "");
    assert_eq!(truncate_marked("anything", 0, TruncateEnd::Tail), "");
}

/// Measures width without going through the code under test.
fn oracle_width(s: &str) -> usize {
    unicode_width::UnicodeWidthStr::width(s)
}

// ── truncate_spanned ────────────────────────────────────────────────────

fn spanned(text: &str, spans: Vec<(usize, usize, char)>) -> SpannedText<char> {
    SpannedText::new(text.to_string(), spans).expect("spans fit their text")
}

/// Each span as `(start char, end char, payload)` of the text it is in.
fn char_spans(out: &SpannedText<char>) -> Vec<(usize, usize, char)> {
    let chars = |byte: ByteCol| out.text()[..byte.index()].chars().count();
    out.spans()
        .iter()
        .map(|(range, payload)| (chars(range.start), chars(range.end), *payload))
        .collect()
}

#[test]
fn a_row_that_fits_comes_back_unchanged() {
    let row = spanned("abcdef", vec![(1, 3, 'x')]);
    assert_eq!(truncate_spanned(row.clone(), 6, TruncateEnd::Head), row);
    assert_eq!(truncate_spanned(row.clone(), 6, TruncateEnd::Tail), row);
}

#[test]
fn a_head_cut_keeps_the_surviving_part_of_a_span_after_the_marker() {
    let out = truncate_spanned(
        spanned("abcdefgh", vec![(0, 2, 'a'), (3, 6, 'b')]),
        5,
        TruncateEnd::Head,
    );
    insta::assert_snapshot!(out.text(), @"…efgh");
    assert_eq!(char_spans(&out), vec![(1, 3, 'b')]);
}

#[test]
fn a_tail_cut_clips_a_span_at_the_kept_end_and_leaves_the_marker_unstyled() {
    let out = truncate_spanned(
        spanned("abcdefgh", vec![(2, 6, 'b'), (7, 8, 'c')]),
        5,
        TruncateEnd::Tail,
    );
    insta::assert_snapshot!(out.text(), @"abcd…");
    assert_eq!(char_spans(&out), vec![(2, 4, 'b')]);
}

#[test]
fn a_zero_budget_is_an_empty_row_and_a_budget_of_one_is_the_marker() {
    let row = spanned("abcdef", vec![(0, 6, 'x')]);
    let empty = truncate_spanned(row.clone(), 0, TruncateEnd::Head);
    assert_eq!((empty.text(), empty.spans().len()), ("", 0));
    let marker = truncate_spanned(row, 1, TruncateEnd::Tail);
    insta::assert_snapshot!(marker.text(), @"…");
    assert!(marker.spans().is_empty());
}

#[test]
fn a_wide_cluster_at_the_cut_is_dropped_whole() {
    // "日" takes two cells: a three-cell budget keeps "a" and the marker.
    let out = truncate_spanned(spanned("a日b", vec![(1, 2, 'w')]), 3, TruncateEnd::Tail);
    insta::assert_snapshot!(out.text(), @"a…");
    assert!(out.spans().is_empty());
}
