use super::super::*;
use hume_test_fixtures::assert_state;

// ── move_right ────────────────────────────────────────────────────────────

#[test]
fn move_right_basic() {
    assert_state!(
        "-[h]>ello\n",
        |(text, sels)| cmd_move_right(&text, sels, 1, MotionMode::Move),
        "h-[e]>llo\n"
    );
}

#[test]
fn move_right_to_eof() {
    assert_state!(
        "hell-[o]>\n",
        |(text, sels)| cmd_move_right(&text, sels, 1, MotionMode::Move),
        "hello-[\n]>"
    );
}

#[test]
fn move_right_clamp_at_eof() {
    assert_state!(
        "hello-[\n]>",
        |(text, sels)| cmd_move_right(&text, sels, 1, MotionMode::Move),
        "hello-[\n]>"
    );
}

#[test]
fn move_right_empty_buffer() {
    assert_state!(
        "-[\n]>",
        |(text, sels)| cmd_move_right(&text, sels, 1, MotionMode::Move),
        "-[\n]>"
    );
}

#[test]
fn move_right_multi_cursor() {
    assert_state!(
        "-[h]>-[e]>llo\n",
        |(text, sels)| cmd_move_right(&text, sels, 1, MotionMode::Move),
        "h-[e]>-[l]>lo\n"
    );
}

#[test]
fn move_right_grapheme_cluster() {
    // "e\u{0301}" is two chars but one grapheme cluster (e + combining acute).
    // move_right from offset 0 must skip the entire cluster to offset 2.
    assert_state!(
        "-[e\u{0301}]>x\n",
        |(text, sels)| cmd_move_right(&text, sels, 1, MotionMode::Move),
        "e\u{0301}-[x]>\n"
    );
}

// ── move_left ─────────────────────────────────────────────────────────────

#[test]
fn move_left_basic() {
    assert_state!(
        "h-[e]>llo\n",
        |(text, sels)| cmd_move_left(&text, sels, 1, MotionMode::Move),
        "-[h]>ello\n"
    );
}

#[test]
fn move_left_clamp_at_start() {
    assert_state!(
        "-[h]>ello\n",
        |(text, sels)| cmd_move_left(&text, sels, 1, MotionMode::Move),
        "-[h]>ello\n"
    );
}

#[test]
fn move_left_empty_buffer() {
    assert_state!(
        "-[\n]>",
        |(text, sels)| cmd_move_left(&text, sels, 1, MotionMode::Move),
        "-[\n]>"
    );
}

#[test]
fn move_left_grapheme_cluster() {
    // "e\u{0301}" is two chars but one grapheme cluster.
    // move_left from offset 2 (after the cluster) must jump to 0.
    assert_state!(
        "e\u{0301}-[x]>\n",
        |(text, sels)| cmd_move_left(&text, sels, 1, MotionMode::Move),
        "-[e]>\u{0301}x\n"
    );
}

#[test]
fn move_left_multi_cursor_merge() {
    // Cursors at 0 and 1. Both move left: 0→0 and 1→0. Same position → merge.
    assert_state!(
        "-[a]>-[b]>c\n",
        |(text, sels)| cmd_move_left(&text, sels, 1, MotionMode::Move),
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
        |(text, sels)| cmd_move_right(&text, sels, 1, MotionMode::Extend),
        "-[he]>llo\n"
    );
}

#[test]
fn extend_right_grows_selection() {
    // Existing forward selection anchor=0, head=1. Extend right: head moves to 2.
    // anchor=0, head=2 → "-[hel]>lo\n".
    assert_state!(
        "-[he]>llo\n",
        |(text, sels)| cmd_move_right(&text, sels, 1, MotionMode::Extend),
        "-[hel]>lo\n"
    );
}

#[test]
fn extend_right_clamp_at_eof() {
    assert_state!(
        "hello-[\n]>",
        |(text, sels)| cmd_move_right(&text, sels, 1, MotionMode::Extend),
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
        |(text, sels)| cmd_move_left(&text, sels, 1, MotionMode::Extend),
        "<[he]-llo\n"
    );
}

#[test]
fn extend_left_shrinks_forward_selection() {
    // Forward selection anchor=0, head=2. Extend left: head moves to 1.
    // anchor=0, head=1 → "-[he]>llo\n".
    assert_state!(
        "-[hel]>lo\n",
        |(text, sels)| cmd_move_left(&text, sels, 1, MotionMode::Extend),
        "-[he]>llo\n"
    );
}

#[test]
fn extend_left_clamp_at_start() {
    assert_state!(
        "-[h]>ello\n",
        |(text, sels)| cmd_move_left(&text, sels, 1, MotionMode::Extend),
        "-[h]>ello\n"
    );
}

#[test]
fn extend_left_reverses_direction() {
    // Forward selection anchor=3,head=3. Extend left 3 times: head→0.
    // anchor=3 > head=0 → becomes a backward selection spanning "hell".
    assert_state!(
        "hel-[l]>o\n",
        |(text, sels)| cmd_move_left(&text, sels, 3, MotionMode::Extend),
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
        |(text, sels)| cmd_move_right(&text, sels, 1, MotionMode::Extend),
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
        |(text, sels)| cmd_move_left(&text, sels, 1, MotionMode::Extend),
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
        |(text, sels)| cmd_move_right(&text, sels, 2, MotionMode::Extend),
        "-[foo]> -[bar]>\n"
    );
}

