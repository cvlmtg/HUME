use super::*;
use crate::assert_state;

// ── move_right ────────────────────────────────────────────────────────────

#[test]
fn move_right_basic() {
    assert_state!("-[h]>ello\n", |(buf, sels)| cmd_move_right(&buf, sels, 1, MotionMode::Move), "h-[e]>llo\n");
}

#[test]
fn move_right_to_eof() {
    assert_state!("hell-[o]>\n", |(buf, sels)| cmd_move_right(&buf, sels, 1, MotionMode::Move), "hello-[\n]>");
}

#[test]
fn move_right_clamp_at_eof() {
    assert_state!("hello-[\n]>", |(buf, sels)| cmd_move_right(&buf, sels, 1, MotionMode::Move), "hello-[\n]>");
}

#[test]
fn move_right_empty_buffer() {
    assert_state!("-[\n]>", |(buf, sels)| cmd_move_right(&buf, sels, 1, MotionMode::Move), "-[\n]>");
}

#[test]
fn move_right_multi_cursor() {
    assert_state!("-[h]>-[e]>llo\n", |(buf, sels)| cmd_move_right(&buf, sels, 1, MotionMode::Move), "h-[e]>-[l]>lo\n");
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
    assert_state!("h-[e]>llo\n", |(buf, sels)| cmd_move_left(&buf, sels, 1, MotionMode::Move), "-[h]>ello\n");
}

#[test]
fn move_left_clamp_at_start() {
    assert_state!("-[h]>ello\n", |(buf, sels)| cmd_move_left(&buf, sels, 1, MotionMode::Move), "-[h]>ello\n");
}

#[test]
fn move_left_empty_buffer() {
    assert_state!("-[\n]>", |(buf, sels)| cmd_move_left(&buf, sels, 1, MotionMode::Move), "-[\n]>");
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
    assert_state!("-[a]>-[b]>c\n", |(buf, sels)| cmd_move_left(&buf, sels, 1, MotionMode::Move), "-[a]>bc\n");
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
    assert_state!("hello-[\n]>", |(buf, sels)| cmd_move_right(&buf, sels, 1, MotionMode::Extend), "hello-[\n]>");
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
    assert_state!("-[h]>ello\n", |(buf, sels)| cmd_move_left(&buf, sels, 1, MotionMode::Extend), "-[h]>ello\n");
}

#[test]
fn extend_left_reverses_direction() {
    // Forward selection anchor=3,head=3. Extend left 3 times: head→0.
    // anchor=3 > head=0 → becomes a backward selection spanning "hell".
    assert_state!("hel-[l]>o\n", |(buf, sels)| cmd_move_left(&buf, sels, 3, MotionMode::Extend), "<[hell]-o\n");
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
    assert_state!("hello\nwor-[l]>d\n", |(buf, sels)| cmd_goto_first_line(&buf, sels, 1, MotionMode::Move), "-[h]>ello\nworld\n");
}

#[test]
fn goto_first_line_already_at_start() {
    assert_state!("-[h]>ello\nworld\n", |(buf, sels)| cmd_goto_first_line(&buf, sels, 1, MotionMode::Move), "-[h]>ello\nworld\n");
}

#[test]
fn goto_first_line_single_line_buffer() {
    assert_state!("hel-[l]>o\n", |(buf, sels)| cmd_goto_first_line(&buf, sels, 1, MotionMode::Move), "-[h]>ello\n");
}

#[test]
fn goto_first_line_empty_buffer() {
    assert_state!("-[\n]>", |(buf, sels)| cmd_goto_first_line(&buf, sels, 1, MotionMode::Move), "-[\n]>");
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
    assert_state!("-[h]>ello\nworld\n", |(buf, sels)| cmd_goto_last_line(&buf, sels, 1, MotionMode::Move), "hello\n-[w]>orld\n");
}

#[test]
fn goto_last_line_already_at_last() {
    assert_state!("hello\n-[w]>orld\n", |(buf, sels)| cmd_goto_last_line(&buf, sels, 1, MotionMode::Move), "hello\n-[w]>orld\n");
}

