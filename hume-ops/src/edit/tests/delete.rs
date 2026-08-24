use super::super::*;
use hume_test_fixtures::assert_state;
use pretty_assertions::assert_eq;

// ── dedent_tab_backward ───────────────────────────────────────────────────

#[test]
fn dedent_spaces_to_prev_tab_stop() {
    // "    x" cursor at col 4 (after 4 spaces, on 'x'? No — cursor on a space).
    // Cursor at col 4 means 4 spaces before it. tw=4 → prev_stop 0, delete all 4.
    // Text: "    \n" with cursor at char 4 (on '\n'). Hmm — let's put content after.
    // "    x\n": cursor on 'x' (char 4, col 4). prev_stop 0. Delete [0,4) = 4 spaces.
    assert_state!(
        "    -[x]>\n",
        |(text, sels)| dedent_tab_backward(text, sels, 4),
        "-[x]>\n"
    );
}

#[test]
fn dedent_six_spaces_to_display_col_four() {
    // "      x\n" (6 spaces + x). cursor on 'x' (col 6). prev_stop 4. Delete 2 spaces.
    assert_state!(
        "      -[x]>\n",
        |(text, sels)| dedent_tab_backward(text, sels, 4),
        "    -[x]>\n"
    );
}

#[test]
fn dedent_hard_tab_to_prev_stop() {
    // "\t\tx\n": two tabs (col 8). cursor on 'x' (col 8). prev_stop 4. Delete 1 tab.
    assert_state!(
        "\t\t-[x]>\n",
        |(text, sels)| dedent_tab_backward(text, sels, 4),
        "\t-[x]>\n"
    );
}

#[test]
fn dedent_single_tab_to_zero() {
    // "\tx\n": one tab (col 4). cursor on 'x' (col 4). prev_stop 0. Delete the tab.
    assert_state!(
        "\t-[x]>\n",
        |(text, sels)| dedent_tab_backward(text, sels, 4),
        "-[x]>\n"
    );
}

#[test]
fn dedent_mid_indent_snaps_to_prev_stop() {
    // "    \n" (4 spaces, whole line ws). cursor on '\n' (col 4)? No — cursor on
    // a space mid-indent. "    \n" cursor at char 2 (col 2). prev_stop 0. Delete 2 spaces.
    assert_state!(
        "  -[ ]>  \n",
        |(text, sels)| dedent_tab_backward(text, sels, 4),
        "-[ ]>  \n"
    );
}

#[test]
fn dedent_mixed_tabs_spaces() {
    // "  \tx\n" (2 spaces + tab = col 4). cursor on 'x' (col 4). prev_stop 0.
    // Delete [0,3) = "  \t".
    assert_state!(
        "  \t-[x]>\n",
        |(text, sels)| dedent_tab_backward(text, sels, 4),
        "-[x]>\n"
    );
}

#[test]
fn dedent_tab_width_8() {
    // "        x\n" (8 spaces). cursor on 'x' (col 8). tw=8 → prev_stop 0. Delete 8.
    assert_state!(
        "        -[x]>\n",
        |(text, sels)| dedent_tab_backward(text, sels, 8),
        "-[x]>\n"
    );
}

#[test]
fn dedent_two_cursors_in_leading_ws() {
    // Two lines, each "  x", cursor on 'x' (col 2). prev_stop 0. Delete 2 each.
    assert_state!(
        "  -[x]>\n  -[y]>\n",
        |(text, sels)| dedent_tab_backward(text, sels, 4),
        "-[x]>\n-[y]>\n"
    );
}

#[test]
fn dedent_at_display_col_one_deletes_one_space() {
    // " x\n" (1 space + x). cursor on 'x' (col 1). prev_stop 0. Delete 1 space.
    assert_state!(
        " -[x]>\n",
        |(text, sels)| dedent_tab_backward(text, sels, 4),
        "-[x]>\n"
    );
}

#[test]
fn dedent_two_cursors_same_line_independent() {
    // Two cursors on the same line "     \n" (5 spaces):
    // cursor 0 at col 2 → prev_stop 0, deletes 2 chars (positions 0..2).
    // cursor 1 at col 5 (on '\n') → prev_stop 4, target=4. After cursor 0's
    // delete, old_pos=2; target 4 > 2 → no clamp needed. Delete 1 char (pos 4).
    // Result: 2 spaces remain, then '\n'.
    //
    // Independent oracle: "     \n" → cursor 0 deletes "  " from front,
    // cursor 1 deletes the last space before '\n'. Result: "  \n".
    assert_state!(
        "  -[ ]>  -[\n]>",
        |(text, sels)| dedent_tab_backward(text, sels, 4),
        "-[ ]> -[\n]>"
    );
}

