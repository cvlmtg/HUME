use super::super::*;
use hume_test_fixtures::assert_state;

// `inner_argument`/`around_argument` register from `register_structural`
// (hume-editor's `commands/structural.rs`) as closures, not through a
// `cmd_*` wrapper — these two exist only so the tests below can drive the
// bare functions through the same `apply_text_object_by_mode` dispatch the
// shipped closures use.
fn cmd_inner_argument(
    text: &BufferText,
    sels: SelectionSet,
    _count: usize,
    mode: MotionMode,
) -> SelectionSet {
    apply_text_object_by_mode(text, sels, mode, inner_argument)
}

fn cmd_around_argument(
    text: &BufferText,
    sels: SelectionSet,
    _count: usize,
    mode: MotionMode,
) -> SelectionSet {
    apply_text_object_by_mode(text, sels, mode, around_argument)
}

// ── Arguments ─────────────────────────────────────────────────────────────

// ── inner_argument ────────────────────────────────────────────────────────

#[test]
fn inner_argument_first() {
    assert_state!(
        "foo(-[a]>aa, bbb, ccc)\n",
        |(text, sels)| cmd_inner_argument(&text, sels, 0, MotionMode::Move),
        "foo(-[aaa]>, bbb, ccc)\n"
    );
}

#[test]
fn inner_argument_middle() {
    assert_state!(
        "foo(aaa, -[b]>bb, ccc)\n",
        |(text, sels)| cmd_inner_argument(&text, sels, 0, MotionMode::Move),
        "foo(aaa, -[bbb]>, ccc)\n"
    );
}

#[test]
fn inner_argument_last() {
    assert_state!(
        "foo(aaa, bbb, -[c]>cc)\n",
        |(text, sels)| cmd_inner_argument(&text, sels, 0, MotionMode::Move),
        "foo(aaa, bbb, -[ccc]>)\n"
    );
}

#[test]
fn inner_argument_single() {
    assert_state!(
        "foo(-[a]>aa)\n",
        |(text, sels)| cmd_inner_argument(&text, sels, 0, MotionMode::Move),
        "foo(-[aaa]>)\n"
    );
}

#[test]
fn inner_argument_trims_whitespace() {
    // Leading/trailing spaces inside the segment are excluded.
    assert_state!(
        "foo(  -[a]>aa  , bbb)\n",
        |(text, sels)| cmd_inner_argument(&text, sels, 0, MotionMode::Move),
        "foo(  -[aaa]>  , bbb)\n"
    );
}

#[test]
fn inner_argument_nested_parens_skips_inner_comma() {
    // The comma inside bar(x, y) is at depth 1 — not a segment boundary.
    assert_state!(
        "foo(-[b]>ar(x, y), z)\n",
        |(text, sels)| cmd_inner_argument(&text, sels, 0, MotionMode::Move),
        "foo(-[bar(x, y)]>, z)\n"
    );
}

#[test]
fn inner_argument_nested_brackets_skips_inner_comma() {
    assert_state!(
        "foo(-[b]>ar[x, y], z)\n",
        |(text, sels)| cmd_inner_argument(&text, sels, 0, MotionMode::Move),
        "foo(-[bar[x, y]]>, z)\n"
    );
}

#[test]
fn inner_argument_nested_braces_skips_inner_comma() {
    // The comma inside {a: 1, b: 2} is at depth 1 — not a segment boundary.
    // Cursor in the second argument selects "ccc", not something split by the inner comma.
    assert_state!(
        "foo({a: 1, b: 2}, cc-[c]>)\n",
        |(text, sels)| cmd_inner_argument(&text, sels, 0, MotionMode::Move),
        "foo({a: 1, b: 2}, -[ccc]>)\n"
    );
}

#[test]
fn inner_argument_picks_tightest_bracket_pair() {
    // The cursor is inside (aaa, bbb) which is itself inside [...].
    // The tightest enclosing pair is (), not [].
    assert_state!(
        "[(aaa, -[b]>bb), ccc]\n",
        |(text, sels)| cmd_inner_argument(&text, sels, 0, MotionMode::Move),
        "[(aaa, -[bbb]>), ccc]\n"
    );
}

