use super::super::*;
use hume_editing::selection::{DisplayColOrigin, Selection, SelectionSet, StickyDisplayCol};
use hume_test_fixtures::assert_state;

/// Test-only shorthand: these tests exercise the word-snap pass-through, not
/// `DisplayColOrigin` itself, so every latch below is `BufferLine`
/// arbitrarily.
fn sticky(display_col: u32) -> StickyDisplayCol {
    StickyDisplayCol {
        display_col,
        origin: DisplayColOrigin::BufferLine,
        wrap_width: None,
    }
}

// ── Word ──────────────────────────────────────────────────────────────────

#[test]
fn inner_word_middle() {
    // head=o (last char of `hello`).
    assert_state!(
        "-[h]>ello world\n",
        |(text, sels)| cmd_inner_word(&text, sels, 0, MotionMode::Move),
        "-[hello]> world\n"
    );
}

#[test]
fn inner_word_cursor_at_end_of_word() {
    assert_state!(
        "hell-[o]> world\n",
        |(text, sels)| cmd_inner_word(&text, sels, 0, MotionMode::Move),
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
        |(text, sels)| cmd_inner_word(&text, sels, 0, MotionMode::Move),
        "foo-[  ]>bar\n"
    );
}

#[test]
fn inner_word_cursor_on_punctuation() {
    // Both `!!` are Punctuation — selected as one run.
    assert_state!(
        "foo-[!]>!\n",
        |(text, sels)| cmd_inner_word(&text, sels, 0, MotionMode::Move),
        "foo-[!!]>\n"
    );
}

#[test]
fn around_word_first_word_of_buffer_takes_trailing() {
    // "hello" is the first word of the buffer — no leading run is possible —
    // so it falls back to its trailing space; head = the space char.
    assert_state!(
        "-[h]>ello world\n",
        |(text, sels)| cmd_around_word(&text, sels, 0, MotionMode::Move),
        "-[hello ]>world\n"
    );
}

#[test]
fn around_word_leading_preferred() {
    // "world" isn't the first word on its line, so it takes its leading
    // space, regardless of what follows it (here, EOL).
    assert_state!(
        "hello -[w]>orld\n",
        |(text, sels)| cmd_around_word(&text, sels, 0, MotionMode::Move),
        "hello-[ world]>\n"
    );
}

#[test]
fn around_word_mid_line_takes_leading() {
    // "world" isn't the first word on its line — takes its leading space
    // even though a trailing space exists too.
    assert_state!(
        "hello -[w]>orld baz\n",
        |(text, sels)| cmd_around_word(&text, sels, 0, MotionMode::Move),
        "hello-[ world]> baz\n"
    );
}

#[test]
fn inner_word_includes_combining_grapheme() {
    // Text: "cafe\u{0301} world\n"
    // char offsets: c(0) a(1) f(2) e(3) ◌́(4) ' '(5) w(6) ...
    // Grapheme clusters: {c}{a}{f}{e◌́}{ }{w}...
    //
    // Stepping by grapheme boundary (not codepoint) matters here: the
    // combining codepoint at offset 4 is classified as Punctuation, which
    // would be a false word/punct boundary inside the grapheme if stepped
    // one codepoint at a time. Stepping by grapheme boundary instead: the
    // next cluster after offset 3 starts at offset 5 (space), so the word
    // ends at offset 4 (last codepoint of the {e◌́} grapheme) — the full
    // cluster is included.
    assert_state!(
        "-[c]>afe\u{0301} world\n",
        |(text, sels)| cmd_inner_word(&text, sels, 0, MotionMode::Move),
        "-[cafe\u{0301}]> world\n"
    );
}

// ── WORD ──────────────────────────────────────────────────────────────────

#[test]
#[allow(non_snake_case)]
fn inner_uppercase_word_spans_punctuation() {
    // `hello.world` is one WORD (no whitespace boundary within it).
    assert_state!(
        "-[h]>ello.world foo\n",
        |(text, sels)| cmd_inner_uppercase_word(&text, sels, 0, MotionMode::Move),
        "-[hello.world]> foo\n"
    );
}

