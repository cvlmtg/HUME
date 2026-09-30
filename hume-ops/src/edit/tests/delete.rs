use super::super::*;
use hume_editing::word::WordChars;
use hume_rope::offset::CharOffset;
use pretty_assertions::assert_eq;
use test_fixtures::assert_state;

// ── dedent_tab_backward ───────────────────────────────────────────────────

#[test]
fn dedent_spaces_to_prev_tab_stop() {
    // "    x" cursor at col 4 (after 4 spaces, on 'x'? No, cursor on a space).
    // Cursor at col 4 means 4 spaces before it. tw=4 → prev_stop 0, delete all 4.
    // Text: "    \n" with cursor at char 4 (on '\n'). Hmm, let's put content after.
    // "    x\n": cursor on 'x' (char 4, col 4). prev_stop 0. Delete [0,4) = 4 spaces.
    assert_state!(
        "    -[x]>\n",
        |(text, sels)| dedent_tab_backward(test_fixtures::testing::state(text, sels), 4),
        "-[x]>\n"
    );
}

#[test]
fn dedent_six_spaces_to_display_col_four() {
    // "      x\n" (6 spaces + x). cursor on 'x' (col 6). prev_stop 4. Delete 2 spaces.
    assert_state!(
        "      -[x]>\n",
        |(text, sels)| dedent_tab_backward(test_fixtures::testing::state(text, sels), 4),
        "    -[x]>\n"
    );
}

#[test]
fn dedent_hard_tab_to_prev_stop() {
    // "\t\tx\n": two tabs (col 8). cursor on 'x' (col 8). prev_stop 4. Delete 1 tab.
    assert_state!(
        "\t\t-[x]>\n",
        |(text, sels)| dedent_tab_backward(test_fixtures::testing::state(text, sels), 4),
        "\t-[x]>\n"
    );
}

#[test]
fn dedent_single_tab_to_zero() {
    // "\tx\n": one tab (col 4). cursor on 'x' (col 4). prev_stop 0. Delete the tab.
    assert_state!(
        "\t-[x]>\n",
        |(text, sels)| dedent_tab_backward(test_fixtures::testing::state(text, sels), 4),
        "-[x]>\n"
    );
}

#[test]
fn dedent_mid_indent_snaps_to_prev_stop() {
    // "    \n" (4 spaces, whole line ws). cursor on '\n' (col 4)? No, cursor on
    // a space mid-indent. "    \n" cursor at char 2 (col 2). prev_stop 0. Delete 2 spaces.
    assert_state!(
        "  -[ ]>  \n",
        |(text, sels)| dedent_tab_backward(test_fixtures::testing::state(text, sels), 4),
        "-[ ]>  \n"
    );
}

#[test]
fn dedent_mixed_tabs_spaces() {
    // "  \tx\n" (2 spaces + tab = col 4). cursor on 'x' (col 4). prev_stop 0.
    // Delete [0,3) = "  \t".
    assert_state!(
        "  \t-[x]>\n",
        |(text, sels)| dedent_tab_backward(test_fixtures::testing::state(text, sels), 4),
        "-[x]>\n"
    );
}

#[test]
fn dedent_tab_width_8() {
    // "        x\n" (8 spaces). cursor on 'x' (col 8). tw=8 → prev_stop 0. Delete 8.
    assert_state!(
        "        -[x]>\n",
        |(text, sels)| dedent_tab_backward(test_fixtures::testing::state(text, sels), 8),
        "-[x]>\n"
    );
}

#[test]
fn dedent_two_cursors_in_leading_ws() {
    // Two lines, each "  x", cursor on 'x' (col 2). prev_stop 0. Delete 2 each.
    assert_state!(
        "  -[x]>\n  -[y]>\n",
        |(text, sels)| dedent_tab_backward(test_fixtures::testing::state(text, sels), 4),
        "-[x]>\n-[y]>\n"
    );
}

#[test]
fn dedent_at_display_col_one_deletes_one_space() {
    // " x\n" (1 space + x). cursor on 'x' (col 1). prev_stop 0. Delete 1 space.
    assert_state!(
        " -[x]>\n",
        |(text, sels)| dedent_tab_backward(test_fixtures::testing::state(text, sels), 4),
        "-[x]>\n"
    );
}