#[test]
fn goto_last_line_single_line_buffer() {
    assert_state!("-[\n]>", |(buf, sels)| cmd_goto_last_line(&buf, sels, 1, MotionMode::Move), "-[\n]>");
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
    assert_state!("hel-[l]>o\n", |(buf, sels)| cmd_goto_line_start(&buf, sels, 1, MotionMode::Move), "-[h]>ello\n");
}

#[test]
fn goto_line_start_already_at_start() {
    assert_state!("-[h]>ello\n", |(buf, sels)| cmd_goto_line_start(&buf, sels, 1, MotionMode::Move), "-[h]>ello\n");
}

#[test]
fn goto_line_start_second_line() {
    assert_state!("hello\nwor-[l]>d\n", |(buf, sels)| cmd_goto_line_start(&buf, sels, 1, MotionMode::Move), "hello\n-[w]>orld\n");
}

#[test]
fn goto_line_start_empty_buffer() {
    assert_state!("-[\n]>", |(buf, sels)| cmd_goto_line_start(&buf, sels, 1, MotionMode::Move), "-[\n]>");
}

// ── goto_line_end ─────────────────────────────────────────────────────────

#[test]
fn goto_line_end_from_start() {
    assert_state!("-[h]>ello\n", |(buf, sels)| cmd_goto_line_end(&buf, sels, 1, MotionMode::Move), "hell-[o]>\n");
}

#[test]
fn goto_line_end_already_at_end() {
    assert_state!("hell-[o]>\n", |(buf, sels)| cmd_goto_line_end(&buf, sels, 1, MotionMode::Move), "hell-[o]>\n");
}

#[test]
fn goto_line_end_stops_before_newline() {
    // Cursor must land on 'o', not on '\n'.
    assert_state!("-[h]>ello\nworld\n", |(buf, sels)| cmd_goto_line_end(&buf, sels, 1, MotionMode::Move), "hell-[o]>\nworld\n");
}

#[test]
fn goto_line_end_empty_line() {
    // Line contains only '\n'. Cursor stays on it.
    assert_state!("-[\n]>", |(buf, sels)| cmd_goto_line_end(&buf, sels, 1, MotionMode::Move), "-[\n]>");
}

#[test]
fn goto_line_end_last_line_no_newline() {
    assert_state!("-[h]>ello\n", |(buf, sels)| cmd_goto_line_end(&buf, sels, 1, MotionMode::Move), "hell-[o]>\n");
}

#[test]
fn goto_line_end_empty_buffer() {
    assert_state!("-[\n]>", |(buf, sels)| cmd_goto_line_end(&buf, sels, 1, MotionMode::Move), "-[\n]>");
}

// ── goto_first_nonblank ───────────────────────────────────────────────────

#[test]
fn goto_first_nonblank_skips_spaces() {
    assert_state!("-[ ]> hello\n", |(buf, sels)| cmd_goto_first_nonblank(&buf, sels, 1, MotionMode::Move), "  -[h]>ello\n");
}

#[test]
fn goto_first_nonblank_from_middle() {
    assert_state!("  hel-[l]>o\n", |(buf, sels)| cmd_goto_first_nonblank(&buf, sels, 1, MotionMode::Move), "  -[h]>ello\n");
}

#[test]
fn goto_first_nonblank_skips_tab() {
    assert_state!("-[\t]>hello\n", |(buf, sels)| cmd_goto_first_nonblank(&buf, sels, 1, MotionMode::Move), "\t-[h]>ello\n");
}

#[test]
fn goto_first_nonblank_no_leading_whitespace() {
    assert_state!("-[h]>ello\n", |(buf, sels)| cmd_goto_first_nonblank(&buf, sels, 1, MotionMode::Move), "-[h]>ello\n");
}

#[test]
fn goto_first_nonblank_all_blank_line() {
    // Line is all spaces — no non-blank found, cursor is unchanged.
    assert_state!("-[ ]>  \n", |(buf, sels)| cmd_goto_first_nonblank(&buf, sels, 1, MotionMode::Move), "-[ ]>  \n");
    assert_state!(" -[ ]>\n", |(buf, sels)| cmd_goto_first_nonblank(&buf, sels, 1, MotionMode::Move), " -[ ]>\n");
}

