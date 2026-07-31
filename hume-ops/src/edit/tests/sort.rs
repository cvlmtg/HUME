use super::super::*;
use hume_test_fixtures::assert_state;
use pretty_assertions::assert_eq;

// ── sort_rows ─────────────────────────────────────────────────────────────────

#[test]
fn sort_whole_lines_selected_as_one_multiline_span() {
    // Multi-row selection: keeps its char range unchanged — the group's total
    // length is invariant under a row permutation, so the same bracket
    // positions still bound the (now reordered) block.
    assert_state!(
        "-[banana\napple\ncherry\n]>",
        |(buf, sels)| sort_rows(buf, sels, SortOpts::default()).unwrap(),
        "-[apple\nbanana\ncherry\n]>"
    );
}

#[test]
fn sort_swaps_whole_rows_keyed_by_a_single_char_each() {
    // The motivating case: two 1-char selections on adjacent rows swap the
    // whole rows they sit on, even though the surrounding text (`C`/`D` vs
    // `F`/`G`) has nothing to do with the sort order.
    assert_state!(
        "C -[B]> D\nF -[A]> G\n",
        |(buf, sels)| sort_rows(buf, sels, SortOpts::default()).unwrap(),
        "F -[A]> G\nC -[B]> D\n"
    );
}

#[test]
fn sort_groups_are_independent_across_a_gap() {
    // Two non-adjacent groups (separated by an unselected line) sort
    // independently — the gap line is untouched and no text crosses it.
    assert_state!(
        "-[b]>\n-[a]>\nx\n-[d]>\n-[c]>\n",
        |(buf, sels)| sort_rows(buf, sels, SortOpts::default()).unwrap(),
        "-[a]>\n-[b]>\nx\n-[c]>\n-[d]>\n"
    );
}

#[test]
fn sort_single_row_group_is_refused() {
    // Validity: a single-row group can't be permuted — flip this to a
    // 2-adjacent-row selection and the refusal disappears.
    let (buf, sels) = hume_test_fixtures::testing::parse_state("-[a]>\nx\n-[b]>\n");
    assert_eq!(
        sort_rows(buf, sels, SortOpts::default()),
        Err(SortRefusal::NoAdjacentRows)
    );
}

#[test]
fn sort_already_ordered_input_is_refused() {
    // Validity: an identity edit would still push an undo revision and mark
    // a clean buffer dirty (`History::record` has no identity guard) — this
    // refusal is what lets the caller skip applying anything.
    let (buf, sels) = hume_test_fixtures::testing::parse_state("-[a]>\n-[b]>\n");
    assert_eq!(
        sort_rows(buf, sels, SortOpts::default()),
        Err(SortRefusal::AlreadySorted)
    );
}

#[test]
fn sort_reverse_flips_the_order() {
    assert_state!(
        "-[a]>\n-[b]>\n",
        |(buf, sels)| sort_rows(
            buf,
            sels,
            SortOpts {
                reverse: true,
                ..Default::default()
            }
        )
        .unwrap(),
        "-[b]>\n-[a]>\n"
    );
}

#[test]
fn sort_insensitive_folds_case_for_comparison_only() {
    // `-i` only changes the comparison — the output keeps the original case.
    assert_state!(
        "-[Banana]>\n-[apple]>\n",
        |(buf, sels)| sort_rows(
            buf,
            sels,
            SortOpts {
                insensitive: true,
                ..Default::default()
            }
        )
        .unwrap(),
        "-[apple]>\n-[Banana]>\n"
    );
}

#[test]
fn sort_numeric_auto_detects_and_orders_correctly() {
    assert_state!(
        "-[2]>\n-[10]>\n-[1]>\n",
        |(buf, sels)| sort_rows(buf, sels, SortOpts::default()).unwrap(),
        "-[1]>\n-[2]>\n-[10]>\n"
    );

    // Independent oracle: pins numeric detection actually firing. A pure
    // lexicographic sort of the same three strings produces a different
    // order ("1", "10", "2") — if numeric detection silently stopped firing,
    // the assertion above would start seeing this order instead.
    let mut lexicographic = vec!["2", "10", "1"];
    lexicographic.sort();
    assert_eq!(lexicographic, vec!["1", "10", "2"]);
}

