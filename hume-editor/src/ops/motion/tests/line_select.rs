use super::super::*;
use crate::assert_state;

// ── cmd_select_line / cmd_select_line_backward ────────────────────────────

#[test]
fn select_line_from_mid_line() {
    // Cursor mid-line → select full line forward.
    assert_state!(
        "hello -[w]>orld\nfoo\n",
        |(buf, sels)| cmd_select_line(&buf, sels, 1, MotionMode::Move),
        "-[hello world\n]>foo\n"
    );
}

#[test]
fn select_line_already_full_line_jumps_to_next() {
    // Selection already covers full line → jump to next line.
    assert_state!(
        "-[hello world\n]>foo\n",
        |(buf, sels)| cmd_select_line(&buf, sels, 1, MotionMode::Move),
        "hello world\n-[foo\n]>"
    );
}

#[test]
fn select_line_clamps_at_last_line() {
    // Already on last line → no change.
    assert_state!(
        "hello\n-[foo\n]>",
        |(buf, sels)| cmd_select_line(&buf, sels, 1, MotionMode::Move),
        "hello\n-[foo\n]>"
    );
}

#[test]
fn select_line_backward_from_mid_line() {
    // Cursor mid-line → select full line backward (anchor=`\n`, head=start).
    assert_state!(
        "hello -[w]>orld\nfoo\n",
        |(buf, sels)| cmd_select_line_backward(&buf, sels, 1, MotionMode::Move),
        "<[hello world\n]-foo\n"
    );
}

#[test]
fn select_line_backward_already_at_start_jumps_to_prev() {
    // Selection already starts at line boundary → jump to previous line.
    assert_state!(
        "aaa\n<[bbb\n]-ccc\n",
        |(buf, sels)| cmd_select_line_backward(&buf, sels, 1, MotionMode::Move),
        "<[aaa\n]-bbb\nccc\n"
    );
}

#[test]
fn select_line_backward_clamps_at_first_line() {
    // Already on first line → no change.
    assert_state!(
        "<[hello\n]-world\n",
        |(buf, sels)| cmd_select_line_backward(&buf, sels, 1, MotionMode::Move),
        "<[hello\n]-world\n"
    );
}

// ── cmd_select_line / cmd_select_line_backward (extend mode) ─────────────

#[test]
fn extend_select_line_accumulates_downward() {
    // Each press accumulates one more line.
    assert_state!(
        "-[hello\n]>foo\nbar\n",
        |(buf, sels)| cmd_select_line(&buf, sels, 1, MotionMode::Extend),
        "-[hello\nfoo\n]>bar\n"
    );
}

#[test]
fn extend_select_line_clamps_at_last_line() {
    // Already at last line → no change.
    assert_state!(
        "hello\n-[foo\n]>",
        |(buf, sels)| cmd_select_line(&buf, sels, 1, MotionMode::Extend),
        "hello\n-[foo\n]>"
    );
}

#[test]
fn extend_select_line_backward_accumulates_upward() {
    // Each press accumulates one more line upward.
    assert_state!(
        "aaa\n<[bbb\n]-ccc\n",
        |(buf, sels)| cmd_select_line_backward(&buf, sels, 1, MotionMode::Extend),
        "<[aaa\nbbb\n]-ccc\n"
    );
}

#[test]
fn extend_select_line_backward_clamps_at_first_line() {
    // Already at first line → no change.
    assert_state!(
        "<[hello\n]-world\n",
        |(buf, sels)| cmd_select_line_backward(&buf, sels, 1, MotionMode::Extend),
        "<[hello\n]-world\n"
    );
}

#[test]
fn extend_select_line_from_mid_line() {
    // Starting from a partial selection, the first extend covers the full line.
    assert_state!(
        "hello -[w]>orld\nfoo\n",
        |(buf, sels)| cmd_select_line(&buf, sels, 1, MotionMode::Extend),
        "-[hello world\n]>foo\n"
    );
}

#[test]
fn extend_select_line_backward_from_mid_line() {
    // Starting from a partial selection, the first backward extend covers the full line.
    assert_state!(
        "hello -[w]>orld\nfoo\n",
        |(buf, sels)| cmd_select_line_backward(&buf, sels, 1, MotionMode::Extend),
        "<[hello world\n]-foo\n"
    );
}

#[test]
fn select_line_empty_line() {
    // A bare `\n` line: the cursor is already on the only character (the `\n`),
    // so `x` immediately jumps to the next line.
    assert_state!(
        "hello\n-[\n]>world\n",
        |(buf, sels)| cmd_select_line(&buf, sels, 1, MotionMode::Move),
        "hello\n\n-[world\n]>"
    );
}

#[test]
fn select_line_backward_empty_line() {
    // A bare `\n` line: cursor is at line start → `X` jumps to the previous line.
    assert_state!(
        "hello\n-[\n]>world\n",
        |(buf, sels)| cmd_select_line_backward(&buf, sels, 1, MotionMode::Move),
        "<[hello\n]-\nworld\n"
    );
}