// ── move_down ─────────────────────────────────────────────────────────────

#[test]
fn move_down_basic() {
    assert_state!("-[h]>ello\nworld\n", |(buf, sels)| cmd_move_down(&buf, sels, 1, MotionMode::Move), "hello\n-[w]>orld\n");
}

#[test]
fn move_down_preserves_column() {
    assert_state!("hel-[l]>o\nworld\n", |(buf, sels)| cmd_move_down(&buf, sels, 1, MotionMode::Move), "hello\nwor-[l]>d\n");
}

#[test]
fn move_down_clamps_to_shorter_line() {
    assert_state!("hel-[l]>o\nab\n", |(buf, sels)| cmd_move_down(&buf, sels, 1, MotionMode::Move), "hello\na-[b]>\n");
}

#[test]
fn move_down_clamp_on_last_line() {
    assert_state!("hello\n-[w]>orld\n", |(buf, sels)| cmd_move_down(&buf, sels, 1, MotionMode::Move), "hello\n-[w]>orld\n");
}

#[test]
fn move_down_to_empty_line() {
    assert_state!("-[h]>ello\n\nworld\n", |(buf, sels)| cmd_move_down(&buf, sels, 1, MotionMode::Move), "hello\n-[\n]>world\n");
}

#[test]
fn move_down_empty_buffer() {
    assert_state!("-[\n]>", |(buf, sels)| cmd_move_down(&buf, sels, 1, MotionMode::Move), "-[\n]>");
}

#[test]
fn move_down_multi_cursor_merge() {
    // Two cursors on line 0. Both move to line 1 — they converge and merge.
    assert_state!("-[h]>ello\n-[w]>orld\n", |(buf, sels)| cmd_move_down(&buf, sels, 1, MotionMode::Move), "hello\n-[w]>orld\n");
}

// ── move_up ───────────────────────────────────────────────────────────────

#[test]
fn move_up_basic() {
    assert_state!("hello\n-[w]>orld\n", |(buf, sels)| cmd_move_up(&buf, sels, 1, MotionMode::Move), "-[h]>ello\nworld\n");
}

#[test]
fn move_up_preserves_column() {
    assert_state!("hello\nwor-[l]>d\n", |(buf, sels)| cmd_move_up(&buf, sels, 1, MotionMode::Move), "hel-[l]>o\nworld\n");
}

#[test]
fn move_up_clamp_on_first_line() {
    assert_state!("-[h]>ello\nworld\n", |(buf, sels)| cmd_move_up(&buf, sels, 1, MotionMode::Move), "-[h]>ello\nworld\n");
}

#[test]
fn move_up_clamps_to_shorter_line() {
    // "ab" is 2 chars, "hello" is 5. Cursor at col 3 on "hello" → clamps to end of "ab".
    assert_state!("ab\nhel-[l]>o\n", |(buf, sels)| cmd_move_up(&buf, sels, 1, MotionMode::Move), "a-[b]>\nhello\n");
}
// ── cmd_select_next_word (w) ──────────────────────────────────────────────

#[test]
fn select_next_word_basic() {
    // From 'h', selects "world" (the next word). Fresh anchor at word start.
    assert_state!("-[h]>ello world\n", |(buf, sels)| cmd_select_next_word(&buf, sels, 1, MotionMode::Move), "hello -[world]>\n");
}

#[test]
fn select_next_word_from_mid_word() {
    // Cursor in the middle of "hello" — still jumps to next word "world".
    assert_state!("hel-[l]>o world\n", |(buf, sels)| cmd_select_next_word(&buf, sels, 1, MotionMode::Move), "hello -[world]>\n");
}

#[test]
fn select_next_word_from_whitespace() {
    // From the space between words, selects the next word "world".
    assert_state!("hello-[ ]>world\n", |(buf, sels)| cmd_select_next_word(&buf, sels, 1, MotionMode::Move), "hello -[world]>\n");
}

