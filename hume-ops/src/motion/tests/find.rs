use super::super::*;
use test_fixtures::assert_state;

// ── find_char_forward / find_char_backward ────────────────────────────────

// Helper wrappers with fixed mode so assert_state! closures stay tidy.
fn fwd(text: BufferText, sels: SelectionSet, ch: char, kind: FindKind) -> SelectionSet {
    find_char_forward(&text, sels, 1, MotionMode::Move, ch, kind)
}
fn bwd(text: BufferText, sels: SelectionSet, ch: char, kind: FindKind) -> SelectionSet {
    find_char_backward(&text, sels, 1, MotionMode::Move, ch, kind)
}
fn fwd_ext(text: BufferText, sels: SelectionSet, ch: char, kind: FindKind) -> SelectionSet {
    find_char_forward(&text, sels, 1, MotionMode::Extend, ch, kind)
}
fn fwd_count(
    text: BufferText,
    sels: SelectionSet,
    ch: char,
    kind: FindKind,
    n: usize,
) -> SelectionSet {
    find_char_forward(&text, sels, n, MotionMode::Move, ch, kind)
}

#[test]
fn find_forward_inclusive_basic() {
    // Cursor on 'h'; `fa` jumps to the first 'a'.
    assert_state!(
        "-[h]>ello a world\n",
        |(text, sels)| fwd(text, sels, 'a', FindKind::Inclusive),
        "hello -[a]> world\n"
    );
}

#[test]
fn find_forward_inclusive_first_char_on_line() {
    // Target is the very last content char.
    assert_state!(
        "-[h]>ello\n",
        |(text, sels)| fwd(text, sels, 'o', FindKind::Inclusive),
        "hell-[o]>\n"
    );
}

#[test]
fn find_forward_inclusive_not_found() {
    // No 'z' on this line: no-op.
    assert_state!(
        "-[h]>ello\n",
        |(text, sels)| fwd(text, sels, 'z', FindKind::Inclusive),
        "-[h]>ello\n"
    );
}

#[test]
fn find_forward_does_not_cross_newline() {
    // 'a' appears only on the second line. The motion must not cross '\n'.
    assert_state!(
        "-[h]>ello\nabc\n",
        |(text, sels)| fwd(text, sels, 'a', FindKind::Inclusive),
        "-[h]>ello\nabc\n"
    );
}

#[test]
fn find_forward_skips_char_under_cursor() {
    // Cursor is already on 'a'; `fa` should find the *next* 'a', not the current one.
    assert_state!(
        "-[a]>bc a def\n",
        |(text, sels)| fwd(text, sels, 'a', FindKind::Inclusive),
        "abc -[a]> def\n"
    );
}

#[test]
fn find_forward_exclusive_basic() {
    // `ta` stops one grapheme before 'a': the space is one grapheme before 'a'.
    assert_state!(
        "-[h]>ello a world\n",
        |(text, sels)| fwd(text, sels, 'a', FindKind::Exclusive),
        "hello-[ ]>a world\n"
    );
}

#[test]
fn find_forward_exclusive_adjacent_is_noop() {
    // 'a' is the immediately next grapheme; exclusive adjustment lands back at head.
    assert_state!(
        "-[h]>a world\n",
        |(text, sels)| fwd(text, sels, 'a', FindKind::Exclusive),
        "-[h]>a world\n"
    );
}

#[test]
fn find_forward_count() {
    // `2fa` jumps to the second 'a'.
    assert_state!(
        "-[h]>a ba\n",
        |(text, sels)| fwd_count(text, sels, 'a', FindKind::Inclusive, 2),
        "ha b-[a]>\n"
    );
}

#[test]
fn find_backward_inclusive_basic() {
    // `Fa` finds the previous 'a'.
    assert_state!(
        "hello a worl-[d]>\n",
        |(text, sels)| bwd(text, sels, 'a', FindKind::Inclusive),
        "hello -[a]> world\n"
    );
}