#[test]
fn select_line_multi_cursor() {
    // Two cursors on different lines each independently select their full line.
    // The resulting line selections are non-overlapping and stay separate.
    assert_state!(
        "hello -[w]>orld\nfoo -[b]>ar\nbaz\n",
        |(buf, sels)| cmd_select_line(&buf, sels, 1, MotionMode::Move),
        "-[hello world\n]>-[foo bar\n]>baz\n"
    );
}

#[test]
fn select_line_multi_cursor_same_line_merges() {
    // Two cursors on the same line both produce identical line selections,
    // which `map` (which always merges) collapses to a single selection.
    assert_state!(
        "hell-[o]> -[w]>orld\nfoo\n",
        |(buf, sels)| cmd_select_line(&buf, sels, 1, MotionMode::Move),
        "-[hello world\n]>foo\n"
    );
}

#[test]
fn extend_select_line_multi_cursor_merges() {
    // Two adjacent full-line selections each extend to the next line; because the
    // resulting ranges overlap, `map` (which always merges) unifies them into one.
    //
    // sel1 (-[hello world\n]>) end=11 → extends to line 1 → (0,15)
    // sel2 (-[foo\n]>)         end=15 → extends to line 2 → (12,19)
    // (0,15) and (12,19) overlap → merged to (0,19)
    assert_state!(
        "-[hello world\n]>-[foo\n]>bar\n",
        |(buf, sels)| cmd_select_line(&buf, sels, 1, MotionMode::Extend),
        "-[hello world\nfoo\nbar\n]>"
    );
}

// ── extend_select_line: shrink-on-reversal scenario ───────────────────────
//
// Walks the exact sequence a user gets pressing Ctrl+x / Ctrl+X repeatedly
// on "a\nb\nc\n" with "b" selected: grow down, shrink back to "b", cross the
// anchor to grow up (flipping direction), then cross back to shrink down to
// "b" again. a=0,'\n'=1,b=2,'\n'=3,c=4,'\n'=5.

#[test]
fn line_shrink_scenario_step1_grows_down() {
    assert_state!(
        "a\n-[b\n]>c\n",
        |(buf, sels)| cmd_select_line(&buf, sels, 1, MotionMode::Extend),
        "a\n-[b\nc\n]>"
    );
}

#[test]
fn line_shrink_scenario_step2_shrinks_up_to_anchor_line() {
    assert_state!(
        "a\n-[b\nc\n]>",
        |(buf, sels)| cmd_select_line_backward(&buf, sels, 1, MotionMode::Extend),
        "a\n-[b\n]>c\n"
    );
}

#[test]
fn line_shrink_scenario_step3_crosses_anchor_flips_backward() {
    assert_state!(
        "a\n-[b\n]>c\n",
        |(buf, sels)| cmd_select_line_backward(&buf, sels, 1, MotionMode::Extend),
        "<[a\nb\n]-c\n"
    );
}

#[test]
fn line_shrink_scenario_step4_crosses_back_shrinks_down() {
    assert_state!(
        "<[a\nb\n]-c\n",
        |(buf, sels)| cmd_select_line(&buf, sels, 1, MotionMode::Extend),
        "a\n-[b\n]>c\n"
    );
}

#[test]
fn line_extend_after_flip_shrinks_to_new_anchor_line() {
    // Flipped "b\nc\n": anchor on line 2 ("c"), head on line 1. Extend-x moves
    // the head's line down onto the anchor's line, shrinking to "c\n". Without
    // the flip the same press clamps at the last line (no-op).
    assert_state!(
        "a\n<[b\nc\n]-",
        |(buf, sels)| cmd_select_line(&buf, sels, 1, MotionMode::Extend),
        "a\nb\n-[c\n]>"
    );
}

#[test]
fn line_extend_backward_after_flip_grows_over_old_span() {
    // Same flipped start: extend-X moves the head's line up to line 0, and the
    // span is rebuilt from the anchor's line (2) — the whole buffer, backward.
    // Without the flip the same press shrinks to "b\n" instead.
    assert_state!(
        "a\n<[b\nc\n]-",
        |(buf, sels)| cmd_select_line_backward(&buf, sels, 1, MotionMode::Extend),
        "<[a\nb\nc\n]-"
    );
}

#[test]
fn extend_select_line_backward_selection_shrinks_from_last_line() {
    // The clamp must be head-relative: the selection's END sits on the
    // trailing `\n` here, but the HEAD (the end that's actually moving) is
    // nowhere near the last line, so an end-relative clamp would wrongly
    // no-op instead of letting this shrink.
    assert_state!(
        "<[a\nb\nc\n]-",
        |(buf, sels)| cmd_select_line(&buf, sels, 1, MotionMode::Extend),
        "a\n<[b\nc\n]-"
    );
}