// ── select-word / select-uppercase-word (mm/MM around-word body) ──────────
//
// mm/MM (`cmd_select_word_around`/`cmd_select_uppercase_word_around`) and
// maw/maW (`cmd_around_word`/`cmd_around_uppercase_word`) share the same
// word_unit_at body and select identical spans — leading-preferred, trailing
// fallback for the first word of a line, same as w/W/b/B. `mm`/`MM` only
// exist as a separate name because they stay gated behind
// word-selects-whitespace (see mm/MM in keymap/defaults.rs) — maw/maW are
// always available regardless of the setting. Extend keeps bare inner-word
// units, matching cmd_inner_word's Extend arm exactly.

#[test]
fn select_word_around_move_first_word_of_buffer_takes_trailing() {
    // "hello" is the first word of the buffer — no leading run is possible —
    // so it falls back to its trailing space.
    assert_state!(
        "-[h]>ello world\n",
        |(text, sels)| cmd_select_word_around(&text, sels, 0, MotionMode::Move),
        "-[hello ]>world\n"
    );
}

#[test]
fn select_word_around_move_leading_preferred() {
    // "world" isn't the first word on its line, so it takes its leading
    // space regardless of what follows it (here, EOL).
    assert_state!(
        "hello -[w]>orld\n",
        |(text, sels)| cmd_select_word_around(&text, sels, 0, MotionMode::Move),
        "hello-[ world]>\n"
    );
}

#[test]
fn select_word_around_move_matches_around_word() {
    // mm and maw select the identical span mid-line.
    assert_state!(
        "foo -[b]>ar baz\n",
        |(text, sels)| cmd_select_word_around(&text, sels, 0, MotionMode::Move),
        "foo-[ bar]> baz\n"
    );
    assert_state!(
        "foo -[b]>ar baz\n",
        |(text, sels)| cmd_around_word(&text, sels, 0, MotionMode::Move),
        "foo-[ bar]> baz\n"
    );
}

#[test]
fn select_word_around_move_indented_first_word_keeps_indent() {
    // "foo" is the first word on its (indented) line — the leading run is
    // indentation and is never absorbed; the trailing space is used instead.
    assert_state!(
        "x\n  -[f]>oo bar\n",
        |(text, sels)| cmd_select_word_around(&text, sels, 0, MotionMode::Move),
        "x\n  -[foo ]>bar\n"
    );
}

#[test]
fn select_word_around_extend_honors_the_setting() {
    // Extend arm uses word_unit_at, same as Move — "hello" is the first word
    // of the buffer, so its unit includes the trailing space, and the union
    // grows to cover it too (unlike cmd_inner_word's Extend arm, which stays
    // bare — compare extend_text_object_preserves_backward_direction above).
    assert_state!(
        "<[he]-llo world\n",
        |(text, sels)| cmd_select_word_around(&text, sels, 0, MotionMode::Extend),
        "<[hello ]-world\n"
    );
}

// ── mm/maw with the cursor on whitespace ──────────────────────────────────
//
// There is no word under the cursor: snap to the adjacent word (following
// preferred, preceding fallback) and apply the normal unit rule to it. The
// whitespace under the cursor is never selected for its own sake — an
// inter-word space reappears as the following word's leading run, but
// newlines and indentation never enter the span.

#[test]
fn around_word_on_interior_newline_selects_next_word_without_eol() {
    // Cursor on the newline ending "hello": snap forward to "world", which
    // is the first word of its line with EOL after it — bare. The newline
    // itself is never part of the span.
    assert_state!(
        "hello-[\n]>world\n",
        |(text, sels)| cmd_around_word(&text, sels, 0, MotionMode::Move),
        "hello\n-[world]>\n"
    );
}

#[test]
fn around_word_on_trailing_structural_newline_snaps_backward() {
    // Cursor on the buffer's structural '\n': nothing follows, so snap back
    // to "hello" — first word of its line, no trailing space → bare.
    assert_state!(
        "hello-[\n]>",
        |(text, sels)| cmd_around_word(&text, sels, 0, MotionMode::Move),
        "-[hello]>\n"
    );
}

#[test]
fn around_word_on_blank_line_snaps_forward_past_the_newline_run() {
    // Cursor on a blank line: the whole newline run is the whitespace under
    // the cursor; the adjacent word after it is "world" — selected bare,
    // with none of the newlines.
    assert_state!(
        "hello\n-[\n]>world\n",
        |(text, sels)| cmd_around_word(&text, sels, 0, MotionMode::Move),
        "hello\n\n-[world]>\n"
    );
}

