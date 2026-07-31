use super::super::*;
use crate::assert_state;

// ── goto_line_start ───────────────────────────────────────────────────────

#[test]
fn goto_line_start_from_middle() {
    assert_state!(
        "hel-[l]>o\n",
        |(buf, sels)| cmd_goto_line_start(&buf, sels, 1, MotionMode::Move),
        "-[h]>ello\n"
    );
}

#[test]
fn goto_line_start_already_at_start() {
    assert_state!(
        "-[h]>ello\n",
        |(buf, sels)| cmd_goto_line_start(&buf, sels, 1, MotionMode::Move),
        "-[h]>ello\n"
    );
}

#[test]
fn goto_line_start_second_line() {
    assert_state!(
        "hello\nwor-[l]>d\n",
        |(buf, sels)| cmd_goto_line_start(&buf, sels, 1, MotionMode::Move),
        "hello\n-[w]>orld\n"
    );
}

#[test]
fn goto_line_start_empty_buffer() {
    assert_state!(
        "-[\n]>",
        |(buf, sels)| cmd_goto_line_start(&buf, sels, 1, MotionMode::Move),
        "-[\n]>"
    );
}

// ── goto_line_end ─────────────────────────────────────────────────────────

#[test]
fn goto_line_end_from_start() {
    assert_state!(
        "-[h]>ello\n",
        |(buf, sels)| cmd_goto_line_end(&buf, sels, 1, MotionMode::Move),
        "hell-[o]>\n"
    );
}

#[test]
fn goto_line_end_already_at_end() {
    assert_state!(
        "hell-[o]>\n",
        |(buf, sels)| cmd_goto_line_end(&buf, sels, 1, MotionMode::Move),
        "hell-[o]>\n"
    );
}

#[test]
fn goto_line_end_stops_before_newline() {
    // Cursor must land on 'o', not on '\n'.
    assert_state!(
        "-[h]>ello\nworld\n",
        |(buf, sels)| cmd_goto_line_end(&buf, sels, 1, MotionMode::Move),
        "hell-[o]>\nworld\n"
    );
}

#[test]
fn goto_line_end_empty_line() {
    // Line contains only '\n'. Cursor stays on it.
    assert_state!(
        "-[\n]>",
        |(buf, sels)| cmd_goto_line_end(&buf, sels, 1, MotionMode::Move),
        "-[\n]>"
    );
}

#[test]
fn goto_line_end_last_line_no_newline() {
    assert_state!(
        "-[h]>ello\n",
        |(buf, sels)| cmd_goto_line_end(&buf, sels, 1, MotionMode::Move),
        "hell-[o]>\n"
    );
}

#[test]
fn goto_line_end_empty_buffer() {
    assert_state!(
        "-[\n]>",
        |(buf, sels)| cmd_goto_line_end(&buf, sels, 1, MotionMode::Move),
        "-[\n]>"
    );
}

// ── goto_first_nonblank ───────────────────────────────────────────────────

#[test]
fn goto_first_nonblank_skips_spaces() {
    assert_state!(
        "-[ ]> hello\n",
        |(buf, sels)| cmd_goto_first_nonblank(&buf, sels, 1, MotionMode::Move),
        "  -[h]>ello\n"
    );
}

#[test]
fn goto_first_nonblank_from_middle() {
    assert_state!(
        "  hel-[l]>o\n",
        |(buf, sels)| cmd_goto_first_nonblank(&buf, sels, 1, MotionMode::Move),
        "  -[h]>ello\n"
    );
}

#[test]
fn goto_first_nonblank_skips_tab() {
    assert_state!(
        "-[\t]>hello\n",
        |(buf, sels)| cmd_goto_first_nonblank(&buf, sels, 1, MotionMode::Move),
        "\t-[h]>ello\n"
    );
}

#[test]
fn goto_first_nonblank_no_leading_whitespace() {
    assert_state!(
        "-[h]>ello\n",
        |(buf, sels)| cmd_goto_first_nonblank(&buf, sels, 1, MotionMode::Move),
        "-[h]>ello\n"
    );
}

