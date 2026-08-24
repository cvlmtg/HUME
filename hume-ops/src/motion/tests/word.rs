use super::super::*;
use hume_test_fixtures::assert_state;

// ── cmd_select_next_word (w) ──────────────────────────────────────────────

#[test]
fn select_next_word_basic() {
    // From 'h', selects "world" (the next word). Fresh anchor at word start.
    assert_state!(
        "-[h]>ello world\n",
        |(text, sels)| cmd_select_next_word(&text, sels, 1, MotionMode::Move),
        "hello -[world]>\n"
    );
}

#[test]
fn select_next_word_from_mid_word() {
    // Cursor in the middle of "hello" — still jumps to next word "world".
    assert_state!(
        "hel-[l]>o world\n",
        |(text, sels)| cmd_select_next_word(&text, sels, 1, MotionMode::Move),
        "hello -[world]>\n"
    );
}

#[test]
fn select_next_word_from_whitespace() {
    // From the space between words, selects the next word "world".
    assert_state!(
        "hello-[ ]>world\n",
        |(text, sels)| cmd_select_next_word(&text, sels, 1, MotionMode::Move),
        "hello -[world]>\n"
    );
}

#[test]
fn select_next_word_crosses_newline() {
    // w crosses the newline and selects the first word on the next line.
    assert_state!(
        "-[h]>ello\nworld\n",
        |(text, sels)| cmd_select_next_word(&text, sels, 1, MotionMode::Move),
        "hello\n-[world]>\n"
    );
}

#[test]
fn select_next_word_crosses_multiple_blank_lines() {
    // Multiple blank lines between words — w still reaches the next word.
    assert_state!(
        "-[h]>ello\n\n\nworld\n",
        |(text, sels)| cmd_select_next_word(&text, sels, 1, MotionMode::Move),
        "hello\n\n\n-[world]>\n"
    );
}

#[test]
fn select_next_word_at_last_word_is_noop() {
    // Cursor on the last word in the buffer — no-op.
    assert_state!(
        "hello -[world]>\n",
        |(text, sels)| cmd_select_next_word(&text, sels, 1, MotionMode::Move),
        "hello -[world]>\n"
    );
}

#[test]
fn select_next_word_at_eof_is_noop() {
    // Cursor on trailing '\n' — no-op.
    assert_state!(
        "hello-[\n]>",
        |(text, sels)| cmd_select_next_word(&text, sels, 1, MotionMode::Move),
        "hello-[\n]>"
    );
}

#[test]
fn select_next_word_empty_buffer_is_noop() {
    assert_state!(
        "-[\n]>",
        |(text, sels)| cmd_select_next_word(&text, sels, 1, MotionMode::Move),
        "-[\n]>"
    );
}

#[test]
fn select_next_word_word_to_punct() {
    // "hello" and "." are different word classes — w selects ".".
    assert_state!(
        "-[h]>ello.world\n",
        |(text, sels)| cmd_select_next_word(&text, sels, 1, MotionMode::Move),
        "hello-[.]>world\n"
    );
}

#[test]
fn select_next_word_punct_to_word() {
    // From ".", the next word class token is "hello".
    assert_state!(
        "-[.]>hello\n",
        |(text, sels)| cmd_select_next_word(&text, sels, 1, MotionMode::Move),
        ".-[hello]>\n"
    );
}

#[test]
fn select_next_word_count_2() {
    // count=2: skips "world", selects "foo".
    assert_state!(
        "-[h]>ello world foo\n",
        |(text, sels)| cmd_select_next_word(&text, sels, 2, MotionMode::Move),
        "hello world -[foo]>\n"
    );
}

#[test]
fn select_next_word_count_stops_at_last_word() {
    // count=3 but only 2 words remain after cursor — stops at "foo".
    assert_state!(
        "-[h]>ello world foo\n",
        |(text, sels)| cmd_select_next_word(&text, sels, 3, MotionMode::Move),
        "hello world -[foo]>\n"
    );
}

