use super::super::*;
use hume_test_fixtures::assert_state;

// ── goto_next_paragraph (`}`) ────────────────────────────────────────────────

#[test]
fn goto_next_paragraph_basic() {
    // Selects the next paragraph. No trailing gap here — "foo" is the last
    // paragraph — so the span stops at its own text.
    assert_state!(
        "-[h]>ello\nworld\n\nfoo\n",
        |(text, sels)| cmd_goto_next_paragraph(&text, sels, 1, MotionMode::Move),
        "hello\nworld\n\n<[foo]-\n"
    );
}

#[test]
fn goto_next_paragraph_multiline() {
    assert_state!(
        "-[a]>\n\nfoo\nbar\n",
        |(text, sels)| cmd_goto_next_paragraph(&text, sels, 1, MotionMode::Move),
        "a\n\n<[foo\nbar]-\n"
    );
}

#[test]
fn goto_next_paragraph_includes_trailing_gap() {
    // The target paragraph isn't the last one, so its own trailing gap is
    // part of the selection too.
    assert_state!(
        "-[a]>\n\nfoo\n\nbar\n",
        |(text, sels)| cmd_goto_next_paragraph(&text, sels, 1, MotionMode::Move),
        "a\n\n<[foo\n\n]-bar\n"
    );
}

#[test]
fn goto_next_paragraph_multiple_empty_lines() {
    assert_state!(
        "-[\n]>\n\nfoo\n",
        |(text, sels)| cmd_goto_next_paragraph(&text, sels, 1, MotionMode::Move),
        "\n\n\n<[foo]-\n"
    );
}

#[test]
fn goto_next_paragraph_from_empty_line() {
    assert_state!(
        "-[\n]>\nfoo\n",
        |(text, sels)| cmd_goto_next_paragraph(&text, sels, 1, MotionMode::Move),
        "\n\n<[foo]-\n"
    );
}

#[test]
fn goto_next_paragraph_no_paragraph_below_is_noop() {
    assert_state!(
        "-[h]>ello\nworld\n",
        |(text, sels)| cmd_goto_next_paragraph(&text, sels, 1, MotionMode::Move),
        "-[h]>ello\nworld\n"
    );
}

#[test]
fn goto_next_paragraph_at_eof_is_noop() {
    assert_state!(
        "hello-[\n]>",
        |(text, sels)| cmd_goto_next_paragraph(&text, sels, 1, MotionMode::Move),
        "hello-[\n]>"
    );
}

#[test]
fn goto_next_paragraph_empty_buffer_is_noop() {
    assert_state!(
        "-[\n]>",
        |(text, sels)| cmd_goto_next_paragraph(&text, sels, 1, MotionMode::Move),
        "-[\n]>"
    );
}

#[test]
fn goto_next_paragraph_sequential() {
    // Two consecutive `}` presses walk through three paragraphs.
    assert_state!(
        "-[a]>\n\nb\n\nc\n",
        |(text, sels)| cmd_goto_next_paragraph(&text, sels, 1, MotionMode::Move),
        "a\n\n<[b\n\n]-c\n"
    );
    assert_state!(
        "a\n\n<[b\n\n]-c\n",
        |(text, sels)| cmd_goto_next_paragraph(&text, sels, 1, MotionMode::Move),
        "a\n\nb\n\n<[c]-\n"
    );
}

#[test]
fn goto_next_paragraph_count_two_matches_two_presses() {
    assert_state!(
        "-[a]>\n\nb\n\nc\n",
        |(text, sels)| cmd_goto_next_paragraph(&text, sels, 2, MotionMode::Move),
        "a\n\nb\n\n<[c]-\n"
    );
}

// ── goto_prev_paragraph (`{`) ────────────────────────────────────────────────

#[test]
fn goto_prev_paragraph_basic() {
    assert_state!(
        "hello\n\nwor-[l]>d\n",
        |(text, sels)| cmd_goto_prev_paragraph(&text, sels, 1, MotionMode::Move),
        "<[hello\n\n]-world\n"
    );
}

#[test]
fn goto_prev_paragraph_multiline() {
    assert_state!(
        "foo\nbar\n\nba-[z]>\n",
        |(text, sels)| cmd_goto_prev_paragraph(&text, sels, 1, MotionMode::Move),
        "<[foo\nbar\n\n]-baz\n"
    );
}

#[test]
fn goto_prev_paragraph_multiple_empty_lines() {
    assert_state!(
        "hello\n\n\nwor-[l]>d\n",
        |(text, sels)| cmd_goto_prev_paragraph(&text, sels, 1, MotionMode::Move),
        "<[hello\n\n\n]-world\n"
    );
}