#[test]
fn inner_argument_cursor_on_comma_associates_with_next() {
    // Cursor on the comma — treated as belonging to the following segment.
    assert_state!(
        "foo(aaa-[,]> bbb)\n",
        |(text, sels)| cmd_inner_argument(&text, sels, 0, MotionMode::Move),
        "foo(aaa, -[bbb]>)\n"
    );
}

#[test]
fn inner_argument_cursor_on_open_bracket() {
    assert_state!(
        "foo-[(]>aaa, bbb)\n",
        |(text, sels)| cmd_inner_argument(&text, sels, 0, MotionMode::Move),
        "foo(-[aaa]>, bbb)\n"
    );
}

#[test]
fn inner_argument_cursor_on_close_bracket() {
    assert_state!(
        "foo(aaa, bbb-[)]>\n",
        |(text, sels)| cmd_inner_argument(&text, sels, 0, MotionMode::Move),
        "foo(aaa, -[bbb]>)\n"
    );
}

#[test]
fn inner_argument_empty_brackets_is_noop() {
    assert_state!(
        "foo-[(]>)\n",
        |(text, sels)| cmd_inner_argument(&text, sels, 0, MotionMode::Move),
        "foo-[(]>)\n"
    );
}

#[test]
fn inner_argument_no_enclosing_bracket_is_noop() {
    assert_state!(
        "foo-[,]>bar\n",
        |(text, sels)| cmd_inner_argument(&text, sels, 0, MotionMode::Move),
        "foo-[,]>bar\n"
    );
}

#[test]
fn inner_argument_array_items() {
    assert_state!(
        "[-[1]>11, 222, 333]\n",
        |(text, sels)| cmd_inner_argument(&text, sels, 0, MotionMode::Move),
        "[-[111]>, 222, 333]\n"
    );
}

#[test]
fn inner_argument_object_fields() {
    assert_state!(
        "{-[f]>oo, a: b}\n",
        |(text, sels)| cmd_inner_argument(&text, sels, 0, MotionMode::Move),
        "{-[foo]>, a: b}\n"
    );
}

#[test]
fn inner_argument_multi_cursor() {
    assert_state!(
        "foo(-[a]>aa, bbb, -[c]>cc)\n",
        |(text, sels)| cmd_inner_argument(&text, sels, 0, MotionMode::Move),
        "foo(-[aaa]>, bbb, -[ccc]>)\n"
    );
}

#[test]
fn inner_argument_trims_nbsp_matching_blank_class() {
    // NBSP (U+00A0) is `Space`-classified by `hume_editing::word::blank_class`
    // — the same rule `m a w` uses — so it must trim like an ordinary space,
    // not survive as part of the selected argument.
    assert_state!(
        "foo(aaa,\u{a0}-[b]>bb)\n",
        |(text, sels)| cmd_inner_argument(&text, sels, 0, MotionMode::Move),
        "foo(aaa,\u{a0}-[bbb]>)\n"
    );
}

// ── crossed / malformed nesting (characterization) ──────────────────────────
//
// `find_tightest_bracket_pair` resolves each bracket type ((), [], {}) as an
// independent candidate and picks the smallest span — never "nearest
// unmatched open, of any type". These pin that behavior on inputs where the
// two rules disagree, ahead of the pass-combining rewrite in `pair.rs`.

#[test]
fn crossed_nesting_picks_the_smallest_span_not_the_nearest_open() {
    // `(` at 1 is the nearest unmatched open, but its partner `)` at 10 gives
    // `()` a span of 9. `{}` (0, 5) is tighter and wins, even though its open
    // is farther from the cursor than `(`'s.
    assert_state!(
        "{(a-[b]>c}    )\n",
        |(text, sels)| cmd_inner_argument(&text, sels, 0, MotionMode::Move),
        "{-[(abc]>}    )\n"
    );
}

#[test]
fn crossed_nesting_equal_spans_break_the_tie_in_bracket_pairs_order() {
    // `()` = (0, 3) and `{}` = (1, 4) are both span 3. `BRACKET_PAIRS` lists
    // `()` first, so it wins the tie.
    assert_state!(
        "({-[a]>)}\n",
        |(text, sels)| cmd_inner_argument(&text, sels, 0, MotionMode::Move),
        "(-[{a]>)}\n"
    );
}