#[test]
fn select_next_word_crosses_newline() {
    // w crosses the newline and selects the first word on the next line.
    assert_state!("-[h]>ello\nworld\n", |(buf, sels)| cmd_select_next_word(&buf, sels, 1, MotionMode::Move), "hello\n-[world]>\n");
}

#[test]
fn select_next_word_crosses_multiple_blank_lines() {
    // Multiple blank lines between words — w still reaches the next word.
    assert_state!("-[h]>ello\n\n\nworld\n", |(buf, sels)| cmd_select_next_word(&buf, sels, 1, MotionMode::Move), "hello\n\n\n-[world]>\n");
}

#[test]
fn select_next_word_at_last_word_is_noop() {
    // Cursor on the last word in the buffer — no-op.
    assert_state!("hello -[world]>\n", |(buf, sels)| cmd_select_next_word(&buf, sels, 1, MotionMode::Move), "hello -[world]>\n");
}

#[test]
fn select_next_word_at_eof_is_noop() {
    // Cursor on trailing '\n' — no-op.
    assert_state!("hello-[\n]>", |(buf, sels)| cmd_select_next_word(&buf, sels, 1, MotionMode::Move), "hello-[\n]>");
}

#[test]
fn select_next_word_empty_buffer_is_noop() {
    assert_state!("-[\n]>", |(buf, sels)| cmd_select_next_word(&buf, sels, 1, MotionMode::Move), "-[\n]>");
}

#[test]
fn select_next_word_word_to_punct() {
    // "hello" and "." are different word classes — w selects ".".
    assert_state!("-[h]>ello.world\n", |(buf, sels)| cmd_select_next_word(&buf, sels, 1, MotionMode::Move), "hello-[.]>world\n");
}

#[test]
fn select_next_word_punct_to_word() {
    // From ".", the next word class token is "hello".
    assert_state!("-[.]>hello\n", |(buf, sels)| cmd_select_next_word(&buf, sels, 1, MotionMode::Move), ".-[hello]>\n");
}

#[test]
fn select_next_word_count_2() {
    // count=2: skips "world", selects "foo".
    assert_state!("-[h]>ello world foo\n", |(buf, sels)| cmd_select_next_word(&buf, sels, 2, MotionMode::Move), "hello world -[foo]>\n");
}

#[test]
fn select_next_word_count_stops_at_last_word() {
    // count=3 but only 2 words remain after cursor — stops at "foo".
    assert_state!("-[h]>ello world foo\n", |(buf, sels)| cmd_select_next_word(&buf, sels, 3, MotionMode::Move), "hello world -[foo]>\n");
}

// ── cmd_select_prev_word (b) ──────────────────────────────────────────────

#[test]
fn select_prev_word_basic() {
    // From "world", selects the previous word "hello".
    assert_state!("hello -[world]>\n", |(buf, sels)| cmd_select_prev_word(&buf, sels, 1, MotionMode::Move), "-[hello]> world\n");
}

#[test]
fn select_prev_word_from_mid_word() {
    // Cursor in the middle of "world" — jumps to previous word "hello".
    assert_state!("hello wor-[l]>d\n", |(buf, sels)| cmd_select_prev_word(&buf, sels, 1, MotionMode::Move), "-[hello]> world\n");
}

#[test]
fn select_prev_word_from_whitespace() {
    // From the space between words, selects the previous word "hello".
    assert_state!("hello-[ ]>world\n", |(buf, sels)| cmd_select_prev_word(&buf, sels, 1, MotionMode::Move), "-[hello]> world\n");
}

#[test]
fn select_prev_word_from_punct() {
    // Cursor on the '.' punctuation — selects the preceding word "hello".
    assert_state!("hello-[.]>world\n", |(buf, sels)| cmd_select_prev_word(&buf, sels, 1, MotionMode::Move), "-[hello]>.world\n");
}