#[test]
fn dedent_two_cursors_same_line_target_overlap() {
    // Two cursors on the same line where cursor 1's natural prev-stop target
    // falls behind the boundary cursor 0 already consumed.
    //
    // "      \n" (6 spaces): cursor 0 at col 5 (char 5), cursor 1 at col 6 ('\n').
    // Cursor 0: col 5, prev_stop = floor(4/4)*4 = 4. retain 4, delete 1. old_pos=5.
    // Cursor 1: col 6, prev_stop = floor(5/4)*4 = 4. target = char_pos_at_col_4 = 4.
    //   Without fix: target(4) < old_pos(5) → no-op; bug: cursor 1 leaves 1 space.
    //   With fix:    target clamped to max(4,5) = 5; delete 1 char. Both land at 4.
    //
    // Independent oracle: cursor 0 deletes the space at col 4..5; cursor 1 deletes
    // the space at col 5..6 (clamped start). Net: 2 spaces removed → 4 remain.
    assert_state!(
        "     -[ ]>-[\n]>",
        |(text, sels)| dedent_tab_backward(text, sels, 4),
        "    -[\n]>"
    );
}

#[test]
fn dedent_two_cursors_same_line_same_target() {
    // Two cursors whose natural tab-stop targets collide on the SAME
    // position (col 3 and col 4 on a 4-space indent — SelectionSet prevents
    // duplicate positions, so the two land one apart). Tests the
    // `target.max(b.old_pos()) >= p` guard that prevents a zero-length or
    // inverted delete when the second cursor sits at the first cursor's
    // old_pos.
    //
    // Independent oracle: all 4 spaces deleted.
    assert_state!(
        "   -[ ]>-[\n]>",
        |(text, sels)| dedent_tab_backward(text, sels, 4),
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
        |(text, sels)| dedent_tab_backward(text, sels, 4),
        "-[ ]>x\n"
    );
}

// ── delete_char_forward ───────────────────────────────────────────────────

#[test]
fn delete_forward_at_cursor_start() {
    // Cursor on 'h'; deletes 'h'; cursor stays at 0 (now on 'e').
    assert_state!(
        "-[h]>ello\n",
        |(text, sels)| delete_char_forward(text, sels),
        "-[e]>llo\n"
    );
}

#[test]
fn delete_forward_at_cursor_middle() {
    assert_state!(
        "h-[e]>llo\n",
        |(text, sels)| delete_char_forward(text, sels),
        "h-[l]>lo\n"
    );
}

#[test]
fn delete_forward_at_eof_is_noop() {
    assert_state!(
        "hello-[\n]>",
        |(text, sels)| delete_char_forward(text, sels),
        "hello-[\n]>"
    );
}

#[test]
fn delete_forward_empty_buffer_is_noop() {
    assert_state!(
        "-[\n]>",
        |(text, sels)| delete_char_forward(text, sels),
        "-[\n]>"
    );
}

#[test]
fn delete_forward_selection() {
    // Selection [0,3] inclusive → remove [0,4) → "o", cursor at 0.
    assert_state!(
        "-[hell]>o\n",
        |(text, sels)| delete_char_forward(text, sels),
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
        |(text, sels)| delete_char_forward(text, sels),
        "-[e]>-[l]>o\n"
    );
}

#[test]
fn delete_forward_adjacent_cursors_merge() {
    // Cursors at 2 and 3. Both delete forward; both land at 2 → merge.
    assert_state!(
        "he-[l]>-[l]>o\n",
        |(text, sels)| delete_char_forward(text, sels),
        "he-[o]>\n"
    );
}

#[test]
fn delete_forward_grapheme_cluster() {
    // "e\u{0301}x": é is 2 chars, 1 grapheme. Cursor at 0 deletes whole cluster.
    assert_state!(
        "-[e\u{0301}]>x\n",
        |(text, sels)| delete_char_forward(text, sels),
        "-[x]>\n"
    );
}

// ── delete_char_backward ─────────────────────────────────────────────────

#[test]
fn delete_backward_at_cursor_end() {
    // Cursor at EOF (offset 5); backspace deletes 'o'; cursor at 4.
    assert_state!(
        "hello-[\n]>",
        |(text, sels)| delete_char_backward(text, sels),
        "hell-[\n]>"
    );
}

#[test]
fn delete_backward_at_cursor_middle() {
    // Cursor at 3 ('l'); backspace deletes 'l' at 2; cursor at 2.
    assert_state!(
        "hel-[l]>o\n",
        |(text, sels)| delete_char_backward(text, sels),
        "he-[l]>o\n"
    );
}

