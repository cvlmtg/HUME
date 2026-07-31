use super::super::*;
use crate::assert_state;

// ── Quotes ────────────────────────────────────────────────────────────────

#[test]
fn inner_double_quote_cursor_inside() {
    assert_state!(
        "\"hel-[l]>o\"\n",
        |(buf, sels)| cmd_inner_double_quote(&buf, sels, 0, MotionMode::Move),
        "\"-[hello]>\"\n"
    );
}

#[test]
fn around_double_quote_cursor_inside() {
    // around includes both quote chars; head = closing `"`.
    assert_state!(
        "\"hel-[l]>o\"\n",
        |(buf, sels)| cmd_around_double_quote(&buf, sels, 0, MotionMode::Move),
        "-[\"hello\"]>\n"
    );
}

#[test]
fn inner_double_quote_cursor_on_open() {
    assert_state!(
        "-[\"]>hello\"\n",
        |(buf, sels)| cmd_inner_double_quote(&buf, sels, 0, MotionMode::Move),
        "\"-[hello]>\"\n"
    );
}

#[test]
fn inner_double_quote_cursor_on_close() {
    assert_state!(
        "\"hello-[\"]>\n",
        |(buf, sels)| cmd_inner_double_quote(&buf, sels, 0, MotionMode::Move),
        "\"-[hello]>\"\n"
    );
}

#[test]
fn inner_double_quote_empty_is_noop() {
    assert_state!(
        "-[\"]>\"foo\n",
        |(buf, sels)| cmd_inner_double_quote(&buf, sels, 0, MotionMode::Move),
        "-[\"]>\"foo\n"
    );
}

#[test]
fn inner_double_quote_second_pair() {
    // Two pairs on the same line — cursor in second pair selects second.
    assert_state!(
        "\"a\" \"b-[c]>\"\n",
        |(buf, sels)| cmd_inner_double_quote(&buf, sels, 0, MotionMode::Move),
        "\"a\" \"-[bc]>\"\n"
    );
}

#[test]
fn inner_single_quote_basic() {
    assert_state!(
        "'hel-[l]>o'\n",
        |(buf, sels)| cmd_inner_single_quote(&buf, sels, 0, MotionMode::Move),
        "'-[hello]>'\n"
    );
}

#[test]
fn inner_backtick_basic() {
    assert_state!(
        "`hel-[l]>o`\n",
        |(buf, sels)| cmd_inner_backtick(&buf, sels, 0, MotionMode::Move),
        "`-[hello]>`\n"
    );
}

#[test]
fn inner_double_quote_not_inside_is_noop() {
    assert_state!(
        "hel-[l]>o\n",
        |(buf, sels)| cmd_inner_double_quote(&buf, sels, 0, MotionMode::Move),
        "hel-[l]>o\n"
    );
}

// ── around_quote variants ─────────────────────────────────────────────────

#[test]
fn around_single_quote_basic() {
    assert_state!(
        "'hel-[l]>o'\n",
        |(buf, sels)| cmd_around_single_quote(&buf, sels, 0, MotionMode::Move),
        "-['hello']>\n"
    );
}

#[test]
fn around_backtick_basic() {
    assert_state!(
        "`hel-[l]>o`\n",
        |(buf, sels)| cmd_around_backtick(&buf, sels, 0, MotionMode::Move),
        "-[`hello`]>\n"
    );
}