#[test]
fn select_prev_word_from_trailing_newline() {
    // Cursor on the trailing '\n' — selects the last word on the line.
    assert_state!("hello world-[\n]>", |(buf, sels)| cmd_select_prev_word(&buf, sels, 1, MotionMode::Move), "hello -[world]>\n");
}

#[test]
fn select_prev_word_crosses_newline() {
    // b crosses the newline and selects the last word on the previous line.
    assert_state!("hello\n-[world]>\n", |(buf, sels)| cmd_select_prev_word(&buf, sels, 1, MotionMode::Move), "-[hello]>\nworld\n");
}

#[test]
fn select_prev_word_at_first_word_is_noop() {
    // Cursor on first word — no-op.
    assert_state!("-[hello]> world\n", |(buf, sels)| cmd_select_prev_word(&buf, sels, 1, MotionMode::Move), "-[hello]> world\n");
}

#[test]
fn select_prev_word_in_first_word_mid_is_noop() {
    // Cursor in the middle of the first word — no previous word, no-op.
    assert_state!("hel-[l]>o world\n", |(buf, sels)| cmd_select_prev_word(&buf, sels, 1, MotionMode::Move), "hel-[l]>o world\n");
}

#[test]
fn select_prev_word_at_buffer_start_is_noop() {
    assert_state!("-[h]>ello\n", |(buf, sels)| cmd_select_prev_word(&buf, sels, 1, MotionMode::Move), "-[h]>ello\n");
}

#[test]
fn select_prev_word_empty_buffer_is_noop() {
    assert_state!("-[\n]>", |(buf, sels)| cmd_select_prev_word(&buf, sels, 1, MotionMode::Move), "-[\n]>");
}

#[test]
fn select_prev_word_count_2() {
    // count=2: from "foo", skips "world", selects "hello".
    assert_state!("hello world -[foo]>\n", |(buf, sels)| cmd_select_prev_word(&buf, sels, 2, MotionMode::Move), "-[hello]> world foo\n");
}

#[test]
fn select_prev_word_count_overshoots() {
    // count=5 but only 2 words precede "foo" — stops at "hello" rather than erroring.
    assert_state!("hello world -[foo]>\n", |(buf, sels)| cmd_select_prev_word(&buf, sels, 5, MotionMode::Move), "-[hello]> world foo\n");
}

// ── WORD variants (W / B) ─────────────────────────────────────────────────

#[test]
fn select_next_WORD_skips_punct() {
    // W: "hello.world" is a single WORD — W selects it entirely.
    assert_state!("-[h]>ello.world bar\n", |(buf, sels)| cmd_select_next_WORD(&buf, sels, 1, MotionMode::Move), "hello.world -[bar]>\n");
}

#[test]
fn select_next_WORD_crosses_newline() {
    // W at end of a line crosses the newline and selects the first WORD on the next line.
    assert_state!("-[h]>ello.world\nbar\n", |(buf, sels)| cmd_select_next_WORD(&buf, sels, 1, MotionMode::Move), "hello.world\n-[bar]>\n");
}

#[test]
fn select_next_word_stops_at_punct() {
    // w (lowercase): "hello" and "." are separate word-class tokens.
    assert_state!("-[h]>ello.world bar\n", |(buf, sels)| cmd_select_next_word(&buf, sels, 1, MotionMode::Move), "hello-[.]>world bar\n");
}

#[test]
fn select_prev_WORD_skips_punct() {
    // B: from "bar", jumps back over "hello.world" as ONE WORD (the dot is not
    // a WORD boundary), selecting the whole token.
    assert_state!("hello.world -[bar]>\n", |(buf, sels)| cmd_select_prev_WORD(&buf, sels, 1, MotionMode::Move), "-[hello.world]> bar\n");
}

#[test]
fn select_prev_WORD_crosses_newline() {
    // B at the start of a line crosses the newline and selects the last WORD on the previous line.
    assert_state!("hello.world\n-[bar]>\n", |(buf, sels)| cmd_select_prev_WORD(&buf, sels, 1, MotionMode::Move), "-[hello.world]>\nbar\n");
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
    assert_state!("-[\n]>", |(buf, sels)| cmd_next_paragraph(&buf, sels, 1, MotionMode::Move), "-[\n]>");
}