#[test]
fn delete_backward_at_start_is_noop() {
    assert_state!(
        "-[h]>ello\n",
        |(text, sels)| delete_char_backward(text, sels),
        "-[h]>ello\n"
    );
}

#[test]
fn delete_backward_empty_buffer_is_noop() {
    assert_state!(
        "-[\n]>",
        |(text, sels)| delete_char_backward(text, sels),
        "-[\n]>"
    );
}

#[test]
fn delete_backward_selection() {
    // Same as delete_forward for multi-char selections: removes selected region.
    assert_state!(
        "-[hell]>o\n",
        |(text, sels)| delete_char_backward(text, sels),
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
        |(text, sels)| delete_char_backward(text, sels),
        "h-[l]>-[o]>\n"
    );
}

#[test]
fn delete_backward_grapheme_cluster() {
    // "e\u{0301}x": é is 2 chars (offsets 0-1). Cursor at 2 (on 'x').
    // prev_grapheme_boundary(2) = 0. Deletes entire é cluster.
    assert_state!(
        "e\u{0301}-[x]>\n",
        |(text, sels)| delete_char_backward(text, sels),
        "-[x]>\n"
    );
}

#[test]
fn delete_backward_adjacent_cursors_merge() {
    // Cursors at 2 and 3. Backspace at 2: delete offset 1. Backspace at 3:
    // delete offset 2 in original. Both cursors land at 1 → merge.
    assert_state!(
        "he-[l]>-[l]>o\n",
        |(text, sels)| delete_char_backward(text, sels),
        "h-[l]>o\n"
    );
}

// ── delete_word_backward ─────────────────────────────────────────────────

#[test]
fn delete_word_backward_at_end_of_word() {
    // Cursor after "hello"; Ctrl-W deletes the word; cursor at buffer start.
    assert_state!(
        "hello-[\n]>",
        |(text, sels)| delete_word_backward(text, sels),
        "-[\n]>"
    );
}

#[test]
fn delete_word_backward_mid_word() {
    // Cursor at offset 3 (inside "hello" on 'l'); deletes "hel" → cursor after "lo".
    assert_state!(
        "hel-[l]>o\n",
        |(text, sels)| delete_word_backward(text, sels),
        "-[l]>o\n"
    );
}

#[test]
fn delete_word_backward_skips_whitespace() {
    // Cursor after "hello world" whitespace + "world"; deletes back past whitespace.
    assert_state!(
        "hello world-[\n]>",
        |(text, sels)| delete_word_backward(text, sels),
        "hello -[\n]>"
    );
}

#[test]
fn delete_word_backward_at_start_is_noop() {
    assert_state!(
        "-[h]>ello\n",
        |(text, sels)| delete_word_backward(text, sels),
        "-[h]>ello\n"
    );
}

#[test]
fn delete_word_backward_empty_buffer_is_noop() {
    assert_state!(
        "-[\n]>",
        |(text, sels)| delete_word_backward(text, sels),
        "-[\n]>"
    );
}

#[test]
fn delete_word_backward_selection() {
    // Multi-char selection: delegates to delete_sel_region.
    assert_state!(
        "-[hell]>o\n",
        |(text, sels)| delete_word_backward(text, sels),
        "-[o]>\n"
    );
}

#[test]
fn delete_word_backward_two_cursors() {
    // Cursors at offsets 5 and 11 in "hello world". First deletes "hello"
    // (offsets 0..5), second deletes "world" (offsets 6..11).
    assert_state!(
        "hello-[\n]>world-[\n]>",
        |(text, sels)| delete_word_backward(text, sels),
        "-[\n]>-[\n]>"
    );
}

#[test]
fn delete_word_backward_two_cursors_same_word() {
    // Two cursors inside the same word: heads at 2 ('o') and 5 ('r') in "foobar\n".
    // Cursor 1 (head=2): word_start=0 >= old_pos=0 → delete [0,2) → "foobar\n"
    //   becomes "obar\n"; cursor 1 at new offset 0.
    // Cursor 2 (head=5): word_start=0 < old_pos=2 (prior cursor consumed there) →
    //   overlap-skip → retain [2,5) → cursor 2 at new offset 3, NOT 0.
    assert_state!(
        "fo-[o]>ba-[r]>\n",
        |(text, sels)| delete_word_backward(text, sels),
        "-[o]>ba-[r]>\n"
    );
}