#[test]
fn find_backward_inclusive_not_found() {
    assert_state!(
        "hell-[o]>\n",
        |(text, sels)| bwd(text, sels, 'z', FindKind::Inclusive),
        "hell-[o]>\n"
    );
}

#[test]
fn find_backward_does_not_cross_newline() {
    // 'z' is only on the first line; cursor on second line must not find it.
    assert_state!(
        "z\n-[a]>bc\n",
        |(text, sels)| bwd(text, sels, 'z', FindKind::Inclusive),
        "z\n-[a]>bc\n"
    );
}

#[test]
fn find_backward_exclusive_basic() {
    // `Ta` stops one grapheme after 'a' (cursor is between 'a' and its original pos).
    assert_state!(
        "hello a worl-[d]>\n",
        |(text, sels)| bwd(text, sels, 'a', FindKind::Exclusive),
        "hello a-[ ]>world\n"
    );
}

#[test]
fn find_backward_exclusive_adjacent_is_noop() {
    // Cursor is immediately right of 'a'; exclusive adjustment steps forward
    // from the found position back to head, so the motion is a no-op,
    // symmetric to the forward exclusive adjacent case.
    assert_state!(
        "hello a-[x]>\n",
        |(text, sels)| bwd(text, sels, 'a', FindKind::Exclusive),
        "hello a-[x]>\n"
    );
}

#[test]
fn find_forward_extend_mode() {
    // Extend mode: anchor stays, head moves to found char.
    assert_state!(
        "-[h]>ello a\n",
        |(text, sels)| fwd_ext(text, sels, 'a', FindKind::Inclusive),
        "-[hello a]>\n"
    );
}

#[test]
fn find_forward_multi_cursor() {
    // Two cursors on the same line each find their own next 'a'.
    // cursor1 at 'h'(0) → next 'a' at 1.
    // cursor2 at 'a'(4) → skips it, next 'a' at 8.
    assert_state!(
        "-[h]>a b-[a]> c a\n",
        |(text, sels)| fwd(text, sels, 'a', FindKind::Inclusive),
        "h-[a]> ba c -[a]>\n"
    );
}

#[test]
fn find_backward_at_line_start_noop() {
    // Cursor at line start: nothing to the left, no-op.
    assert_state!(
        "-[h]>ello\n",
        |(text, sels)| bwd(text, sels, 'x', FindKind::Inclusive),
        "-[h]>ello\n"
    );
}

// ── Normalization and clusters ────────────────────────────────────────────

#[test]
fn find_forward_matches_a_precomposed_target_against_a_decomposed_cluster() {
    assert_state!(
        "-[h]>i e\u{301}x\n",
        |(text, sels)| fwd(text, sels, '\u{e9}', FindKind::Inclusive),
        "hi -[e\u{301}]>x\n"
    );
}

#[test]
fn find_forward_bare_base_char_skips_a_cluster_that_carries_a_mark() {
    assert_state!(
        "-[h]>e\u{301}e\n",
        |(text, sels)| fwd(text, sels, 'e', FindKind::Inclusive),
        "he\u{301}-[e]>\n"
    );
}

#[test]
fn find_backward_matches_a_precomposed_target_against_a_decomposed_cluster() {
    assert_state!(
        "e\u{301}xy-[z]>\n",
        |(text, sels)| bwd(text, sels, '\u{e9}', FindKind::Inclusive),
        "-[e\u{301}]>xyz\n"
    );
}

#[test]
fn find_forward_matches_a_char_that_normalizes_to_the_target() {
    assert_state!(
        "-[a]>\u{212a}\n",
        |(text, sels)| fwd(text, sels, 'K', FindKind::Inclusive),
        "a-[\u{212a}]>\n"
    );
}

#[test]
fn find_forward_till_stops_before_a_multi_char_cluster() {
    assert_state!(
        "-[h]>ie\u{301}\n",
        |(text, sels)| fwd(text, sels, '\u{e9}', FindKind::Exclusive),
        "h-[i]>e\u{301}\n"
    );
}