#[test]
fn next_paragraph_at_eof() {
    assert_state!("hello-[\n]>", |(buf, sels)| cmd_next_paragraph(&buf, sels, 1, MotionMode::Move), "hello-[\n]>");
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
    assert_state!("-[h]>ello\n", |(buf, sels)| cmd_move_right(&buf, sels, 3, MotionMode::Move), "hel-[l]>o\n");
}

#[test]
fn move_right_count_clamps_at_eof() {
    // count=100 far exceeds the buffer length — clamps at the trailing '\n'.
    assert_state!("-[h]>ello\n", |(buf, sels)| cmd_move_right(&buf, sels, 100, MotionMode::Move), "hello-[\n]>");
}

#[test]
fn move_left_count_3() {
    // \n(5) → o(4) → l(3) → l(2)
    assert_state!("hello-[\n]>", |(buf, sels)| cmd_move_left(&buf, sels, 3, MotionMode::Move), "he-[l]>lo\n");
}

#[test]
fn extend_right_count_3() {
    // Extend: anchor stays at old head (0), head folds 3 steps: 0→1→2→3.
    // Selection anchor=0, head=3: covers "hell".
    assert_state!("-[h]>ello\n", |(buf, sels)| cmd_move_right(&buf, sels, 3, MotionMode::Extend), "-[hell]>o\n");
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
    assert_state!("-[\n]>", |(buf, sels)| cmd_goto_first_nonblank(&buf, sels, 1, MotionMode::Move), "-[\n]>");
}