#[test]
fn around_word_on_indentation_excludes_the_indent() {
    // Cursor on the indentation: snap forward to "foo", whose unit takes
    // the trailing space (first word of the line) — the indent is excluded,
    // same as pressing maw on "foo" itself.
    assert_state!(
        "-[ ]> foo bar\n",
        |(text, sels)| cmd_around_word(&text, sels, 0, MotionMode::Move),
        "  -[foo ]>bar\n"
    );
}

#[test]
fn around_word_on_whitespace_only_buffer_is_noop() {
    // No word adjacent to the run in either direction — no-op.
    assert_state!(
        "-[ ]>  \n",
        |(text, sels)| cmd_around_word(&text, sels, 0, MotionMode::Move),
        "-[ ]>  \n"
    );
}

#[test]
fn select_word_around_extend_at_buffer_end_never_consumes_trailing_newline() {
    // Regression: the extend retry from past the selection end lands on the
    // structural '\n'; its unit must resolve to the preceding word (already
    // covered), not grow the selection onto the newline.
    assert_state!(
        "-[hello world]>\n",
        |(text, sels)| cmd_select_word_around(&text, sels, 0, MotionMode::Extend),
        "-[hello world]>\n"
    );
}

#[test]
#[allow(non_snake_case)]
fn select_uppercase_word_around_move_spans_punctuation_and_whitespace() {
    // "hello.world" is one WORD and the first word of the buffer, so it
    // falls back to its trailing space.
    assert_state!(
        "-[h]>ello.world foo\n",
        |(text, sels)| cmd_select_uppercase_word_around(&text, sels, 0, MotionMode::Move),
        "-[hello.world ]>foo\n"
    );
}

#[test]
fn inner_word_multi_cursor_different_words() {
    assert_state!(
        "-[h]>ello -[w]>orld\n",
        |(text, sels)| cmd_inner_word(&text, sels, 0, MotionMode::Move),
        "-[hello]> -[world]>\n"
    );
}

#[test]
fn inner_word_multi_cursor_same_word_merges() {
    // Two cursors in the same word — both select "hello", merge to one selection.
    assert_state!(
        "-[h]>el-[l]>o world\n",
        |(text, sels)| cmd_inner_word(&text, sels, 0, MotionMode::Move),
        "-[hello]> world\n"
    );
}

#[test]
fn around_word_multi_cursor() {
    // "hello world foo\n": cursor 0 on 'h'(0) → "hello "(0..5); cursor 1 on 'f'(12) → " foo"(11..14).
    assert_state!(
        "-[h]>ello world-[ ]>foo\n",
        |(text, sels)| cmd_around_word(&text, sels, 0, MotionMode::Move),
        "-[hello ]>world-[ foo]>\n"
    );
}

#[test]
#[allow(non_snake_case)]
fn inner_uppercase_word_multi_cursor() {
    assert_state!(
        "-[h]>ello.world -[f]>oo\n",
        |(text, sels)| cmd_inner_uppercase_word(&text, sels, 0, MotionMode::Move),
        "-[hello.world]> -[foo]>\n"
    );
}

// ── around_WORD ───────────────────────────────────────────────────────────

#[test]
#[allow(non_snake_case)]
fn around_uppercase_word_includes_trailing_space() {
    assert_state!(
        "-[h]>ello.world foo\n",
        |(text, sels)| cmd_around_uppercase_word(&text, sels, 0, MotionMode::Move),
        "-[hello.world ]>foo\n"
    );
}

#[test]
#[allow(non_snake_case)]
fn around_uppercase_word_no_trailing_space_uses_leading() {
    // Last WORD has no trailing space — grabs leading space instead.
    assert_state!(
        "hello.world -[f]>oo\n",
        |(text, sels)| cmd_around_uppercase_word(&text, sels, 0, MotionMode::Move),
        "hello.world-[ foo]>\n"
    );
}

