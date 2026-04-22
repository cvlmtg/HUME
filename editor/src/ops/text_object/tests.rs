use super::*;
use crate::assert_state;

// ── Line ──────────────────────────────────────────────────────────────────

#[test]
fn inner_line_middle() {
    // Selection covers `world`, head=d (last char before \n).
    assert_state!(
        "hello\n-[w]>orld\nfoo\n",
        |(buf, sels)| cmd_inner_line(&buf, sels, MotionMode::Move),
        "hello\n-[world]>\nfoo\n"
    );
}

#[test]
fn inner_line_start_of_line() {
    assert_state!(
        "-[h]>ello world\n",
        |(buf, sels)| cmd_inner_line(&buf, sels, MotionMode::Move),
        "-[hello world]>\n"
    );
}

#[test]
fn inner_line_end_of_content() {
    assert_state!(
        "hello worl-[d]>\n",
        |(buf, sels)| cmd_inner_line(&buf, sels, MotionMode::Move),
        "-[hello world]>\n"
    );
}

#[test]
fn inner_line_empty_line_is_noop() {
    // An empty line is just "\n" — no content, so inner_line returns None
    // and the selection is preserved.
    assert_state!(
        "hello\n-[\n]>world\n",
        |(buf, sels)| cmd_inner_line(&buf, sels, MotionMode::Move),
        "hello\n-[\n]>world\n"
    );
}

#[test]
fn inner_line_combining_grapheme_before_newline() {
    // "cafe\u{0301}" = c(0) a(1) f(2) e(3) combining_acute(4) \n(5).
    // inner_line must include the full last grapheme cluster, so the
    // selection end must be 4 (the combining mark) not 3 (the 'e' alone).
    // The old `last - 1` arithmetic would have produced a broken
    // mid-cluster end position.
    assert_state!(
        "-[c]>afe\u{0301}\n",
        |(buf, sels)| cmd_inner_line(&buf, sels, MotionMode::Move),
        "-[cafe\u{0301}]>\n"
    );
}

#[test]
fn around_line_includes_newline() {
    // Selection covers `world\n`; head is the newline char.
    assert_state!(
        "hello\n-[w]>orld\nfoo\n",
        |(buf, sels)| cmd_around_line(&buf, sels, MotionMode::Move),
        "hello\n-[world\n]>foo\n"
    );
}

#[test]
fn around_line_empty_line() {
    // An empty line is just "\n"; around_line selects that single char.
    // anchor == head, so serialises as a cursor (|).
    assert_state!(
        "hello\n-[\n]>world\n",
        |(buf, sels)| cmd_around_line(&buf, sels, MotionMode::Move),
        "hello\n-[\n]>world\n"
    );
}

// ── Word ──────────────────────────────────────────────────────────────────

#[test]
fn inner_word_middle() {
    // head=o (last char of `hello`).
    assert_state!(
        "-[h]>ello world\n",
        |(buf, sels)| cmd_inner_word(&buf, sels, MotionMode::Move),
        "-[hello]> world\n"
    );
}

#[test]
fn inner_word_cursor_at_end_of_word() {
    assert_state!(
        "hell-[o]> world\n",
        |(buf, sels)| cmd_inner_word(&buf, sels, MotionMode::Move),
        "-[hello]> world\n"
    );
}

#[test]
fn inner_word_cursor_on_whitespace() {
    // Two spaces between `foo` and `bar`; cursor on the first space.
    // inner_word selects the entire whitespace run (both spaces).
    // head = second space, serialised as `#[ | ]#`.
    assert_state!(
        "foo-[ ]> bar\n",
        |(buf, sels)| cmd_inner_word(&buf, sels, MotionMode::Move),
        "foo-[  ]>bar\n"
    );
}

#[test]
fn inner_word_cursor_on_punctuation() {
    // Both `!!` are Punctuation — selected as one run.
    assert_state!(
        "foo-[!]>!\n",
        |(buf, sels)| cmd_inner_word(&buf, sels, MotionMode::Move),
        "foo-[!!]>\n"
    );
}

#[test]
fn around_word_includes_trailing_space() {
    // Trailing space is included; head = the space char.
    assert_state!(
        "-[h]>ello world\n",
        |(buf, sels)| cmd_around_word(&buf, sels, MotionMode::Move),
        "-[hello ]>world\n"
    );
}

