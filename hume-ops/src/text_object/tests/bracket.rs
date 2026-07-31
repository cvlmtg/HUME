use super::super::*;
use hume_test_fixtures::assert_state;

// ── Brackets ──────────────────────────────────────────────────────────────

#[test]
fn inner_paren_cursor_inside() {
    assert_state!(
        "(-[h]>ello)\n",
        |(buf, sels)| cmd_inner_paren(&buf, sels, 0, MotionMode::Move),
        "(-[hello]>)\n"
    );
}

#[test]
fn around_paren_cursor_inside() {
    // around includes the parens themselves; head = `)`.
    assert_state!(
        "(-[h]>ello)\n",
        |(buf, sels)| cmd_around_paren(&buf, sels, 0, MotionMode::Move),
        "-[(hello)]>\n"
    );
}

#[test]
fn inner_paren_cursor_on_open() {
    // Cursor ON `(` — treated as if inside; same result as cursor inside.
    assert_state!(
        "-[(]>hello)\n",
        |(buf, sels)| cmd_inner_paren(&buf, sels, 0, MotionMode::Move),
        "(-[hello]>)\n"
    );
}

#[test]
fn inner_paren_cursor_on_close() {
    assert_state!(
        "(hello-[)]>\n",
        |(buf, sels)| cmd_inner_paren(&buf, sels, 0, MotionMode::Move),
        "(-[hello]>)\n"
    );
}

#[test]
fn inner_paren_empty_is_noop() {
    assert_state!(
        "-[(]>)\n",
        |(buf, sels)| cmd_inner_paren(&buf, sels, 0, MotionMode::Move),
        "-[(]>)\n"
    );
}

#[test]
fn inner_paren_nested_cursor_on_inner() {
    // Cursor inside inner `(b)` — selects `b`, which is a single char.
    // anchor == head, so serialises as a cursor.
    assert_state!(
        "(a(-[b]>)c)\n",
        |(buf, sels)| cmd_inner_paren(&buf, sels, 0, MotionMode::Move),
        "(a(-[b]>)c)\n"
    );
}

#[test]
fn inner_paren_nested_cursor_on_outer_content() {
    // Cursor on `a` (outside inner parens) — innermost enclosing pair
    // is the outer `(...)`, selects `a(b)c`.
    assert_state!(
        "(-[a]>(b)c)\n",
        |(buf, sels)| cmd_inner_paren(&buf, sels, 0, MotionMode::Move),
        "(-[a(b)c]>)\n"
    );
}

#[test]
fn inner_brace_basic() {
    assert_state!(
        "{-[h]>ello}\n",
        |(buf, sels)| cmd_inner_brace(&buf, sels, 0, MotionMode::Move),
        "{-[hello]>}\n"
    );
}

#[test]
fn inner_bracket_basic() {
    assert_state!(
        "[-[h]>ello]\n",
        |(buf, sels)| cmd_inner_bracket(&buf, sels, 0, MotionMode::Move),
        "[-[hello]>]\n"
    );
}

#[test]
fn inner_angle_basic() {
    assert_state!(
        "<-[h]>ello>\n",
        |(buf, sels)| cmd_inner_angle(&buf, sels, 0, MotionMode::Move),
        "<-[hello]>>\n"
    );
}

#[test]
fn inner_paren_no_match_is_noop() {
    assert_state!(
        "hel-[l]>o\n",
        |(buf, sels)| cmd_inner_paren(&buf, sels, 0, MotionMode::Move),
        "hel-[l]>o\n"
    );
}

#[test]
fn inner_paren_multiline() {
    // Bracket pair spans two lines; inner content is `\nhello\n`.
    // anchor = `\n` after `(`, head = `\n` before `)`.
    assert_state!(
        "(\n-[h]>ello\n)\n",
        |(buf, sels)| cmd_inner_paren(&buf, sels, 0, MotionMode::Move),
        "(-[\nhello\n]>)\n"
    );
}

#[test]
fn inner_paren_two_cursors_same_pair_merge() {
    // Both cursors inside the same parens — both map to the same range → merge.
    assert_state!(
        "(-[h]>el-[l]>o)\n",
        |(buf, sels)| cmd_inner_paren(&buf, sels, 0, MotionMode::Move),
        "(-[hello]>)\n"
    );
}

// ── around_bracket variants ───────────────────────────────────────────────

#[test]
fn around_brace_basic() {
    assert_state!(
        "{-[h]>ello}\n",
        |(buf, sels)| cmd_around_brace(&buf, sels, 0, MotionMode::Move),
        "-[{hello}]>\n"
    );
}

#[test]
fn around_bracket_basic() {
    assert_state!(
        "[-[h]>ello]\n",
        |(buf, sels)| cmd_around_bracket(&buf, sels, 0, MotionMode::Move),
        "-[[hello]]>\n"
    );
}