#[test]
fn delete_word_backward_punctuation_group() {
    // Cursor after "foo.bar()"; punctuation group "()" is one word.
    assert_state!(
        "foo.bar()-[\n]>",
        |(text, sels)| delete_word_backward(text, sels),
        "foo.bar-[\n]>"
    );
}

#[test]
fn delete_word_backward_only_whitespace_goes_to_start() {
    // Buffer containing only whitespace before cursor.
    assert_state!(
        "   -[x]>\n",
        |(text, sels)| delete_word_backward(text, sels),
        "-[x]>\n"
    );
}

// ── delete_selection ──────────────────────────────────────────────────────

#[test]
fn delete_selection_cursor_deletes_char() {
    // Cursor on 'h' — deletes 'h'; cursor lands on 'e' (what was next).
    assert_state!(
        "-[h]>ello\n",
        |(text, sels)| delete_selection(text, sels),
        "-[e]>llo\n"
    );
}

#[test]
fn delete_selection_cursor_at_end_of_word() {
    // Cursor on 'o' (last word char) — deletes 'o'; cursor lands on '\n'.
    assert_state!(
        "hell-[o]>\n",
        |(text, sels)| delete_selection(text, sels),
        "hell-[\n]>"
    );
}

#[test]
fn delete_selection_cursor_on_structural_newline_is_noop() {
    // Cursor on the trailing '\n' — buffer invariant, no-op.
    assert_state!(
        "hello-[\n]>",
        |(text, sels)| delete_selection(text, sels),
        "hello-[\n]>"
    );
}

#[test]
fn delete_selection_empty_buffer_is_noop() {
    // Only the structural '\n' — cursor is on it, no-op.
    assert_state!(
        "-[\n]>",
        |(text, sels)| delete_selection(text, sels),
        "-[\n]>"
    );
}

#[test]
fn delete_selection_multi_char_forward() {
    // Forward selection covering "hell" — cursor lands at start (pos 0).
    assert_state!(
        "-[hell]>o\n",
        |(text, sels)| delete_selection(text, sels),
        "-[o]>\n"
    );
}

#[test]
fn delete_selection_multi_char_backward() {
    // Backward selection — same result as forward; cursor lands at start.
    assert_state!(
        "<[hell]-o\n",
        |(text, sels)| delete_selection(text, sels),
        "-[o]>\n"
    );
}

#[test]
fn delete_selection_two_cursors() {
    // Cursors on 'h' (pos 0) and 'l' (pos 2) — both deleted independently.
    assert_state!(
        "-[h]>el-[l]>o\n",
        |(text, sels)| delete_selection(text, sels),
        "-[e]>l-[o]>\n"
    );
}

#[test]
fn delete_selection_adjacent_selections_merge_cursors() {
    // Cursors on 'h' (0) and 'e' (1) — after deleting both, cursors both
    // land at 0 and merge into one.
    assert_state!(
        "-[h]>-[e]>llo\n",
        |(text, sels)| delete_selection(text, sels),
        "-[l]>lo\n"
    );
}

#[test]
fn delete_selection_grapheme_cluster() {
    // "e\u{0301}" is 2 chars (e + combining acute) but one grapheme cluster.
    // Cursor on 'e' (pos 0) deletes the entire cluster (both chars).
    assert_state!(
        "-[e]>\u{0301}x\n",
        |(text, sels)| delete_selection(text, sels),
        "-[x]>\n"
    );
}

#[test]
fn delete_selection_multi_char_ends_at_grapheme_base() {
    // Multi-char selection whose head (sel.end()) lands on the base codepoint
    // 'e' of the grapheme {e\u{0301}} = é. The fix extends the delete to
    // include the combining mark at position 4, so no orphaned accent remains.
    // Text: "cafe\u{0301} x\n". Selection anchor=0, head=3 ('e').
    // Without the fix: only chars 0-3 deleted → "\u{0301} x\n" (broken).
    // With the fix: chars 0-4 deleted → " x\n" (correct).
    assert_state!(
        "-[cafe]>\u{0301} x\n",
        |(text, sels)| delete_selection(text, sels),
        "-[ ]>x\n"
    );
}

// ── delete_selection — last-line whole-line deletion ──────────────────────

#[test]
fn delete_selection_last_line_removes_line_not_content() {
    // "foo\nbar\n": x on last line selects [4,7] (anchor 4, head on structural
    // '\n' at 7). Deleting must remove the preceding '\n' so "bar" vanishes
    // entirely — result "foo\n", cursor at start of "foo" (pos 0 = 'f').
    assert_state!(
        "foo\n-[bar\n]>",
        |(text, sels)| delete_selection(text, sels),
        "-[f]>oo\n"
    );
}