#[test]
fn goto_first_nonblank_all_blank_line() {
    // Line is all spaces — no non-blank found, cursor is unchanged.
    assert_state!(
        "-[ ]>  \n",
        |(buf, sels)| cmd_goto_first_nonblank(&buf, sels, 1, MotionMode::Move),
        "-[ ]>  \n"
    );
    assert_state!(
        " -[ ]>\n",
        |(buf, sels)| cmd_goto_first_nonblank(&buf, sels, 1, MotionMode::Move),
        " -[ ]>\n"
    );
}

// ── move_down ─────────────────────────────────────────────────────────────

#[test]
fn move_down_basic() {
    assert_state!(
        "-[h]>ello\nworld\n",
        |(buf, sels)| cmd_move_down(&buf, sels, 1, MotionMode::Move),
        "hello\n-[w]>orld\n"
    );
}

#[test]
fn move_down_preserves_column() {
    assert_state!(
        "hel-[l]>o\nworld\n",
        |(buf, sels)| cmd_move_down(&buf, sels, 1, MotionMode::Move),
        "hello\nwor-[l]>d\n"
    );
}

#[test]
fn move_down_clamps_to_shorter_line() {
    assert_state!(
        "hel-[l]>o\nab\n",
        |(buf, sels)| cmd_move_down(&buf, sels, 1, MotionMode::Move),
        "hello\na-[b]>\n"
    );
}

#[test]
fn move_down_clamp_on_last_line() {
    assert_state!(
        "hello\n-[w]>orld\n",
        |(buf, sels)| cmd_move_down(&buf, sels, 1, MotionMode::Move),
        "hello\n-[w]>orld\n"
    );
}

#[test]
fn move_down_to_empty_line() {
    assert_state!(
        "-[h]>ello\n\nworld\n",
        |(buf, sels)| cmd_move_down(&buf, sels, 1, MotionMode::Move),
        "hello\n-[\n]>world\n"
    );
}

#[test]
fn move_down_empty_buffer() {
    assert_state!(
        "-[\n]>",
        |(buf, sels)| cmd_move_down(&buf, sels, 1, MotionMode::Move),
        "-[\n]>"
    );
}

#[test]
fn move_down_multi_cursor_merge() {
    // Two cursors on line 0. Both move to line 1 — they converge and merge.
    assert_state!(
        "-[h]>ello\n-[w]>orld\n",
        |(buf, sels)| cmd_move_down(&buf, sels, 1, MotionMode::Move),
        "hello\n-[w]>orld\n"
    );
}

// ── move_up ───────────────────────────────────────────────────────────────

#[test]
fn move_up_basic() {
    assert_state!(
        "hello\n-[w]>orld\n",
        |(buf, sels)| cmd_move_up(&buf, sels, 1, MotionMode::Move),
        "-[h]>ello\nworld\n"
    );
}

#[test]
fn move_up_preserves_column() {
    assert_state!(
        "hello\nwor-[l]>d\n",
        |(buf, sels)| cmd_move_up(&buf, sels, 1, MotionMode::Move),
        "hel-[l]>o\nworld\n"
    );
}

#[test]
fn move_up_clamp_on_first_line() {
    assert_state!(
        "-[h]>ello\nworld\n",
        |(buf, sels)| cmd_move_up(&buf, sels, 1, MotionMode::Move),
        "-[h]>ello\nworld\n"
    );
}

#[test]
fn move_up_clamps_to_shorter_line() {
    // "ab" is 2 chars, "hello" is 5. Cursor at col 3 on "hello" → clamps to end of "ab".
    assert_state!(
        "ab\nhel-[l]>o\n",
        |(buf, sels)| cmd_move_up(&buf, sels, 1, MotionMode::Move),
        "a-[b]>\nhello\n"
    );
}

#[test]
fn move_down_count_3() {
    // From 'a' on line 0, move down 3 lines — lands on 'd'.
    assert_state!(
        "-[a]>\nb\nc\nd\ne\n",
        |(buf, sels)| cmd_move_down(&buf, sels, 3, MotionMode::Move),
        "a\nb\nc\n-[d]>\ne\n"
    );
}

// ── multi-cursor goto_line motions ────────────────────────────────────────