#[test]
fn around_word_no_trailing_space_uses_leading() {
    // "world" at end of line has no trailing space, so leading space included.
    assert_state!(
        "hello -[w]>orld\n",
        |(buf, sels)| cmd_around_word(&buf, sels, MotionMode::Move),
        "hello-[ world]>\n"
    );
}

#[test]
fn inner_word_includes_combining_grapheme() {
    // Text: "cafe\u{0301} world\n"
    // char offsets: c(0) a(1) f(2) e(3) ◌́(4) ' '(5) w(6) ...
    // Grapheme clusters: {c}{a}{f}{e◌́}{ }{w}...
    //
    // Old code (end += 1) stops at offset 3 because the combining codepoint
    // at offset 4 is classified as Punctuation — a false word/punct boundary
    // inside the grapheme. New code steps by grapheme boundary: the next
    // cluster after offset 3 starts at offset 5 (space), so the word ends
    // at offset 4 (last codepoint of the {e◌́} grapheme) — the full cluster
    // is included.
    assert_state!(
        "-[c]>afe\u{0301} world\n",
        |(buf, sels)| cmd_inner_word(&buf, sels, MotionMode::Move),
        "-[cafe\u{0301}]> world\n"
    );
}

// ── WORD ──────────────────────────────────────────────────────────────────

#[test]
#[allow(non_snake_case)]
fn inner_WORD_spans_punctuation() {
    // `hello.world` is one WORD (no whitespace boundary within it).
    assert_state!(
        "-[h]>ello.world foo\n",
        |(buf, sels)| cmd_inner_WORD(&buf, sels, MotionMode::Move),
        "-[hello.world]> foo\n"
    );
}

// ── Brackets ──────────────────────────────────────────────────────────────

#[test]
fn inner_paren_cursor_inside() {
    assert_state!(
        "(-[h]>ello)\n",
        |(buf, sels)| cmd_inner_paren(&buf, sels, MotionMode::Move),
        "(-[hello]>)\n"
    );
}

#[test]
fn around_paren_cursor_inside() {
    // around includes the parens themselves; head = `)`.
    assert_state!(
        "(-[h]>ello)\n",
        |(buf, sels)| cmd_around_paren(&buf, sels, MotionMode::Move),
        "-[(hello)]>\n"
    );
}

#[test]
fn inner_paren_cursor_on_open() {
    // Cursor ON `(` — treated as if inside; same result as cursor inside.
    assert_state!(
        "-[(]>hello)\n",
        |(buf, sels)| cmd_inner_paren(&buf, sels, MotionMode::Move),
        "(-[hello]>)\n"
    );
}

#[test]
fn inner_paren_cursor_on_close() {
    assert_state!(
        "(hello-[)]>\n",
        |(buf, sels)| cmd_inner_paren(&buf, sels, MotionMode::Move),
        "(-[hello]>)\n"
    );
}

#[test]
fn inner_paren_empty_is_noop() {
    assert_state!(
        "-[(]>)\n",
        |(buf, sels)| cmd_inner_paren(&buf, sels, MotionMode::Move),
        "-[(]>)\n"
    );
}

#[test]
fn inner_paren_nested_cursor_on_inner() {
    // Cursor inside inner `(b)` — selects `b`, which is a single char.
    // anchor == head, so serialises as a cursor.
    assert_state!(
        "(a(-[b]>)c)\n",
        |(buf, sels)| cmd_inner_paren(&buf, sels, MotionMode::Move),
        "(a(-[b]>)c)\n"
    );
}

#[test]
fn inner_paren_nested_cursor_on_outer_content() {
    // Cursor on `a` (outside inner parens) — innermost enclosing pair
    // is the outer `(...)`, selects `a(b)c`.
    assert_state!(
        "(-[a]>(b)c)\n",
        |(buf, sels)| cmd_inner_paren(&buf, sels, MotionMode::Move),
        "(-[a(b)c]>)\n"
    );
}

#[test]
fn inner_brace_basic() {
    assert_state!(
        "{-[h]>ello}\n",
        |(buf, sels)| cmd_inner_brace(&buf, sels, MotionMode::Move),
        "{-[hello]>}\n"
    );
}

#[test]
fn inner_bracket_basic() {
    assert_state!(
        "[-[h]>ello]\n",
        |(buf, sels)| cmd_inner_bracket(&buf, sels, MotionMode::Move),
        "[-[hello]>]\n"
    );
}