#[test]
fn a_type_with_no_closing_bracket_is_dropped_not_ranked() {
    // `{` at 1 has no matching `}` anywhere, so `{}` is dropped from the
    // candidate set entirely — not treated as "nearest open, unmatched close
    // ignored". `()` = (0, 8) wins by being the only resolved candidate.
    assert_state!(
        "({-[a]>aa, b)\n",
        |(text, sels)| cmd_inner_argument(&text, sels, 0, MotionMode::Move),
        "(-[{aaa, b]>)\n"
    );
}

#[test]
fn cursor_on_an_open_bracket_only_shortcuts_that_type() {
    // Cursor sits on `(`: only `()` takes the on-open shortcut (span found
    // without scanning left, right scan starts at pos + 1, past the `(`
    // itself). `[]` isn't on the cursor's char, so it still scans both
    // directions and resolves to the wider (0, 7).
    assert_state!(
        "[-[(]>a, b)]\n",
        |(text, sels)| cmd_inner_argument(&text, sels, 0, MotionMode::Move),
        "[(-[a]>, b)]\n"
    );
}

#[test]
fn cursor_on_a_close_bracket_only_shortcuts_that_type() {
    // Mirror of the above: cursor sits on `)`, only `()` takes the on-close
    // shortcut.
    assert_state!(
        "[(a, b-[)]>]\n",
        |(text, sels)| cmd_inner_argument(&text, sels, 0, MotionMode::Move),
        "[(a, -[b]>)]\n"
    );
}

#[test]
fn the_rightward_pass_must_resolve_every_surviving_type() {
    // `}` at 6 resolves `{}` = (0, 6) span 6 first, but `()` = (4, 9) span 5
    // resolves later scanning the same rightward pass and wins. A rightward
    // scan that stopped at the first type to resolve (rather than every
    // surviving type) would return the `{}` span instead.
    assert_state!(
        "{xxx(-[a]>}xx)\n",
        |(text, sels)| cmd_inner_argument(&text, sels, 0, MotionMode::Move),
        "{xxx(-[a}xx]>)\n"
    );
}

// ── around_argument ───────────────────────────────────────────────────────

#[test]
fn around_argument_first() {
    // Deletes "aaa, " — no orphan space before bbb.
    assert_state!(
        "foo(-[a]>aa, bbb, ccc)\n",
        |(text, sels)| cmd_around_argument(&text, sels, 0, MotionMode::Move),
        "foo(-[aaa, ]>bbb, ccc)\n"
    );
}

#[test]
fn around_argument_middle() {
    // Deletes ", bbb" — eats the preceding comma.
    assert_state!(
        "foo(aaa, -[b]>bb, ccc)\n",
        |(text, sels)| cmd_around_argument(&text, sels, 0, MotionMode::Move),
        "foo(aaa-[, bbb]>, ccc)\n"
    );
}

#[test]
fn around_argument_last() {
    // Deletes ", ccc" — eats the preceding comma.
    assert_state!(
        "foo(aaa, bbb, -[c]>cc)\n",
        |(text, sels)| cmd_around_argument(&text, sels, 0, MotionMode::Move),
        "foo(aaa, bbb-[, ccc]>)\n"
    );
}

#[test]
fn around_argument_single_equals_inner() {
    // No comma to eat — same as inner.
    assert_state!(
        "foo(-[a]>aa)\n",
        |(text, sels)| cmd_around_argument(&text, sels, 0, MotionMode::Move),
        "foo(-[aaa]>)\n"
    );
}

#[test]
fn around_argument_nested() {
    // First arg is a nested call — around eats trailing ", ".
    assert_state!(
        "foo(-[b]>ar(x, y), z)\n",
        |(text, sels)| cmd_around_argument(&text, sels, 0, MotionMode::Move),
        "foo(-[bar(x, y), ]>z)\n"
    );
}