#[test]
fn dedent_two_cursors_same_line_independent() {
    // Two cursors on the same line "     \n" (5 spaces). Cursor 0 at col 2
    // deletes back to the stop at col 0, [0,2). Cursor 1 at col 5 (on '\n')
    // deletes back to the stop at col 4, [4,5). The ranges are disjoint, so
    // "  \n" is left.
    assert_state!(
        "  -[ ]>  -[\n]>",
        |(text, sels)| dedent_tab_backward(test_fixtures::testing::state(text, sels), 4),
        "-[ ]> -[\n]>"
    );
}

#[test]
fn dedent_two_cursors_same_line_target_overlap() {
    // "      \n" (6 spaces): cursor 0 at col 5, cursor 1 at col 6 ('\n').
    // Cursor 0 deletes back to the stop at col 4, [4,5). Cursor 1 deletes back
    // to the stop at col 4 too, [4,6). The union [4,6) goes, so 4 spaces
    // remain and both cursors land at col 4.
    assert_state!(
        "     -[ ]>-[\n]>",
        |(text, sels)| dedent_tab_backward(test_fixtures::testing::state(text, sels), 4),
        "    -[\n]>"
    );
}

#[test]
fn dedent_two_cursors_same_line_same_target() {
    // Cursors at col 3 and col 4 of a 4-space indent both delete back to the
    // stop at col 0, [0,3) and [0,4): the union goes, so all 4 spaces are
    // deleted.
    assert_state!(
        "   -[ ]>-[\n]>",
        |(text, sels)| dedent_tab_backward(test_fixtures::testing::state(text, sels), 4),
        "-[\n]>"
    );
}

#[test]
fn dedent_mid_indent_with_content() {
    // "  x\n": 2-space indent then 'x'. Cursor on the second space (col 1),
    // inside leading ws with content after. tw=4 → prev_stop 0, delete 1 space.
    // Pins that content after the cursor doesn't disqualify a mid-indent
    // cursor from dedenting (only chars *before* the cursor matter).
    assert_state!(
        " -[ ]>x\n",
        |(text, sels)| dedent_tab_backward(test_fixtures::testing::state(text, sels), 4),
        "-[ ]>x\n"
    );
}

// ── delete_char_forward ───────────────────────────────────────────────────

#[test]
fn delete_forward_at_cursor_start() {
    // Cursor on 'h'; deletes 'h'; cursor stays at 0 (now on 'e').
    assert_state!(
        "-[h]>ello\n",
        |(text, sels)| delete_char_forward(test_fixtures::testing::state(text, sels)),
        "-[e]>llo\n"
    );
}

#[test]
fn delete_forward_at_cursor_middle() {
    assert_state!(
        "h-[e]>llo\n",
        |(text, sels)| delete_char_forward(test_fixtures::testing::state(text, sels)),
        "h-[l]>lo\n"
    );
}

#[test]
fn delete_forward_at_eof_is_noop() {
    assert_state!(
        "hello-[\n]>",
        |(text, sels)| delete_char_forward(test_fixtures::testing::state(text, sels)),
        "hello-[\n]>"
    );
}

#[test]
fn delete_forward_empty_buffer_is_noop() {
    assert_state!(
        "-[\n]>",
        |(text, sels)| delete_char_forward(test_fixtures::testing::state(text, sels)),
        "-[\n]>"
    );
}

#[test]
fn delete_forward_selection() {
    // Selection [0,3] inclusive → remove [0,4) → "o", cursor at 0.
    assert_state!(
        "-[hell]>o\n",
        |(text, sels)| delete_char_forward(test_fixtures::testing::state(text, sels)),
        "-[o]>\n"
    );
}

#[test]
fn delete_forward_two_cursors() {
    // Cursors at 0 ('h') and 2 ('l'). Delete 'h' and first 'l'.
    // Changeset: Delete(1), Retain(1), Delete(1), Retain(2).
    // Result: "elo", cursors at 0 and 1.
    assert_state!(
        "-[h]>e-[l]>lo\n",
        |(text, sels)| delete_char_forward(test_fixtures::testing::state(text, sels)),
        "-[e]>-[l]>o\n"
    );
}

#[test]
fn delete_forward_adjacent_cursors_merge() {
    // Cursors at 2 and 3. Both delete forward; both land at 2 → merge.
    assert_state!(
        "he-[l]>-[l]>o\n",
        |(text, sels)| delete_char_forward(test_fixtures::testing::state(text, sels)),
        "he-[o]>\n"
    );
}