#[test]
fn inner_angle_basic() {
    assert_state!(
        "<-[h]>ello>\n",
        |(buf, sels)| cmd_inner_angle(&buf, sels, MotionMode::Move),
        "<-[hello]>>\n"
    );
}

#[test]
fn inner_paren_no_match_is_noop() {
    assert_state!(
        "hel-[l]>o\n",
        |(buf, sels)| cmd_inner_paren(&buf, sels, MotionMode::Move),
        "hel-[l]>o\n"
    );
}

#[test]
fn inner_paren_multiline() {
    // Bracket pair spans two lines; inner content is `\nhello\n`.
    // anchor = `\n` after `(`, head = `\n` before `)`.
    assert_state!(
        "(\n-[h]>ello\n)\n",
        |(buf, sels)| cmd_inner_paren(&buf, sels, MotionMode::Move),
        "(-[\nhello\n]>)\n"
    );
}

// ── Quotes ────────────────────────────────────────────────────────────────

#[test]
fn inner_double_quote_cursor_inside() {
    assert_state!(
        "\"hel-[l]>o\"\n",
        |(buf, sels)| cmd_inner_double_quote(&buf, sels, MotionMode::Move),
        "\"-[hello]>\"\n"
    );
}

#[test]
fn around_double_quote_cursor_inside() {
    // around includes both quote chars; head = closing `"`.
    assert_state!(
        "\"hel-[l]>o\"\n",
        |(buf, sels)| cmd_around_double_quote(&buf, sels, MotionMode::Move),
        "-[\"hello\"]>\n"
    );
}

#[test]
fn inner_double_quote_cursor_on_open() {
    assert_state!(
        "-[\"]>hello\"\n",
        |(buf, sels)| cmd_inner_double_quote(&buf, sels, MotionMode::Move),
        "\"-[hello]>\"\n"
    );
}

#[test]
fn inner_double_quote_cursor_on_close() {
    assert_state!(
        "\"hello-[\"]>\n",
        |(buf, sels)| cmd_inner_double_quote(&buf, sels, MotionMode::Move),
        "\"-[hello]>\"\n"
    );
}

#[test]
fn inner_double_quote_empty_is_noop() {
    assert_state!(
        "-[\"]>\"foo\n",
        |(buf, sels)| cmd_inner_double_quote(&buf, sels, MotionMode::Move),
        "-[\"]>\"foo\n"
    );
}

#[test]
fn inner_double_quote_second_pair() {
    // Two pairs on the same line — cursor in second pair selects second.
    assert_state!(
        "\"a\" \"b-[c]>\"\n",
        |(buf, sels)| cmd_inner_double_quote(&buf, sels, MotionMode::Move),
        "\"a\" \"-[bc]>\"\n"
    );
}

#[test]
fn inner_single_quote_basic() {
    assert_state!(
        "'hel-[l]>o'\n",
        |(buf, sels)| cmd_inner_single_quote(&buf, sels, MotionMode::Move),
        "'-[hello]>'\n"
    );
}

#[test]
fn inner_backtick_basic() {
    assert_state!(
        "`hel-[l]>o`\n",
        |(buf, sels)| cmd_inner_backtick(&buf, sels, MotionMode::Move),
        "`-[hello]>`\n"
    );
}

#[test]
fn inner_double_quote_not_inside_is_noop() {
    assert_state!(
        "hel-[l]>o\n",
        |(buf, sels)| cmd_inner_double_quote(&buf, sels, MotionMode::Move),
        "hel-[l]>o\n"
    );
}

// ── Multi-cursor ──────────────────────────────────────────────────────────

#[test]
fn inner_word_multi_cursor_different_words() {
    assert_state!(
        "-[h]>ello -[w]>orld\n",
        |(buf, sels)| cmd_inner_word(&buf, sels, MotionMode::Move),
        "-[hello]> -[world]>\n"
    );
}

#[test]
fn inner_word_multi_cursor_same_word_merges() {
    // Two cursors in the same word — both select "hello", merge to one selection.
    assert_state!(
        "-[h]>el-[l]>o world\n",
        |(buf, sels)| cmd_inner_word(&buf, sels, MotionMode::Move),
        "-[hello]> world\n"
    );
}

#[test]
fn around_word_multi_cursor() {
    // "hello world foo\n": cursor 0 on 'h'(0) → "hello "(0..5); cursor 1 on 'f'(12) → " foo"(11..14).
    assert_state!(
        "-[h]>ello world-[ ]>foo\n",
        |(buf, sels)| cmd_around_word(&buf, sels, MotionMode::Move),
        "-[hello ]>world-[ foo]>\n"
    );
}

