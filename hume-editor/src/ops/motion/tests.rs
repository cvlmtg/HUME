use super::*;
use crate::assert_state;

// ── move_right ────────────────────────────────────────────────────────────

#[test]
fn move_right_basic() {
    assert_state!(
        "-[h]>ello\n",
        |(buf, sels)| cmd_move_right(&buf, sels, 1, MotionMode::Move),
        "h-[e]>llo\n"
    );
}

#[test]
fn move_right_to_eof() {
    assert_state!(
        "hell-[o]>\n",
        |(buf, sels)| cmd_move_right(&buf, sels, 1, MotionMode::Move),
        "hello-[\n]>"
    );
}

#[test]
fn move_right_clamp_at_eof() {
    assert_state!(
        "hello-[\n]>",
        |(buf, sels)| cmd_move_right(&buf, sels, 1, MotionMode::Move),
        "hello-[\n]>"
    );
}

#[test]
fn move_right_empty_buffer() {
    assert_state!(
        "-[\n]>",
        |(buf, sels)| cmd_move_right(&buf, sels, 1, MotionMode::Move),
        "-[\n]>"
    );
}

#[test]
fn move_right_multi_cursor() {
    assert_state!(
        "-[h]>-[e]>llo\n",
        |(buf, sels)| cmd_move_right(&buf, sels, 1, MotionMode::Move),
        "h-[e]>-[l]>lo\n"
    );
}

#[test]
fn move_right_grapheme_cluster() {
    // "e\u{0301}" is two chars but one grapheme cluster (e + combining acute).
    // move_right from offset 0 must skip the entire cluster to offset 2.
    assert_state!(
        "-[e\u{0301}]>x\n",
        |(buf, sels)| cmd_move_right(&buf, sels, 1, MotionMode::Move),
        "e\u{0301}-[x]>\n"
    );
}

// ── move_left ─────────────────────────────────────────────────────────────

#[test]
fn move_left_basic() {
    assert_state!(
        "h-[e]>llo\n",
        |(buf, sels)| cmd_move_left(&buf, sels, 1, MotionMode::Move),
        "-[h]>ello\n"
    );
}

#[test]
fn move_left_clamp_at_start() {
    assert_state!(
        "-[h]>ello\n",
        |(buf, sels)| cmd_move_left(&buf, sels, 1, MotionMode::Move),
        "-[h]>ello\n"
    );
}

#[test]
fn move_left_empty_buffer() {
    assert_state!(
        "-[\n]>",
        |(buf, sels)| cmd_move_left(&buf, sels, 1, MotionMode::Move),
        "-[\n]>"
    );
}

#[test]
fn move_left_grapheme_cluster() {
    // "e\u{0301}" is two chars but one grapheme cluster.
    // move_left from offset 2 (after the cluster) must jump to 0.
    assert_state!(
        "e\u{0301}-[x]>\n",
        |(buf, sels)| cmd_move_left(&buf, sels, 1, MotionMode::Move),
        "-[e]>\u{0301}x\n"
    );
}

#[test]
fn move_left_multi_cursor_merge() {
    // Cursors at 0 and 1. Both move left: 0→0 and 1→0. Same position → merge.
    assert_state!(
        "-[a]>-[b]>c\n",
        |(buf, sels)| cmd_move_left(&buf, sels, 1, MotionMode::Move),
        "-[a]>bc\n"
    );
}

// ── extend_right ──────────────────────────────────────────────────────────

#[test]
fn extend_right_from_cursor() {
    // Collapsed cursor at 0. Extend right: anchor stays at 0, head moves to 1.
    // Forward selection anchor=0, head=1 → "-[he]>llo\n".
    assert_state!(
        "-[h]>ello\n",
        |(buf, sels)| cmd_move_right(&buf, sels, 1, MotionMode::Extend),
        "-[he]>llo\n"
    );
}

#[test]
fn extend_right_grows_selection() {
    // Existing forward selection anchor=0, head=1. Extend right: head moves to 2.
    // anchor=0, head=2 → "-[hel]>lo\n".
    assert_state!(
        "-[he]>llo\n",
        |(buf, sels)| cmd_move_right(&buf, sels, 1, MotionMode::Extend),
        "-[hel]>lo\n"
    );
}

#[test]
fn extend_right_clamp_at_eof() {
    assert_state!(
        "hello-[\n]>",
        |(buf, sels)| cmd_move_right(&buf, sels, 1, MotionMode::Extend),
        "hello-[\n]>"
    );
}

// ── extend_left ───────────────────────────────────────────────────────────

#[test]
fn extend_left_from_cursor() {
    // Collapsed cursor at 1. Extend left: anchor stays at 1, head moves to 0.
    // Backward selection anchor=1, head=0, selects "he" (2 chars).
    assert_state!(
        "h-[e]>llo\n",
        |(buf, sels)| cmd_move_left(&buf, sels, 1, MotionMode::Extend),
        "<[he]-llo\n"
    );
}

#[test]
fn extend_left_shrinks_forward_selection() {
    // Forward selection anchor=0, head=2. Extend left: head moves to 1.
    // anchor=0, head=1 → "-[he]>llo\n".
    assert_state!(
        "-[hel]>lo\n",
        |(buf, sels)| cmd_move_left(&buf, sels, 1, MotionMode::Extend),
        "-[he]>llo\n"
    );
}

#[test]
fn extend_left_clamp_at_start() {
    assert_state!(
        "-[h]>ello\n",
        |(buf, sels)| cmd_move_left(&buf, sels, 1, MotionMode::Extend),
        "-[h]>ello\n"
    );
}

#[test]
fn extend_left_reverses_direction() {
    // Forward selection anchor=3,head=3. Extend left 3 times: head→0.
    // anchor=3 > head=0 → becomes a backward selection spanning "hell".
    assert_state!(
        "hel-[l]>o\n",
        |(buf, sels)| cmd_move_left(&buf, sels, 3, MotionMode::Extend),
        "<[hell]-o\n"
    );
}

#[test]
fn extend_right_crosses_newline() {
    // Cursor on '\n' at end of first line. Extend right: head crosses newline
    // onto the first char of the next line.
    // "hello\nworld\n": '\n'=5, 'w'=6. anchor=5, head→6.
    assert_state!(
        "hello-[\n]>world\n",
        |(buf, sels)| cmd_move_right(&buf, sels, 1, MotionMode::Extend),
        "hello-[\nw]>orld\n"
    );
}

#[test]
fn extend_left_crosses_newline() {
    // Cursor on first char of second line. Extend left: head crosses newline
    // onto the '\n' of the previous line. "hello\nworld\n": '\n'=5, 'w'=6.
    // anchor=6 stays on 'w'; head→5 ('\n'). Backward selection covers "\nw".
    assert_state!(
        "hello\n-[w]>orld\n",
        |(buf, sels)| cmd_move_left(&buf, sels, 1, MotionMode::Extend),
        "hello<[\nw]-orld\n"
    );
}

#[test]
fn extend_right_multi_cursor() {
    // Two independent cursors both extend right by 2. They grow their own
    // selections without merging (ranges remain disjoint).
    // "foo bar\n": f=0,o=1,o=2,' '=3,b=4,a=5,r=6,'\n'=7.
    // cursor1 anchor=0,head=0 → head=2 → "-[foo]>"
    // cursor2 anchor=4,head=4 → head=6 → "-[bar]>"
    assert_state!(
        "-[f]>oo -[b]>ar\n",
        |(buf, sels)| cmd_move_right(&buf, sels, 2, MotionMode::Extend),
        "-[foo]> -[bar]>\n"
    );
}

// ── goto_first_line ───────────────────────────────────────────────────────

#[test]
fn goto_first_line_from_middle() {
    assert_state!(
        "hello\nwor-[l]>d\n",
        |(buf, sels)| cmd_goto_first_line(&buf, sels, 1, MotionMode::Move),
        "-[h]>ello\nworld\n"
    );
}

#[test]
fn goto_first_line_already_at_start() {
    assert_state!(
        "-[h]>ello\nworld\n",
        |(buf, sels)| cmd_goto_first_line(&buf, sels, 1, MotionMode::Move),
        "-[h]>ello\nworld\n"
    );
}

#[test]
fn goto_first_line_single_line_buffer() {
    assert_state!(
        "hel-[l]>o\n",
        |(buf, sels)| cmd_goto_first_line(&buf, sels, 1, MotionMode::Move),
        "-[h]>ello\n"
    );
}

#[test]
fn goto_first_line_empty_buffer() {
    assert_state!(
        "-[\n]>",
        |(buf, sels)| cmd_goto_first_line(&buf, sels, 1, MotionMode::Move),
        "-[\n]>"
    );
}