#[test]
fn delete_forward_grapheme_cluster() {
    // "e\u{0301}x": é is 2 chars, 1 grapheme. Cursor at 0 deletes whole cluster.
    assert_state!(
        "-[e\u{0301}]>x\n",
        |(text, sels)| delete_char_forward(test_fixtures::testing::state(text, sels)),
        "-[x]>\n"
    );
}

// ── delete_char_backward ─────────────────────────────────────────────────

#[test]
fn delete_backward_at_cursor_end() {
    // Cursor at EOF (offset 5); backspace deletes 'o'; cursor at 4.
    assert_state!(
        "hello-[\n]>",
        |(text, sels)| delete_char_backward(test_fixtures::testing::state(text, sels)),
        "hell-[\n]>"
    );
}

#[test]
fn delete_backward_at_cursor_middle() {
    // Cursor at 3 ('l'); backspace deletes 'l' at 2; cursor at 2.
    assert_state!(
        "hel-[l]>o\n",
        |(text, sels)| delete_char_backward(test_fixtures::testing::state(text, sels)),
        "he-[l]>o\n"
    );
}

#[test]
fn delete_backward_at_start_is_noop() {
    assert_state!(
        "-[h]>ello\n",
        |(text, sels)| delete_char_backward(test_fixtures::testing::state(text, sels)),
        "-[h]>ello\n"
    );
}

#[test]
fn delete_backward_empty_buffer_is_noop() {
    assert_state!(
        "-[\n]>",
        |(text, sels)| delete_char_backward(test_fixtures::testing::state(text, sels)),
        "-[\n]>"
    );
}

#[test]
fn delete_backward_selection() {
    // Same as delete_forward for multi-char selections: removes selected region.
    assert_state!(
        "-[hell]>o\n",
        |(text, sels)| delete_char_backward(test_fixtures::testing::state(text, sels)),
        "-[o]>\n"
    );
}

#[test]
fn delete_backward_two_cursors() {
    // Cursors at 2 and 4 in "hello". Backspace at 2 deletes 'e' (offset 1).
    // Backspace at 4 deletes 'l' (offset 3).
    // Changeset: Retain(1), Delete(1), Retain(1), Delete(1), Retain(1).
    // Result: "hlo", cursors at 1 and 2.
    assert_state!(
        "he-[l]>l-[o]>\n",
        |(text, sels)| delete_char_backward(test_fixtures::testing::state(text, sels)),
        "h-[l]>-[o]>\n"
    );
}

#[test]
fn delete_backward_grapheme_cluster() {
    // "e\u{0301}x": é is 2 chars (offsets 0-1). Cursor at 2 (on 'x').
    // The cluster before 2 starts at 0. Deletes entire é cluster.
    assert_state!(
        "e\u{0301}-[x]>\n",
        |(text, sels)| delete_char_backward(test_fixtures::testing::state(text, sels)),
        "-[x]>\n"
    );
}

#[test]
fn delete_backward_adjacent_cursors_merge() {
    // Cursors at 2 and 3. Backspace at 2: delete offset 1. Backspace at 3:
    // delete offset 2 in original. Both cursors land at 1 → merge.
    assert_state!(
        "he-[l]>-[l]>o\n",
        |(text, sels)| delete_char_backward(test_fixtures::testing::state(text, sels)),
        "h-[l]>o\n"
    );
}

// ── delete_word_backward ─────────────────────────────────────────────────

#[test]
fn delete_word_backward_at_end_of_word() {
    // Cursor after "hello"; Ctrl-w deletes the word; cursor at buffer start.
    assert_state!(
        "hello-[\n]>",
        |(text, sels)| delete_word_backward(
            test_fixtures::testing::state(text, sels),
            WordChars::default()
        ),
        "-[\n]>"
    );
}

#[test]
fn delete_word_backward_mid_word() {
    // Cursor at offset 3 (inside "hello" on 'l'); deletes "hel" → cursor after "lo".
    assert_state!(
        "hel-[l]>o\n",
        |(text, sels)| delete_word_backward(
            test_fixtures::testing::state(text, sels),
            WordChars::default()
        ),
        "-[l]>o\n"
    );
}

