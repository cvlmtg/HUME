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

#[test]
fn around_paragraph_gap_reaches_buffer_end() {
    // The paragraph's trailing gap runs all the way to EOF (no further
    // paragraph below) — the span must stop at the buffer's last valid
    // position (content domain), not walk onto the phantom trailing line
    // one past it. See `paragraph_span`'s doc comment.
    assert_state!(
        "-[a]>\n\n\n",
        |(text, sels)| cmd_around_paragraph(&text, sels, 0, MotionMode::Move),
        "-[a\n\n\n]>"
    );
}

// ── Grapheme clusters ────────────────────────────────────────────────────────

#[test]
fn inner_paragraph_combining_grapheme_at_end() {
    // "cafe\u{0301}" = c(0) a(1) f(2) e(3) combining_acute(4) \n(5) — the
    // paragraph's last line ends in a 2-codepoint grapheme cluster. The span
    // end must land on the combining mark (4), not the 'e' alone (3).
    assert_state!(
        "-[c]>afe\u{0301}\n\nworld\n",
        |(text, sels)| cmd_inner_paragraph(&text, sels, 0, MotionMode::Move),
        "-[cafe\u{0301}]>\n\nworld\n"
    );
}

// ── Whitespace-only lines ────────────────────────────────────────────────────

#[test]
fn inner_paragraph_whitespace_only_line_does_not_split() {
    // A whitespace-only line is not empty (Helix semantics — is_empty_line
    // requires zero content chars), so it doesn't break the paragraph: all
    // three lines are one paragraph.
    assert_state!(
        "-[a]>\n   \nb\n\nc\n",
        |(text, sels)| cmd_inner_paragraph(&text, sels, 0, MotionMode::Move),
        "-[a\n   \nb]>\n\nc\n"
    );
}

// ── Extend variants ───────────────────────────────────────────────────────────

#[test]
fn extend_inner_paragraph_unions_with_selection() {
    assert_state!(
        "para -[o]>ne\nline two\n\nworld\n",
        |(text, sels)| cmd_inner_paragraph(&text, sels, 0, MotionMode::Extend),
        "-[para one\nline two]>\n\nworld\n"
    );
}

#[test]
fn extend_around_paragraph_grows_past_current() {
    // Selection already covers the first paragraph via a prior `map`;
    // pressing extend-`map` again should grow to include the next paragraph
    // and its gap too (the past-end retry in apply_text_object_extend).
    assert_state!(
        "-[hello\n\n]>world\n\nfoo\n",
        |(text, sels)| cmd_around_paragraph(&text, sels, 0, MotionMode::Extend),
        "-[hello\n\nworld\n\n]>foo\n"
    );
}

// ── Multi-cursor ─────────────────────────────────────────────────────────────

#[test]
fn inner_paragraph_multi_cursor_distinct_paragraphs() {
    assert_state!(
        "-[h]>ello\n\n-[w]>orld\n",
        |(text, sels)| cmd_inner_paragraph(&text, sels, 0, MotionMode::Move),
        "-[hello]>\n\n-[world]>\n"
    );
}