#[test]
fn goto_line_start_multi_cursor() {
    assert_state!(
        "hel-[l]>o\nwor-[l]>d\n",
        |(buf, sels)| cmd_goto_line_start(&buf, sels, 1, MotionMode::Move),
        "-[h]>ello\n-[w]>orld\n"
    );
}

#[test]
fn goto_line_end_multi_cursor() {
    assert_state!(
        "-[h]>ello\n-[w]>orld\n",
        |(buf, sels)| cmd_goto_line_end(&buf, sels, 1, MotionMode::Move),
        "hell-[o]>\nworl-[d]>\n"
    );
}

#[test]
fn goto_first_nonblank_multi_cursor() {
    // Both cursors are mid-line; each jumps to the first non-blank of its line.
    assert_state!(
        "  hel-[l]>o\n  wor-[l]>d\n",
        |(buf, sels)| cmd_goto_first_nonblank(&buf, sels, 1, MotionMode::Move),
        "  -[h]>ello\n  -[w]>orld\n"
    );
}

// ── multi-cursor merge on move_up ─────────────────────────────────────────

#[test]
fn move_up_multi_cursor_merge() {
    // Line 0 is "a\n" (1 content char). Two cursors on line 1 at cols 0 and 2.
    // Both move up: col 0 → 'a'(0); col 2 → clamps to 'a'(0). They merge.
    // Text content "a\norld\n" is unchanged; only one cursor remains.
    assert_state!(
        "a\n-[o]>r-[l]>d\n",
        |(buf, sels)| cmd_move_up(&buf, sels, 1, MotionMode::Move),
        "-[a]>\norld\n"
    );
}

#[test]
fn goto_first_nonblank_empty_buffer() {
    assert_state!(
        "-[\n]>",
        |(buf, sels)| cmd_goto_first_nonblank(&buf, sels, 1, MotionMode::Move),
        "-[\n]>"
    );
}

// ── extend line-start / line-end / first-nonblank ─────────────────────────

#[test]
fn extend_line_start_from_mid_line() {
    // Cursor on 'l' in "hello"; extend to line start: anchor stays at 'l', head at 'h'.
    assert_state!(
        "hel-[l]>o\n",
        |(buf, sels)| cmd_goto_line_start(&buf, sels, 1, MotionMode::Extend),
        "<[hell]-o\n"
    );
}

#[test]
fn extend_line_start_already_at_start() {
    // Already at line start — no-op.
    assert_state!(
        "-[h]>ello\n",
        |(buf, sels)| cmd_goto_line_start(&buf, sels, 1, MotionMode::Extend),
        "-[h]>ello\n"
    );
}

#[test]
fn extend_line_end_from_start() {
    // Cursor on 'h'; extend to end: anchor stays at 'h', head at 'o'.
    assert_state!(
        "-[h]>ello\n",
        |(buf, sels)| cmd_goto_line_end(&buf, sels, 1, MotionMode::Extend),
        "-[hello]>\n"
    );
}

#[test]
fn extend_line_end_already_at_end() {
    // Already at line end — no-op.
    assert_state!(
        "hell-[o]>\n",
        |(buf, sels)| cmd_goto_line_end(&buf, sels, 1, MotionMode::Extend),
        "hell-[o]>\n"
    );
}

#[test]
fn extend_first_nonblank_from_mid_line() {
    // Cursor on 'l'; extend to first nonblank 'h': backward extension.
    assert_state!(
        "hel-[l]>o\n",
        |(buf, sels)| cmd_goto_first_nonblank(&buf, sels, 1, MotionMode::Extend),
        "<[hell]-o\n"
    );
}

#[test]
fn extend_first_nonblank_from_indent() {
    // Text "  hello\n" (2 spaces), cursor at ' '(0); extend to 'h'(2).
    // anchor stays at 0, head = 2 → selection covers "  h".
    // Serialized with ]> after head: "-[  h]>ello\n".
    assert_state!(
        "-[ ]> hello\n",
        |(buf, sels)| cmd_goto_first_nonblank(&buf, sels, 1, MotionMode::Extend),
        "-[  h]>ello\n"
    );
}