#[test]
fn inner_line_multi_cursor_same_line_merges() {
    // Two cursors on the same line both select that line's content, then merge.
    assert_state!(
        "-[h]>el-[l]>o\n",
        |(buf, sels)| cmd_inner_line(&buf, sels, MotionMode::Move),
        "-[hello]>\n"
    );
}

#[test]
fn inner_line_multi_cursor_different_lines() {
    assert_state!(
        "-[h]>ello\n-[w]>orld\n",
        |(buf, sels)| cmd_inner_line(&buf, sels, MotionMode::Move),
        "-[hello]>\n-[world]>\n"
    );
}

#[test]
fn around_line_multi_cursor_different_lines() {
    assert_state!(
        "-[h]>ello\n-[w]>orld\n",
        |(buf, sels)| cmd_around_line(&buf, sels, MotionMode::Move),
        "-[hello\n]>-[world\n]>"
    );
}

#[test]
#[allow(non_snake_case)]
fn inner_WORD_multi_cursor() {
    assert_state!(
        "-[h]>ello.world -[f]>oo\n",
        |(buf, sels)| cmd_inner_WORD(&buf, sels, MotionMode::Move),
        "-[hello.world]> -[foo]>\n"
    );
}

#[test]
fn inner_paren_two_cursors_same_pair_merge() {
    // Both cursors inside the same parens — both map to the same range → merge.
    assert_state!(
        "(-[h]>el-[l]>o)\n",
        |(buf, sels)| cmd_inner_paren(&buf, sels, MotionMode::Move),
        "(-[hello]>)\n"
    );
}

// ── around_WORD ───────────────────────────────────────────────────────────

#[test]
#[allow(non_snake_case)]
fn around_WORD_includes_trailing_space() {
    assert_state!(
        "-[h]>ello.world foo\n",
        |(buf, sels)| cmd_around_WORD(&buf, sels, MotionMode::Move),
        "-[hello.world ]>foo\n"
    );
}

#[test]
#[allow(non_snake_case)]
fn around_WORD_no_trailing_space_uses_leading() {
    // Last WORD has no trailing space — grabs leading space instead.
    assert_state!(
        "hello.world -[f]>oo\n",
        |(buf, sels)| cmd_around_WORD(&buf, sels, MotionMode::Move),
        "hello.world-[ foo]>\n"
    );
}

#[test]
#[allow(non_snake_case)]
fn around_WORD_end_of_buffer_with_leading_space_uses_WORD_boundary() {
    // B1 regression: the fallback path for "WORD at end of buffer with no
    // trailing space" was calling inner_word_impl with the wrong predicate
    // (is_word_boundary instead of is_WORD_boundary). This test catches
    // that by using a WORD that contains punctuation — `is_word_boundary`
    // would split "foo.bar" into two words while `is_WORD_boundary` keeps
    // it as one WORD, so the leading-space extent would differ.
    assert_state!(
        "  -[f]>oo.bar\n",
        |(buf, sels)| cmd_around_WORD(&buf, sels, MotionMode::Move),
        "-[  foo.bar]>\n"
    );
}

#[test]
#[allow(non_snake_case)]
fn around_WORD_cursor_on_whitespace_extends_to_next_WORD() {
    assert_state!(
        "foo-[ ]>bar\n",
        |(buf, sels)| cmd_around_WORD(&buf, sels, MotionMode::Move),
        "foo-[ bar]>\n"
    );
}

#[test]
#[allow(non_snake_case)]
fn around_WORD_multi_cursor() {
    // "hello world foo\n": cursor on 'h'(0) → "hello "(0..5); cursor on 'f'(12) → " foo"(11..14).
    assert_state!(
        "-[h]>ello world-[ ]>foo\n",
        |(buf, sels)| cmd_around_WORD(&buf, sels, MotionMode::Move),
        "-[hello ]>world-[ foo]>\n"
    );
}

#[test]
#[allow(non_snake_case)]
fn around_WORD_treats_punctuation_as_part_of_word() {
    // WORD includes adjacent punctuation; `around_word` (lower-case) would stop at '.'.
    // "foo.bar baz\n" — cursor on 'f': around_WORD selects "foo.bar " (whole WORD + space).
    // around_word would only select "foo " (stopping at '.').
    assert_state!(
        "-[f]>oo.bar baz\n",
        |(buf, sels)| cmd_around_WORD(&buf, sels, MotionMode::Move),
        "-[foo.bar ]>baz\n"
    );
}

