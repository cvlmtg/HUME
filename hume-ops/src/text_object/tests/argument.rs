use super::super::*;
use hume_test_fixtures::assert_state;

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
// come from the lexical scan above — the same rule Phase 5 applies to a
// tree-sitter `parameter.inside` capture.

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

// ── extend mode ───────────────────────────────────────────────────────────

#[test]
fn extend_inner_argument_basic() {
    assert_state!(
        "foo(aaa, -[b]>bb, ccc)\n",
        |(text, sels)| cmd_inner_argument(&text, sels, 0, MotionMode::Extend),
        "foo(aaa, -[bbb]>, ccc)\n"
    );
}