#[test]
fn prev_paragraph_empty_buffer() {
    assert_state!("-[\n]>", |(buf, sels)| cmd_prev_paragraph(&buf, sels, 1, MotionMode::Move), "-[\n]>");
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

// ── extend_select word motions (union semantics) ──────────────────────────

#[test]
fn extend_select_next_word_from_cursor() {
    // From a collapsed cursor at 'h', extend-w unions cursor pos with next word.
    // select_next_word from pos 0 jumps to "world" (6,10).
    // Union: min(0,6)=0, max(0,10)=10 → selection (0,10) = "hello world".
    assert_state!(
        "-[h]>ello world foo\n",
        |(buf, sels)| cmd_select_next_word(&buf, sels, 1, MotionMode::Extend),
        "-[hello world]> foo\n"
    );
}

#[test]
fn extend_select_next_word_grows_selection() {
    // Start with "world" selected via `w`; extend-w unions with "foo".
    // s1 = "world" (6,10); motion from pos 10 → "foo" (12,14).
    // Union: min(6,12)=6, max(10,14)=14 → "world foo".
    assert_state!(
        "-[h]>ello world foo\n",
        |(buf, sels)| {
            let s1 = cmd_select_next_word(&buf, sels, 1, MotionMode::Move); // selects "world" (6,10)
            cmd_select_next_word(&buf, s1, 1, MotionMode::Extend)       // union with "foo" (12,14)
        },
        "hello -[world foo]>\n"
    );
}

#[test]
fn extend_select_prev_word_extends_backward() {
    // Start with "world" selected via `w`; extend-b unions with "hello".
    // s1 = "world" (6,10); backward motion from start()=6 → "hello" (0,4).
    // Union: min(6,0)=0, max(10,4)=10 → "hello world".
    assert_state!(
        "-[h]>ello world\n",
        |(buf, sels)| {
            let s1 = cmd_select_next_word(&buf, sels, 1, MotionMode::Move); // selects "world" (6,10)
            cmd_select_prev_word(&buf, s1, 1, MotionMode::Extend)       // union with "hello" (0,4)
        },
        "-[hello world]>\n"
    );
}

#[test]
fn extend_select_prev_word_from_multi_word_selection() {
    // Regression: from a multi-word selection "-[bar baz]>", pressing extend-b
    // should grow backward to include "foo", not be a no-op.
    //
    // Bug: old code used sel.head (=end of "baz") as motion origin.
    // select_prev_word from inside the selection found "baz" itself → union was
    // a no-op. Fix: backward variant uses sel.start() as origin, which is at
    // the start of "bar", so select_prev_word finds "foo".
    //
    // "foo bar baz\n": f=0,o=1,o=2,' '=3,b=4,a=5,r=6,' '=7,b=8,a=9,z=10,'\n'=11
    // "-[bar baz]>" = anchor=4, head=10; start()=4, end()=10.
    // select_prev_word(buf, start()=4) → "foo" at (0,2).
    // Union: min(4,0)=0, max(10,2)=10 → (0,10) = "foo bar baz".
    assert_state!(
        "foo -[bar baz]>\n",
        |(buf, sels)| cmd_select_prev_word(&buf, sels, 1, MotionMode::Extend),
        "-[foo bar baz]>\n"
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
    // Two cursors each independently union with the next word. Because
    // select_next_word skips the word under the cursor and returns the
    // *following* word, each cursor unites with the word after its current one.
    //
    // "foo bar baz qux\n": f=0..2,' '=3,b=4..6,' '=7,b=8..10,' '=11,q=12..14
    // cursor1 at 'f'(0): end()=0, select_next_word → "bar"(4,6). union(0,0,4,6)=(0,6)="foo bar".
    // cursor2 at 'b'(8): end()=8, select_next_word → "qux"(12,14). union(8,8,12,14)=(8,14)="baz qux".
    // Results (0,6) and (8,14) are disjoint — no merge.
    assert_state!(
        "-[f]>oo bar -[b]>az qux\n",
        |(buf, sels)| cmd_select_next_word(&buf, sels, 1, MotionMode::Extend),
        "-[foo bar]> -[baz qux]>\n"
    );
}

// ── cmd_select_line / cmd_select_line_backward ────────────────────────────

#[test]
fn select_line_from_mid_line() {
    // Cursor mid-line → select full line forward.
    assert_state!(
        "hello -[w]>orld\nfoo\n",
        |(buf, sels)| cmd_select_line(&buf, sels, MotionMode::Move),
        "-[hello world\n]>foo\n"
    );
}

#[test]
fn select_line_already_full_line_jumps_to_next() {
    // Selection already covers full line → jump to next line.
    assert_state!(
        "-[hello world\n]>foo\n",
        |(buf, sels)| cmd_select_line(&buf, sels, MotionMode::Move),
        "hello world\n-[foo\n]>"
    );
}

#[test]
fn select_line_clamps_at_last_line() {
    // Already on last line → no change.
    assert_state!(
        "hello\n-[foo\n]>",
        |(buf, sels)| cmd_select_line(&buf, sels, MotionMode::Move),
        "hello\n-[foo\n]>"
    );
}

#[test]
fn select_line_backward_from_mid_line() {
    // Cursor mid-line → select full line backward (anchor=`\n`, head=start).
    assert_state!(
        "hello -[w]>orld\nfoo\n",
        |(buf, sels)| cmd_select_line_backward(&buf, sels, MotionMode::Move),
        "<[hello world\n]-foo\n"
    );
}

#[test]
fn select_line_backward_already_at_start_jumps_to_prev() {
    // Selection already starts at line boundary → jump to previous line.
    assert_state!(
        "aaa\n<[bbb\n]-ccc\n",
        |(buf, sels)| cmd_select_line_backward(&buf, sels, MotionMode::Move),
        "<[aaa\n]-bbb\nccc\n"
    );
}

#[test]
fn select_line_backward_clamps_at_first_line() {
    // Already on first line → no change.
    assert_state!(
        "<[hello\n]-world\n",
        |(buf, sels)| cmd_select_line_backward(&buf, sels, MotionMode::Move),
        "<[hello\n]-world\n"
    );
}

// ── cmd_select_line / cmd_select_line_backward (extend mode) ─────────────

#[test]
fn extend_select_line_accumulates_downward() {
    // Each press accumulates one more line.
    assert_state!(
        "-[hello\n]>foo\nbar\n",
        |(buf, sels)| cmd_select_line(&buf, sels, MotionMode::Extend),
        "-[hello\nfoo\n]>bar\n"
    );
}

#[test]
fn extend_select_line_clamps_at_last_line() {
    // Already at last line → no change.
    assert_state!(
        "hello\n-[foo\n]>",
        |(buf, sels)| cmd_select_line(&buf, sels, MotionMode::Extend),
        "hello\n-[foo\n]>"
    );
}

#[test]
fn extend_select_line_backward_accumulates_upward() {
    // Each press accumulates one more line upward.
    assert_state!(
        "aaa\n<[bbb\n]-ccc\n",
        |(buf, sels)| cmd_select_line_backward(&buf, sels, MotionMode::Extend),
        "<[aaa\nbbb\n]-ccc\n"
    );
}

#[test]
fn extend_select_line_backward_clamps_at_first_line() {
    // Already at first line → no change.
    assert_state!(
        "<[hello\n]-world\n",
        |(buf, sels)| cmd_select_line_backward(&buf, sels, MotionMode::Extend),
        "<[hello\n]-world\n"
    );
}

#[test]
fn extend_select_line_from_mid_line() {
    // Starting from a partial selection, the first extend covers the full line.
    assert_state!(
        "hello -[w]>orld\nfoo\n",
        |(buf, sels)| cmd_select_line(&buf, sels, MotionMode::Extend),
        "-[hello world\n]>foo\n"
    );
}

#[test]
fn extend_select_line_backward_from_mid_line() {
    // Starting from a partial selection, the first backward extend covers the full line.
    assert_state!(
        "hello -[w]>orld\nfoo\n",
        |(buf, sels)| cmd_select_line_backward(&buf, sels, MotionMode::Extend),
        "<[hello world\n]-foo\n"
    );
}

#[test]
fn select_line_empty_line() {
    // A bare `\n` line: the cursor is already on the only character (the `\n`),
    // so `x` immediately jumps to the next line.
    assert_state!(
        "hello\n-[\n]>world\n",
        |(buf, sels)| cmd_select_line(&buf, sels, MotionMode::Move),
        "hello\n\n-[world\n]>"
    );
}

#[test]
fn select_line_backward_empty_line() {
    // A bare `\n` line: cursor is at line start → `X` jumps to the previous line.
    assert_state!(
        "hello\n-[\n]>world\n",
        |(buf, sels)| cmd_select_line_backward(&buf, sels, MotionMode::Move),
        "<[hello\n]-\nworld\n"
    );
}

#[test]
fn select_line_multi_cursor() {
    // Two cursors on different lines each independently select their full line.
    // The resulting line selections are non-overlapping and stay separate.
    assert_state!(
        "hello -[w]>orld\nfoo -[b]>ar\nbaz\n",
        |(buf, sels)| cmd_select_line(&buf, sels, MotionMode::Move),
        "-[hello world\n]>-[foo bar\n]>baz\n"
    );
}

#[test]
fn select_line_multi_cursor_same_line_merges() {
    // Two cursors on the same line both produce identical line selections,
    // which map_and_merge collapses to a single selection.
    assert_state!(
        "hell-[o]> -[w]>orld\nfoo\n",
        |(buf, sels)| cmd_select_line(&buf, sels, MotionMode::Move),
        "-[hello world\n]>foo\n"
    );
}

#[test]
fn extend_select_line_multi_cursor_merges() {
    // Two adjacent full-line selections each extend to the next line; because the
    // resulting ranges overlap, map_and_merge unifies them into one selection.
    //
    // sel1 (-[hello world\n]>) end=11 → extends to line 1 → (0,15)
    // sel2 (-[foo\n]>)         end=15 → extends to line 2 → (12,19)
    // (0,15) and (12,19) overlap → merged to (0,19)
    assert_state!(
        "-[hello world\n]>-[foo\n]>bar\n",
        |(buf, sels)| cmd_select_line(&buf, sels, MotionMode::Extend),
        "-[hello world\nfoo\nbar\n]>"
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
