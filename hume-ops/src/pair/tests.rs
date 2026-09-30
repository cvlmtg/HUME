use super::find_tightest_bracket_pair;
use hume_editing::text::BufferText;
use hume_rope::offset::CharOffset;

// These assert find_tightest_bracket_pair's own (open, close) contract
// directly, rather than only through cmd_inner_argument in
// hume-ops/src/text_object/tests/argument.rs. That file's characterization
// tests stay (they pin `mia`/`maa`'s observable behavior), these pin the
// resolver's return value. Notably the dropped-type case below: through
// find_comma_segments, that case's expected span happens to come out right
// even under a wrong pair choice, since the segmenter's own depth-skip masks
// it; asserting the pair directly closes that gap.

fn resolve(text: &str, pos: usize) -> Option<(usize, usize)> {
    let text = BufferText::from(text);
    find_tightest_bracket_pair(&text, text.snap(CharOffset::new(pos)))
        .map(|r| (r.start().offset().index(), r.last().offset().index()))
}

#[test]
fn crossed_nesting_picks_the_smallest_span_not_the_nearest_open() {
    // `(` at 1 is the nearest unmatched open, but its partner `)` (absent
    // here: the trailing `)` at 10 is unmatched) gives `()` a span of 9.
    // `{}` at (0, 5) is tighter and wins, even though its open is farther
    // from the cursor than `(`'s.
    assert_eq!(resolve("{(abc}    )\n", 3), Some((0, 5)));
}

#[test]
fn crossed_nesting_equal_spans_break_the_tie_in_bracket_pairs_order() {
    // `()` = (0, 3) and `{}` = (1, 4) are both span 3. `BRACKET_PAIRS` lists
    // `()` first, so it wins the tie.
    assert_eq!(resolve("({a)}\n", 2), Some((0, 3)));
}

#[test]
fn bracket_type_with_no_closing_bracket_is_dropped_not_ranked() {
    // `{` at 1 has no matching `}` anywhere, so `{}` is dropped from the
    // candidate set entirely, not treated as "nearest open, unmatched close
    // ignored". `()` = (0, 8) wins by being the only resolved candidate.
    assert_eq!(resolve("({aaa, b)\n", 3), Some((0, 8)));
}

#[test]
fn cursor_on_an_open_bracket_only_shortcuts_that_type() {
    // Cursor sits on `(`: only `()` takes the on-open shortcut (span found
    // without scanning left; right scan starts at pos + 1, past the `(`
    // itself). `[]` isn't on the cursor's char, so it still scans both
    // directions and resolves to the wider (0, 7).
    assert_eq!(resolve("[(a, b)]\n", 1), Some((1, 6)));
}

#[test]
fn cursor_on_a_close_bracket_only_shortcuts_that_type() {
    // Mirror of the above: cursor sits on `)`, only `()` takes the
    // on-close shortcut.
    assert_eq!(resolve("[(a, b)]\n", 6), Some((1, 6)));
}

#[test]
fn on_open_shortcut_still_scans_right_when_no_other_type_would() {
    // Only `()` is present: no other type's normal (non-shortcut)
    // resolution can incidentally start the rightward scan on its behalf.
    // The on-open shortcut must mark this type as needing its close found
    // just like the normal path does, or the rightward scan never starts
    // and `)` at 5 is never found.
    assert_eq!(resolve("(a, b)\n", 0), Some((0, 5)));
}

#[test]
fn on_close_shortcut_needs_no_further_scan_when_no_other_type_would() {
    // Mirror of the above: only `()` is present, cursor on `)`. The
    // on-close shortcut resolves the pair the moment the leftward scan
    // finds `(`, with no rightward scan needed at all.
    assert_eq!(resolve("(a, b)\n", 5), Some((0, 5)));
}

#[test]
fn smallest_span_can_resolve_after_a_larger_candidate_already_did() {
    // `}` at 6 resolves `{}` = (0, 6) span 6 first, but `()` = (4, 9) span 5
    // resolves later scanning the same rightward pass and wins. A resolver
    // that stopped at the first type to resolve (rather than every
    // surviving type) would return the `{}` span instead.
    assert_eq!(resolve("{xxx(a}xx)\n", 5), Some((4, 9)));
}

#[test]
fn no_enclosing_bracket_of_any_type_is_none() {
    assert_eq!(resolve("abc\n", 1), None);
}

/// The partner the bracket nearest the head of `marked`'s primary selection
/// resolves to, as a char offset.
fn partner(marked: &str) -> Option<usize> {
    let (text, sels) = test_fixtures::testing::parse_state(marked);
    let state = test_fixtures::testing::state(text, sels);
    super::matching_bracket(state.view().primary()).map(|pos| pos.offset().index())
}

#[test]
fn matching_bracket_lands_on_the_cluster_holding_a_prepended_open() {
    // U+0600 is a prepend mark: it joins the `(` after it into one cluster
    // starting at 0. The partner of `)` is that cluster, not the `(` char.
    assert_eq!(partner("\u{600}(x-[)]>\n"), Some(0));
}

#[test]
fn matching_bracket_from_a_prepended_open_cluster_finds_the_close() {
    assert_eq!(partner("-[\u{600}(]>x)\n"), Some(3));
}
