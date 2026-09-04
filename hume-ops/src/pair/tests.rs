use super::find_tightest_bracket_pair;
use hume_editing::text::BufferText;

// These assert find_tightest_bracket_pair's own (open, close) contract
// directly, rather than only through cmd_inner_argument in
// hume-ops/src/text_object/tests/argument.rs — that file's characterization
// tests stay (they pin `mia`/`maa`'s observable behavior), these pin the
// resolver's return value. Notably the dropped-type case below: through
// find_comma_segments, that case's expected span happens to come out right
// even under a wrong pair choice, since the segmenter's own depth-skip masks
// it — asserting the pair directly closes that gap.

fn resolve(text: &str, pos: usize) -> Option<(usize, usize)> {
    find_tightest_bracket_pair(&BufferText::from(text), pos)
}

#[test]
fn crossed_nesting_picks_the_smallest_span_not_the_nearest_open() {
    // `(` at 1 is the nearest unmatched open, but its partner `)` (absent
    // here — the trailing `)` at 10 is unmatched) gives `()` a span of 9.
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
    // candidate set entirely — not treated as "nearest open, unmatched close
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