#[test]
#[allow(non_snake_case)]
fn around_uppercase_word_first_word_of_line_uses_uppercase_word_boundary() {
    // Regression: word_unit_at must call inner_word_impl with the right
    // predicate (is_uppercase_word_boundary, not is_word_boundary). This
    // test catches that by using a WORD that contains punctuation —
    // `is_word_boundary` would split "foo.bar" into two words while
    // `is_uppercase_word_boundary` keeps it as one WORD, so the resulting
    // span would differ: "foo.bar" is the first (and only) WORD of the
    // buffer — its leading run is indentation, never absorbed, and there's
    // no trailing space (EOL follows), so the correct result is bare
    // "foo.bar", not the wrong predicate's bare "foo".
    assert_state!(
        "  -[f]>oo.bar\n",
        |(text, sels)| cmd_around_uppercase_word(&text, sels, 0, MotionMode::Move),
        "  -[foo.bar]>\n"
    );
}

#[test]
#[allow(non_snake_case)]
fn around_uppercase_word_cursor_on_whitespace_extends_to_next_uppercase_word() {
    assert_state!(
        "foo-[ ]>bar\n",
        |(text, sels)| cmd_around_uppercase_word(&text, sels, 0, MotionMode::Move),
        "foo-[ bar]>\n"
    );
}

#[test]
#[allow(non_snake_case)]
fn around_uppercase_word_multi_cursor() {
    // "hello world foo\n": cursor on 'h'(0) → "hello "(0..5); cursor on 'f'(12) → " foo"(11..14).
    assert_state!(
        "-[h]>ello world-[ ]>foo\n",
        |(text, sels)| cmd_around_uppercase_word(&text, sels, 0, MotionMode::Move),
        "-[hello ]>world-[ foo]>\n"
    );
}

#[test]
#[allow(non_snake_case)]
fn around_uppercase_word_treats_punctuation_as_part_of_word() {
    // WORD includes adjacent punctuation; `around_word` (lower-case) would stop at '.'.
    // "foo.bar baz\n" — cursor on 'f': around_WORD selects "foo.bar " (whole WORD + space).
    // around_word would only select "foo " (stopping at '.').
    assert_state!(
        "-[f]>oo.bar baz\n",
        |(text, sels)| cmd_around_uppercase_word(&text, sels, 0, MotionMode::Move),
        "-[foo.bar ]>baz\n"
    );
}

#[test]
fn around_word_stops_at_punctuation() {
    // Contrast: around_word (lower-case) on "foo.bar baz\n", cursor on 'f'.
    // Inner word = "foo" (0..2), the first word of the buffer — no leading
    // run is possible. Next char = '.' (Punctuation, not Space) → no
    // trailing space either → no expansion. Result: just "foo".
    assert_state!(
        "-[f]>oo.bar baz\n",
        |(text, sels)| cmd_around_word(&text, sels, 0, MotionMode::Move),
        "-[foo]>.bar baz\n"
    );
}

// ── edge cases ────────────────────────────────────────────────────────────

#[test]
fn inner_word_on_structural_newline() {
    // Empty buffer: cursor on structural '\n'. inner_word selects the '\n'
    // (Eol class), which equals the original cursor — no visible change.
    assert_state!(
        "-[\n]>",
        |(text, sels)| cmd_inner_word(&text, sels, 0, MotionMode::Move),
        "-[\n]>"
    );
}

#[test]
#[allow(non_snake_case)]
fn inner_uppercase_word_on_structural_newline() {
    assert_state!(
        "-[\n]>",
        |(text, sels)| cmd_inner_uppercase_word(&text, sels, 0, MotionMode::Move),
        "-[\n]>"
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
        |(text, sels)| cmd_inner_word(&text, sels, 0, MotionMode::Extend),
        "<[hello]- world\n"
    );
}

// ── select-word-nearest-on-line ────────────────────────────────────────────
//
// This block passes `around = false` throughout, isolating the nearest-word
// scan logic from the whitespace-bookend expansion. See the `_around_word`
// block below for `around = true` (word-selects-whitespace on), which reuses
// the same scan but expands the winning anchor via `word_unit_at`.

#[test]
fn nearest_on_word_selects_inner_word() {
    // Head lands mid-word — same as inner-word.
    assert_state!(
        "hello wor-[l]>d foo\n",
        |(text, sels)| cmd_select_word_nearest_on_line(&text, sels, 0, MotionMode::Move, false),
        "hello -[world]> foo\n"
    );
}

#[test]
fn nearest_on_whitespace_prev_closer() {
    // "foo   bar" — head on first space (index 3); dist to "foo" end (2) = 1,
    // dist to "bar" start (6) = 3. Prev is closer → select "foo".
    assert_state!(
        "foo-[ ]>  bar\n",
        |(text, sels)| cmd_select_word_nearest_on_line(&text, sels, 0, MotionMode::Move, false),
        "-[foo]>   bar\n"
    );
}