#[test]
fn goto_first_line_multi_cursor() {
    assert_state!(
        "-[a]>bc\ndef\nghi-[j]>\n",
        |(buf, sels)| cmd_goto_first_line(&buf, sels, 1, MotionMode::Move),
        "-[a]>bc\ndef\nghij\n"
    );
}

// ── goto_last_line ────────────────────────────────────────────────────────

#[test]
fn goto_last_line_from_first() {
    assert_state!(
        "-[h]>ello\nworld\n",
        |(buf, sels)| cmd_goto_last_line(&buf, sels, 1, MotionMode::Move),
        "hello\n-[w]>orld\n"
    );
}

#[test]
fn goto_last_line_already_at_last() {
    assert_state!(
        "hello\n-[w]>orld\n",
        |(buf, sels)| cmd_goto_last_line(&buf, sels, 1, MotionMode::Move),
        "hello\n-[w]>orld\n"
    );
}

#[test]
fn goto_last_line_single_line_buffer() {
    assert_state!(
        "-[\n]>",
        |(buf, sels)| cmd_goto_last_line(&buf, sels, 1, MotionMode::Move),
        "-[\n]>"
    );
}

#[test]
fn goto_last_line_multi_line() {
    assert_state!(
        "aaa\n-[b]>bb\nccc\n",
        |(buf, sels)| cmd_goto_last_line(&buf, sels, 1, MotionMode::Move),
        "aaa\nbbb\n-[c]>cc\n"
    );
}

#[test]
fn goto_last_line_multi_cursor() {
    // Both cursors converge to the same position — merged into one.
    assert_state!(
        "-[a]>aa\nbbb\n-[c]>cc\n",
        |(buf, sels)| cmd_goto_last_line(&buf, sels, 1, MotionMode::Move),
        "aaa\nbbb\n-[c]>cc\n"
    );
}

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
// ── cmd_select_next_word (w) ──────────────────────────────────────────────

#[test]
fn select_next_word_basic() {
    // From 'h', selects "world" (the next word). Fresh anchor at word start.
    assert_state!(
        "-[h]>ello world\n",
        |(buf, sels)| cmd_select_next_word(&buf, sels, 1, MotionMode::Move),
        "hello -[world]>\n"
    );
}

#[test]
fn select_next_word_from_mid_word() {
    // Cursor in the middle of "hello" — still jumps to next word "world".
    assert_state!(
        "hel-[l]>o world\n",
        |(buf, sels)| cmd_select_next_word(&buf, sels, 1, MotionMode::Move),
        "hello -[world]>\n"
    );
}

#[test]
fn select_next_word_from_whitespace() {
    // From the space between words, selects the next word "world".
    assert_state!(
        "hello-[ ]>world\n",
        |(buf, sels)| cmd_select_next_word(&buf, sels, 1, MotionMode::Move),
        "hello -[world]>\n"
    );
}

#[test]
fn select_next_word_crosses_newline() {
    // w crosses the newline and selects the first word on the next line.
    assert_state!(
        "-[h]>ello\nworld\n",
        |(buf, sels)| cmd_select_next_word(&buf, sels, 1, MotionMode::Move),
        "hello\n-[world]>\n"
    );
}

#[test]
fn select_next_word_crosses_multiple_blank_lines() {
    // Multiple blank lines between words — w still reaches the next word.
    assert_state!(
        "-[h]>ello\n\n\nworld\n",
        |(buf, sels)| cmd_select_next_word(&buf, sels, 1, MotionMode::Move),
        "hello\n\n\n-[world]>\n"
    );
}

#[test]
fn select_next_word_at_last_word_is_noop() {
    // Cursor on the last word in the buffer — no-op.
    assert_state!(
        "hello -[world]>\n",
        |(buf, sels)| cmd_select_next_word(&buf, sels, 1, MotionMode::Move),
        "hello -[world]>\n"
    );
}

#[test]
fn select_next_word_at_eof_is_noop() {
    // Cursor on trailing '\n' — no-op.
    assert_state!(
        "hello-[\n]>",
        |(buf, sels)| cmd_select_next_word(&buf, sels, 1, MotionMode::Move),
        "hello-[\n]>"
    );
}

#[test]
fn select_next_word_empty_buffer_is_noop() {
    assert_state!(
        "-[\n]>",
        |(buf, sels)| cmd_select_next_word(&buf, sels, 1, MotionMode::Move),
        "-[\n]>"
    );
}

#[test]
fn select_next_word_word_to_punct() {
    // "hello" and "." are different word classes — w selects ".".
    assert_state!(
        "-[h]>ello.world\n",
        |(buf, sels)| cmd_select_next_word(&buf, sels, 1, MotionMode::Move),
        "hello-[.]>world\n"
    );
}

#[test]
fn select_next_word_punct_to_word() {
    // From ".", the next word class token is "hello".
    assert_state!(
        "-[.]>hello\n",
        |(buf, sels)| cmd_select_next_word(&buf, sels, 1, MotionMode::Move),
        ".-[hello]>\n"
    );
}

#[test]
fn select_next_word_count_2() {
    // count=2: skips "world", selects "foo".
    assert_state!(
        "-[h]>ello world foo\n",
        |(buf, sels)| cmd_select_next_word(&buf, sels, 2, MotionMode::Move),
        "hello world -[foo]>\n"
    );
}

#[test]
fn select_next_word_count_stops_at_last_word() {
    // count=3 but only 2 words remain after cursor — stops at "foo".
    assert_state!(
        "-[h]>ello world foo\n",
        |(buf, sels)| cmd_select_next_word(&buf, sels, 3, MotionMode::Move),
        "hello world -[foo]>\n"
    );
}

// ── cmd_select_prev_word (b) ──────────────────────────────────────────────

#[test]
fn select_prev_word_basic() {
    // From "world", selects the previous word "hello".
    assert_state!(
        "hello -[world]>\n",
        |(buf, sels)| cmd_select_prev_word(&buf, sels, 1, MotionMode::Move),
        "-[hello]> world\n"
    );
}

#[test]
fn select_prev_word_from_mid_word() {
    // Cursor in the middle of "world" — jumps to previous word "hello".
    assert_state!(
        "hello wor-[l]>d\n",
        |(buf, sels)| cmd_select_prev_word(&buf, sels, 1, MotionMode::Move),
        "-[hello]> world\n"
    );
}

#[test]
fn select_prev_word_from_whitespace() {
    // From the space between words, selects the previous word "hello".
    assert_state!(
        "hello-[ ]>world\n",
        |(buf, sels)| cmd_select_prev_word(&buf, sels, 1, MotionMode::Move),
        "-[hello]> world\n"
    );
}

#[test]
fn select_prev_word_from_punct() {
    // Cursor on the '.' punctuation — selects the preceding word "hello".
    assert_state!(
        "hello-[.]>world\n",
        |(buf, sels)| cmd_select_prev_word(&buf, sels, 1, MotionMode::Move),
        "-[hello]>.world\n"
    );
}

#[test]
fn select_prev_word_from_trailing_newline() {
    // Cursor on the trailing '\n' — selects the last word on the line.
    assert_state!(
        "hello world-[\n]>",
        |(buf, sels)| cmd_select_prev_word(&buf, sels, 1, MotionMode::Move),
        "hello -[world]>\n"
    );
}

#[test]
fn select_prev_word_crosses_newline() {
    // b crosses the newline and selects the last word on the previous line.
    assert_state!(
        "hello\n-[world]>\n",
        |(buf, sels)| cmd_select_prev_word(&buf, sels, 1, MotionMode::Move),
        "-[hello]>\nworld\n"
    );
}

#[test]
fn select_prev_word_at_first_word_is_noop() {
    // Cursor on first word — no-op.
    assert_state!(
        "-[hello]> world\n",
        |(buf, sels)| cmd_select_prev_word(&buf, sels, 1, MotionMode::Move),
        "-[hello]> world\n"
    );
}

#[test]
fn select_prev_word_in_first_word_mid_is_noop() {
    // Cursor in the middle of the first word — no previous word, no-op.
    assert_state!(
        "hel-[l]>o world\n",
        |(buf, sels)| cmd_select_prev_word(&buf, sels, 1, MotionMode::Move),
        "hel-[l]>o world\n"
    );
}

#[test]
fn select_prev_word_at_buffer_start_is_noop() {
    assert_state!(
        "-[h]>ello\n",
        |(buf, sels)| cmd_select_prev_word(&buf, sels, 1, MotionMode::Move),
        "-[h]>ello\n"
    );
}

#[test]
fn select_prev_word_empty_buffer_is_noop() {
    assert_state!(
        "-[\n]>",
        |(buf, sels)| cmd_select_prev_word(&buf, sels, 1, MotionMode::Move),
        "-[\n]>"
    );
}