#[test]
fn around_word_stops_at_punctuation() {
    // Contrast: around_word (lower-case) on "foo.bar baz\n", cursor on 'f'.
    // Inner word = "foo" (0..2). Next char = '.' (Punctuation, not Space) →
    // no trailing space. No leading space (cursor at col 0) → no expansion.
    // Result: just "foo".
    assert_state!(
        "-[f]>oo.bar baz\n",
        |(buf, sels)| cmd_around_word(&buf, sels, MotionMode::Move),
        "-[foo]>.bar baz\n"
    );
}

// ── around_bracket variants ───────────────────────────────────────────────

#[test]
fn around_brace_basic() {
    assert_state!(
        "{-[h]>ello}\n",
        |(buf, sels)| cmd_around_brace(&buf, sels, MotionMode::Move),
        "-[{hello}]>\n"
    );
}

#[test]
fn around_bracket_basic() {
    assert_state!(
        "[-[h]>ello]\n",
        |(buf, sels)| cmd_around_bracket(&buf, sels, MotionMode::Move),
        "-[[hello]]>\n"
    );
}

#[test]
fn around_angle_basic() {
    assert_state!(
        "<-[h]>ello>\n",
        |(buf, sels)| cmd_around_angle(&buf, sels, MotionMode::Move),
        "-[<hello>]>\n"
    );
}

// ── around_quote variants ─────────────────────────────────────────────────

#[test]
fn around_single_quote_basic() {
    assert_state!(
        "'hel-[l]>o'\n",
        |(buf, sels)| cmd_around_single_quote(&buf, sels, MotionMode::Move),
        "-['hello']>\n"
    );
}

#[test]
fn around_backtick_basic() {
    assert_state!(
        "`hel-[l]>o`\n",
        |(buf, sels)| cmd_around_backtick(&buf, sels, MotionMode::Move),
        "-[`hello`]>\n"
    );
}

// ── multi-line bracket for non-paren types ────────────────────────────────

#[test]
fn inner_brace_multiline() {
    assert_state!(
        "{\n-[h]>ello\n}\n",
        |(buf, sels)| cmd_inner_brace(&buf, sels, MotionMode::Move),
        "{-[\nhello\n]>}\n"
    );
}

// ── edge cases ────────────────────────────────────────────────────────────

#[test]
fn inner_word_on_structural_newline() {
    // Empty buffer: cursor on structural '\n'. inner_word selects the '\n'
    // (Eol class), which equals the original cursor — no visible change.
    assert_state!(
        "-[\n]>",
        |(buf, sels)| cmd_inner_word(&buf, sels, MotionMode::Move),
        "-[\n]>"
    );
}

#[test]
#[allow(non_snake_case)]
fn inner_WORD_on_structural_newline() {
    assert_state!(
        "-[\n]>",
        |(buf, sels)| cmd_inner_WORD(&buf, sels, MotionMode::Move),
        "-[\n]>"
    );
}

// ── apply_text_object_extend (union semantics) ────────────────────────────

#[test]
fn extend_inner_paren_grows_selection() {
    // "hello (world) foo\n": '('=6, ')'=12. Forward sel from 'h'(0) to 'w'(7).
    // extend_inner_paren at head=7 ('w' inside parens):
    //   inner_bracket(7) → inner = (7, 11) = "world".
    //   Union: min(0,7)=0, max(7,11)=11. head=11 ('d').
    // Serialized: ]> at position 12 (before ')') → "-[hello (world]>) foo\n".
    assert_state!(
        "-[hello (w]>orld) foo\n",
        |(buf, sels)| cmd_inner_paren(&buf, sels, MotionMode::Extend),
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
            let s1 = cmd_inner_word(&buf, sels, MotionMode::Move);  // selects "hello" (0,4)
            cmd_inner_paren(&buf, s1, MotionMode::Extend)// no parens → no-op → "hello" unchanged
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
        |(buf, sels)| cmd_around_paren(&buf, sels, MotionMode::Extend),
        "-[hello (world)]> foo\n"
    );
}