#[test]
fn delete_word_backward_skips_whitespace() {
    // Cursor after "hello world" whitespace + "world"; deletes back past whitespace.
    assert_state!(
        "hello world-[\n]>",
        |(text, sels)| delete_word_backward(
            test_fixtures::testing::state(text, sels),
            WordChars::default()
        ),
        "hello -[\n]>"
    );
}

#[test]
fn delete_word_backward_at_start_is_noop() {
    assert_state!(
        "-[h]>ello\n",
        |(text, sels)| delete_word_backward(
            test_fixtures::testing::state(text, sels),
            WordChars::default()
        ),
        "-[h]>ello\n"
    );
}

#[test]
fn delete_word_backward_empty_buffer_is_noop() {
    assert_state!(
        "-[\n]>",
        |(text, sels)| delete_word_backward(
            test_fixtures::testing::state(text, sels),
            WordChars::default()
        ),
        "-[\n]>"
    );
}

#[test]
fn delete_word_backward_selection() {
    // Multi-char selection: delegates to delete_sel_region.
    assert_state!(
        "-[hell]>o\n",
        |(text, sels)| delete_word_backward(
            test_fixtures::testing::state(text, sels),
            WordChars::default()
        ),
        "-[o]>\n"
    );
}

#[test]
fn delete_word_backward_two_cursors() {
    // Cursors at offsets 5 and 11 in "hello world". First deletes "hello"
    // (offsets 0..5), second deletes "world" (offsets 6..11).
    assert_state!(
        "hello-[\n]>world-[\n]>",
        |(text, sels)| delete_word_backward(
            test_fixtures::testing::state(text, sels),
            WordChars::default()
        ),
        "-[\n]>-[\n]>"
    );
}

#[test]
fn delete_word_backward_two_cursors_same_word() {
    // Heads at 2 ('o') and 5 ('r') in "foobar\n" share one word. Each deletes
    // back to the word start, [0,2) and [0,5), so their union goes and both
    // cursors land on what follows it.
    assert_state!(
        "fo-[o]>ba-[r]>\n",
        |(text, sels)| delete_word_backward(
            test_fixtures::testing::state(text, sels),
            WordChars::default()
        ),
        "-[r]>\n"
    );
}

#[test]
fn delete_word_backward_punctuation_group() {
    // Cursor after "foo.bar()"; punctuation group "()" is one word.
    assert_state!(
        "foo.bar()-[\n]>",
        |(text, sels)| delete_word_backward(
            test_fixtures::testing::state(text, sels),
            WordChars::default()
        ),
        "foo.bar-[\n]>"
    );
}

#[test]
fn delete_word_backward_only_whitespace_goes_to_start() {
    // Buffer containing only whitespace before cursor.
    assert_state!(
        "   -[x]>\n",
        |(text, sels)| delete_word_backward(
            test_fixtures::testing::state(text, sels),
            WordChars::default()
        ),
        "-[x]>\n"
    );
}

#[test]
fn delete_word_backward_with_extra_word_char_deletes_whole_run() {
    // With '-' configured as a word char, Ctrl-w after "foo-bar" deletes the
    // whole hyphenated run in one press, not just "bar".
    assert_state!(
        "foo-bar-[\n]>",
        |(text, sels)| delete_word_backward(
            test_fixtures::testing::state(text, sels),
            WordChars::new("-")
        ),
        "-[\n]>"
    );
}

#[test]
fn delete_word_backward_two_cursors_in_one_word_chars_run() {
    // Heads at 2 ('o') and 6 ('r') in "foo-bar\n". With '-' a word char,
    // "foo-bar" is one run, so both cursors' ranges start at 0 and the union
    // [0,6) goes. Under the default word rule the second cursor's range would
    // start at 4 instead, and "ba" alone would go.
    assert_state!(
        "fo-[o]>-ba-[r]>\n",
        |(text, sels)| delete_word_backward(
            test_fixtures::testing::state(text, sels),
            WordChars::new("-")
        ),
        "-[r]>\n"
    );
}

// ── delete_selection ──────────────────────────────────────────────────────

#[test]
fn delete_selection_cursor_deletes_char() {
    // Cursor on 'h': deletes 'h'; cursor lands on 'e' (what was next).
    assert_state!(
        "-[h]>ello\n",
        |(text, sels)| delete_selection(test_fixtures::testing::state(text, sels)),
        "-[e]>llo\n"
    );
}