// ── cmd_select_prev_word (b) ──────────────────────────────────────────────

#[test]
fn select_prev_word_basic() {
    // From "world", selects the previous word "hello".
    assert_state!(
        "hello -[world]>\n",
        |(text, sels)| cmd_select_prev_word(&text, sels, 1, MotionMode::Move),
        "-[hello]> world\n"
    );
}

#[test]
fn select_prev_word_from_mid_word() {
    // Cursor in the middle of "world" — jumps to previous word "hello".
    assert_state!(
        "hello wor-[l]>d\n",
        |(text, sels)| cmd_select_prev_word(&text, sels, 1, MotionMode::Move),
        "-[hello]> world\n"
    );
}

#[test]
fn select_prev_word_from_whitespace() {
    // From the space between words, selects the previous word "hello".
    assert_state!(
        "hello-[ ]>world\n",
        |(text, sels)| cmd_select_prev_word(&text, sels, 1, MotionMode::Move),
        "-[hello]> world\n"
    );
}

#[test]
fn select_prev_word_from_punct() {
    // Cursor on the '.' punctuation — selects the preceding word "hello".
    assert_state!(
        "hello-[.]>world\n",
        |(text, sels)| cmd_select_prev_word(&text, sels, 1, MotionMode::Move),
        "-[hello]>.world\n"
    );
}

#[test]
fn select_prev_word_from_trailing_newline() {
    // Cursor on the trailing '\n' — selects the last word on the line.
    assert_state!(
        "hello world-[\n]>",
        |(text, sels)| cmd_select_prev_word(&text, sels, 1, MotionMode::Move),
        "hello -[world]>\n"
    );
}

#[test]
fn select_prev_word_crosses_newline() {
    // b crosses the newline and selects the last word on the previous line.
    assert_state!(
        "hello\n-[world]>\n",
        |(text, sels)| cmd_select_prev_word(&text, sels, 1, MotionMode::Move),
        "-[hello]>\nworld\n"
    );
}

#[test]
fn select_prev_word_at_first_word_is_noop() {
    // Cursor on first word — no-op.
    assert_state!(
        "-[hello]> world\n",
        |(text, sels)| cmd_select_prev_word(&text, sels, 1, MotionMode::Move),
        "-[hello]> world\n"
    );
}

#[test]
fn select_prev_word_in_first_word_mid_is_noop() {
    // Cursor in the middle of the first word — no previous word, no-op.
    assert_state!(
        "hel-[l]>o world\n",
        |(text, sels)| cmd_select_prev_word(&text, sels, 1, MotionMode::Move),
        "hel-[l]>o world\n"
    );
}

#[test]
fn select_prev_word_at_buffer_start_is_noop() {
    assert_state!(
        "-[h]>ello\n",
        |(text, sels)| cmd_select_prev_word(&text, sels, 1, MotionMode::Move),
        "-[h]>ello\n"
    );
}

#[test]
fn select_prev_word_empty_buffer_is_noop() {
    assert_state!(
        "-[\n]>",
        |(text, sels)| cmd_select_prev_word(&text, sels, 1, MotionMode::Move),
        "-[\n]>"
    );
}

#[test]
fn select_prev_word_count_2() {
    // count=2: from "foo", skips "world", selects "hello".
    assert_state!(
        "hello world -[foo]>\n",
        |(text, sels)| cmd_select_prev_word(&text, sels, 2, MotionMode::Move),
        "-[hello]> world foo\n"
    );
}

#[test]
fn select_prev_word_count_overshoots() {
    // count=5 but only 2 words precede "foo" — stops at "hello" rather than erroring.
    assert_state!(
        "hello world -[foo]>\n",
        |(text, sels)| cmd_select_prev_word(&text, sels, 5, MotionMode::Move),
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
        |(text, sels)| cmd_select_next_uppercase_word(&text, sels, 1, MotionMode::Move),
        "hello.world -[bar]>\n"
    );
}