#[test]
fn extend_text_object_preserves_backward_direction() {
    // Backward selection "<[he]-llo world\n": head=0 ('h'), anchor=1 ('e').
    // extend_inner_word at head=0 → inner_word "hello" (0,4).
    // Union: sel.start()=0, sel.end()=1, word=(0,4).
    //   new_start=min(0,0)=0, new_end=max(1,4)=4, forward=false.
    // Result: Selection::directed(0,4,false) = {anchor=4, head=0}.
    // Serialized: `]-` placed at (anchor+1)=5 → "<[hello]- world\n".
    assert_state!(
        "<[he]-llo world\n",
        |(buf, sels)| cmd_inner_word(&buf, sels, MotionMode::Extend),
        "<[hello]- world\n"
    );
}

// ── apply_text_object_extend: outward growth from already-matched pair ─────

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
        |(buf, sels)| cmd_around_paren(&buf, sels, MotionMode::Extend),
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
        |(buf, sels)| cmd_inner_paren(&buf, sels, MotionMode::Extend),
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
        |(buf, sels)| cmd_around_paren(&buf, sels, MotionMode::Extend),
        "-[(a b)]>\n"
    );
}

// ── Arguments ─────────────────────────────────────────────────────────────

// ── inner_argument ────────────────────────────────────────────────────────

#[test]
fn inner_argument_first() {
    assert_state!(
        "foo(-[a]>aa, bbb, ccc)\n",
        |(buf, sels)| cmd_inner_argument(&buf, sels, MotionMode::Move),
        "foo(-[aaa]>, bbb, ccc)\n"
    );
}

#[test]
fn inner_argument_middle() {
    assert_state!(
        "foo(aaa, -[b]>bb, ccc)\n",
        |(buf, sels)| cmd_inner_argument(&buf, sels, MotionMode::Move),
        "foo(aaa, -[bbb]>, ccc)\n"
    );
}

#[test]
fn inner_argument_last() {
    assert_state!(
        "foo(aaa, bbb, -[c]>cc)\n",
        |(buf, sels)| cmd_inner_argument(&buf, sels, MotionMode::Move),
        "foo(aaa, bbb, -[ccc]>)\n"
    );
}

#[test]
fn inner_argument_single() {
    assert_state!(
        "foo(-[a]>aa)\n",
        |(buf, sels)| cmd_inner_argument(&buf, sels, MotionMode::Move),
        "foo(-[aaa]>)\n"
    );
}

#[test]
fn inner_argument_trims_whitespace() {
    // Leading/trailing spaces inside the segment are excluded.
    assert_state!(
        "foo(  -[a]>aa  , bbb)\n",
        |(buf, sels)| cmd_inner_argument(&buf, sels, MotionMode::Move),
        "foo(  -[aaa]>  , bbb)\n"
    );
}

#[test]
fn inner_argument_nested_parens_skips_inner_comma() {
    // The comma inside bar(x, y) is at depth 1 — not a segment boundary.
    assert_state!(
        "foo(-[b]>ar(x, y), z)\n",
        |(buf, sels)| cmd_inner_argument(&buf, sels, MotionMode::Move),
        "foo(-[bar(x, y)]>, z)\n"
    );
}

#[test]
fn inner_argument_nested_brackets_skips_inner_comma() {
    assert_state!(
        "foo(-[b]>ar[x, y], z)\n",
        |(buf, sels)| cmd_inner_argument(&buf, sels, MotionMode::Move),
        "foo(-[bar[x, y]]>, z)\n"
    );
}

#[test]
fn inner_argument_nested_braces_skips_inner_comma() {
    // The comma inside {a: 1, b: 2} is at depth 1 — not a segment boundary.
    // Cursor in the second argument selects "ccc", not something split by the inner comma.
    assert_state!(
        "foo({a: 1, b: 2}, cc-[c]>)\n",
        |(buf, sels)| cmd_inner_argument(&buf, sels, MotionMode::Move),
        "foo({a: 1, b: 2}, -[ccc]>)\n"
    );
}

#[test]
fn inner_argument_picks_tightest_bracket_pair() {
    // The cursor is inside (aaa, bbb) which is itself inside [...].
    // The tightest enclosing pair is (), not [].
    assert_state!(
        "[(aaa, -[b]>bb), ccc]\n",
        |(buf, sels)| cmd_inner_argument(&buf, sels, MotionMode::Move),
        "[(aaa, -[bbb]>), ccc]\n"
    );
}