#[test]
fn delete_selection_cursor_at_end_of_word() {
    // Cursor on 'o' (last word char): deletes 'o'; cursor lands on '\n'.
    assert_state!(
        "hell-[o]>\n",
        |(text, sels)| delete_selection(test_fixtures::testing::state(text, sels)),
        "hell-[\n]>"
    );
}

#[test]
fn delete_selection_cursor_on_structural_newline_is_noop() {
    // Cursor on the trailing '\n': buffer invariant, no-op.
    assert_state!(
        "hello-[\n]>",
        |(text, sels)| delete_selection(test_fixtures::testing::state(text, sels)),
        "hello-[\n]>"
    );
}

#[test]
fn delete_selection_empty_buffer_is_noop() {
    // Only the structural '\n'. Cursor is on it, no-op.
    assert_state!(
        "-[\n]>",
        |(text, sels)| delete_selection(test_fixtures::testing::state(text, sels)),
        "-[\n]>"
    );
}

#[test]
fn delete_selection_multi_char_forward() {
    // Forward selection covering "hell": cursor lands at start (pos 0).
    assert_state!(
        "-[hell]>o\n",
        |(text, sels)| delete_selection(test_fixtures::testing::state(text, sels)),
        "-[o]>\n"
    );
}

#[test]
fn delete_selection_multi_char_backward() {
    // Backward selection: same result as forward; cursor lands at start.
    assert_state!(
        "<[hell]-o\n",
        |(text, sels)| delete_selection(test_fixtures::testing::state(text, sels)),
        "-[o]>\n"
    );
}

#[test]
fn delete_selection_two_cursors() {
    // Cursors on 'h' (pos 0) and 'l' (pos 2), both deleted independently.
    assert_state!(
        "-[h]>el-[l]>o\n",
        |(text, sels)| delete_selection(test_fixtures::testing::state(text, sels)),
        "-[e]>l-[o]>\n"
    );
}

#[test]
fn delete_selection_adjacent_selections_merge_cursors() {
    // Cursors on 'h' (0) and 'e' (1). After deleting both, cursors both
    // land at 0 and merge into one.
    assert_state!(
        "-[h]>-[e]>llo\n",
        |(text, sels)| delete_selection(test_fixtures::testing::state(text, sels)),
        "-[l]>lo\n"
    );
}

#[test]
fn delete_selection_grapheme_cluster() {
    // "e\u{0301}" is 2 chars (e + combining acute) but one grapheme cluster.
    // A cursor on the cluster deletes both chars.
    assert_state!(
        "-[e\u{0301}]>x\n",
        |(text, sels)| delete_selection(test_fixtures::testing::state(text, sels)),
        "-[x]>\n"
    );
}

#[test]
fn delete_selection_multi_char_ends_at_grapheme_base() {
    // The selection's last cluster is {e\u{0301}} = é. The delete covers both
    // of its chars (0-4), leaving " x\n" with no orphaned accent.
    assert_state!(
        "-[cafe\u{0301}]> x\n",
        |(text, sels)| delete_selection(test_fixtures::testing::state(text, sels)),
        "-[ ]>x\n"
    );
}

// ── delete_selection — last-line whole-line deletion ──────────────────────

#[test]
fn delete_selection_last_line_removes_line_not_content() {
    // "foo\nbar\n": x on the last line selects [4,7]. The line goes and no
    // empty line is left: "foo\n", cursor on the start of "foo".
    assert_state!(
        "foo\n-[bar\n]>",
        |(text, sels)| delete_selection(test_fixtures::testing::state(text, sels)),
        "-[f]>oo\n"
    );
}

#[test]
fn delete_selection_last_line_with_empty_preceding_line() {
    // "foo\n\nbar\n": delete last line → "foo\n\n", cursor on the now-last
    // empty line ('\n' at pos 4).
    assert_state!(
        "foo\n\n-[bar\n]>",
        |(text, sels)| delete_selection(test_fixtures::testing::state(text, sels)),
        "foo\n-[\n]>"
    );
}

#[test]
fn delete_selection_of_a_line_before_the_last_lands_on_the_next_line() {
    assert_state!(
        "a\n-[b\n]>c\n",
        |(text, sels)| delete_selection(test_fixtures::testing::state(text, sels)),
        "a\n-[c]>\n"
    );
}