#[test]
fn around_argument_single_on_outer_bracket_descends_into_nested() {
    // Cursor on the outer open bracket of `foo((a))`: the outer pair has a
    // single (only) argument, which is itself a bracketed pair. The single-
    // argument branch re-resolves through inner_argument rather than
    // trimming the outer segment as-is, so it descends into the nested pair
    // and selects `a`, not `(a)`.
    assert_state!(
        "foo-[(]>(a))\n",
        |(text, sels)| cmd_around_argument(&text, sels, 0, MotionMode::Move),
        "foo((-[a]>))\n"
    );
}

#[test]
fn around_argument_empty_slot_is_noop() {
    // All-whitespace segment (empty argument slot): `trim_segment` yields
    // `None` here exactly as it already does for `inner_argument` — matches
    // that existing no-op rather than the old raw-segment fallback, which
    // used to select " , ".
    assert_state!(
        "foo(-[ ]>, bbb)\n",
        |(text, sels)| cmd_around_argument(&text, sels, 0, MotionMode::Move),
        "foo(-[ ]>, bbb)\n"
    );
}

// ── around_from_inner ────────────────────────────────────────────────────
//
// Exercises the separator rule directly, on an inner span that need not have
// come from the lexical scan above — the same rule hume-editor's structural
// dispatch applies to a tree-sitter `parameter.inside` capture.

fn cmd_around_from_inner(
    text: &BufferText,
    sels: SelectionSet,
    _count: usize,
    _mode: MotionMode,
) -> SelectionSet {
    sels.map(|sel| {
        let (start, end) = around_from_inner(text, (sel.start(), sel.end()));
        Selection::new(start, end)
    })
}

#[test]
fn around_from_inner_first() {
    assert_state!(
        "foo(-[aaa]>, bbb, ccc)\n",
        |(text, sels)| cmd_around_from_inner(&text, sels, 0, MotionMode::Move),
        "foo(-[aaa, ]>bbb, ccc)\n"
    );
}

#[test]
fn around_from_inner_middle() {
    assert_state!(
        "foo(aaa, -[bbb]>, ccc)\n",
        |(text, sels)| cmd_around_from_inner(&text, sels, 0, MotionMode::Move),
        "foo(aaa-[, bbb]>, ccc)\n"
    );
}

#[test]
fn around_from_inner_last() {
    assert_state!(
        "foo(aaa, bbb, -[ccc]>)\n",
        |(text, sels)| cmd_around_from_inner(&text, sels, 0, MotionMode::Move),
        "foo(aaa, bbb-[, ccc]>)\n"
    );
}

#[test]
fn around_from_inner_only_is_unchanged() {
    // No comma on either side — nothing to extend into.
    assert_state!(
        "foo(-[aaa]>)\n",
        |(text, sels)| cmd_around_from_inner(&text, sels, 0, MotionMode::Move),
        "foo(-[aaa]>)\n"
    );
}

#[test]
fn around_from_inner_multiline() {
    // First argument on its own line: eats its leading indentation and the
    // trailing comma, but leaves the following line's indentation for the
    // next argument (only space/tab count as inline blank).
    assert_state!(
        "foo(\n    -[a]>,\n    b\n)\n",
        |(text, sels)| cmd_around_from_inner(&text, sels, 0, MotionMode::Move),
        "foo(-[\n    a,]>\n    b\n)\n"
    );
}

#[test]
fn around_from_inner_multiline_last_argument_eats_the_trailing_newline() {
    // Unlike the first-argument case above (inline blank only, no newline),
    // the preceding-comma branch extends `end` through a newline-inclusive
    // blank run — the last argument in a multi-line list eats the newline
    // before the closing delimiter. Matches the pre-existing lexical scan's
    // behavior exactly; not a new asymmetry introduced alongside it.
    assert_state!(
        "foo(\n    a,\n    -[b]>\n)\n",
        |(text, sels)| cmd_around_from_inner(&text, sels, 0, MotionMode::Move),
        "foo(\n    a-[,\n    b\n]>)\n"
    );
}

// ── extend mode ───────────────────────────────────────────────────────────

#[test]
fn extend_inner_argument_basic() {
    assert_state!(
        "foo(aaa, -[b]>bb, ccc)\n",
        |(text, sels)| cmd_inner_argument(&text, sels, 0, MotionMode::Extend),
        "foo(aaa, -[bbb]>, ccc)\n"
    );
}
