use super::super::*;
use hume_test_fixtures::assert_state;

// ── Inner paragraph ──────────────────────────────────────────────────────────

#[test]
fn inner_paragraph_single_line() {
    assert_state!(
        "-[h]>ello\n\nworld\n",
        |(text, sels)| cmd_inner_paragraph(&text, sels, 0, MotionMode::Move),
        "-[hello]>\n\nworld\n"
    );
}

#[test]
fn inner_paragraph_multiline_excludes_gap() {
    // Cursor on the paragraph's second line — selects both lines, not the
    // blank gap after them.
    assert_state!(
        "para one\n-[l]>ine two\n\nworld\n",
        |(text, sels)| cmd_inner_paragraph(&text, sels, 0, MotionMode::Move),
        "-[para one\nline two]>\n\nworld\n"
    );
}

#[test]
fn inner_paragraph_blank_line_is_noop() {
    // Cursor sits in the gap, not inside any paragraph.
    assert_state!(
        "hello\n-[\n]>world\n",
        |(text, sels)| cmd_inner_paragraph(&text, sels, 0, MotionMode::Move),
        "hello\n-[\n]>world\n"
    );
}

#[test]
fn inner_paragraph_last_paragraph_no_gap() {
    assert_state!(
        "hello\n\n-[w]>orld\n",
        |(text, sels)| cmd_inner_paragraph(&text, sels, 0, MotionMode::Move),
        "hello\n\n-[world]>\n"
    );
}

// ── Around paragraph ─────────────────────────────────────────────────────────

#[test]
fn around_paragraph_includes_gap() {
    assert_state!(
        "-[h]>ello\n\nworld\n",
        |(text, sels)| cmd_around_paragraph(&text, sels, 0, MotionMode::Move),
        "-[hello\n\n]>world\n"
    );
}

#[test]
fn around_paragraph_multiple_blank_lines() {
    assert_state!(
        "-[h]>ello\n\n\nworld\n",
        |(text, sels)| cmd_around_paragraph(&text, sels, 0, MotionMode::Move),
        "-[hello\n\n\n]>world\n"
    );
}

#[test]
fn around_paragraph_last_paragraph_equals_inner() {
    // No trailing gap — `m a p` on the last paragraph is the same as `m i p`.
    assert_state!(
        "hello\n\n-[w]>orld\n",
        |(text, sels)| cmd_around_paragraph(&text, sels, 0, MotionMode::Move),
        "hello\n\n-[world]>\n"
    );
}

#[test]
fn around_paragraph_blank_line_is_noop() {
    assert_state!(
        "hello\n-[\n]>world\n",
        |(text, sels)| cmd_around_paragraph(&text, sels, 0, MotionMode::Move),
        "hello\n-[\n]>world\n"
    );
}