// ── goto_first_line ───────────────────────────────────────────────────────

#[test]
fn goto_first_line_from_middle() {
    assert_state!(
        "hello\nwor-[l]>d\n",
        |(text, sels)| cmd_goto_first_line(&text, sels, 1, MotionMode::Move),
        "-[h]>ello\nworld\n"
    );
}

#[test]
fn goto_first_line_already_at_start() {
    assert_state!(
        "-[h]>ello\nworld\n",
        |(text, sels)| cmd_goto_first_line(&text, sels, 1, MotionMode::Move),
        "-[h]>ello\nworld\n"
    );
}

#[test]
fn goto_first_line_single_line_buffer() {
    assert_state!(
        "hel-[l]>o\n",
        |(text, sels)| cmd_goto_first_line(&text, sels, 1, MotionMode::Move),
        "-[h]>ello\n"
    );
}

#[test]
fn goto_first_line_empty_buffer() {
    assert_state!(
        "-[\n]>",
        |(text, sels)| cmd_goto_first_line(&text, sels, 1, MotionMode::Move),
        "-[\n]>"
    );
}

#[test]
fn goto_first_line_multi_cursor() {
    assert_state!(
        "-[a]>bc\ndef\nghi-[j]>\n",
        |(text, sels)| cmd_goto_first_line(&text, sels, 1, MotionMode::Move),
        "-[a]>bc\ndef\nghij\n"
    );
}

// ── goto_last_line ────────────────────────────────────────────────────────

#[test]
fn goto_last_line_from_first() {
    assert_state!(
        "-[h]>ello\nworld\n",
        |(text, sels)| cmd_goto_last_line(&text, sels, 1, MotionMode::Move),
        "hello\n-[w]>orld\n"
    );
}

#[test]
fn goto_last_line_already_at_last() {
    assert_state!(
        "hello\n-[w]>orld\n",
        |(text, sels)| cmd_goto_last_line(&text, sels, 1, MotionMode::Move),
        "hello\n-[w]>orld\n"
    );
}

#[test]
fn goto_last_line_single_line_buffer() {
    assert_state!(
        "-[\n]>",
        |(text, sels)| cmd_goto_last_line(&text, sels, 1, MotionMode::Move),
        "-[\n]>"
    );
}

#[test]
fn goto_last_line_multi_line() {
    assert_state!(
        "aaa\n-[b]>bb\nccc\n",
        |(text, sels)| cmd_goto_last_line(&text, sels, 1, MotionMode::Move),
        "aaa\nbbb\n-[c]>cc\n"
    );
}

#[test]
fn goto_last_line_multi_cursor() {
    // Both cursors converge to the same position — merged into one.
    assert_state!(
        "-[a]>aa\nbbb\n-[c]>cc\n",
        |(text, sels)| cmd_goto_last_line(&text, sels, 1, MotionMode::Move),
        "aaa\nbbb\n-[c]>cc\n"
    );
}

#[test]
fn move_right_count_3() {
    // h(0) → e(1) → l(2) → l(3)
    assert_state!(
        "-[h]>ello\n",
        |(text, sels)| cmd_move_right(&text, sels, 3, MotionMode::Move),
        "hel-[l]>o\n"
    );
}

#[test]
fn move_right_count_clamps_at_eof() {
    // count=100 far exceeds the buffer length — clamps at the trailing '\n'.
    assert_state!(
        "-[h]>ello\n",
        |(text, sels)| cmd_move_right(&text, sels, 100, MotionMode::Move),
        "hello-[\n]>"
    );
}

#[test]
fn move_left_count_3() {
    // \n(5) → o(4) → l(3) → l(2)
    assert_state!(
        "hello-[\n]>",
        |(text, sels)| cmd_move_left(&text, sels, 3, MotionMode::Move),
        "he-[l]>lo\n"
    );
}

#[test]
fn extend_right_count_3() {
    // Extend: anchor stays at old head (0), head folds 3 steps: 0→1→2→3.
    // Selection anchor=0, head=3: covers "hell".
    assert_state!(
        "-[h]>ello\n",
        |(text, sels)| cmd_move_right(&text, sels, 3, MotionMode::Extend),
        "-[hell]>o\n"
    );
}

#[test]
fn move_right_count_grapheme_cluster() {
    // Text: "e◌́x\n". Grapheme clusters: {e◌́}(0..2), {x}(2), {\n}(3).
    // count=2 from offset 0: step1 → 2 (x), step2 → 3 (\n). Clamped to len-1=3.
    assert_state!(
        "-[e\u{0301}]>x\n",
        |(text, sels)| cmd_move_right(&text, sels, 2, MotionMode::Move),
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
        |(text, sels)| cmd_move_right(&text, sels, 3, MotionMode::Move),
        "hel-[l]>o-[\n]>"
    );
}