#[test]
fn sort_decimal_keys_order_numerically() {
    assert_state!(
        "-[9.5]>\n-[10.2]>\n-[2.75]>\n",
        |(buf, sels)| sort_rows(buf, sels, SortOpts::default()).unwrap(),
        "-[2.75]>\n-[9.5]>\n-[10.2]>\n"
    );

    // Independent oracle: pins float detection actually firing. A pure
    // lexicographic sort of the same three strings produces a different
    // order ("10.2", "2.75", "9.5") — if float detection silently stopped
    // firing, the assertion above would start seeing this order instead.
    let mut lexicographic = vec!["9.5", "10.2", "2.75"];
    lexicographic.sort();
    assert_eq!(lexicographic, vec!["10.2", "2.75", "9.5"]);
}

#[test]
fn sort_non_finite_float_keys_fall_back_to_lexicographic() {
    // "inf" parses as a float but isn't order-total, so the `is_finite`
    // guard must reject it and fall the whole group back to text order —
    // "10.5" < "2.5" < "inf" — rather than numeric order, which would put
    // "2.5" before "10.5" (2.5 < 10.5 < inf). The two orders disagree on
    // "2.5" vs "10.5", so a dropped guard shows up as a wrong result here,
    // not just a coincidentally-matching one.
    assert_state!(
        "-[2.5]>\n-[inf]>\n-[10.5]>\n",
        |(buf, sels)| sort_rows(buf, sels, SortOpts::default()).unwrap(),
        "-[10.5]>\n-[2.5]>\n-[inf]>\n"
    );
}

#[test]
fn sort_mixed_numeric_and_text_keys_falls_back_to_lexicographic() {
    // One non-numeric key ("a") disqualifies the whole group from numeric
    // comparison — the group falls back to plain string order.
    assert_state!(
        "-[2]>\n-[a]>\n-[1]>\n",
        |(buf, sels)| sort_rows(buf, sels, SortOpts::default()).unwrap(),
        "-[1]>\n-[2]>\n-[a]>\n"
    );
}

#[test]
fn sort_is_stable_for_equal_keys() {
    // Two rows share the key "b" (the trailing digit is outside the
    // selection, so it never enters the key); they keep their original
    // relative order after the sort.
    assert_state!(
        "-[b]>1\n-[a]>\n-[b]>2\n",
        |(buf, sels)| sort_rows(buf, sels, SortOpts::default()).unwrap(),
        "-[a]>\n-[b]>1\n-[b]>2\n"
    );
}

#[test]
fn sort_follows_a_selection_at_a_nonzero_column() {
    // The row moves verbatim, so a selection partway through its line keeps
    // the same column offset on its new line.
    assert_state!(
        "xx-[b]>\nyy-[a]>\n",
        |(buf, sels)| sort_rows(buf, sels, SortOpts::default()).unwrap(),
        "yy-[a]>\nxx-[b]>\n"
    );
}

#[test]
fn sort_compound_key_from_two_selections_on_one_row() {
    // Two selections on the same row concatenate into one key, in document
    // order — neither selection is discarded.
    assert_state!(
        "-[b]> -[2]> x\n-[a1]> y\n",
        |(buf, sels)| sort_rows(buf, sels, SortOpts::default()).unwrap(),
        "-[a1]> y\n-[b]> -[2]> x\n"
    );
}

#[test]
fn sort_preserves_combining_grapheme_clusters_through_remap() {
    // `sel.end_inclusive` extends a collapsed cursor on 'e' through the
    // combining acute accent that follows it, so the key ("e\u{0301}") and
    // the post-sort remap both cover the whole grapheme, not just 'e'.
    assert_state!(
        "-[e]>\u{0301}\n-[a]>\n",
        |(buf, sels)| sort_rows(buf, sels, SortOpts::default()).unwrap(),
        "-[a]>\n-[e]>\u{0301}\n"
    );
}

#[test]
fn sort_blank_line_inside_a_run_gets_an_empty_key_and_sorts_first() {
    // One multi-line selection spans all three rows ("b", the blank line,
    // "a"). The blank row has no content to key on — its key is "" — so it
    // sorts ahead of both letters. The selection spans multiple rows, so it
    // keeps its char range unchanged and still wraps the whole reordered block.
    assert_state!(
        "-[b\n\na\n]>",
        |(buf, sels)| sort_rows(buf, sels, SortOpts::default()).unwrap(),
        "-[\na\nb\n]>"
    );
}