#[test]
fn select_prev_word_count_2() {
    // count=2: from "foo", skips "world", selects "hello".
    assert_state!(
        "hello world -[foo]>\n",
        |(buf, sels)| cmd_select_prev_word(&buf, sels, 2, MotionMode::Move),
        "-[hello]> world foo\n"
    );
}

#[test]
fn select_prev_word_count_overshoots() {
    // count=5 but only 2 words precede "foo" — stops at "hello" rather than erroring.
    assert_state!(
        "hello world -[foo]>\n",
        |(buf, sels)| cmd_select_prev_word(&buf, sels, 5, MotionMode::Move),
        "-[hello]> world foo\n"
    );
}

// ── WORD variants (W / B) ─────────────────────────────────────────────────

#[test]
#[allow(non_snake_case)]
fn select_next_uppercase_word_skips_punct() {
    // W: "hello.world" is a single WORD — W selects it entirely.
    assert_state!(
        "-[h]>ello.world bar\n",
        |(buf, sels)| cmd_select_next_uppercase_word(&buf, sels, 1, MotionMode::Move),
        "hello.world -[bar]>\n"
    );
}

#[test]
#[allow(non_snake_case)]
fn select_next_uppercase_word_crosses_newline() {
    // W at end of a line crosses the newline and selects the first WORD on the next line.
    assert_state!(
        "-[h]>ello.world\nbar\n",
        |(buf, sels)| cmd_select_next_uppercase_word(&buf, sels, 1, MotionMode::Move),
        "hello.world\n-[bar]>\n"
    );
}

#[test]
fn select_next_word_stops_at_punct() {
    // w (lowercase): "hello" and "." are separate word-class tokens.
    assert_state!(
        "-[h]>ello.world bar\n",
        |(buf, sels)| cmd_select_next_word(&buf, sels, 1, MotionMode::Move),
        "hello-[.]>world bar\n"
    );
}

#[test]
#[allow(non_snake_case)]
fn select_prev_uppercase_word_skips_punct() {
    // B: from "bar", jumps back over "hello.world" as ONE WORD (the dot is not
    // a WORD boundary), selecting the whole token.
    assert_state!(
        "hello.world -[bar]>\n",
        |(buf, sels)| cmd_select_prev_uppercase_word(&buf, sels, 1, MotionMode::Move),
        "-[hello.world]> bar\n"
    );
}

#[test]
#[allow(non_snake_case)]
fn select_prev_uppercase_word_crosses_newline() {
    // B at the start of a line crosses the newline and selects the last WORD on the previous line.
    assert_state!(
        "hello.world\n-[bar]>\n",
        |(buf, sels)| cmd_select_prev_uppercase_word(&buf, sels, 1, MotionMode::Move),
        "-[hello.world]>\nbar\n"
    );
}

// ── grapheme cluster correctness ──────────────────────────────────────────

#[test]
fn select_next_word_skips_combining_grapheme() {
    // Text: "cafe\u{0301} world\n" — graphemes: {c}{a}{f}{e◌́}{ }{w}{o}{r}{l}{d}{\n}
    // The combining codepoint U+0301 (offset 4) must not create a false word
    // boundary inside the grapheme cluster {e◌́}. w selects "world".
    assert_state!(
        "-[c]>afe\u{0301} world\n",
        |(buf, sels)| cmd_select_next_word(&buf, sels, 1, MotionMode::Move),
        "cafe\u{0301} -[world]>\n"
    );
}

#[test]
fn select_prev_word_skips_combining_grapheme() {
    // Text: "cafe\u{0301} world\n", cursor on 'w'.
    // b must step over the combining grapheme {e◌́} as a unit (Word class)
    // and select all of "cafe\u{0301}" as one word.
    assert_state!(
        "cafe\u{0301} -[w]>orld\n",
        |(buf, sels)| cmd_select_prev_word(&buf, sels, 1, MotionMode::Move),
        "-[cafe\u{0301}]> world\n"
    );
}

// ── next_paragraph (]p) ───────────────────────────────────────────────────

#[test]
fn next_paragraph_basic() {
    // Skip "hello\nworld" paragraph and the empty gap line, land on "foo".
    assert_state!(
        "-[h]>ello\nworld\n\nfoo\n",
        |(buf, sels)| cmd_next_paragraph(&buf, sels, 1, MotionMode::Move),
        "hello\nworld\n\n-[f]>oo\n"
    );
}

#[test]
fn next_paragraph_no_paragraph_below() {
    // No empty line below — land at EOF.
    assert_state!(
        "-[h]>ello\nworld\n",
        |(buf, sels)| cmd_next_paragraph(&buf, sels, 1, MotionMode::Move),
        "hello\nworld-[\n]>"
    );
}

#[test]
fn next_paragraph_from_empty_line() {
    // Starting on an empty line — skip the gap, land on the next paragraph.
    assert_state!(
        "-[\n]>\nfoo\n",
        |(buf, sels)| cmd_next_paragraph(&buf, sels, 1, MotionMode::Move),
        "\n\n-[f]>oo\n"
    );
}

#[test]
fn next_paragraph_multiple_empty_lines() {
    // Multiple empty lines in the gap — skip all of them.
    assert_state!(
        "-[\n]>\n\nfoo\n",
        |(buf, sels)| cmd_next_paragraph(&buf, sels, 1, MotionMode::Move),
        "\n\n\n-[f]>oo\n"
    );
}

#[test]
fn next_paragraph_empty_buffer() {
    assert_state!(
        "-[\n]>",
        |(buf, sels)| cmd_next_paragraph(&buf, sels, 1, MotionMode::Move),
        "-[\n]>"
    );
}

#[test]
fn next_paragraph_at_eof() {
    assert_state!(
        "hello-[\n]>",
        |(buf, sels)| cmd_next_paragraph(&buf, sels, 1, MotionMode::Move),
        "hello-[\n]>"
    );
}

// ── prev_paragraph ([p) ───────────────────────────────────────────────────

#[test]
fn prev_paragraph_basic() {
    // Land on the empty gap line above "world".
    assert_state!(
        "hello\n\nwor-[l]>d\n",
        |(buf, sels)| cmd_prev_paragraph(&buf, sels, 1, MotionMode::Move),
        "hello\n-[\n]>world\n"
    );
}

#[test]
fn prev_paragraph_multiple_empty_lines() {
    // Multiple empty lines — land on the first (topmost) one.
    assert_state!(
        "hello\n\n\nwor-[l]>d\n",
        |(buf, sels)| cmd_prev_paragraph(&buf, sels, 1, MotionMode::Move),
        "hello\n-[\n]>\nworld\n"
    );
}

#[test]
fn prev_paragraph_no_paragraph_above() {
    // No gap above — land on line 0 (no-op if already there).
    assert_state!(
        "-[h]>ello\nworld\n",
        |(buf, sels)| cmd_prev_paragraph(&buf, sels, 1, MotionMode::Move),
        "-[h]>ello\nworld\n"
    );
}

#[test]
fn prev_paragraph_from_empty_line() {
    // Starting on the empty gap line — skip gap + paragraph, land on the
    // empty line above the paragraph before it.
    assert_state!(
        "hello\n-[\n]>world\n",
        |(buf, sels)| cmd_prev_paragraph(&buf, sels, 1, MotionMode::Move),
        "-[h]>ello\n\nworld\n"
    );
}

// ── multi-paragraph navigation ────────────────────────────────────────────

#[test]
fn next_paragraph_sequential() {
    // Two consecutive ]p motions walk through three paragraphs.
    assert_state!(
        "-[a]>\n\nb\n\nc\n",
        |(buf, sels)| cmd_next_paragraph(&buf, sels, 1, MotionMode::Move),
        "a\n\n-[b]>\n\nc\n"
    );
    assert_state!(
        "a\n\n-[b]>\n\nc\n",
        |(buf, sels)| cmd_next_paragraph(&buf, sels, 1, MotionMode::Move),
        "a\n\nb\n\n-[c]>\n"
    );
}

#[test]
fn prev_paragraph_sequential() {
    // Two consecutive [p motions walk backward through three paragraphs.
    assert_state!(
        "a\n\nb\n\n-[c]>\n",
        |(buf, sels)| cmd_prev_paragraph(&buf, sels, 1, MotionMode::Move),
        "a\n\nb\n-[\n]>c\n"
    );
    assert_state!(
        "a\n\nb\n-[\n]>c\n",
        |(buf, sels)| cmd_prev_paragraph(&buf, sels, 1, MotionMode::Move),
        "a\n-[\n]>b\n\nc\n"
    );
}