#[test]
fn nearest_on_whitespace_next_closer() {
    // "foo   bar" — head on last space (index 5); dist to "foo" end (2) = 3,
    // dist to "bar" start (6) = 1. Next is closer → select "bar".
    assert_state!(
        "foo  -[ ]>bar\n",
        |(text, sels)| cmd_select_word_nearest_on_line(&text, sels, 0, MotionMode::Move, false),
        "foo   -[bar]>\n"
    );
}

#[test]
fn nearest_on_whitespace_tie_picks_prev() {
    // "foo   bar" — head on middle space (index 4); dist to "foo" end (2) = 2,
    // dist to "bar" start (6) = 2. Exact tie → prev → select "foo".
    assert_state!(
        "foo -[ ]> bar\n",
        |(text, sels)| cmd_select_word_nearest_on_line(&text, sels, 0, MotionMode::Move, false),
        "-[foo]>   bar\n"
    );
}

#[test]
fn nearest_at_line_start_whitespace_no_cross_to_prev_line() {
    // Cursor is on the leading space of line 1. Prev word ("end") is on line 0 —
    // must NOT be selected. Next word ("start") on the same line is selected.
    assert_state!(
        "end\n-[ ]>start\n",
        |(text, sels)| cmd_select_word_nearest_on_line(&text, sels, 0, MotionMode::Move, false),
        "end\n -[start]>\n"
    );
}

#[test]
fn nearest_at_line_end_whitespace_no_cross_to_next_line() {
    // Cursor is on trailing space before the newline on line 0. Next word
    // ("next") is on line 1 — must NOT be selected. Prev word ("end") is
    // selected.
    assert_state!(
        "end -[ ]>\nnext\n",
        |(text, sels)| cmd_select_word_nearest_on_line(&text, sels, 0, MotionMode::Move, false),
        "-[end]>  \nnext\n"
    );
}

#[test]
fn nearest_on_blank_line_is_noop() {
    // A line with only a newline has no words — selection unchanged.
    assert_state!(
        "hello\n-[\n]>world\n",
        |(text, sels)| cmd_select_word_nearest_on_line(&text, sels, 0, MotionMode::Move, false),
        "hello\n-[\n]>world\n"
    );
}

#[test]
fn nearest_on_whitespace_only_line_is_noop() {
    // A line of pure spaces has no words — selection unchanged.
    assert_state!(
        "hello\n-[ ]>  \nworld\n",
        |(text, sels)| cmd_select_word_nearest_on_line(&text, sels, 0, MotionMode::Move, false),
        "hello\n-[ ]>  \nworld\n"
    );
}

#[test]
fn nearest_preserves_sticky_display_col_on_word() {
    // sel.sticky_display_col = Some(5) must survive the snap to a word.
    let text = BufferText::from("hello world\n");
    let sels = SelectionSet::single(Selection::with_sticky_display_col(6, 6, sticky(5)));
    let result = cmd_select_word_nearest_on_line(&text, sels, 0, MotionMode::Move, false);
    let sel = result.primary();
    // "world" spans chars 6–10.
    assert_eq!((sel.anchor(), sel.head()), (6, 10), "expected word range");
    assert_eq!(
        sel.sticky_display_col(),
        Some(sticky(5)),
        "sticky_display_col must be preserved"
    );
}

#[test]
fn nearest_preserves_sticky_display_col_on_whitespace() {
    // Head on space, sticky_display_col = Some(3). After snapping to "hi",
    // sticky_display_col still Some(3).
    let text = BufferText::from("hi   world\n");
    //                    0123456789
    // spaces at 2,3,4; head=3 (space), prev word = "hi" ends at 1.
    let sels = SelectionSet::single(Selection::with_sticky_display_col(3, 3, sticky(3)));
    let result = cmd_select_word_nearest_on_line(&text, sels, 0, MotionMode::Move, false);
    let sel = result.primary();
    assert_eq!((sel.anchor(), sel.head()), (0, 1), "expected 'hi' range");
    assert_eq!(
        sel.sticky_display_col(),
        Some(sticky(3)),
        "sticky_display_col must be preserved"
    );
}