#[test]
fn around_angle_basic() {
    assert_state!(
        "<-[h]>ello>\n",
        |(buf, sels)| cmd_around_angle(&buf, sels, 0, MotionMode::Move),
        "-[<hello>]>\n"
    );
}

// ── multi-line bracket for non-paren types ────────────────────────────────

#[test]
fn inner_brace_multiline() {
    assert_state!(
        "{\n-[h]>ello\n}\n",
        |(buf, sels)| cmd_inner_brace(&buf, sels, 0, MotionMode::Move),
        "{-[\nhello\n]>}\n"
    );
}

#[test]
fn extend_inner_paren_grows_selection() {
    // "hello (world) foo\n": '('=6, ')'=12. Forward sel from 'h'(0) to 'w'(7).
    // extend_inner_paren at head=7 ('w' inside parens):
    //   inner_bracket(7) → inner = (7, 11) = "world".
    //   Union: min(0,7)=0, max(7,11)=11. head=11 ('d').
    // Serialized: ]> at position 12 (before ')') → "-[hello (world]>) foo\n".
    assert_state!(
        "-[hello (w]>orld) foo\n",
        |(buf, sels)| cmd_inner_paren(&buf, sels, 0, MotionMode::Extend),
        "-[hello (world]>) foo\n"
    );
}

#[test]
fn extend_text_object_noop_on_no_match() {
    // When extend text-object has no match, selection is unchanged.
    // inner_paren on "hello\n" finds no parens → returns None → sel unchanged.
    assert_state!(
        "-[h]>ello\n",
        |(buf, sels)| {
            let s1 = cmd_inner_word(&buf, sels, 0, MotionMode::Move); // selects "hello" (0,4)
            cmd_inner_paren(&buf, s1, 0, MotionMode::Extend) // no parens → no-op → "hello" unchanged
        },
        "-[hello]>\n"
    );
}

#[test]
fn extend_around_paren_grows_selection() {
    // "hello (world) foo\n": forward selection from 'h'(0) to 'w'(7).
    // extend_around_paren at head=7 ('w' inside parens):
    //   around_bracket(7) finds "(world)" (6,13).
    //   Union: min(0,6)=0, max(7,13)=13 → (0,13) = "hello (world)".
    assert_state!(
        "-[hello (w]>orld) foo\n",
        |(buf, sels)| cmd_around_paren(&buf, sels, 0, MotionMode::Extend),
        "-[hello (world)]> foo\n"
    );
}

#[test]
fn extend_around_paren_from_matched_pair_grows_outward() {
    // Regression: selection is already "(b)" via a prior `ma(`; pressing
    // extend-`ma(` again should grow to the enclosing "(a (b) a)".
    //
    // "(a (b) a)\n": (=0,a=1,' '=2,(=3,b=4,)=5,' '=6,a=7,)=8,\n=9
    // Selection: anchor=3, head=5 (covers "(b)").
    //
    // First try: around_bracket(head=5) finds ')' at 5 → same pair (3,5).
    // Union is a no-op. Retry from next_grapheme_boundary(end()=5)=6 (' ').
    // around_bracket(6): scan_left finds '(' at 0 (skipping the inner pair),
    // scan_right finds ')' at 8 → (0,8). Union: (0,8). Grows.
    assert_state!(
        "(a -[(b)]> a)\n",
        |(buf, sels)| cmd_around_paren(&buf, sels, 0, MotionMode::Extend),
        "-[(a (b) a)]>\n"
    );
}

#[test]
fn extend_inner_paren_from_matched_pair_grows_outward() {
    // Same setup: selection "(b)" in "(a (b) a)\n".
    // First try: inner_bracket(head=5) → (4,4) = "b". Union no-op (subset).
    // Retry from pos 6: inner_bracket(6) → inner of outer pair = (1,7) = "a (b) a".
    // Union: (1,7). anchor=1, head=7 → "(-[a (b) a]>)\n".
    assert_state!(
        "(a -[(b)]> a)\n",
        |(buf, sels)| cmd_inner_paren(&buf, sels, 0, MotionMode::Extend),
        "(-[a (b) a]>)\n"
    );
}

#[test]
fn extend_around_paren_no_outer_pair_is_noop() {
    // When the selection already covers the outermost pair, there is no
    // enclosing pair to grow into — the command is a no-op.
    //
    // "(a b)\n": (=0,a=1,' '=2,b=3,)=4,\n=5. Selection anchor=0, head=4.
    // First try: around_bracket(head=4=')') → (0,4). Union no-op.
    // Retry from pos 5 ('\n'): scan_left hits ')' at 4 (depth=1), then
    // '(' at 0 (depth=0→continues), exits at i=0 → None. No-op.
    assert_state!(
        "-[(a b)]>\n",
        |(buf, sels)| cmd_around_paren(&buf, sels, 0, MotionMode::Extend),
        "-[(a b)]>\n"
    );
}