// ── extend variants ───────────────────────────────────────────────────────

#[test]
fn extend_next_paragraph_creates_selection() {
    // Anchor stays at 0, head moves to 'w' at the start of "world".
    assert_state!(
        "-[h]>ello\n\nworld\n",
        |(buf, sels)| cmd_next_paragraph(&buf, sels, 1, MotionMode::Extend),
        "-[hello\n\nw]>orld\n"
    );
}

#[test]
fn extend_prev_paragraph_creates_selection() {
    // Anchor stays on 'w', head moves back to the empty gap line.
    assert_state!(
        "hello\n\n-[w]>orld\n",
        |(buf, sels)| cmd_prev_paragraph(&buf, sels, 1, MotionMode::Extend),
        "hello\n<[\nw]-orld\n"
    );
}

// ── count prefix ──────────────────────────────────────────────────────────

#[test]
fn move_right_count_3() {
    // h(0) → e(1) → l(2) → l(3)
    assert_state!(
        "-[h]>ello\n",
        |(buf, sels)| cmd_move_right(&buf, sels, 3, MotionMode::Move),
        "hel-[l]>o\n"
    );
}

#[test]
fn move_right_count_clamps_at_eof() {
    // count=100 far exceeds the buffer length — clamps at the trailing '\n'.
    assert_state!(
        "-[h]>ello\n",
        |(buf, sels)| cmd_move_right(&buf, sels, 100, MotionMode::Move),
        "hello-[\n]>"
    );
}

#[test]
fn move_left_count_3() {
    // \n(5) → o(4) → l(3) → l(2)
    assert_state!(
        "hello-[\n]>",
        |(buf, sels)| cmd_move_left(&buf, sels, 3, MotionMode::Move),
        "he-[l]>lo\n"
    );
}