#[test]
fn delete_selection_last_line_with_empty_preceding_line() {
    // "foo\n\nbar\n": delete last line → "foo\n\n", cursor on the now-last
    // empty line ('\n' at pos 4).
    assert_state!(
        "foo\n\n-[bar\n]>",
        |(text, sels)| delete_selection(text, sels),
        "foo\n-[\n]>"
    );
}

#[test]
fn delete_selection_last_line_single_line_still_empties() {
    // Single-line buffer "foo\n": selection [0,3] — no preceding line, so the
    // normal cap applies: only content deleted, structural '\n' kept.
    assert_state!(
        "-[foo\n]>",
        |(text, sels)| delete_selection(text, sels),
        "-[\n]>"
    );
}

#[test]
fn delete_selection_whole_buffer_caps_at_last_content_char() {
    // "foo\nbar\n", selection [0,7]: start==0, no preceding line, normal cap.
    // "foo\n" is not at line boundary for preceding detection (start==0).
    assert_state!(
        "-[foo\nbar\n]>",
        |(text, sels)| delete_selection(text, sels),
        "-[\n]>"
    );
}

#[test]
fn delete_selection_partial_last_line_still_caps() {
    // "foo\nbar\n", select [5,7] (head on structural '\n', but NOT at line
    // start — anchor is mid-line 'a'). Must use normal capped path: deletes
    // "ar", leaves "foo\nb\n", cursor at pos 5 = '\n' (deletion point).
    assert_state!(
        "foo\nb-[ar\n]>",
        |(text, sels)| delete_selection(text, sels),
        "foo\nb-[\n]>"
    );
}

#[test]
fn delete_selection_three_lines_delete_last() {
    // "a\nb\nc\n": x selects last line [4,6] (anchor 4 'c', head 6 '\n').
    // After delete: "a\nb\n", cursor at start of "b" line (pos 2).
    assert_state!(
        "a\nb\n-[c\n]>",
        |(text, sels)| delete_selection(text, sels),
        "a\n-[b]>\n"
    );
}

// ── delete_selection — blank last line (collapsed cursor) ────────────────
//
// A collapsed cursor on the structural trailing '\n' means the cursor sits
// on a blank last line. Pressing `d` must remove it (by consuming the
// preceding '\n'), not silently no-op.

#[test]
fn delete_selection_blank_last_line_one_above() {
    // "a\n\n": cursor on the structural '\n' at pos 2. Deleting removes the
    // blank last line; result is "a\n", cursor on 'a' (pos 0 = line start).
    assert_state!(
        "a\n-[\n]>",
        |(text, sels)| delete_selection(text, sels),
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
        |(text, sels)| delete_selection(text, sels),
        "a\n-[\n]>"
    );
}

#[test]
fn delete_selection_lone_blank_line_is_noop() {
    // Single-char buffer "\n": the structural '\n' IS the entire buffer.
    // No line above exists, so deletion is a no-op (invariant preserved).
    assert_state!(
        "-[\n]>",
        |(text, sels)| delete_selection(text, sels),
        "-[\n]>"
    );
}

#[test]
fn delete_selection_last_line_multi_cursor_cursor_lands_at_merged_line_start() {
    // Multi-cursor dd-on-last-line. The first cursor deletes 'b' (char 1),
    // advancing b.old_pos. The second cursor covers the whole last line
    // "c\n" [anchor=3, head=4]. The cursor produced for that deletion must
    // land at char 0 (start of the merged "a" line), not at char 1.
    //
    // char_col = del_start - line_to_char(prev_line) = 2 - 0 = 2
    // retain(0); cursor_new = b.new_pos().saturating_sub(2) = 1 - 2 = 0 ✓
    use crate::edit::delete_selection;
    use hume_editing::selection::SelectionSet;
    let text = hume_editing::text::BufferText::from("ab\nc\n");
    // primary=1 so the last-line selection is the primary; we assert its cursor.
    let sels = SelectionSet::from_vec(
        vec![
            hume_editing::selection::Selection::collapsed(1), // on 'b'
            hume_editing::selection::Selection::new(3, 4),    // last line "c\n"
        ],
        1, // primary is the last-line cursor
    );
    let (new_buf, new_sels, _cs) = delete_selection(text, sels);
    // 'b' deleted and "c\n" merged into preceding line → "a\n"
    assert_eq!(new_buf.to_string(), "a\n");
    // Primary cursor must land at char 0 (start of merged "a" line).
    assert_eq!(
        new_sels.primary().head(),
        0,
        "cursor must land at merged-line start"
    );
}