#[test]
fn extend_select_line_single_line_buffer_forward_is_noop() {
    assert_state!(
        "-[hello\n]>",
        |(buf, sels)| cmd_select_line(&buf, sels, 1, MotionMode::Extend),
        "-[hello\n]>"
    );
}

#[test]
fn extend_select_line_single_line_buffer_backward_is_noop() {
    assert_state!(
        "-[hello\n]>",
        |(buf, sels)| cmd_select_line_backward(&buf, sels, 1, MotionMode::Extend),
        "-[hello\n]>"
    );
}

#[test]
fn extend_select_line_crosses_empty_line() {
    // Growing downward from line 0 into an empty line (just a bare `\n`)
    // works via ordinary line arithmetic — no special-casing needed.
    assert_state!(
        "-[a\n]>\nb\n",
        |(buf, sels)| cmd_select_line(&buf, sels, 1, MotionMode::Extend),
        "-[a\n\n]>b\n"
    );
}

// ── cmd_select_line / cmd_select_line_backward (count) ────────────────────

#[test]
fn select_line_move_count_three_selects_three_lines() {
    // `3x` moves the same way three separate `x` presses would: the 1st
    // press selects the cursor's own line ("b"), the 2nd and 3rd each jump
    // to the next line, landing on "d" as a single-line selection — not
    // growing a 3-line span (that's `Ctrl+3x`).
    assert_state!(
        "a\n-[b]>\nc\nd\ne\n",
        |(buf, sels)| cmd_select_line(&buf, sels, 3, MotionMode::Move),
        "a\nb\nc\n-[d\n]>e\n"
    );
}

#[test]
fn select_line_backward_move_count_three_selects_three_lines() {
    // `3X` moves the same way three separate `X` presses would, landing on
    // "b" as a single-line selection — not growing a 3-line span (that's
    // `Ctrl+3X`). Cursor is mid-line ("dd"'s second char), not at line
    // start — a selection starting exactly at line start instead hits the
    // jump-to-previous-line branch (see `select_line_backward_already_at_start_jumps_to_prev`).
    assert_state!(
        "a\nb\nc\nd-[d]>\ne\n",
        |(buf, sels)| cmd_select_line_backward(&buf, sels, 3, MotionMode::Move),
        "a\n<[b\n]-c\ndd\ne\n"
    );
}

#[test]
fn extend_select_line_count_three_grows_three_lines_at_once() {
    // A single `3x`-extend call grows 3 lines in one step, equivalent to 3
    // separate single presses (see `extend_select_line_accumulates_downward`).
    assert_state!(
        "-[a\n]>b\nc\nd\n",
        |(buf, sels)| cmd_select_line(&buf, sels, 3, MotionMode::Extend),
        "-[a\nb\nc\nd\n]>"
    );
}

#[test]
fn extend_select_line_backward_count_three_grows_three_lines_at_once() {
    assert_state!(
        "a\nb\nc\n<[d\n]-",
        |(buf, sels)| cmd_select_line_backward(&buf, sels, 3, MotionMode::Extend),
        "<[a\nb\nc\nd\n]-"
    );
}

#[test]
fn select_line_move_count_exceeds_buffer_clamps_at_last_line() {
    // count larger than the remaining lines clamps at the last line: each
    // repeated press stops advancing once there's no next line, ending on a
    // single-line selection there — not growing to span every line.
    assert_state!(
        "-[a]>\nb\nc\n",
        |(buf, sels)| cmd_select_line(&buf, sels, 10, MotionMode::Move),
        "a\nb\n-[c\n]>"
    );
}

// A `usize::MAX` count must return instantly (proving `repeat_motion`'s
// fixed-point early exit) rather than looping `usize::MAX` times: each of
// these hangs forever without the early exit, since a naive `for _ in
// 0..count` loop has no way to notice the motion already clamped.

#[test]
fn select_line_move_huge_count_clamps_instantly() {
    assert_state!(
        "-[a]>\nb\nc\n",
        |(buf, sels)| cmd_select_line(&buf, sels, usize::MAX, MotionMode::Move),
        "a\nb\n-[c\n]>"
    );
}

#[test]
fn select_line_backward_move_huge_count_clamps_instantly() {
    assert_state!(
        "a\nb\n-[c]>\n",
        |(buf, sels)| cmd_select_line_backward(&buf, sels, usize::MAX, MotionMode::Move),
        "<[a\n]-b\nc\n"
    );
}

#[test]
fn extend_select_line_huge_count_clamps_instantly() {
    assert_state!(
        "-[a\n]>b\nc\n",
        |(buf, sels)| cmd_select_line(&buf, sels, usize::MAX, MotionMode::Extend),
        "-[a\nb\nc\n]>"
    );
}

#[test]
fn extend_select_line_backward_huge_count_clamps_instantly() {
    assert_state!(
        "a\nb\n<[c\n]-",
        |(buf, sels)| cmd_select_line_backward(&buf, sels, usize::MAX, MotionMode::Extend),
        "<[a\nb\nc\n]-"
    );
}