#[test]
fn inner_argument_cursor_on_comma_associates_with_next() {
    // Cursor on the comma — treated as belonging to the following segment.
    assert_state!(
        "foo(aaa-[,]> bbb)\n",
        |(buf, sels)| cmd_inner_argument(&buf, sels, MotionMode::Move),
        "foo(aaa, -[bbb]>)\n"
    );
}

#[test]
fn inner_argument_cursor_on_open_bracket() {
    assert_state!(
        "foo-[(]>aaa, bbb)\n",
        |(buf, sels)| cmd_inner_argument(&buf, sels, MotionMode::Move),
        "foo(-[aaa]>, bbb)\n"
    );
}

#[test]
fn inner_argument_cursor_on_close_bracket() {
    assert_state!(
        "foo(aaa, bbb-[)]>\n",
        |(buf, sels)| cmd_inner_argument(&buf, sels, MotionMode::Move),
        "foo(aaa, -[bbb]>)\n"
    );
}

#[test]
fn inner_argument_empty_brackets_is_noop() {
    assert_state!(
        "foo-[(]>)\n",
        |(buf, sels)| cmd_inner_argument(&buf, sels, MotionMode::Move),
        "foo-[(]>)\n"
    );
}

#[test]
fn inner_argument_no_enclosing_bracket_is_noop() {
    assert_state!(
        "foo-[,]>bar\n",
        |(buf, sels)| cmd_inner_argument(&buf, sels, MotionMode::Move),
        "foo-[,]>bar\n"
    );
}

#[test]
fn inner_argument_array_items() {
    assert_state!(
        "[-[1]>11, 222, 333]\n",
        |(buf, sels)| cmd_inner_argument(&buf, sels, MotionMode::Move),
        "[-[111]>, 222, 333]\n"
    );
}

#[test]
fn inner_argument_object_fields() {
    assert_state!(
        "{-[f]>oo, a: b}\n",
        |(buf, sels)| cmd_inner_argument(&buf, sels, MotionMode::Move),
        "{-[foo]>, a: b}\n"
    );
}

#[test]
fn inner_argument_multi_cursor() {
    assert_state!(
        "foo(-[a]>aa, bbb, -[c]>cc)\n",
        |(buf, sels)| cmd_inner_argument(&buf, sels, MotionMode::Move),
        "foo(-[aaa]>, bbb, -[ccc]>)\n"
    );
}

// ── around_argument ───────────────────────────────────────────────────────

#[test]
fn around_argument_first() {
    // Deletes "aaa, " — no orphan space before bbb.
    assert_state!(
        "foo(-[a]>aa, bbb, ccc)\n",
        |(buf, sels)| cmd_around_argument(&buf, sels, MotionMode::Move),
        "foo(-[aaa, ]>bbb, ccc)\n"
    );
}

#[test]
fn around_argument_middle() {
    // Deletes ", bbb" — eats the preceding comma.
    assert_state!(
        "foo(aaa, -[b]>bb, ccc)\n",
        |(buf, sels)| cmd_around_argument(&buf, sels, MotionMode::Move),
        "foo(aaa-[, bbb]>, ccc)\n"
    );
}

#[test]
fn around_argument_last() {
    // Deletes ", ccc" — eats the preceding comma.
    assert_state!(
        "foo(aaa, bbb, -[c]>cc)\n",
        |(buf, sels)| cmd_around_argument(&buf, sels, MotionMode::Move),
        "foo(aaa, bbb-[, ccc]>)\n"
    );
}

#[test]
fn around_argument_single_equals_inner() {
    // No comma to eat — same as inner.
    assert_state!(
        "foo(-[a]>aa)\n",
        |(buf, sels)| cmd_around_argument(&buf, sels, MotionMode::Move),
        "foo(-[aaa]>)\n"
    );
}

#[test]
fn around_argument_nested() {
    // First arg is a nested call — around eats trailing ", ".
    assert_state!(
        "foo(-[b]>ar(x, y), z)\n",
        |(buf, sels)| cmd_around_argument(&buf, sels, MotionMode::Move),
        "foo(-[bar(x, y), ]>z)\n"
    );
}

// ── extend mode ───────────────────────────────────────────────────────────

#[test]
fn extend_inner_argument_basic() {
    assert_state!(
        "foo(aaa, -[b]>bb, ccc)\n",
        |(buf, sels)| cmd_inner_argument(&buf, sels, MotionMode::Extend),
        "foo(aaa, -[bbb]>, ccc)\n"
    );
}