#[test]
fn delete_selection_of_touching_last_lines_leaves_no_blank_line() {
    assert_state!(
        "a\n-[b\n]>-[c\n]>",
        |(text, sels)| delete_selection(test_fixtures::testing::state(text, sels)),
        "-[a]>\n"
    );
}

#[test]
fn delete_selection_of_a_last_line_split_across_two_selections_removes_the_line() {
    assert_state!(
        "a\n-[b]>-[\n]>",
        |(text, sels)| delete_selection(test_fixtures::testing::state(text, sels)),
        "a-[\n]>"
    );
}

#[test]
fn delete_selection_of_cursors_on_the_last_empty_lines_leaves_no_blank_line() {
    assert_state!(
        "a\n-[\n]>-[\n]>",
        |(text, sels)| delete_selection(test_fixtures::testing::state(text, sels)),
        "-[a]>\n"
    );
}

#[test]
fn delete_selection_last_line_single_line_still_empties() {
    // Single-line buffer "foo\n": the content goes and the structural '\n'
    // stays.
    assert_state!(
        "-[foo\n]>",
        |(text, sels)| delete_selection(test_fixtures::testing::state(text, sels)),
        "-[\n]>"
    );
}

#[test]
fn delete_selection_whole_buffer_caps_at_last_content_char() {
    // "foo\nbar\n", selection [0,7]: everything but the structural '\n' goes.
    assert_state!(
        "-[foo\nbar\n]>",
        |(text, sels)| delete_selection(test_fixtures::testing::state(text, sels)),
        "-[\n]>"
    );
}

#[test]
fn delete_selection_partial_last_line_still_caps() {
    // "foo\nbar\n", select [5,7] (from mid-line 'a' through the structural
    // '\n'). "ar" goes and the '\n' stays, since "foo\nb" would not end with
    // one: "foo\nb\n", cursor on the '\n' (the deletion point).
    assert_state!(
        "foo\nb-[ar\n]>",
        |(text, sels)| delete_selection(test_fixtures::testing::state(text, sels)),
        "foo\nb-[\n]>"
    );
}

#[test]
fn delete_selection_three_lines_delete_last() {
    // "a\nb\nc\n": x selects last line [4,6] (anchor 4 'c', head 6 '\n').
    // After delete: "a\nb\n", cursor at start of "b" line (pos 2).
    assert_state!(
        "a\nb\n-[c\n]>",
        |(text, sels)| delete_selection(test_fixtures::testing::state(text, sels)),
        "a\n-[b]>\n"
    );
}

// ── delete_selection — blank last line (collapsed cursor) ────────────────
//
// A cursor on the structural '\n' of a blank last line removes that line.

#[test]
fn delete_selection_blank_last_line_one_above() {
    // "a\n\n": cursor on the structural '\n' at pos 2. Deleting removes the
    // blank last line; result is "a\n", cursor on 'a' (pos 0 = line start).
    assert_state!(
        "a\n-[\n]>",
        |(text, sels)| delete_selection(test_fixtures::testing::state(text, sels)),
        "-[a]>\n"
    );
}

#[test]
fn delete_selection_two_blank_lines_removes_one() {
    // "a\n\n\n": cursor on the structural '\n' at pos 3 (the last blank line).
    // One blank line removed; result "a\n\n", cursor on the remaining blank
    // last line '\n' at pos 2.
    assert_state!(
        "a\n\n-[\n]>",
        |(text, sels)| delete_selection(test_fixtures::testing::state(text, sels)),
        "a\n-[\n]>"
    );
}

#[test]
fn delete_selection_lone_blank_line_is_noop() {
    // Single-char buffer "\n": removing it would leave no '\n', so nothing
    // goes.
    assert_state!(
        "-[\n]>",
        |(text, sels)| delete_selection(test_fixtures::testing::state(text, sels)),
        "-[\n]>"
    );
}