#[test]
fn nearest_no_sticky_display_col_is_cleared() {
    // When input sel has sticky_display_col=None, output must also have
    // sticky_display_col=None.
    let text = BufferText::from("hello world\n");
    let sels = SelectionSet::single(Selection::new(6, 6));
    let result = cmd_select_word_nearest_on_line(&text, sels, 0, MotionMode::Move, false);
    let sel = result.primary();
    assert_eq!(
        sel.sticky_display_col(),
        None,
        "sticky_display_col must stay None"
    );
}

#[test]
fn nearest_extend_grows_selection_to_snapped_word() {
    // Simulates Ctrl+j with an existing selection:
    // Buffer: "hello\n     world\n"
    //          0 1 2 3 4 5 | 6 7 8 9 10 11 12 13 14 15 16
    //         h e l l o \n  _ _ _ _  _ w  o  r  l  d \n
    // After move-down in extend mode: anchor stays at 0 (on 'h'), head lands at 10 (space).
    // Snap uses anchor=0 → nearest_word finds "hello" = [0,4] (anchor is already on a word).
    // new_end = max(10, 4) = 10 → selection stays (0, 10); head is already past "hello".
    let text = BufferText::from("hello\n     world\n");
    let sels = SelectionSet::single(Selection::new(0, 10)); // anchor=0, head=10 (space)
    let result = cmd_select_word_nearest_on_line(&text, sels, 0, MotionMode::Extend, false);
    let sel = result.primary();
    assert_eq!(
        (sel.anchor(), sel.head()),
        (0, 10),
        "extend must not shrink selection when anchor word is already covered"
    );
    assert!(sel.anchor() <= sel.head(), "selection must remain forward");
}

#[test]
fn nearest_extend_preserves_sticky_display_col() {
    let text = BufferText::from("hello world\n");
    let sels = SelectionSet::single(Selection::with_sticky_display_col(0, 5, sticky(7))); // anchor=0, head=5 (space)
    let result = cmd_select_word_nearest_on_line(&text, sels, 0, MotionMode::Extend, false);
    assert_eq!(
        result.primary().sticky_display_col(),
        Some(sticky(7)),
        "sticky_display_col must survive extend mode"
    );
}

// ── select-word-nearest-on-line, around = true (word-selects-whitespace on) ─
//
// Same scan logic as above, but the winning anchor is expanded via
// `word_unit_at` instead of `inner_word_impl` — matching `mm`'s
// leading-preferred, trailing-fallback-for-first-word rule (see the
// `select_word_around_*` block). Expected spans are derived directly from
// that rule, independent of this command's own scan implementation.

#[test]
fn nearest_on_word_around_absorbs_leading_whitespace() {
    // Same head position as `nearest_on_word_selects_inner_word` (direct hit,
    // no whitespace snap needed). "world" isn't the first word on its line,
    // so `around = true` grows the selection to include its leading space —
    // contrast the `false` case, which selects "world" alone.
    assert_state!(
        "hello wor-[l]>d foo\n",
        |(text, sels)| cmd_select_word_nearest_on_line(&text, sels, 0, MotionMode::Move, true),
        "hello-[ world]> foo\n"
    );
}

#[test]
fn nearest_on_whitespace_around_expands_snapped_word() {
    // Same buffer/head as `nearest_on_whitespace_next_closer`. The scan still
    // snaps to "bar" via the nearest-edge rule, but the expansion step then
    // absorbs the word's *full* leading whitespace run (all three spaces),
    // not just the portion between `head` and the word.
    assert_state!(
        "foo  -[ ]>bar\n",
        |(text, sels)| cmd_select_word_nearest_on_line(&text, sels, 0, MotionMode::Move, true),
        "foo-[   bar]>\n"
    );
}

#[test]
fn nearest_on_whitespace_around_keeps_indentation_protected() {
    // Same buffer/head as `nearest_at_line_start_whitespace_no_cross_to_prev_line`.
    // The scan snaps to "start", whose leading run reaches the start of its
    // line (indentation) — `expand_word_unit` must not absorb it, and there
    // is no trailing space to fall back to either, so `around = true`
    // produces the identical span to `around = false` here.
    assert_state!(
        "end\n-[ ]>start\n",
        |(text, sels)| cmd_select_word_nearest_on_line(&text, sels, 0, MotionMode::Move, true),
        "end\n -[start]>\n"
    );
}