#[test]
fn goto_prev_paragraph_no_paragraph_above_is_noop() {
    assert_state!(
        "-[h]>ello\nworld\n",
        |(text, sels)| cmd_goto_prev_paragraph(&text, sels, 1, MotionMode::Move),
        "-[h]>ello\nworld\n"
    );
}

#[test]
fn goto_prev_paragraph_from_gap_selects_nearest_paragraph() {
    // Cursor sits in the gap above one preceding paragraph — that paragraph
    // (plus the gap it's already in) is the target.
    assert_state!(
        "hello\n-[\n]>world\n",
        |(text, sels)| cmd_goto_prev_paragraph(&text, sels, 1, MotionMode::Move),
        "<[hello\n\n]-world\n"
    );
}

#[test]
fn goto_prev_paragraph_from_gap_selects_nearest_not_the_one_before_it() {
    // Two paragraphs precede the gap — the nearest one ("hello") is the
    // target, not the one before it ("foo"). A backward scan that skipped
    // an extra paragraph here would land on "foo" instead.
    assert_state!(
        "foo\n\nhello\n-[\n]>world\n",
        |(text, sels)| cmd_goto_prev_paragraph(&text, sels, 1, MotionMode::Move),
        "foo\n\n<[hello\n\n]-world\n"
    );
}

#[test]
fn goto_prev_paragraph_from_leading_gap_is_noop() {
    // Nothing precedes the gap itself — no previous paragraph exists.
    assert_state!(
        "-[\n]>hello\n",
        |(text, sels)| cmd_goto_prev_paragraph(&text, sels, 1, MotionMode::Move),
        "-[\n]>hello\n"
    );
}

#[test]
fn goto_prev_paragraph_sequential() {
    assert_state!(
        "a\n\nb\n\n-[c]>\n",
        |(text, sels)| cmd_goto_prev_paragraph(&text, sels, 1, MotionMode::Move),
        "a\n\n<[b\n\n]-c\n"
    );
    assert_state!(
        "a\n\n<[b\n\n]-c\n",
        |(text, sels)| cmd_goto_prev_paragraph(&text, sels, 1, MotionMode::Move),
        "<[a\n\n]-b\n\nc\n"
    );
}

#[test]
fn goto_prev_paragraph_empty_buffer_is_noop() {
    assert_state!(
        "-[\n]>",
        |(text, sels)| cmd_goto_prev_paragraph(&text, sels, 1, MotionMode::Move),
        "-[\n]>"
    );
}

// ── Extend variants ───────────────────────────────────────────────────────────

#[test]
fn extend_goto_next_paragraph_creates_selection() {
    assert_state!(
        "-[h]>ello\n\nworld\n",
        |(text, sels)| cmd_goto_next_paragraph(&text, sels, 1, MotionMode::Extend),
        "-[hello\n\nworld]>\n"
    );
}

#[test]
fn extend_goto_next_paragraph_after_move_keeps_both_paragraphs() {
    assert_state!(
        "a\n\n<[b\n\n]-c\n",
        |(text, sels)| cmd_goto_next_paragraph(&text, sels, 1, MotionMode::Extend),
        "a\n\n-[b\n\nc]>\n"
    );
}

#[test]
fn extend_goto_prev_paragraph_creates_selection() {
    assert_state!(
        "hello\n\n-[w]>orld\n",
        |(text, sels)| cmd_goto_prev_paragraph(&text, sels, 1, MotionMode::Extend),
        "<[hello\n\nw]-orld\n"
    );
}

#[test]
fn extend_goto_prev_paragraph_after_move_keeps_both_paragraphs() {
    assert_state!(
        "a\n\n<[b\n\n]-c\n",
        |(text, sels)| cmd_goto_prev_paragraph(&text, sels, 1, MotionMode::Extend),
        "<[a\n\nb\n\n]-c\n"
    );
}

// ── Multi-cursor ─────────────────────────────────────────────────────────────

#[test]
fn goto_next_paragraph_multi_cursor_merges_when_target_overlaps() {
    // The 'w' cursor's target ("foo") fully contains the 'f' cursor's own
    // unchanged position (no paragraph below it), so the two merge.
    assert_state!(
        "hello\n\n-[w]>orld\n\n-[f]>oo\n",
        |(text, sels)| cmd_goto_next_paragraph(&text, sels, 1, MotionMode::Move),
        "hello\n\nworld\n\n<[foo]-\n"
    );
}

#[test]
fn goto_prev_paragraph_multi_cursor() {
    assert_state!(
        "hello\n\n-[w]>orld\n\n-[f]>oo\n",
        |(text, sels)| cmd_goto_prev_paragraph(&text, sels, 1, MotionMode::Move),
        "<[hello\n\n]-<[world\n\n]-foo\n"
    );
}