#[test]
fn delete_selection_last_line_multi_cursor_cursor_lands_on_the_line_left_last() {
    // Multi-cursor dd-on-last-line. The first cursor deletes 'b' (char 1).
    // The second covers the whole last line "c\n" [anchor=3, head=4]. Its
    // cursor lands at char 0, the start of the line left last, not at char 1.
    let text = BufferText::from("ab\nc\n");
    // primary=1 so the last-line selection is the primary; we assert its cursor.
    let sels = test_fixtures::testing::set(
        &text,
        vec![
            test_fixtures::testing::cursor(&text, 1), // on 'b'
            test_fixtures::testing::sel(&text, 3, 4), // last line "c\n"
        ],
        1, // primary is the last-line cursor
    );
    let (new_text, new_sels, _cs) = test_fixtures::testing::parts(
        delete_selection(test_fixtures::testing::state(text, sels)).edited,
    );
    // 'b' and the last line both go → "a\n"
    assert_eq!(new_text.to_string(), "a\n");
    // The primary lands at char 0, the start of the line left last.
    assert_eq!(
        hume_editing::selection::EditView::bind(&new_text, &new_sels)
            .primary()
            .head()
            .offset(),
        CharOffset::new(0),
        "cursor on the start of the line left last"
    );
}

// ── Clusters and wide chars ───────────────────────────────────────────────

#[test]
fn delete_word_backward_removes_a_word_ending_in_a_combining_mark() {
    assert_state!(
        "cafe\u{301}-[ ]>x\n",
        |(text, sels)| delete_word_backward(
            test_fixtures::testing::state(text, sels),
            WordChars::default()
        ),
        "-[ ]>x\n"
    );
}

#[test]
fn delete_word_backward_removes_a_cjk_word() {
    assert_state!(
        "\u{6f22}\u{5b57}-[ ]>x\n",
        |(text, sels)| delete_word_backward(
            test_fixtures::testing::state(text, sels),
            WordChars::default()
        ),
        "-[ ]>x\n"
    );
}

#[test]
fn c_yanks_the_whole_last_cluster() {
    let removal = delete_selection_content(hume_editing::marked::parse("-[cafe\u{301}]>\n"));
    assert_eq!(removal.yanked, vec!["cafe\u{301}"]);
}

#[test]
fn c_yanks_up_to_a_trailing_newline_after_a_cluster() {
    let removal = delete_selection_content(hume_editing::marked::parse("-[cafe\u{301}\n]>"));
    assert_eq!(removal.yanked, vec!["cafe\u{301}"]);
}

#[test]
fn delete_selection_content_removes_a_cluster_selection_and_keeps_the_newline() {
    assert_state!(
        "-[cafe\u{301}\n]>",
        |(text, sels)| delete_selection_content(test_fixtures::testing::state(text, sels)),
        "-[\n]>"
    );
}

#[test]
fn delete_selection_content_removes_a_wide_char_selection() {
    assert_state!(
        "a-[\u{6f22}\u{5b57}]>b\n",
        |(text, sels)| delete_selection_content(test_fixtures::testing::state(text, sels)),
        "a-[b]>\n"
    );
}

// ── `d`: one computation for what is deleted and what is yanked ───────────

/// `d` on `before`: the text and selections after it, and the register
/// entries it writes.
fn removal(before: &str) -> (String, Vec<String>) {
    let removal = delete_selection(hume_editing::marked::parse(before));
    (
        hume_editing::marked::render(removal.edited.state().view()),
        removal
            .yanked
            .iter()
            .map(|piece| piece.text().to_string())
            .collect(),
    )
}

#[test]
fn d_yanks_what_it_deletes() {
    let cases: &[(&str, &str, &[&str])] = &[
        ("a-[bc]>d\n", "a-[d]>\n", &["bc"]),
        ("a-[be\u{301}]>c\n", "a-[c]>\n", &["be\u{301}"]),
        ("a-[e\u{301}]>c\n", "a-[c]>\n", &["e\u{301}"]),
        ("ab-[c\n]>", "ab-[\n]>", &["c"]),
        ("-[a\n]>b\n", "-[b]>\n", &["a\n"]),
        ("a\n-[b\n]>", "-[a]>\n", &["b\n"]),
        ("a\n-[\n]>", "-[a]>\n", &["\n"]),
        ("-[ab\n]>", "-[\n]>", &["ab\n"]),
        ("ab-[\n]>", "ab-[\n]>", &[""]),
        ("-[\n]>", "-[\n]>", &[""]),
        ("-[a]>b-[\n]>", "-[b]>-[\n]>", &["a", ""]),
    ];
    for &(before, after, yanked) in cases {
        let (text, yank) = removal(before);
        assert_eq!(text, after, "text after d on {before:?}");
        assert_eq!(yank, yanked, "register after d on {before:?}");
    }
}