#[test]
#[allow(non_snake_case)]
fn select_next_uppercase_word_crosses_newline() {
    // W at end of a line crosses the newline and selects the first WORD on the next line.
    assert_state!(
        "-[h]>ello.world\nbar\n",
        |(text, sels)| cmd_select_next_uppercase_word(&text, sels, 1, MotionMode::Move),
        "hello.world\n-[bar]>\n"
    );
}

#[test]
fn select_next_word_stops_at_punct() {
    // w (lowercase): "hello" and "." are separate word-class tokens.
    assert_state!(
        "-[h]>ello.world bar\n",
        |(text, sels)| cmd_select_next_word(&text, sels, 1, MotionMode::Move),
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
        |(text, sels)| cmd_select_prev_uppercase_word(&text, sels, 1, MotionMode::Move),
        "-[hello.world]> bar\n"
    );
}

#[test]
#[allow(non_snake_case)]
fn select_prev_uppercase_word_crosses_newline() {
    // B at the start of a line crosses the newline and selects the last WORD on the previous line.
    assert_state!(
        "hello.world\n-[bar]>\n",
        |(text, sels)| cmd_select_prev_uppercase_word(&text, sels, 1, MotionMode::Move),
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
        |(text, sels)| cmd_select_next_word(&text, sels, 1, MotionMode::Move),
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
        |(text, sels)| cmd_select_prev_word(&text, sels, 1, MotionMode::Move),
        "-[cafe\u{0301}]> world\n"
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
        |(text, sels)| cmd_select_next_word(&text, sels, 1, MotionMode::Move),
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
        |(text, sels)| cmd_select_prev_word(&text, sels, 1, MotionMode::Move),
        "-[foo]> -[hello]> world bar\n"
    );
}

// ── around-word variants (w/W/b/B covering their whitespace bookend) ──────
//
// These wrap the same select_next_word/select_prev_word motions used above,
// so movement is identical; only the final span differs. Each covers the
// destination word's whitespace bookend: leading preferred, trailing
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
        |(text, sels)| cmd_select_next_word_around(&text, sels, 1, MotionMode::Move),
        "foo-[ bar]>   baz\n"
    );
}

#[test]
fn select_next_word_around_leading_tab() {
    // Tab classifies as Space — counts as leading whitespace too.
    assert_state!(
        "-[f]>oo\tbar baz\n",
        |(text, sels)| cmd_select_next_word_around(&text, sels, 1, MotionMode::Move),
        "foo-[\tbar]> baz\n"
    );
}

#[test]
fn select_next_word_around_leading_nbsp() {
    // U+00A0 (NBSP) classifies as Space too.
    assert_state!(
        "-[f]>oo\u{00A0}bar baz\n",
        |(text, sels)| cmd_select_next_word_around(&text, sels, 1, MotionMode::Move),
        "foo-[\u{00A0}bar]> baz\n"
    );
}

#[test]
fn select_next_word_around_leading_mid_line_before_eol() {
    // "bar" isn't the first word on its line (that's "foo"), so it takes its
    // leading space even though it's also the last word before EOL.
    assert_state!(
        "-[f]>oo bar\n",
        |(text, sels)| cmd_select_next_word_around(&text, sels, 1, MotionMode::Move),
        "foo-[ bar]>\n"
    );
}

#[test]
fn select_next_word_around_leading_mid_line_before_punctuation() {
    // Same rule applies regardless of what follows the word.
    assert_state!(
        "-[f]>oo bar,baz\n",
        |(text, sels)| cmd_select_next_word_around(&text, sels, 1, MotionMode::Move),
        "foo-[ bar]>,baz\n"
    );
}

