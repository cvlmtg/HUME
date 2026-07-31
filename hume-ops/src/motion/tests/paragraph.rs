use super::super::*;
use hume_test_fixtures::assert_state;

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

#[test]
fn prev_paragraph_empty_buffer() {
    assert_state!(
        "-[\n]>",
        |(buf, sels)| cmd_prev_paragraph(&buf, sels, 1, MotionMode::Move),
        "-[\n]>"
    );
}
