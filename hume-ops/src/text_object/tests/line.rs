use super::super::*;
use hume_test_fixtures::assert_state;

// ── Line ──────────────────────────────────────────────────────────────────

#[test]
fn inner_line_middle() {
    // Selection covers `world`, head=d (last char before \n).
    assert_state!(
        "hello\n-[w]>orld\nfoo\n",
        |(text, sels)| cmd_inner_line(&text, sels, 0, MotionMode::Move),
        "hello\n-[world]>\nfoo\n"
    );
}

#[test]
fn inner_line_start_of_line() {
    assert_state!(
        "-[h]>ello world\n",
        |(text, sels)| cmd_inner_line(&text, sels, 0, MotionMode::Move),
        "-[hello world]>\n"
    );
}

#[test]
fn inner_line_end_of_content() {
    assert_state!(
        "hello worl-[d]>\n",
        |(text, sels)| cmd_inner_line(&text, sels, 0, MotionMode::Move),
        "-[hello world]>\n"
    );
}

#[test]
fn inner_line_empty_line_is_noop() {
    // An empty line is just "\n" — no content, so inner_line returns None
    // and the selection is preserved.
    assert_state!(
        "hello\n-[\n]>world\n",
        |(text, sels)| cmd_inner_line(&text, sels, 0, MotionMode::Move),
        "hello\n-[\n]>world\n"
    );
}

#[test]
fn inner_line_combining_grapheme_before_newline() {
    // "cafe\u{0301}" = c(0) a(1) f(2) e(3) combining_acute(4) \n(5).
    // inner_line must include the full last grapheme cluster, so the
    // selection end must be 4 (the combining mark) not 3 (the 'e' alone).
    // Naive `last - 1` arithmetic would produce a broken mid-cluster end position.
    assert_state!(
        "-[c]>afe\u{0301}\n",
        |(text, sels)| cmd_inner_line(&text, sels, 0, MotionMode::Move),
        "-[cafe\u{0301}]>\n"
    );
}

#[test]
fn around_line_includes_newline() {
    // Selection covers `world\n`; head is the newline char.
    assert_state!(
        "hello\n-[w]>orld\nfoo\n",
        |(text, sels)| cmd_around_line(&text, sels, 0, MotionMode::Move),
        "hello\n-[world\n]>foo\n"
    );
}

#[test]
fn around_line_empty_line() {
    // An empty line is just "\n"; around_line selects that single char.
    // anchor == head, so serialises as a cursor (|).
    assert_state!(
        "hello\n-[\n]>world\n",
        |(text, sels)| cmd_around_line(&text, sels, 0, MotionMode::Move),
        "hello\n-[\n]>world\n"
    );
}

#[test]
fn inner_line_multi_cursor_same_line_merges() {
    // Two cursors on the same line both select that line's content, then merge.
    assert_state!(
        "-[h]>el-[l]>o\n",
        |(text, sels)| cmd_inner_line(&text, sels, 0, MotionMode::Move),
        "-[hello]>\n"
    );
}

#[test]
fn inner_line_multi_cursor_different_lines() {
    assert_state!(
        "-[h]>ello\n-[w]>orld\n",
        |(text, sels)| cmd_inner_line(&text, sels, 0, MotionMode::Move),
        "-[hello]>\n-[world]>\n"
    );
}

#[test]
fn around_line_multi_cursor_different_lines() {
    assert_state!(
        "-[h]>ello\n-[w]>orld\n",
        |(text, sels)| cmd_around_line(&text, sels, 0, MotionMode::Move),
        "-[hello\n]>-[world\n]>"
    );
}