#[test]
fn select_next_word_around_punctuation_destination_gets_leading_space() {
    // w can land on a punctuation run just like a word — it gets the same
    // around treatment.
    assert_state!(
        "-[f]>oo , bar\n",
        |(text, sels)| cmd_select_next_word_around(&text, sels, 1, MotionMode::Move),
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
        |(text, sels)| cmd_select_next_word_around(&text, sels, 1, MotionMode::Move),
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
        |(text, sels)| cmd_select_next_word_around(&text, sels, 1, MotionMode::Move),
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
        |(text, sels)| cmd_select_next_word_around(&text, sels, 1, MotionMode::Move),
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
        |(text, sels)| cmd_select_next_word_around(&text, sels, 1, MotionMode::Move),
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
        |(text, sels)| cmd_select_next_word_around(&text, sels, 2, MotionMode::Move),
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
        |(text, sels)| {
            let s1 = cmd_select_next_word_around(&text, sels, 1, MotionMode::Move); // " two"
            let s2 = cmd_select_next_word_around(&text, s1, 1, MotionMode::Move); // " three"
            cmd_select_next_word_around(&text, s2, 1, MotionMode::Move) // " four", not " three" again
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
        |(text, sels)| cmd_select_next_word_around(&text, sels, 1, MotionMode::Move),
        "foo-[ bar]>-[ baz]>\n"
    );
}

#[test]
fn select_next_word_around_skips_combining_grapheme() {
    // Text: "cafe\u{0301} world\n" — the combining acute must not be
    // misread as a word-class char when scanning for the leading space.
    assert_state!(
        "-[c]>afe\u{0301} world\n",
        |(text, sels)| cmd_select_next_word_around(&text, sels, 1, MotionMode::Move),
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
        |(text, sels)| cmd_select_next_uppercase_word_around(&text, sels, 1, MotionMode::Move),
        "foo,-[ bar]>\n"
    );
}

#[test]
fn select_prev_word_around_first_word_of_buffer_takes_trailing() {
    // "hello" is the first word of the buffer — no leading run is possible —
    // so it falls back to its trailing space (the one before "world").
    assert_state!(
        "hello -[world]>\n",
        |(text, sels)| cmd_select_prev_word_around(&text, sels, 1, MotionMode::Move),
        "-[hello ]>world\n"
    );
}

#[test]
fn select_prev_word_around_leading_mid_line() {
    // Plain word-to-word case: b lands on "bar", which isn't the first word
    // on its line, so it takes its leading space.
    assert_state!(
        "foo bar -[b]>az\n",
        |(text, sels)| cmd_select_prev_word_around(&text, sels, 1, MotionMode::Move),
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
        |(text, sels)| cmd_select_prev_word_around(&text, sels, 1, MotionMode::Move),
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
        |(text, sels)| cmd_select_prev_uppercase_word_around(&text, sels, 1, MotionMode::Move),
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
        |(text, sels)| {
            let s1 = cmd_select_prev_word_around(&text, sels, 1, MotionMode::Move); // " three"
            let s2 = cmd_select_prev_word_around(&text, s1, 1, MotionMode::Move); // " two"
            cmd_select_prev_word_around(&text, s2, 1, MotionMode::Move) // "one ", not " two" again
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
        |(text, sels)| {
            let s1 = cmd_select_prev_uppercase_word_around(&text, sels, 1, MotionMode::Move); // " three.x"
            let s2 = cmd_select_prev_uppercase_word_around(&text, s1, 1, MotionMode::Move); // " two"
            cmd_select_prev_uppercase_word_around(&text, s2, 1, MotionMode::Move) // "one ", not " two" again
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
        |(text, sels)| {
            let s1 = cmd_select_next_word_around(&text, sels, 1, MotionMode::Move); // " two"
            cmd_select_prev_word_around(&text, s1, 1, MotionMode::Move) // "one ", back to start
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
        |(text, sels)| {
            let s1 = cmd_select_prev_word_around(&text, sels, 1, MotionMode::Move); // "one "
            cmd_select_next_word_around(&text, s1, 1, MotionMode::Move) // " two", not stuck on "one"
        },
        "one-[ two]> three four\n"
    );
}

#[test]
fn select_prev_word_around_at_buffer_start_is_noop() {
    // Guard: no previous word exists — no-op, no expansion attempted.
    assert_state!(
        "-[h]>ello\n",
        |(text, sels)| cmd_select_prev_word_around(&text, sels, 1, MotionMode::Move),
        "-[h]>ello\n"
    );
}

#[test]
fn extend_select_next_word_around_grows_with_anchor_unit() {
    // Extend mode honors word-selects-whitespace: the anchor's unit ("bar",
    // not first on its line, takes its leading space) is kept whole as the
    // selection grows forward to the target word's own end — no trailing
    // whitespace is pulled in.
    assert_state!(
        "foo -[b]>ar baz\n",
        |(text, sels)| cmd_select_next_word_around(&text, sels, 1, MotionMode::Extend),
        "foo-[ bar baz]>\n"
    );
}

#[test]
fn extend_select_prev_word_around_grows_backward_onto_leading_whitespace() {
    // Growing backward, the target word's own leading space is absorbed
    // into `head` — the selection can legitimately start on whitespace.
    assert_state!(
        "foo bar -[b]>az\n",
        |(text, sels)| cmd_select_prev_word_around(&text, sels, 1, MotionMode::Extend),
        "foo<[ bar baz]-\n"
    );
}

#[test]
fn extend_select_prev_word_around_shrinks_to_anchor_unit() {
    // The target ("bar") is the anchor's own word — collapses to the
    // anchor's unit (" bar"), not further.
    assert_state!(
        "foo-[ bar baz]>\n",
        |(text, sels)| cmd_select_prev_word_around(&text, sels, 1, MotionMode::Extend),
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
        |(text, sels)| {
            let s1 = cmd_select_next_word_around(&text, sels, 1, MotionMode::Extend);
            let s2 = cmd_select_prev_word_around(&text, s1, 1, MotionMode::Extend);
            let s3 = cmd_select_prev_word_around(&text, s2, 1, MotionMode::Extend);
            cmd_select_next_word_around(&text, s3, 1, MotionMode::Extend)
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
        |(text, sels)| cmd_select_prev_word_around(&text, sels, 1, MotionMode::Extend),
        "  <[one two]-\n"
    );
}

#[test]
fn extend_select_next_word_around_whitespace_anchor_without_adjacent_word() {
    // The anchor sits on indentation at the very start of the buffer — no
    // word is adjacent to that run, so word_unit_at returns None and the
    // anchor unit falls back to the bare whitespace position (must not
    // panic). The extend still reaches "foo" on the next line.
    assert_state!(
        "-[ ]> \nfoo\n",
        |(text, sels)| cmd_select_next_word_around(&text, sels, 1, MotionMode::Extend),
        "-[  \nfoo]>\n"
    );
}

#[test]
fn extend_select_next_word_around_chained_grows_past_two_words() {
    // Two separate extend-w presses grow the selection past "two" onto
    // "three", re-resolving the (unchanged) anchor unit each time.
    assert_state!(
        "-[o]>ne two three\n",
        |(text, sels)| {
            let s1 = cmd_select_next_word_around(&text, sels, 1, MotionMode::Extend); // "-[one two]>"
            cmd_select_next_word_around(&text, s1, 1, MotionMode::Extend)
        },
        "-[one two three]>\n"
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
        |(text, sels)| cmd_select_next_word(&text, sels, 1, MotionMode::Extend),
        "-[hello world]> foo\n"
    );
}

#[test]
fn extend_select_next_word_grows_selection() {
    // Start with "world" selected via `w` (anchor=6,head=10); extend-w finds
    // "foo" (12,14), beyond the anchor's own word "world", so it grows.
    assert_state!(
        "-[h]>ello world foo\n",
        |(text, sels)| {
            let s1 = cmd_select_next_word(&text, sels, 1, MotionMode::Move); // selects "world" (6,10)
            cmd_select_next_word(&text, s1, 1, MotionMode::Extend) // grows to "world foo"
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
        |(text, sels)| {
            let s1 = cmd_select_next_word(&text, sels, 1, MotionMode::Move); // selects "world" (6,10)
            cmd_select_prev_word(&text, s1, 1, MotionMode::Extend) // grows backward to "hello world"
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
        |(text, sels)| cmd_select_prev_word(&text, sels, 1, MotionMode::Extend),
        "foo -[bar]> baz\n"
    );
}

#[test]
fn extend_select_next_word_at_buffer_end_is_noop() {
    // From a selection covering the only word in the buffer, extend-w finds
    // no next word (only '\n' remains) and leaves the selection unchanged.
    assert_state!(
        "-[hello]>\n",
        |(text, sels)| cmd_select_next_word(&text, sels, 1, MotionMode::Extend),
        "-[hello]>\n"
    );
}

#[test]
fn extend_select_prev_word_at_buffer_start_is_noop() {
    // The selection starts at pos 0; there is no previous word. Noop.
    assert_state!(
        "-[hello]> world\n",
        |(text, sels)| cmd_select_prev_word(&text, sels, 1, MotionMode::Extend),
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
        |(text, sels)| cmd_select_next_word(&text, sels, 1, MotionMode::Extend),
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
        |(text, sels)| cmd_select_next_word(&text, sels, 1, MotionMode::Extend),
        "a -[b c]>\n"
    );
}

#[test]
fn word_shrink_scenario_step2_shrinks_to_anchor_word() {
    // select_prev_word from head=4 (inside "c") lands back on "b" — the
    // anchor's own word — so the selection shrinks rather than growing past it.
    assert_state!(
        "a -[b c]>\n",
        |(text, sels)| cmd_select_prev_word(&text, sels, 1, MotionMode::Extend),
        "a -[b]> c\n"
    );
}

#[test]
fn word_shrink_scenario_step3_crosses_anchor_flips_backward() {
    // select_prev_word from head=2 (inside "b") lands on "a", behind the
    // anchor's word — the selection grows backward, flipping direction.
    assert_state!(
        "a -[b]> c\n",
        |(text, sels)| cmd_select_prev_word(&text, sels, 1, MotionMode::Extend),
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
        |(text, sels)| cmd_select_next_word(&text, sels, 1, MotionMode::Extend),
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
        |(text, sels)| cmd_select_next_word(&text, sels, 1, MotionMode::Extend),
        "aaa -[bbb ccc]>\n"
    );
}

#[test]
fn word_no_truncation_shrinks_to_unit() {
    // Shrinks from "bbb ccc" back to just "bbb" — "ccc" is dropped whole, not
    // trimmed to a single char.
    assert_state!(
        "aaa -[bbb ccc]>\n",
        |(text, sels)| cmd_select_prev_word(&text, sels, 1, MotionMode::Extend),
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
        |(text, sels)| cmd_select_prev_word(&text, sels, 1, MotionMode::Extend),
        "<[aaa bbb]- ccc\n"
    );
}

#[test]
fn word_no_truncation_shrink_back_after_cross() {
    assert_state!(
        "<[aaa bbb]- ccc\n",
        |(text, sels)| cmd_select_next_word(&text, sels, 1, MotionMode::Extend),
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
        |(text, sels)| cmd_select_next_word(&text, sels, 1, MotionMode::Extend),
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
        |(text, sels)| cmd_select_prev_word(&text, sels, 1, MotionMode::Extend),
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
        |(text, sels)| cmd_select_next_uppercase_word(&text, sels, 1, MotionMode::Extend),
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
        |(text, sels)| cmd_select_next_word(&text, sels, 2, MotionMode::Extend),
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
        |(text, sels)| cmd_select_next_word(&text, sels, 2, MotionMode::Extend),
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
        |(text, sels)| cmd_select_next_word(&text, sels, 1, MotionMode::Extend),
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
        |(text, sels)| cmd_select_prev_word(&text, sels, 1, MotionMode::Extend),
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
        |(text, sels)| cmd_select_next_word(&text, sels, 1, MotionMode::Extend),
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
        |(text, sels)| cmd_select_prev_word(&text, sels, 1, MotionMode::Extend),
        "<[foo bar baz]-\n"
    );
}
