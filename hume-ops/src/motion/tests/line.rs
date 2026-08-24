use super::super::*;
use hume_test_fixtures::assert_state;

// ── goto_line_start ───────────────────────────────────────────────────────

#[test]
fn goto_line_start_from_middle() {
    assert_state!(
        "hel-[l]>o\n",
        |(text, sels)| cmd_goto_line_start(&text, sels, 1, MotionMode::Move),
        "-[h]>ello\n"
    );
}

#[test]
fn goto_line_start_already_at_start() {
    assert_state!(
        "-[h]>ello\n",
        |(text, sels)| cmd_goto_line_start(&text, sels, 1, MotionMode::Move),
        "-[h]>ello\n"
    );
}

#[test]
fn goto_line_start_second_line() {
    assert_state!(
        "hello\nwor-[l]>d\n",
        |(text, sels)| cmd_goto_line_start(&text, sels, 1, MotionMode::Move),
        "hello\n-[w]>orld\n"
    );
}

#[test]
fn goto_line_start_empty_buffer() {
    assert_state!(
        "-[\n]>",
        |(text, sels)| cmd_goto_line_start(&text, sels, 1, MotionMode::Move),
        "-[\n]>"
    );
}

// ── goto_line_end ─────────────────────────────────────────────────────────

#[test]
fn goto_line_end_from_start() {
    assert_state!(
        "-[h]>ello\n",
        |(text, sels)| cmd_goto_line_end(&text, sels, 1, MotionMode::Move),
        "hell-[o]>\n"
    );
}

#[test]
fn goto_line_end_already_at_end() {
    assert_state!(
        "hell-[o]>\n",
        |(text, sels)| cmd_goto_line_end(&text, sels, 1, MotionMode::Move),
        "hell-[o]>\n"
    );
}

#[test]
fn goto_line_end_stops_before_newline() {
    // Cursor must land on 'o', not on '\n'.
    assert_state!(
        "-[h]>ello\nworld\n",
        |(text, sels)| cmd_goto_line_end(&text, sels, 1, MotionMode::Move),
        "hell-[o]>\nworld\n"
    );
}

#[test]
fn goto_line_end_empty_line() {
    // Line contains only '\n'. Cursor stays on it.
    assert_state!(
        "-[\n]>",
        |(text, sels)| cmd_goto_line_end(&text, sels, 1, MotionMode::Move),
        "-[\n]>"
    );
}

#[test]
fn goto_line_end_last_line_no_newline() {
    assert_state!(
        "-[h]>ello\n",
        |(text, sels)| cmd_goto_line_end(&text, sels, 1, MotionMode::Move),
        "hell-[o]>\n"
    );
}

#[test]
fn goto_line_end_empty_buffer() {
    assert_state!(
        "-[\n]>",
        |(text, sels)| cmd_goto_line_end(&text, sels, 1, MotionMode::Move),
        "-[\n]>"
    );
}

// ── goto_first_nonblank ───────────────────────────────────────────────────

#[test]
fn goto_first_nonblank_skips_spaces() {
    assert_state!(
        "-[ ]> hello\n",
        |(text, sels)| cmd_goto_first_nonblank(&text, sels, 1, MotionMode::Move),
        "  -[h]>ello\n"
    );
}

#[test]
fn goto_first_nonblank_from_middle() {
    assert_state!(
        "  hel-[l]>o\n",
        |(text, sels)| cmd_goto_first_nonblank(&text, sels, 1, MotionMode::Move),
        "  -[h]>ello\n"
    );
}

#[test]
fn goto_first_nonblank_skips_tab() {
    assert_state!(
        "-[\t]>hello\n",
        |(text, sels)| cmd_goto_first_nonblank(&text, sels, 1, MotionMode::Move),
        "\t-[h]>ello\n"
    );
}

#[test]
fn goto_first_nonblank_no_leading_whitespace() {
    assert_state!(
        "-[h]>ello\n",
        |(text, sels)| cmd_goto_first_nonblank(&text, sels, 1, MotionMode::Move),
        "-[h]>ello\n"
    );
}

#[test]
fn goto_first_nonblank_all_blank_line() {
    // Line is all spaces — no non-blank found, cursor is unchanged.
    assert_state!(
        "-[ ]>  \n",
        |(text, sels)| cmd_goto_first_nonblank(&text, sels, 1, MotionMode::Move),
        "-[ ]>  \n"
    );
    assert_state!(
        " -[ ]>\n",
        |(text, sels)| cmd_goto_first_nonblank(&text, sels, 1, MotionMode::Move),
        " -[ ]>\n"
    );
}

// ── multi-cursor goto_line motions ────────────────────────────────────────

#[test]
fn goto_line_start_multi_cursor() {
    assert_state!(
        "hel-[l]>o\nwor-[l]>d\n",
        |(text, sels)| cmd_goto_line_start(&text, sels, 1, MotionMode::Move),
        "-[h]>ello\n-[w]>orld\n"
    );
}

#[test]
fn goto_line_end_multi_cursor() {
    assert_state!(
        "-[h]>ello\n-[w]>orld\n",
        |(text, sels)| cmd_goto_line_end(&text, sels, 1, MotionMode::Move),
        "hell-[o]>\nworl-[d]>\n"
    );
}

#[test]
fn goto_first_nonblank_multi_cursor() {
    // Both cursors are mid-line; each jumps to the first non-blank of its line.
    assert_state!(
        "  hel-[l]>o\n  wor-[l]>d\n",
        |(text, sels)| cmd_goto_first_nonblank(&text, sels, 1, MotionMode::Move),
        "  -[h]>ello\n  -[w]>orld\n"
    );
}

#[test]
fn goto_first_nonblank_empty_buffer() {
    assert_state!(
        "-[\n]>",
        |(text, sels)| cmd_goto_first_nonblank(&text, sels, 1, MotionMode::Move),
        "-[\n]>"
    );
}

// ── extend line-start / line-end / first-nonblank ─────────────────────────

#[test]
fn extend_line_start_from_mid_line() {
    // Cursor on 'l' in "hello"; extend to line start: anchor stays at 'l', head at 'h'.
    assert_state!(
        "hel-[l]>o\n",
        |(text, sels)| cmd_goto_line_start(&text, sels, 1, MotionMode::Extend),
        "<[hell]-o\n"
    );
}

#[test]
fn extend_line_start_already_at_start() {
    // Already at line start — no-op.
    assert_state!(
        "-[h]>ello\n",
        |(text, sels)| cmd_goto_line_start(&text, sels, 1, MotionMode::Extend),
        "-[h]>ello\n"
    );
}

#[test]
fn extend_line_end_from_start() {
    // Cursor on 'h'; extend to end: anchor stays at 'h', head at 'o'.
    assert_state!(
        "-[h]>ello\n",
        |(text, sels)| cmd_goto_line_end(&text, sels, 1, MotionMode::Extend),
        "-[hello]>\n"
    );
}

#[test]
fn extend_line_end_already_at_end() {
    // Already at line end — no-op.
    assert_state!(
        "hell-[o]>\n",
        |(text, sels)| cmd_goto_line_end(&text, sels, 1, MotionMode::Extend),
        "hell-[o]>\n"
    );
}

#[test]
fn extend_first_nonblank_from_mid_line() {
    // Cursor on 'l'; extend to first nonblank 'h': backward extension.
    assert_state!(
        "hel-[l]>o\n",
        |(text, sels)| cmd_goto_first_nonblank(&text, sels, 1, MotionMode::Extend),
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
        |(text, sels)| cmd_goto_first_nonblank(&text, sels, 1, MotionMode::Extend),
        "-[  h]>ello\n"
    );
}