#[test]
fn extend_right_count_3() {
    // Extend: anchor stays at old head (0), head folds 3 steps: 0→1→2→3.
    // Selection anchor=0, head=3: covers "hell".
    assert_state!(
        "-[h]>ello\n",
        |(buf, sels)| cmd_move_right(&buf, sels, 3, MotionMode::Extend),
        "-[hell]>o\n"
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

#[test]
fn move_right_count_grapheme_cluster() {
    // Text: "e◌́x\n". Grapheme clusters: {e◌́}(0..2), {x}(2), {\n}(3).
    // count=2 from offset 0: step1 → 2 (x), step2 → 3 (\n). Clamped to len-1=3.
    assert_state!(
        "-[e\u{0301}]>x\n",
        |(buf, sels)| cmd_move_right(&buf, sels, 2, MotionMode::Move),
        "e\u{0301}x-[\n]>"
    );
}

#[test]
fn multi_cursor_count_independent_movement() {
    // Two cursors: 'h'(0) and 'l'(2). move_right count=3.
    // Cursor 0: 0→1→2→3 (second 'l'). Cursor 2: 2→3→4→5 ('\n').
    // No merge — different positions.
    assert_state!(
        "-[h]>el-[l]>o\n",
        |(buf, sels)| cmd_move_right(&buf, sels, 3, MotionMode::Move),
        "hel-[l]>o-[\n]>"
    );
}

// ── multi-cursor word motions ──────────────────────────────────────────────

#[test]
fn select_next_word_multi_cursor() {
    // Two cursors: each independently selects the next word from its position.
    // Cursor 1 at 'h'(0): next word is "foo"(6..8).
    // Cursor 2 at 'f'(6): next word is "bar"(10..12).
    assert_state!(
        "-[h]>ello -[f]>oo bar\n",
        |(buf, sels)| cmd_select_next_word(&buf, sels, 1, MotionMode::Move),
        "hello -[foo]> -[bar]>\n"
    );
}

#[test]
fn select_prev_word_multi_cursor() {
    // Two cursors each jump to the previous word independently.
    // Cursor 1 on "hello" (head=8) → prev word "foo" → [0,2].
    // Cursor 2 on "world" (head=14) → prev word "hello" → [4,8].
    // No merging because [0,2] and [4,8] are disjoint.
    assert_state!(
        "foo -[hello]> -[world]> bar\n",
        |(buf, sels)| cmd_select_prev_word(&buf, sels, 1, MotionMode::Move),
        "-[foo]> -[hello]> world bar\n"
    );
}

// ── around-word variants (w/W/b/B covering surrounding whitespace) ────────
//
// These wrap the same select_next_word/select_prev_word motions used above,
// so movement is identical; only the final span differs. Each covers the
// destination word's surrounding whitespace: leading preferred, trailing
// fallback when the word is the first on its line (any leading run there is
// indentation, never absorbed) or when there's no leading run at all. EOL is
// never consumed on either side. Used when `word-selects-whitespace` is on
// (see `run_native_body`).

#[test]
fn select_next_word_around_leading_basic() {
    // "bar" isn't the first word on its line, so its single leading space is
    // absorbed. The three spaces after "bar" belong to "baz"'s leading run
    // instead, and are left untouched.
    assert_state!(
        "-[f]>oo bar   baz\n",
        |(buf, sels)| cmd_select_next_word_around(&buf, sels, 1, MotionMode::Move),
        "foo-[ bar]>   baz\n"
    );
}

#[test]
fn select_next_word_around_leading_tab() {
    // Tab classifies as Space — counts as leading whitespace too.
    assert_state!(
        "-[f]>oo\tbar baz\n",
        |(buf, sels)| cmd_select_next_word_around(&buf, sels, 1, MotionMode::Move),
        "foo-[\tbar]> baz\n"
    );
}

#[test]
fn select_next_word_around_leading_nbsp() {
    // U+00A0 (NBSP) classifies as Space too.
    assert_state!(
        "-[f]>oo\u{00A0}bar baz\n",
        |(buf, sels)| cmd_select_next_word_around(&buf, sels, 1, MotionMode::Move),
        "foo-[\u{00A0}bar]> baz\n"
    );
}

#[test]
fn select_next_word_around_leading_mid_line_before_eol() {
    // "bar" isn't the first word on its line (that's "foo"), so it takes its
    // leading space even though it's also the last word before EOL.
    assert_state!(
        "-[f]>oo bar\n",
        |(buf, sels)| cmd_select_next_word_around(&buf, sels, 1, MotionMode::Move),
        "foo-[ bar]>\n"
    );
}

#[test]
fn select_next_word_around_leading_mid_line_before_punctuation() {
    // Same rule applies regardless of what follows the word.
    assert_state!(
        "-[f]>oo bar,baz\n",
        |(buf, sels)| cmd_select_next_word_around(&buf, sels, 1, MotionMode::Move),
        "foo-[ bar]>,baz\n"
    );
}

#[test]
fn select_next_word_around_punctuation_destination_gets_leading_space() {
    // w can land on a punctuation run just like a word — it gets the same
    // around treatment.
    assert_state!(
        "-[f]>oo , bar\n",
        |(buf, sels)| cmd_select_next_word_around(&buf, sels, 1, MotionMode::Move),
        "foo-[ ,]> bar\n"
    );
}

#[test]
fn select_next_word_around_first_word_of_line_indented_takes_trailing() {
    // "bar" is the first word on its line — the leading run is indentation
    // and is never absorbed; the trailing space (before "baz") is used
    // instead, same as the un-indented first-word case.
    assert_state!(
        "-[f]>oo\n  bar baz\n",
        |(buf, sels)| cmd_select_next_word_around(&buf, sels, 1, MotionMode::Move),
        "foo\n  -[bar ]>baz\n"
    );
}

#[test]
fn select_next_word_around_first_word_of_line_indented_no_trailing_is_bare() {
    // "foo" is the first (and only) word on its line, indented, with EOL
    // right after it — neither side qualifies, so the indentation is kept
    // and the result is bare.
    assert_state!(
        "x\n-[ ]>   foo\n",
        |(buf, sels)| cmd_select_next_word_around(&buf, sels, 1, MotionMode::Move),
        "x\n    -[foo]>\n"
    );
}

#[test]
fn select_next_word_around_eol_never_consumed() {
    // "world" is followed by the trailing '\n' (Eol, not Space) and preceded
    // by the newline that starts its own line (also Eol) — neither side
    // extends. The around variant is a no-op here, same as bare `w`.
    assert_state!(
        "-[h]>ello\nworld\n",
        |(buf, sels)| cmd_select_next_word_around(&buf, sels, 1, MotionMode::Move),
        "hello\n-[world]>\n"
    );
}

#[test]
fn select_next_word_around_at_last_word_is_noop() {
    // Guard: the motion itself is a no-op (already on the last word), so no
    // expansion is attempted even though "world" has a leading space that
    // would otherwise be absorbed.
    assert_state!(
        "hello -[world]>\n",
        |(buf, sels)| cmd_select_next_word_around(&buf, sels, 1, MotionMode::Move),
        "hello -[world]>\n"
    );
}

#[test]
fn select_next_word_around_count_2_expands_only_final_span() {
    // count=2 hops through "world" (which has extra surrounding spaces of
    // its own) on the way to "foo" — only the final landing span gets
    // expanded, not each intermediate hop.
    // "hello   world  foo\n": positions 13-14 are the two spaces before "foo".
    assert_state!(
        "-[h]>ello   world  foo\n",
        |(buf, sels)| cmd_select_next_word_around(&buf, sels, 2, MotionMode::Move),
        "hello   world-[  foo]>\n"
    );
}

#[test]
fn select_next_word_around_second_press_advances_past_first_word() {
    // Forward search always uses `head()` as the origin (see
    // apply_word_select's doc comment), and a leading expansion only ever
    // moves `start` — so `head()` lands on the found word's own last char
    // and the next press's search continues correctly from there. Chains
    // three SEPARATE `w` presses (not a single count=3 call) to pin that down.
    assert_state!(
        "-[o]>ne two three four\n",
        |(buf, sels)| {
            let s1 = cmd_select_next_word_around(&buf, sels, 1, MotionMode::Move); // " two"
            let s2 = cmd_select_next_word_around(&buf, s1, 1, MotionMode::Move); // " three"
            cmd_select_next_word_around(&buf, s2, 1, MotionMode::Move) // " four", not " three" again
        },
        "one two three-[ four]>\n"
    );
}

#[test]
fn select_next_word_around_multi_cursor_adjacent_cursors_stay_disjoint() {
    // Cursor 1 lands on "bar" and absorbs its leading space; cursor 2 lands
    // on "baz" and absorbs *its* leading space (the one right after "bar").
    // The two expanded spans are adjacent but don't overlap, so they stay
    // separate selections rather than merging.
    // "foo bar baz\n": f=0..2,' '=3,b=4..6,' '=7,b=8..10,'\n'=11.
    assert_state!(
        "-[f]>oo -[b]>ar baz\n",
        |(buf, sels)| cmd_select_next_word_around(&buf, sels, 1, MotionMode::Move),
        "foo-[ bar]>-[ baz]>\n"
    );
}

#[test]
fn select_next_word_around_skips_combining_grapheme() {
    // Text: "cafe\u{0301} world\n" — the combining acute must not be
    // misread as a word-class char when scanning for the leading space.
    assert_state!(
        "-[c]>afe\u{0301} world\n",
        |(buf, sels)| cmd_select_next_word_around(&buf, sels, 1, MotionMode::Move),
        "cafe\u{0301}-[ world]>\n"
    );
}

#[test]
#[allow(non_snake_case)]
fn select_next_uppercase_word_around_punct_leading() {
    // W: "foo," is one WORD (punctuation merged in) and isn't "bar"'s own
    // line-start, so "bar" takes its leading space.
    assert_state!(
        "-[f]>oo, bar\n",
        |(buf, sels)| cmd_select_next_uppercase_word_around(&buf, sels, 1, MotionMode::Move),
        "foo,-[ bar]>\n"
    );
}

#[test]
fn select_prev_word_around_first_word_of_buffer_takes_trailing() {
    // "hello" is the first word of the buffer — no leading run is possible —
    // so it falls back to its trailing space (the one before "world").
    assert_state!(
        "hello -[world]>\n",
        |(buf, sels)| cmd_select_prev_word_around(&buf, sels, 1, MotionMode::Move),
        "-[hello ]>world\n"
    );
}

#[test]
fn select_prev_word_around_leading_mid_line() {
    // Plain word-to-word case: b lands on "bar", which isn't the first word
    // on its line, so it takes its leading space.
    assert_state!(
        "foo bar -[b]>az\n",
        |(buf, sels)| cmd_select_prev_word_around(&buf, sels, 1, MotionMode::Move),
        "foo-[ bar]> baz\n"
    );
}

#[test]
fn select_prev_word_around_leading_mid_line_before_punctuation() {
    // Cursor starts on the punctuation right after "bar"; b lands on "bar"
    // directly, which still isn't the first word on its line, so it takes
    // its leading space regardless of what follows.
    assert_state!(
        "foo bar-[,]>baz\n",
        |(buf, sels)| cmd_select_prev_word_around(&buf, sels, 1, MotionMode::Move),
        "foo-[ bar]>,baz\n"
    );
}

#[test]
#[allow(non_snake_case)]
fn select_prev_uppercase_word_around_first_word_of_buffer_takes_trailing() {
    // B: "hello.world" is one WORD and is the first word of the buffer, so
    // it falls back to its trailing space (before "bar").
    assert_state!(
        "hello.world -[bar]>\n",
        |(buf, sels)| cmd_select_prev_uppercase_word_around(&buf, sels, 1, MotionMode::Move),
        "-[hello.world ]>bar\n"
    );
}

#[test]
fn select_prev_word_around_second_press_advances_past_first_word() {
    // Regression: `select_prev_word`'s "am I still on the word I just found"
    // check uses `current.start()` as the search origin (not `head()`,
    // which after a *first-word* landing can sit in that word's trailing
    // whitespace, just outside its own bounds — see apply_word_select's doc
    // comment). Chains three presses: the first two land mid-line (leading
    // absorption moves `start`, not `head`, so the bug can't occur there
    // anyway); the third lands on "one", the first word of the buffer, which
    // *does* absorb trailing whitespace into `head` — proving the next press
    // still advances instead of getting stuck re-selecting "one".
    assert_state!(
        "one two three -[f]>our\n",
        |(buf, sels)| {
            let s1 = cmd_select_prev_word_around(&buf, sels, 1, MotionMode::Move); // " three"
            let s2 = cmd_select_prev_word_around(&buf, s1, 1, MotionMode::Move); // " two"
            cmd_select_prev_word_around(&buf, s2, 1, MotionMode::Move) // "one ", not " two" again
        },
        "-[one ]>two three four\n"
    );
}

#[test]
#[allow(non_snake_case)]
fn select_prev_uppercase_word_around_second_press_advances_past_first_word() {
    // Same regression as select_prev_word_around_second_press_advances_past_first_word,
    // for B: "three.x" is one WORD (punctuation merged in).
    assert_state!(
        "one two three.x -[f]>our\n",
        |(buf, sels)| {
            let s1 = cmd_select_prev_uppercase_word_around(&buf, sels, 1, MotionMode::Move); // " three.x"
            let s2 = cmd_select_prev_uppercase_word_around(&buf, s1, 1, MotionMode::Move); // " two"
            cmd_select_prev_uppercase_word_around(&buf, s2, 1, MotionMode::Move) // "one ", not " two" again
        },
        "-[one ]>two three.x four\n"
    );
}

#[test]
fn select_word_around_w_then_b_round_trip() {
    // w lands on "two" (leading-absorbed: " two", head on "o"); b then
    // searches from `start()` (the leading space), skips it, and steps back
    // to "one" — the first word of the buffer, which falls back to trailing
    // absorption ("one ", head on the trailing space). Confirms the two
    // directions compose correctly across a leading-vs-trailing unit switch.
    assert_state!(
        "-[o]>ne two three four\n",
        |(buf, sels)| {
            let s1 = cmd_select_next_word_around(&buf, sels, 1, MotionMode::Move); // " two"
            cmd_select_prev_word_around(&buf, s1, 1, MotionMode::Move) // "one ", back to start
        },
        "-[one ]>two three four\n"
    );
}

#[test]
fn select_word_around_b_then_w_round_trip() {
    // b from inside "two" steps back to "one" (first word of buffer,
    // trailing-absorbed: "one ", head on the trailing space). w then
    // searches from that space (`head()`), finds "two" again, and takes its
    // leading space — proving forward search isn't fooled by a head sitting
    // on whitespace left behind by a first-word backward landing.
    assert_state!(
        "one -[t]>wo three four\n",
        |(buf, sels)| {
            let s1 = cmd_select_prev_word_around(&buf, sels, 1, MotionMode::Move); // "one "
            cmd_select_next_word_around(&buf, s1, 1, MotionMode::Move) // " two", not stuck on "one"
        },
        "one-[ two]> three four\n"
    );
}

#[test]
fn select_prev_word_around_at_buffer_start_is_noop() {
    // Guard: no previous word exists — no-op, no expansion attempted.
    assert_state!(
        "-[h]>ello\n",
        |(buf, sels)| cmd_select_prev_word_around(&buf, sels, 1, MotionMode::Move),
        "-[h]>ello\n"
    );
}

#[test]
fn extend_select_next_word_around_grows_with_anchor_unit() {
    // Extend mode honors word-selects-whitespace: the anchor's unit ("bar",
    // not first on its line, takes its leading space) is kept whole as the
    // selection grows forward to the target word's own end — no trailing
    // whitespace is pulled in, which is exactly why the old bare-anchor
    // reversion (see apply_word_select_extend's doc) is no longer needed.
    assert_state!(
        "foo -[b]>ar baz\n",
        |(buf, sels)| cmd_select_next_word_around(&buf, sels, 1, MotionMode::Extend),
        "foo-[ bar baz]>\n"
    );
}

#[test]
fn extend_select_prev_word_around_grows_backward_onto_leading_whitespace() {
    // Growing backward, the target word's own leading space is absorbed
    // into `head` — the selection can legitimately start on whitespace.
    assert_state!(
        "foo bar -[b]>az\n",
        |(buf, sels)| cmd_select_prev_word_around(&buf, sels, 1, MotionMode::Extend),
        "foo<[ bar baz]-\n"
    );
}

#[test]
fn extend_select_prev_word_around_shrinks_to_anchor_unit() {
    // The target ("bar") is the anchor's own word — collapses to the
    // anchor's unit (" bar"), not further.
    assert_state!(
        "foo-[ bar baz]>\n",
        |(buf, sels)| cmd_select_prev_word_around(&buf, sels, 1, MotionMode::Extend),
        "foo-[ bar]> baz\n"
    );
}

#[test]
fn extend_select_word_around_round_trip_across_anchor() {
    // "a b c\n": extend-w from "b" grows onto "c" (anchor unit " b", target
    // raw "c") → "a-[ b c]>". extend-b walks back to the anchor's own unit,
    // collapsing → "a-[ b]> c". A second extend-b walks past the anchor and
    // out the other side, flipping direction → "<[a b]- c". The final
    // extend-w crosses back and collapses to the anchor's own unit again.
    assert_state!(
        "a -[b]> c\n",
        |(buf, sels)| {
            let s1 = cmd_select_next_word_around(&buf, sels, 1, MotionMode::Extend);
            let s2 = cmd_select_prev_word_around(&buf, s1, 1, MotionMode::Extend);
            let s3 = cmd_select_prev_word_around(&buf, s2, 1, MotionMode::Extend);
            cmd_select_next_word_around(&buf, s3, 1, MotionMode::Extend)
        },
        "a-[ b]> c\n"
    );
}

#[test]
fn extend_select_prev_word_around_backward_edge_excludes_indentation() {
    // Growing backward onto "one", the first word of its (indented) line —
    // its leading run is indentation and is never absorbed into `head`.
    assert_state!(
        "  one -[t]>wo\n",
        |(buf, sels)| cmd_select_prev_word_around(&buf, sels, 1, MotionMode::Extend),
        "  <[one two]-\n"
    );
}

#[test]
fn extend_select_next_word_around_chained_grows_past_two_words() {
    // Two separate extend-w presses grow the selection past "two" onto
    // "three", re-resolving the (unchanged) anchor unit each time.
    assert_state!(
        "-[o]>ne two three\n",
        |(buf, sels)| {
            let s1 = cmd_select_next_word_around(&buf, sels, 1, MotionMode::Extend); // "-[one two]>"
            cmd_select_next_word_around(&buf, s1, 1, MotionMode::Extend)
        },
        "-[one two three]>\n"
    );
}

// ── multi-cursor paragraph motions ────────────────────────────────────────

#[test]
fn next_paragraph_multi_cursor() {
    // Two cursors in different paragraphs, each jumps to the start of the next one.
    // "hello\n\nworld\n\nfoo\n": cursor at 'w'(7) → 'f'(14); cursor at 'f'(14) → '\n'(17).
    assert_state!(
        "hello\n\n-[w]>orld\n\n-[f]>oo\n",
        |(buf, sels)| cmd_next_paragraph(&buf, sels, 1, MotionMode::Move),
        "hello\n\nworld\n\n-[f]>oo-[\n]>"
    );
}

#[test]
fn prev_paragraph_multi_cursor() {
    // Same buffer; each cursor jumps backward to the gap above its paragraph.
    // Cursor at 'w'(7) → '\n'(6) (gap). Cursor at 'f'(14) → '\n'(13) (gap).
    assert_state!(
        "hello\n\n-[w]>orld\n\n-[f]>oo\n",
        |(buf, sels)| cmd_prev_paragraph(&buf, sels, 1, MotionMode::Move),
        "hello\n-[\n]>world\n-[\n]>foo\n"
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

// ── empty buffer edge cases ───────────────────────────────────────────────

#[test]
fn goto_first_nonblank_empty_buffer() {
    assert_state!(
        "-[\n]>",
        |(buf, sels)| cmd_goto_first_nonblank(&buf, sels, 1, MotionMode::Move),
        "-[\n]>"
    );
}

#[test]
fn prev_paragraph_empty_buffer() {
    assert_state!(
        "-[\n]>",
        |(buf, sels)| cmd_prev_paragraph(&buf, sels, 1, MotionMode::Move),
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

// ── extend_select word motions (anchor-unit grow/shrink) ──────────────────

#[test]
fn extend_select_next_word_from_cursor() {
    // From a collapsed cursor at 'h', the anchor's word is "hello" (0,4).
    // select_next_word from head=0 finds "world" (6,10), which lies beyond
    // the anchor's word, so the selection grows to cover both.
    assert_state!(
        "-[h]>ello world foo\n",
        |(buf, sels)| cmd_select_next_word(&buf, sels, 1, MotionMode::Extend),
        "-[hello world]> foo\n"
    );
}

#[test]
fn extend_select_next_word_grows_selection() {
    // Start with "world" selected via `w` (anchor=6,head=10); extend-w finds
    // "foo" (12,14), beyond the anchor's own word "world", so it grows.
    assert_state!(
        "-[h]>ello world foo\n",
        |(buf, sels)| {
            let s1 = cmd_select_next_word(&buf, sels, 1, MotionMode::Move); // selects "world" (6,10)
            cmd_select_next_word(&buf, s1, 1, MotionMode::Extend) // grows to "world foo"
        },
        "hello -[world foo]>\n"
    );
}

#[test]
fn extend_select_prev_word_extends_backward() {
    // Start with "world" selected via `w` (anchor=6,head=10); extend-b finds
    // "hello" (0,4), behind the anchor's word, so the selection grows
    // backward — flipping to a backward selection (head=0, anchor=10) while
    // still covering both words in full.
    assert_state!(
        "-[h]>ello world\n",
        |(buf, sels)| {
            let s1 = cmd_select_next_word(&buf, sels, 1, MotionMode::Move); // selects "world" (6,10)
            cmd_select_prev_word(&buf, s1, 1, MotionMode::Extend) // grows backward to "hello world"
        },
        "<[hello world]-\n"
    );
}

#[test]
fn extend_select_prev_word_from_multi_word_selection() {
    // From a multi-word selection "-[bar baz]>" (anchor=4, head=10), extend-b
    // searches from the head (10, inside "baz") and finds "bar" — the word
    // immediately before "baz" — which is exactly the anchor's own word
    // ("bar", 4..6). Target == anchor unit, so the selection shrinks back to
    // just "bar" rather than growing to include "foo".
    //
    // "foo bar baz\n": f=0,o=1,o=2,' '=3,b=4,a=5,r=6,' '=7,b=8,a=9,z=10,'\n'=11
    assert_state!(
        "foo -[bar baz]>\n",
        |(buf, sels)| cmd_select_prev_word(&buf, sels, 1, MotionMode::Extend),
        "foo -[bar]> baz\n"
    );
}

#[test]
fn extend_select_next_word_at_buffer_end_is_noop() {
    // From a selection covering the only word in the buffer, extend-w finds
    // no next word (only '\n' remains) and leaves the selection unchanged.
    assert_state!(
        "-[hello]>\n",
        |(buf, sels)| cmd_select_next_word(&buf, sels, 1, MotionMode::Extend),
        "-[hello]>\n"
    );
}

#[test]
fn extend_select_prev_word_at_buffer_start_is_noop() {
    // The selection starts at pos 0; there is no previous word. Noop.
    assert_state!(
        "-[hello]> world\n",
        |(buf, sels)| cmd_select_prev_word(&buf, sels, 1, MotionMode::Extend),
        "-[hello]> world\n"
    );
}

#[test]
fn extend_select_next_word_multi_cursor() {
    // Two cursors each independently grow toward the next word beyond their
    // own anchor's word. Because select_next_word skips the word under the
    // cursor and returns the *following* word, each cursor grows to include
    // the word after its current one.
    //
    // "foo bar baz qux\n": f=0..2,' '=3,b=4..6,' '=7,b=8..10,' '=11,q=12..14
    // cursor1 at 'f'(0): anchor unit "foo"(0,2); select_next_word(head=0) → "bar"(4,6) → grows to "foo bar".
    // cursor2 at 'b'(8): anchor unit "baz"(8,10); select_next_word(head=8) → "qux"(12,14) → grows to "baz qux".
    // Results (0,6) and (8,14) are disjoint — no merge.
    assert_state!(
        "-[f]>oo bar -[b]>az qux\n",
        |(buf, sels)| cmd_select_next_word(&buf, sels, 1, MotionMode::Extend),
        "-[foo bar]> -[baz qux]>\n"
    );
}

// ── extend_select word motions: shrink-on-reversal scenario ──────────────
//
// Walks the exact sequence a user gets pressing Ctrl+w / Ctrl+b repeatedly
// on "a b c" with "b" selected: grow forward, shrink back to "b", cross the
// anchor to grow backward (flipping direction), then cross back to shrink
// forward to "b" again. "a b c\n": a=0,' '=1,b=2,' '=3,c=4,'\n'=5.

#[test]
fn word_shrink_scenario_step1_grows_forward() {
    assert_state!(
        "a -[b]> c\n",
        |(buf, sels)| cmd_select_next_word(&buf, sels, 1, MotionMode::Extend),
        "a -[b c]>\n"
    );
}

#[test]
fn word_shrink_scenario_step2_shrinks_to_anchor_word() {
    // select_prev_word from head=4 (inside "c") lands back on "b" — the
    // anchor's own word — so the selection shrinks rather than growing past it.
    assert_state!(
        "a -[b c]>\n",
        |(buf, sels)| cmd_select_prev_word(&buf, sels, 1, MotionMode::Extend),
        "a -[b]> c\n"
    );
}

#[test]
fn word_shrink_scenario_step3_crosses_anchor_flips_backward() {
    // select_prev_word from head=2 (inside "b") lands on "a", behind the
    // anchor's word — the selection grows backward, flipping direction.
    assert_state!(
        "a -[b]> c\n",
        |(buf, sels)| cmd_select_prev_word(&buf, sels, 1, MotionMode::Extend),
        "<[a b]- c\n"
    );
}

#[test]
fn word_shrink_scenario_step4_crosses_back_shrinks_forward() {
    // select_next_word from head=0 (inside "a") lands back on "b" — the
    // anchor's own word — so the selection shrinks back to "b" and re-flips
    // to forward.
    assert_state!(
        "<[a b]- c\n",
        |(buf, sels)| cmd_select_next_word(&buf, sels, 1, MotionMode::Extend),
        "a -[b]> c\n"
    );
}

// ── extend_select word motions: no truncation across the anchor ──────────
//
// Same round trip with multi-char words, to prove a word is never partially
// cut when the motion crosses the anchor — only ever included or excluded
// whole. "aaa bbb ccc\n": a=0..2,' '=3,b=4..6,' '=7,c=8..10,'\n'=11.

#[test]
fn word_no_truncation_grows_forward() {
    assert_state!(
        "aaa -[bbb]> ccc\n",
        |(buf, sels)| cmd_select_next_word(&buf, sels, 1, MotionMode::Extend),
        "aaa -[bbb ccc]>\n"
    );
}

#[test]
fn word_no_truncation_shrinks_to_unit() {
    // Shrinks from "bbb ccc" back to just "bbb" — "ccc" is dropped whole, not
    // trimmed to a single char.
    assert_state!(
        "aaa -[bbb ccc]>\n",
        |(buf, sels)| cmd_select_prev_word(&buf, sels, 1, MotionMode::Extend),
        "aaa -[bbb]> ccc\n"
    );
}

#[test]
fn word_no_truncation_crossing_anchor_keeps_word_whole() {
    // From "bbb" alone, extend-b crosses the anchor into "aaa". The anchor's
    // word "bbb" stays fully selected (not cut down to one char) even though
    // the selection direction flips to backward.
    assert_state!(
        "aaa -[bbb]> ccc\n",
        |(buf, sels)| cmd_select_prev_word(&buf, sels, 1, MotionMode::Extend),
        "<[aaa bbb]- ccc\n"
    );
}

#[test]
fn word_no_truncation_shrink_back_after_cross() {
    assert_state!(
        "<[aaa bbb]- ccc\n",
        |(buf, sels)| cmd_select_next_word(&buf, sels, 1, MotionMode::Extend),
        "aaa -[bbb]> ccc\n"
    );
}

// ── extend_select word motions: flip redirects the extend ────────────────
//
// Flipping a selection (`Ctrl+e` / `o`) swaps anchor and head, and the
// anchor's word is re-derived from the new anchor on the next press — so
// flip genuinely hands the "fixed" end to the other side of the selection.

#[test]
fn word_extend_after_flip_shrinks_to_new_anchor_word() {
    // Flipped "b c": anchor on 'c'(4), head on 'b'(2). Extend-w's target from
    // the head is "c" — the new anchor's own word — so the selection collapses
    // to it. Without the flip the same press is a no-op (no word after "c").
    assert_state!(
        "a <[b c]-\n",
        |(buf, sels)| cmd_select_next_word(&buf, sels, 1, MotionMode::Extend),
        "a b -[c]>\n"
    );
}

#[test]
fn word_extend_backward_after_flip_grows_over_old_span() {
    // Same flipped start: extend-b's target "a" lies behind the new anchor's
    // word "c", so the selection grows backward from "c" over everything.
    // Without the flip the same press shrinks to "b" instead.
    assert_state!(
        "a <[b c]-\n",
        |(buf, sels)| cmd_select_prev_word(&buf, sels, 1, MotionMode::Extend),
        "<[a b c]-\n"
    );
}

#[test]
fn extend_select_next_uppercase_word_unit_spans_punctuation() {
    // Under `W` rules, "foo-bar" is a single WORD unit (punctuation merges
    // with the adjacent word class). The anchor's unit is computed with the
    // same uppercase boundary fn, so it spans the whole hyphenated word, not
    // just "foo". "foo-bar baz\n": f=0,o=1,o=2,-=3,b=4,a=5,r=6,' '=7,b=8..10.
    assert_state!(
        "-[f]>oo-bar baz\n",
        |(buf, sels)| cmd_select_next_uppercase_word(&buf, sels, 1, MotionMode::Extend),
        "-[foo-bar baz]>\n"
    );
}

// ── extend_select word motions: count > 1 within a single press ──────────
//
// `apply_word_select_extend`'s loop re-derives the anchor's unit and moves
// from the *current* head on every iteration (not just once at entry), so a
// count > 1 press must behave exactly like pressing the same key `count`
// times in a row — this is genuinely new code (the loop body didn't exist
// before bidirectional extend), so it needs its own coverage beyond count=1.

#[test]
fn extend_select_next_word_count_2_grows_two_words_forward() {
    // Each of the 2 iterations grows forward from the previous head, keeping
    // the same anchor unit ("foo") throughout — no flip involved.
    // "foo bar baz qux\n": f=0..2,' '=3,b=4..6,' '=7,b=8..10,' '=11,q=12..14.
    assert_state!(
        "-[foo]> bar baz qux\n",
        |(buf, sels)| cmd_select_next_word(&buf, sels, 2, MotionMode::Extend),
        "-[foo bar baz]> qux\n"
    );
}

#[test]
fn extend_select_next_word_count_2_flips_then_continues_forward() {
    // Start already flipped backward over "b c" (anchor on 'c'=4, head on
    // 'b'=2) — the shape a prior extend-b press across the anchor leaves
    // behind. A count=2 extend-w press must, within a *single* dispatch:
    // iteration 1 — motion from head=2 lands on "c", the anchor's own word,
    //   so the selection collapses (flips forward) to just "c" (matches the
    //   single-press behavior in `word_extend_after_flip_shrinks_to_new_anchor_word`);
    // iteration 2 — motion from the new head=4 lands on "d", beyond the
    //   anchor's word, so the selection grows forward to "c d".
    // "a b c d e\n": a=0,' '=1,b=2,' '=3,c=4,' '=5,d=6,' '=7,e=8,'\n'=9.
    assert_state!(
        "a <[b c]- d e\n",
        |(buf, sels)| cmd_select_next_word(&buf, sels, 2, MotionMode::Extend),
        "a b -[c d]> e\n"
    );
}

// ── extend_select word motions: anchor inside a combining grapheme cluster ─
//
// `anchor_unit` re-derives the anchor's word on every press from whatever
// position the anchor currently holds — which, per `Selection::new(unit_end,
// word_start)` in the backward-grow branch, can legitimately be the *last
// codepoint* of a multi-codepoint grapheme cluster (not just a cluster
// start), whenever the anchor's own word ends in a combining sequence.

#[test]
fn extend_select_next_word_anchor_ending_in_combining_cluster_stays_whole() {
    // "café" = c,a,f,e,´(U+0301 combining acute) — the last two codepoints
    // form one grapheme cluster. Anchor sits on the *last codepoint* of that
    // cluster (8), which is exactly what a backward-crossing extend leaves as
    // `unit_end` when the anchor's word ends in a combining sequence — a
    // normal, reachable selection shape, not a contrived position.
    //
    // Fail oracle: read `classify_char` on the raw anchor codepoint instead
    // of snapping to the cluster start first — the combining mark alone
    // classifies as `Punctuation` (not `Word`), so the anchor's own word gets
    // misread as just that trailing mark and truncated to "foo café-[´ bar]>"
    // instead of keeping "café" whole.
    assert_state!(
        "foo <[cafe\u{0301}]- bar\n",
        |(buf, sels)| cmd_select_next_word(&buf, sels, 1, MotionMode::Extend),
        "foo -[cafe\u{0301} bar]>\n"
    );
}

#[test]
fn extend_select_prev_word_anchor_ending_in_combining_cluster_stays_whole() {
    // Same cluster, opposite direction: anchor still on the combining mark
    // (8), extend-b should grow backward to include "foo" while keeping
    // "café" whole rather than treating the accent as a separate unit.
    assert_state!(
        "foo <[cafe\u{0301}]- bar\n",
        |(buf, sels)| cmd_select_prev_word(&buf, sels, 1, MotionMode::Extend),
        "<[foo cafe\u{0301}]- bar\n"
    );
}

#[test]
fn extend_select_next_word_whitespace_anchor_is_single_position() {
    // When the anchor sits on whitespace, its "unit" is just that one
    // position (not a word) — growing toward the next word doesn't try to
    // preserve or extend the whitespace run.
    assert_state!(
        "a -[ ]> b\n",
        |(buf, sels)| cmd_select_next_word(&buf, sels, 1, MotionMode::Extend),
        "a -[  b]>\n"
    );
}

#[test]
fn extend_select_prev_word_multi_cursor_shrink_causes_merge() {
    // Two selections ("bar" and a cursor on "baz") each shrink-cross their
    // own anchor backward toward "foo"/"bar" respectively. The results
    // overlap ([0,6] and [4,10]), so `map`'s merge unifies them into one
    // selection spanning "foo bar baz".
    // "foo bar baz\n": f=0..2,' '=3,b=4..6,' '=7,b=8..10,'\n'=11.
    assert_state!(
        "foo -[bar]> -[b]>az\n",
        |(buf, sels)| cmd_select_prev_word(&buf, sels, 1, MotionMode::Extend),
        "<[foo bar baz]-\n"
    );
}

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

// ── find_char_forward / find_char_backward ────────────────────────────────

// Helper wrappers with fixed mode so assert_state! closures stay tidy.
fn fwd(buf: Text, sels: SelectionSet, ch: char, kind: FindKind) -> SelectionSet {
    find_char_forward(&buf, sels, MotionMode::Move, 1, ch, kind)
}
fn bwd(buf: Text, sels: SelectionSet, ch: char, kind: FindKind) -> SelectionSet {
    find_char_backward(&buf, sels, MotionMode::Move, 1, ch, kind)
}
fn fwd_ext(buf: Text, sels: SelectionSet, ch: char, kind: FindKind) -> SelectionSet {
    find_char_forward(&buf, sels, MotionMode::Extend, 1, ch, kind)
}
fn fwd_count(buf: Text, sels: SelectionSet, ch: char, kind: FindKind, n: usize) -> SelectionSet {
    find_char_forward(&buf, sels, MotionMode::Move, n, ch, kind)
}

#[test]
fn find_forward_inclusive_basic() {
    // Cursor on 'h'; `fa` jumps to the first 'a'.
    assert_state!(
        "-[h]>ello a world\n",
        |(buf, sels)| fwd(buf, sels, 'a', FindKind::Inclusive),
        "hello -[a]> world\n"
    );
}

#[test]
fn find_forward_inclusive_first_char_on_line() {
    // Target is the very last content char.
    assert_state!(
        "-[h]>ello\n",
        |(buf, sels)| fwd(buf, sels, 'o', FindKind::Inclusive),
        "hell-[o]>\n"
    );
}

#[test]
fn find_forward_inclusive_not_found() {
    // No 'z' on this line — no-op.
    assert_state!(
        "-[h]>ello\n",
        |(buf, sels)| fwd(buf, sels, 'z', FindKind::Inclusive),
        "-[h]>ello\n"
    );
}

#[test]
fn find_forward_does_not_cross_newline() {
    // 'a' appears only on the second line — the motion must not cross '\n'.
    assert_state!(
        "-[h]>ello\nabc\n",
        |(buf, sels)| fwd(buf, sels, 'a', FindKind::Inclusive),
        "-[h]>ello\nabc\n"
    );
}

#[test]
fn find_forward_skips_char_under_cursor() {
    // Cursor is already on 'a'; `fa` should find the *next* 'a', not the current one.
    assert_state!(
        "-[a]>bc a def\n",
        |(buf, sels)| fwd(buf, sels, 'a', FindKind::Inclusive),
        "abc -[a]> def\n"
    );
}

#[test]
fn find_forward_exclusive_basic() {
    // `ta` stops one grapheme before 'a' — the space is one grapheme before 'a'.
    assert_state!(
        "-[h]>ello a world\n",
        |(buf, sels)| fwd(buf, sels, 'a', FindKind::Exclusive),
        "hello-[ ]>a world\n"
    );
}

#[test]
fn find_forward_exclusive_adjacent_is_noop() {
    // 'a' is the immediately next grapheme; exclusive adjustment lands back at head.
    assert_state!(
        "-[h]>a world\n",
        |(buf, sels)| fwd(buf, sels, 'a', FindKind::Exclusive),
        "-[h]>a world\n"
    );
}

#[test]
fn find_forward_count() {
    // `2fa` jumps to the second 'a'.
    assert_state!(
        "-[h]>a ba\n",
        |(buf, sels)| fwd_count(buf, sels, 'a', FindKind::Inclusive, 2),
        "ha b-[a]>\n"
    );
}

#[test]
fn find_backward_inclusive_basic() {
    // `Fa` finds the previous 'a'.
    assert_state!(
        "hello a worl-[d]>\n",
        |(buf, sels)| bwd(buf, sels, 'a', FindKind::Inclusive),
        "hello -[a]> world\n"
    );
}

#[test]
fn find_backward_inclusive_not_found() {
    assert_state!(
        "hell-[o]>\n",
        |(buf, sels)| bwd(buf, sels, 'z', FindKind::Inclusive),
        "hell-[o]>\n"
    );
}

#[test]
fn find_backward_does_not_cross_newline() {
    // 'z' is only on the first line; cursor on second line must not find it.
    assert_state!(
        "z\n-[a]>bc\n",
        |(buf, sels)| bwd(buf, sels, 'z', FindKind::Inclusive),
        "z\n-[a]>bc\n"
    );
}

#[test]
fn find_backward_exclusive_basic() {
    // `Ta` stops one grapheme after 'a' (cursor is between 'a' and its original pos).
    assert_state!(
        "hello a worl-[d]>\n",
        |(buf, sels)| bwd(buf, sels, 'a', FindKind::Exclusive),
        "hello a-[ ]>world\n"
    );
}

#[test]
fn find_backward_exclusive_adjacent_is_noop() {
    // Cursor is immediately right of 'a'; exclusive adjustment steps forward
    // from the found position back to head — so the motion is a no-op,
    // symmetric to the forward exclusive adjacent case.
    assert_state!(
        "hello a-[x]>\n",
        |(buf, sels)| bwd(buf, sels, 'a', FindKind::Exclusive),
        "hello a-[x]>\n"
    );
}

#[test]
fn find_forward_extend_mode() {
    // Extend mode: anchor stays, head moves to found char.
    assert_state!(
        "-[h]>ello a\n",
        |(buf, sels)| fwd_ext(buf, sels, 'a', FindKind::Inclusive),
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
        |(buf, sels)| fwd(buf, sels, 'a', FindKind::Inclusive),
        "h-[a]> ba c -[a]>\n"
    );
}

#[test]
fn find_backward_at_line_start_noop() {
    // Cursor at line start — nothing to the left, no-op.
    assert_state!(
        "-[h]>ello\n",
        |(buf, sels)| bwd(buf, sels, 'x', FindKind::Inclusive),
        "-[h]>ello\n"
    );
}
