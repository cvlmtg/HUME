use super::*;
use crate::assert_state;
use pretty_assertions::assert_eq;

// ── insert_char ───────────────────────────────────────────────────────────

#[test]
fn insert_char_at_cursor_start() {
    // Cursor on 'h'; 'x' inserted before it; cursor advances to 'h'.
    assert_state!(
        "-[h]>ello\n",
        |(buf, sels)| insert_char(buf, sels, 'x'),
        "x-[h]>ello\n"
    );
}

#[test]
fn insert_char_at_cursor_middle() {
    // Cursor on second 'l' (offset 3); 'x' inserted, cursor on 'l'.
    assert_state!(
        "hel-[l]>o\n",
        |(buf, sels)| insert_char(buf, sels, 'x'),
        "helx-[l]>o\n"
    );
}

#[test]
fn insert_char_at_cursor_eof() {
    // Cursor at EOF (offset 5); 'x' appended; cursor at new EOF.
    assert_state!(
        "hello-[\n]>",
        |(buf, sels)| insert_char(buf, sels, 'x'),
        "hellox-[\n]>"
    );
}

#[test]
fn insert_char_into_empty_buffer() {
    assert_state!(
        "-[\n]>",
        |(buf, sels)| insert_char(buf, sels, 'x'),
        "x-[\n]>"
    );
}

#[test]
fn insert_char_replaces_forward_selection() {
    // Selection anchor=0, head=3 covers 'h','e','l','l' (4 chars).
    // Delete [0,4), insert 'x', cursor at 1.
    assert_state!(
        "-[hell]>o\n",
        |(buf, sels)| insert_char(buf, sels, 'x'),
        "x-[o]>\n"
    );
}

#[test]
fn insert_char_replaces_selection_grapheme_base() {
    // Selection head lands on the base codepoint 'e' of {e\u{0301}} = é.
    // The fix extends the delete to include the combining mark, so typing
    // 'Z' fully replaces "café" rather than leaving an orphaned accent.
    // Text: "cafe\u{0301} x\n". Selection anchor=0, head=3 ('e').
    // Result: chars 0-4 deleted, 'Z' inserted → "Z x\n", cursor at 1 (' ').
    assert_state!(
        "-[cafe]>\u{0301} x\n",
        |(buf, sels)| insert_char(buf, sels, 'Z'),
        "Z-[ ]>x\n"
    );
}

#[test]
fn insert_char_replaces_whole_buffer() {
    assert_state!(
        "-[hello]>\n",
        |(buf, sels)| insert_char(buf, sels, 'x'),
        "x-[\n]>"
    );
}

#[test]
fn insert_char_replaces_backward_selection() {
    // anchor=3, head=0 covers chars 0-3 ('h','e','l','l') — "hell" (4 chars).
    // Delete [0,4), insert 'x' at 0, cursor at 1.
    // Text "hello" → remove "hell" → "o", insert 'x' → "xo".
    assert_state!(
        "<[hell]-o\n",
        |(buf, sels)| insert_char(buf, sels, 'x'),
        "x-[o]>\n"
    );
}

#[test]
fn insert_char_two_cursors() {
    // Cursors at 0 and 3. Insert 'x' at both positions.
    // Changeset: Insert("x"), Retain(3), Insert("x"), Retain(4).
    // Result: "xfoox bar", cursors at 1 and 5.
    assert_state!(
        "-[f]>oo-[ ]>bar\n",
        |(buf, sels)| insert_char(buf, sels, 'x'),
        "x-[f]>oox-[ ]>bar\n"
    );
}

#[test]
fn insert_char_unicode() {
    // Insert a multi-byte char (2 bytes in UTF-8, 1 char offset).
    assert_state!(
        "caf-[é]>\n",
        |(buf, sels)| insert_char(buf, sels, 'à'),
        "cafà-[é]>\n"
    );
}

// ── delete_char_forward ───────────────────────────────────────────────────

#[test]
fn delete_forward_at_cursor_start() {
    // Cursor on 'h'; deletes 'h'; cursor stays at 0 (now on 'e').
    assert_state!(
        "-[h]>ello\n",
        |(buf, sels)| delete_char_forward(buf, sels),
        "-[e]>llo\n"
    );
}

#[test]
fn delete_forward_at_cursor_middle() {
    assert_state!(
        "h-[e]>llo\n",
        |(buf, sels)| delete_char_forward(buf, sels),
        "h-[l]>lo\n"
    );
}

#[test]
fn delete_forward_at_eof_is_noop() {
    assert_state!(
        "hello-[\n]>",
        |(buf, sels)| delete_char_forward(buf, sels),
        "hello-[\n]>"
    );
}

#[test]
fn delete_forward_empty_buffer_is_noop() {
    assert_state!(
        "-[\n]>",
        |(buf, sels)| delete_char_forward(buf, sels),
        "-[\n]>"
    );
}

#[test]
fn delete_forward_selection() {
    // Selection [0,3] inclusive → remove [0,4) → "o", cursor at 0.
    assert_state!(
        "-[hell]>o\n",
        |(buf, sels)| delete_char_forward(buf, sels),
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
        |(buf, sels)| delete_char_forward(buf, sels),
        "-[e]>-[l]>o\n"
    );
}

#[test]
fn delete_forward_adjacent_cursors_merge() {
    // Cursors at 2 and 3. Both delete forward; both land at 2 → merge.
    assert_state!(
        "he-[l]>-[l]>o\n",
        |(buf, sels)| delete_char_forward(buf, sels),
        "he-[o]>\n"
    );
}

#[test]
fn delete_forward_grapheme_cluster() {
    // "e\u{0301}x": é is 2 chars, 1 grapheme. Cursor at 0 deletes whole cluster.
    assert_state!(
        "-[e\u{0301}]>x\n",
        |(buf, sels)| delete_char_forward(buf, sels),
        "-[x]>\n"
    );
}

// ── delete_char_backward ─────────────────────────────────────────────────

#[test]
fn delete_backward_at_cursor_end() {
    // Cursor at EOF (offset 5); backspace deletes 'o'; cursor at 4.
    assert_state!(
        "hello-[\n]>",
        |(buf, sels)| delete_char_backward(buf, sels),
        "hell-[\n]>"
    );
}

#[test]
fn delete_backward_at_cursor_middle() {
    // Cursor at 3 ('l'); backspace deletes 'l' at 2; cursor at 2.
    assert_state!(
        "hel-[l]>o\n",
        |(buf, sels)| delete_char_backward(buf, sels),
        "he-[l]>o\n"
    );
}

#[test]
fn delete_backward_at_start_is_noop() {
    assert_state!(
        "-[h]>ello\n",
        |(buf, sels)| delete_char_backward(buf, sels),
        "-[h]>ello\n"
    );
}

#[test]
fn delete_backward_empty_buffer_is_noop() {
    assert_state!(
        "-[\n]>",
        |(buf, sels)| delete_char_backward(buf, sels),
        "-[\n]>"
    );
}

#[test]
fn delete_backward_selection() {
    // Same as delete_forward for multi-char selections: removes selected region.
    assert_state!(
        "-[hell]>o\n",
        |(buf, sels)| delete_char_backward(buf, sels),
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
        |(buf, sels)| delete_char_backward(buf, sels),
        "h-[l]>-[o]>\n"
    );
}

#[test]
fn delete_backward_grapheme_cluster() {
    // "e\u{0301}x": é is 2 chars (offsets 0-1). Cursor at 2 (on 'x').
    // prev_grapheme_boundary(2) = 0. Deletes entire é cluster.
    assert_state!(
        "e\u{0301}-[x]>\n",
        |(buf, sels)| delete_char_backward(buf, sels),
        "-[x]>\n"
    );
}

#[test]
fn delete_backward_adjacent_cursors_merge() {
    // Cursors at 2 and 3. Backspace at 2: delete offset 1. Backspace at 3:
    // delete offset 2 in original. Both cursors land at 1 → merge.
    assert_state!(
        "he-[l]>-[l]>o\n",
        |(buf, sels)| delete_char_backward(buf, sels),
        "h-[l]>o\n"
    );
}

// ── delete_selection ──────────────────────────────────────────────────────

#[test]
fn delete_selection_cursor_deletes_char() {
    // Cursor on 'h' — deletes 'h'; cursor lands on 'e' (what was next).
    assert_state!(
        "-[h]>ello\n",
        |(buf, sels)| delete_selection(buf, sels),
        "-[e]>llo\n"
    );
}

#[test]
fn delete_selection_cursor_at_end_of_word() {
    // Cursor on 'o' (last word char) — deletes 'o'; cursor lands on '\n'.
    assert_state!(
        "hell-[o]>\n",
        |(buf, sels)| delete_selection(buf, sels),
        "hell-[\n]>"
    );
}

#[test]
fn delete_selection_cursor_on_structural_newline_is_noop() {
    // Cursor on the trailing '\n' — buffer invariant, no-op.
    assert_state!(
        "hello-[\n]>",
        |(buf, sels)| delete_selection(buf, sels),
        "hello-[\n]>"
    );
}

#[test]
fn delete_selection_empty_buffer_is_noop() {
    // Only the structural '\n' — cursor is on it, no-op.
    assert_state!(
        "-[\n]>",
        |(buf, sels)| delete_selection(buf, sels),
        "-[\n]>"
    );
}

#[test]
fn delete_selection_multi_char_forward() {
    // Forward selection covering "hell" — cursor lands at start (pos 0).
    assert_state!(
        "-[hell]>o\n",
        |(buf, sels)| delete_selection(buf, sels),
        "-[o]>\n"
    );
}

#[test]
fn delete_selection_multi_char_backward() {
    // Backward selection — same result as forward; cursor lands at start.
    assert_state!(
        "<[hell]-o\n",
        |(buf, sels)| delete_selection(buf, sels),
        "-[o]>\n"
    );
}

#[test]
fn delete_selection_two_cursors() {
    // Cursors on 'h' (pos 0) and 'l' (pos 2) — both deleted independently.
    assert_state!(
        "-[h]>el-[l]>o\n",
        |(buf, sels)| delete_selection(buf, sels),
        "-[e]>l-[o]>\n"
    );
}

#[test]
fn delete_selection_adjacent_selections_merge_cursors() {
    // Cursors on 'h' (0) and 'e' (1) — after deleting both, cursors both
    // land at 0 and merge into one.
    assert_state!(
        "-[h]>-[e]>llo\n",
        |(buf, sels)| delete_selection(buf, sels),
        "-[l]>lo\n"
    );
}

#[test]
fn delete_selection_grapheme_cluster() {
    // "e\u{0301}" is 2 chars (e + combining acute) but one grapheme cluster.
    // Cursor on 'e' (pos 0) deletes the entire cluster (both chars).
    assert_state!(
        "-[e]>\u{0301}x\n",
        |(buf, sels)| delete_selection(buf, sels),
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
        |(buf, sels)| delete_selection(buf, sels),
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
        |(buf, sels)| delete_selection(buf, sels),
        "-[f]>oo\n"
    );
}

#[test]
fn delete_selection_last_line_with_empty_preceding_line() {
    // "foo\n\nbar\n": delete last line → "foo\n\n", cursor on the now-last
    // empty line ('\n' at pos 4).
    assert_state!(
        "foo\n\n-[bar\n]>",
        |(buf, sels)| delete_selection(buf, sels),
        "foo\n-[\n]>"
    );
}

#[test]
fn delete_selection_last_line_single_line_still_empties() {
    // Single-line buffer "foo\n": selection [0,3] — no preceding line, so the
    // normal cap applies: only content deleted, structural '\n' kept.
    assert_state!(
        "-[foo\n]>",
        |(buf, sels)| delete_selection(buf, sels),
        "-[\n]>"
    );
}

#[test]
fn delete_selection_whole_buffer_caps_at_last_content_char() {
    // "foo\nbar\n", selection [0,7]: start==0, no preceding line, normal cap.
    // "foo\n" is not at line boundary for preceding detection (start==0).
    assert_state!(
        "-[foo\nbar\n]>",
        |(buf, sels)| delete_selection(buf, sels),
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
        |(buf, sels)| delete_selection(buf, sels),
        "foo\nb-[\n]>"
    );
}

#[test]
fn delete_selection_three_lines_delete_last() {
    // "a\nb\nc\n": x selects last line [4,6] (anchor 4 'c', head 6 '\n').
    // After delete: "a\nb\n", cursor at start of "b" line (pos 2).
    assert_state!(
        "a\nb\n-[c\n]>",
        |(buf, sels)| delete_selection(buf, sels),
        "a\n-[b]>\n"
    );
}

#[test]
fn delete_selection_last_line_multi_cursor_cursor_lands_at_merged_line_start() {
    // Regression: multi-cursor dd-on-last-line. The first cursor deletes 'b'
    // (char 1), advancing b.old_pos. The second cursor covers the whole last
    // line "c\n" [anchor=3, head=4]. The cursor produced for that deletion must
    // land at char 0 (start of the merged "a" line), not at char 1.
    //
    // Old code computed: cursor_new = b.new_pos() + cursor_line_start.saturating_sub(b.old_pos())
    //   = 1 + (0.saturating_sub(2)) = 1 + 0 = 1  ← wrong
    // New code: col_in_line = del_start - line_to_char(prev_line) = 2 - 0 = 2
    //           retain(0); cursor_new = b.new_pos().saturating_sub(2) = 1 - 2 = 0 ✓
    use crate::ops::edit::delete_selection;
    use hume_editing::selection::SelectionSet;
    let buf = hume_editing::text::Text::from("ab\nc\n");
    // primary=1 so the last-line selection is the primary; we assert its cursor.
    let sels = SelectionSet::from_vec(
        vec![
            hume_editing::selection::Selection::collapsed(1), // on 'b'
            hume_editing::selection::Selection::new(3, 4),    // last line "c\n"
        ],
        1, // primary is the last-line cursor
    );
    let (new_buf, new_sels, _cs) = delete_selection(buf, sels);
    // 'b' deleted and "c\n" merged into preceding line → "a\n"
    assert_eq!(new_buf.to_string(), "a\n");
    // Primary cursor must land at char 0 (start of merged "a" line).
    assert_eq!(
        new_sels.primary().head(),
        0,
        "cursor must land at merged-line start"
    );
}

// ── paste_after ───────────────────────────────────────────────────────────

fn pa(buf: Text, sels: SelectionSet, values: &[String]) -> (Text, SelectionSet, ChangeSet) {
    paste_after(buf, sels, values)
}

fn pb(buf: Text, sels: SelectionSet, values: &[String]) -> (Text, SelectionSet, ChangeSet) {
    paste_before(buf, sels, values)
}

#[test]
fn paste_after_single_cursor() {
    // Cursor on 'h' — insert "XY" after 'h'; selection covers "XY".
    assert_state!(
        "-[h]>ello\n",
        |(buf, sels)| pa(buf, sels, &["XY".to_string()]),
        "h-[XY]>ello\n"
    );
}

#[test]
fn paste_after_mid_word() {
    // Cursor on 'e' (pos 1) — insert "XY" after 'e'; selection covers "XY".
    assert_state!(
        "h-[e]>llo\n",
        |(buf, sels)| pa(buf, sels, &["XY".to_string()]),
        "he-[XY]>llo\n"
    );
}

#[test]
fn paste_after_cursor_on_structural_newline() {
    // Cursor on the trailing '\n' — insertion is clamped to pos 5 (before '\n').
    // "hello\n" → "helloXY\n"; cursor lands on 'Y' (pos 6).
    assert_state!(
        "hello-[\n]>",
        |(buf, sels)| pa(buf, sels, &["XY".to_string()]),
        "hello-[XY]>\n"
    );
}

#[test]
fn paste_after_two_cursors_n_to_n() {
    // Two cursors (pos 0 and 4); two values — each cursor gets its own slot.
    assert_state!(
        "-[h]>ell-[o]>\n",
        |(buf, sels)| pa(buf, sels, &["AB".to_string(), "CD".to_string()]),
        "h-[AB]>ello-[CD]>\n"
    );
}

#[test]
fn paste_after_count_mismatch_uses_joined() {
    // 2 cursors, 1 value → both cursors get the full "XY".
    assert_state!(
        "-[h]>ell-[o]>\n",
        |(buf, sels)| pa(buf, sels, &["XY".to_string()]),
        "h-[XY]>ello-[XY]>\n"
    );
}

#[test]
fn paste_after_unicode() {
    // Paste a string with a combining character. Cursor lands on last char.
    assert_state!(
        "-[h]>i\n",
        |(buf, sels)| pa(buf, sels, &["e\u{0301}".to_string()]),
        "h-[e\u{0301}]>i\n"
    );
}

#[test]
fn paste_after_replaces_forward_selection() {
    // Multi-char selection "hel" is replaced by "XY". Selection covers "XY".
    assert_state!(
        "-[hel]>lo\n",
        |(buf, sels)| pa(buf, sels, &["XY".to_string()]),
        "-[XY]>lo\n"
    );
}

#[test]
fn paste_after_replaces_backward_selection() {
    // Direction doesn't matter for replace — same result as forward.
    assert_state!(
        "<[hel]-lo\n",
        |(buf, sels)| pa(buf, sels, &["XY".to_string()]),
        "-[XY]>lo\n"
    );
}

#[test]
fn paste_after_replace_multi_cursor_n_to_n() {
    // Two non-cursor selections; two values — each replaced independently.
    // "-[he]>l-[lo]>\n": "he" replaced by "AB", "lo" replaced by "CD".
    assert_state!(
        "-[he]>l-[lo]>\n",
        |(buf, sels)| pa(buf, sels, &["AB".to_string(), "CD".to_string()]),
        "-[AB]>l-[CD]>\n"
    );
}

#[test]
fn paste_after_mixed_cursor_and_selection() {
    // One cursor (inserts) + one multi-char selection (replaces).
    // "-[h]>el-[lo]>\n": cursor at 'h' inserts "AB" after it; "lo" is replaced by "CD".
    assert_state!(
        "-[h]>el-[lo]>\n",
        |(buf, sels)| pa(buf, sels, &["AB".to_string(), "CD".to_string()]),
        "h-[AB]>el-[CD]>\n"
    );
}

#[test]
fn paste_after_empty_string_cursor_is_noop() {
    assert_state!(
        "-[h]>ello\n",
        |(buf, sels)| pa(buf, sels, &["".to_string()]),
        "-[h]>ello\n"
    );
}

#[test]
fn paste_after_empty_string_over_selection_deletes_and_lands_at_start() {
    // Empty text with a multi-char selection: the selection is deleted,
    // cursor lands at the start of the deleted region.
    assert_state!(
        "-[hel]>lo\n",
        |(buf, sels)| pa(buf, sels, &["".to_string()]),
        "-[l]>o\n"
    );
}

// ── paste_before ──────────────────────────────────────────────────────────

#[test]
fn paste_before_single_cursor() {
    // Cursor on 'h' — insert "XY" before 'h'; selection covers "XY".
    assert_state!(
        "-[h]>ello\n",
        |(buf, sels)| pb(buf, sels, &["XY".to_string()]),
        "-[XY]>hello\n"
    );
}

#[test]
fn paste_before_mid_word() {
    // Cursor on 'e' (pos 1) — insert "XY" before 'e'; selection covers "XY".
    assert_state!(
        "h-[e]>llo\n",
        |(buf, sels)| pb(buf, sels, &["XY".to_string()]),
        "h-[XY]>ello\n"
    );
}

#[test]
fn paste_before_two_cursors_n_to_n() {
    // Two cursors; two values — each cursor gets its own slot.
    // Text after: AB + hell + CD + o + \n; each selection covers its value.
    assert_state!(
        "-[h]>ell-[o]>\n",
        |(buf, sels)| pb(buf, sels, &["AB".to_string(), "CD".to_string()]),
        "-[AB]>hell-[CD]>o\n"
    );
}

#[test]
fn paste_before_count_mismatch_uses_joined() {
    // 2 cursors, 1 value → both cursors get the full "XY".
    assert_state!(
        "-[h]>ell-[o]>\n",
        |(buf, sels)| pb(buf, sels, &["XY".to_string()]),
        "-[XY]>hell-[XY]>o\n"
    );
}

#[test]
fn paste_before_replaces_selection() {
    // Multi-char selection — paste_before also replaces (same as paste_after for selections).
    assert_state!(
        "-[hel]>lo\n",
        |(buf, sels)| pb(buf, sels, &["XY".to_string()]),
        "-[XY]>lo\n"
    );
}

// ── paste empty-values (no-op path) ──────────────────────────────────────

#[test]
fn paste_after_empty_values_is_noop() {
    let (buf, sels) = crate::testing::parse_state("-[h]>ello\n");
    let buf_str = buf.to_string();
    let (new_buf, new_sels, _cs) = paste_after(buf, sels.clone(), &[]);
    assert_eq!(new_buf.to_string(), buf_str);
    assert_eq!(new_sels, sels);
}

#[test]
fn paste_before_empty_values_is_noop() {
    let (buf, sels) = crate::testing::parse_state("-[h]>ello\n");
    let buf_str = buf.to_string();
    let (new_buf, new_sels, _cs) = paste_before(buf, sels.clone(), &[]);
    assert_eq!(new_buf.to_string(), buf_str);
    assert_eq!(new_sels, sels);
}

// ── repeat_edit (count prefix for edits) ──────────────────────────────────

#[test]
fn repeat_delete_forward_count_3() {
    // 3x: delete 'h', then 'e', then 'l' — cursor lands on the second 'l'.
    assert_state!(
        "-[h]>ello\n",
        |(buf, sels)| repeat_edit(3, buf, sels, delete_char_forward),
        "-[l]>o\n"
    );
}

#[test]
fn repeat_delete_forward_count_exceeds_buffer() {
    // count=100 on a 3-char buffer ("hi\n"). Deletes 'h' and 'i', then
    // 98 no-ops on the structural '\n' (cannot be deleted).
    assert_state!(
        "-[h]>i\n",
        |(buf, sels)| repeat_edit(100, buf, sels, delete_char_forward),
        "-[\n]>"
    );
}

#[test]
fn repeat_delete_backward_count_2() {
    // 2<BS>: delete 'l' (offset 3), then 'e' (offset 2) from "hello\n".
    // Cursor was on 'l'(3); after first delete it sits on 'l'(2→now 'l'),
    // after second delete it sits on 'l' which is now at offset 2.
    assert_state!(
        "hel-[l]>o\n",
        |(buf, sels)| repeat_edit(2, buf, sels, delete_char_backward),
        "h-[l]>o\n"
    );
}

// ── insert_char edge cases ────────────────────────────────────────────────

#[test]
fn insert_char_newline() {
    // Inserting '\n' is mechanically identical to any other char: it goes
    // before the cursor character, cursor stays on the original char (now shifted).
    assert_state!(
        "-[h]>ello\n",
        |(buf, sels)| insert_char(buf, sels, '\n'),
        "\n-[h]>ello\n"
    );
}

#[test]
fn insert_char_combining_codepoint() {
    // Inserting a bare combining accent (U+0301) before 'h'. Mechanically
    // fine — the accent is stored as its own codepoint at position 0, and
    // the cursor lands on 'h' (now at position 1).
    assert_state!(
        "-[h]>ello\n",
        |(buf, sels)| insert_char(buf, sels, '\u{0301}'),
        "\u{0301}-[h]>ello\n"
    );
}

// ── paste with multiline text ─────────────────────────────────────────────

#[test]
fn paste_after_multiline_text() {
    // Paste "foo\nbar" after 'h'. Text: "h" + "foo\nbar" + "ello\n".
    // Selection covers the entire pasted span "foo\nbar".
    assert_state!(
        "-[h]>ello\n",
        |(buf, sels)| pa(buf, sels, &["foo\nbar".to_string()]),
        "h-[foo\nbar]>ello\n"
    );
}

#[test]
fn paste_before_multiline_text() {
    // Paste "foo\nbar" before 'h'. Text: "foo\nbar" + "hello\n".
    // Selection covers the entire pasted span "foo\nbar".
    assert_state!(
        "-[h]>ello\n",
        |(buf, sels)| pb(buf, sels, &["foo\nbar".to_string()]),
        "-[foo\nbar]>hello\n"
    );
}

// ── linewise paste (content ending in '\n') ───────────────────────────────

#[test]
fn paste_after_linewise_cursor_inserts_below() {
    // Cursor on 'e' (line 0 of "hello\nworld\n"). Paste "X\n" → new line
    // below line 0. Buffer becomes "hello\nX\nworld\n"; selection covers "X\n".
    assert_state!(
        "h-[e]>llo\nworld\n",
        |(buf, sels)| pa(buf, sels, &["X\n".to_string()]),
        "hello\n-[X\n]>world\n"
    );
}

#[test]
fn paste_before_linewise_cursor_inserts_above() {
    // Cursor on 'e' (line 0). Paste "X\n" before → new line above line 0.
    // Buffer becomes "X\nhello\nworld\n"; selection covers "X\n".
    assert_state!(
        "h-[e]>llo\nworld\n",
        |(buf, sels)| pb(buf, sels, &["X\n".to_string()]),
        "-[X\n]>hello\nworld\n"
    );
}

#[test]
fn paste_after_linewise_cursor_on_last_line_preserves_invariant() {
    // Cursor on 'l' (line 1, the last content line). Paste "X\n" after →
    // appended as new last line. Buffer becomes "hello\nworld\nX\n"; selection
    // covers "X\n" (including the structural trailing newline).
    assert_state!(
        "hello\nwor-[l]>d\n",
        |(buf, sels)| pa(buf, sels, &["X\n".to_string()]),
        "hello\nworld\n-[X\n]>"
    );
}

#[test]
fn paste_after_linewise_multiline_content() {
    // Paste "a\nb\n" (two lines) after cursor on line 0 → two new lines below.
    // Buffer becomes "hello\na\nb\nworld\n"; selection covers "a\nb\n".
    assert_state!(
        "h-[e]>llo\nworld\n",
        |(buf, sels)| pa(buf, sels, &["a\nb\n".to_string()]),
        "hello\n-[a\nb\n]>world\n"
    );
}

#[test]
fn paste_after_linewise_over_full_line_selection() {
    // Full-line selection (head on '\n') — before and after both empty →
    // three-way collapses to just the pasted line.
    assert_state!(
        "-[hello\n]>world\n",
        |(buf, sels)| pa(buf, sels, &["X\n".to_string()]),
        "-[X\n]>world\n"
    );
}

#[test]
fn paste_after_linewise_over_content_selection_only() {
    // Selection covers all content but NOT the '\n' — both before and after empty →
    // three-way collapses to just the pasted line.
    assert_state!(
        "-[hello]>\nworld\n",
        |(buf, sels)| pa(buf, sels, &["X\n".to_string()]),
        "-[X\n]>world\n"
    );
}

// ── linewise three-way split over partial selection ───────────────────────

#[test]
fn paste_after_linewise_three_way_split_both_sides() {
    // Partial selection "ell" within "hello" — before="h", after="o".
    // Three-way split produces h / X / o on separate lines.
    assert_state!(
        "h-[ell]>o\nworld\n",
        |(buf, sels)| pa(buf, sels, &["X\n".to_string()]),
        "h\n-[X\n]>o\nworld\n"
    );
}

#[test]
fn paste_after_linewise_three_way_split_empty_before() {
    // Selection starts at column 0 — before is empty, after="o".
    // Two-part: X / o
    assert_state!(
        "-[ell]>o\nworld\n",
        |(buf, sels)| pa(buf, sels, &["X\n".to_string()]),
        "-[X\n]>o\nworld\n"
    );
}

#[test]
fn paste_after_linewise_three_way_split_empty_after() {
    // Selection ends just before the '\n' — before="h", after empty.
    // Two-part: h / X
    assert_state!(
        "h-[ello]>\nworld\n",
        |(buf, sels)| pa(buf, sels, &["X\n".to_string()]),
        "h\n-[X\n]>world\n"
    );
}

// ── linewise paste: two non-collapsed selections on the same line ─────────

#[test]
fn paste_after_linewise_two_selections_same_line_each_replaced() {
    // Two non-collapsed selections on the same line ("he" and "lo").
    // Each selected fragment is replaced independently by the pasted line;
    // the unselected gap ("l") is retained and pushed onto its own line by the
    // pasted '\n'. Both pasted ranges are selected; they are distinct, so no
    // merge occurs.
    assert_state!(
        "-[he]>l-[lo]>\n",
        |(buf, sels)| pa(buf, sels, &["X\n".to_string()]),
        "-[X\n]>l\n-[X\n]>"
    );
    // Two-line buffer: same invariant; "world" line untouched.
    assert_state!(
        "-[he]>l-[lo]>\nworld\n",
        |(buf, sels)| pa(buf, sels, &["X\n".to_string()]),
        "-[X\n]>l\n-[X\n]>world\n"
    );
}

// ── linewise paste_before coverage ───────────────────────────────────────

#[test]
fn paste_before_linewise_cursor_on_last_line() {
    // Cursor on 'l' (line 1, the last content line). Paste "X\n" before →
    // inserted above line 1. Buffer becomes "hello\nX\nworld\n".
    assert_state!(
        "hello\nwor-[l]>d\n",
        |(buf, sels)| pb(buf, sels, &["X\n".to_string()]),
        "hello\n-[X\n]>world\n"
    );
}

#[test]
fn paste_before_linewise_multiline_content() {
    // Paste "a\nb\n" (two lines) before cursor on line 0 → two new lines above.
    // Buffer becomes "a\nb\nhello\nworld\n"; selection covers "a\nb\n".
    assert_state!(
        "h-[e]>llo\nworld\n",
        |(buf, sels)| pb(buf, sels, &["a\nb\n".to_string()]),
        "-[a\nb\n]>hello\nworld\n"
    );
}

#[test]
fn paste_before_linewise_over_full_line_selection() {
    // Full-line selection — three-way split with both sides empty; identical
    // to paste_after for non-collapsed selections.
    assert_state!(
        "-[hello\n]>world\n",
        |(buf, sels)| pb(buf, sels, &["X\n".to_string()]),
        "-[X\n]>world\n"
    );
}

#[test]
fn paste_before_linewise_three_way_split_both_sides() {
    // Partial selection "ell" within "hello" — before="h", after="o".
    assert_state!(
        "h-[ell]>o\nworld\n",
        |(buf, sels)| pb(buf, sels, &["X\n".to_string()]),
        "h\n-[X\n]>o\nworld\n"
    );
}

#[test]
fn paste_before_linewise_two_selections_same_line_each_replaced() {
    // Two non-collapsed selections on the same line — each replaced independently;
    // the before/after distinction only applies to cursor selections, so the result
    // is identical to paste_after for non-collapsed selections.
    assert_state!(
        "-[he]>l-[lo]>\n",
        |(buf, sels)| pb(buf, sels, &["X\n".to_string()]),
        "-[X\n]>l\n-[X\n]>"
    );
}

// ── linewise paste: overlapping line ranges (multi-line selections) ───────

#[test]
fn paste_after_linewise_overlapping_line_ranges_each_replaced() {
    // Selection 1: "c\nx" — spans lines 0-1 (positions 2-4 in "abc\nxyz\nfoo\n").
    // Selection 2: "z\nf" — spans lines 1-2 (positions 6-8).
    // Each selection is replaced independently: sel1 replaces "c\nx", retaining
    // "ab" (emitted on its own line via the prefix '\n'). Gap "y" (between sel1
    // end and sel2 start) is retained on its own line. Sel2 replaces "z\nf",
    // retaining "oo". Both pasted "X\n" ranges are selected.
    use hume_editing::selection::{Selection, SelectionSet};
    // parse_state requires at least one selection marker; we ignore the returned sels.
    let (buf, _) = crate::testing::parse_state("-[a]>bc\nxyz\nfoo\n");
    let sels = SelectionSet::from_vec(
        vec![
            Selection::new(2, 4), // "c\nx" — first_line=0, last_line=1
            Selection::new(6, 8), // "z\nf" — first_line=1, last_line=2
        ],
        0,
    );
    let (new_buf, _new_sels, _cs) = pa(buf, sels, &["X\n".to_string()]);
    let result = new_buf.to_string();
    assert_eq!(
        result, "ab\nX\ny\nX\noo\n",
        "each selection replaced; gaps on own lines"
    );
    // "xy" must not appear — "x" was part of sel1, "z" was part of sel2, only "y" survives.
    assert!(
        !result.contains("xy"),
        "sel1 content must not leak into gap"
    );
}

// ── repeat_edit count=0 ───────────────────────────────────────────────────

#[test]
fn repeat_edit_count_zero_is_noop() {
    // count=0 produces an identity ChangeSet and leaves buf+sels unchanged.
    assert_state!(
        "-[h]>ello\n",
        |(buf, sels)| repeat_edit(0, buf, sels, delete_char_forward),
        "-[h]>ello\n"
    );
}

// ── yank → paste round-trip ───────────────────────────────────────────────

#[test]
fn yank_then_paste_after_round_trip() {
    use crate::ops::register::yank_selections;
    let (buf, sels) = crate::testing::parse_state("-[h]>ello\n");
    let yanked = yank_selections(&buf, &sels);
    assert_eq!(yanked, vec!["h"], "yank captures the cursor char");

    assert_state!(
        "-[h]>ello\n",
        |(buf, sels)| {
            let values = yank_selections(&buf, &sels);
            pa(buf, sels, &values)
        },
        "h-[h]>ello\n"
    );
}

#[test]
fn yank_multi_cursor_then_paste_after_n_to_n() {
    use crate::ops::register::yank_selections;
    let (buf, sels) = crate::testing::parse_state("-[h]>ell-[o]>\n");
    let yanked = yank_selections(&buf, &sels);
    assert_eq!(yanked, vec!["h", "o"]);

    assert_state!(
        "-[h]>ell-[o]>\n",
        |(buf, sels)| {
            let values = yank_selections(&buf, &sels);
            pa(buf, sels, &values)
        },
        "h-[h]>ello-[o]>\n"
    );
}

// ── replace_selections ────────────────────────────────────────────────────

#[test]
fn replace_cursor_single_char() {
    // Cursor on 'h'; replace with 'x' → cursor stays on 'x'.
    assert_state!(
        "-[h]>ello\n",
        |(buf, sels)| replace_selections(buf, sels, 'x'),
        "-[x]>ello\n"
    );
}

#[test]
fn replace_cursor_middle() {
    // Cursor on 'l' at offset 2; replace with 'x'.
    assert_state!(
        "he-[l]>lo\n",
        |(buf, sels)| replace_selections(buf, sels, 'x'),
        "he-[x]>lo\n"
    );
}

#[test]
fn replace_cursor_on_structural_newline_is_noop() {
    // Structural trailing '\n' is skipped like any other '\n'.
    assert_state!(
        "hello-[\n]>",
        |(buf, sels)| replace_selections(buf, sels, 'x'),
        "hello-[\n]>"
    );
}

#[test]
fn replace_cursor_on_mid_buffer_newline_is_noop() {
    // Cursor on the '\n' between two lines — preserved, not replaced.
    assert_state!(
        "hello-[\n]>world\n",
        |(buf, sels)| replace_selections(buf, sels, 'x'),
        "hello-[\n]>world\n"
    );
}

#[test]
fn replace_empty_buffer_is_noop() {
    // Text is just the structural '\n'.
    assert_state!(
        "-[\n]>",
        |(buf, sels)| replace_selections(buf, sels, 'x'),
        "-[\n]>"
    );
}

#[test]
fn replace_forward_selection() {
    // Forward selection covers "hell" (offsets 0-3); replace each with 'x'.
    assert_state!(
        "-[hell]>o\n",
        |(buf, sels)| replace_selections(buf, sels, 'x'),
        "-[xxxx]>o\n"
    );
}

#[test]
fn replace_backward_selection() {
    // Backward selection anchor=3, head=0 covers "hell"; direction preserved.
    assert_state!(
        "<[hell]-o\n",
        |(buf, sels)| replace_selections(buf, sels, 'x'),
        "<[xxxx]-o\n"
    );
}

#[test]
fn replace_whole_line() {
    // Forward selection covers all content chars (not the structural '\n').
    assert_state!(
        "-[hello]>\n",
        |(buf, sels)| replace_selections(buf, sels, 'x'),
        "-[xxxxx]>\n"
    );
}

#[test]
fn replace_two_cursors() {
    // Two cursors; each independently replaced.
    assert_state!(
        "-[h]>ell-[o]>\n",
        |(buf, sels)| replace_selections(buf, sels, 'x'),
        "-[x]>ell-[x]>\n"
    );
}

#[test]
fn replace_two_selections() {
    // Two non-overlapping selections each get all their chars replaced.
    assert_state!(
        "-[he]>l-[lo]>\n",
        |(buf, sels)| replace_selections(buf, sels, 'x'),
        "-[xx]>l-[xx]>\n"
    );
}

#[test]
fn replace_grapheme_cluster_cursor() {
    // Cursor on 'é' (e + U+0301, 2 codepoints). Replaced with 'x' (1 codepoint).
    // Text shrinks by 1 char; cursor lands on 'x'.
    assert_state!(
        "caf-[e]>\u{0301}z\n",
        |(buf, sels)| replace_selections(buf, sels, 'x'),
        "caf-[x]>z\n"
    );
}

#[test]
fn replace_multiline_selection_skips_newline() {
    // Selection spans two lines. The '\n' between them is retained;
    // only the visible characters are replaced. Lines stay separate.
    assert_state!(
        "-[hello\nworld]>\n",
        |(buf, sels)| replace_selections(buf, sels, 'x'),
        "-[xxxxx\nxxxxx]>\n"
    );
}

#[test]
fn replace_selection_including_structural_trailing_newline_preserves_newline() {
    // When the selection reaches the structural trailing '\n', that newline
    // must be preserved — replace_selections skips '\n' graphemes entirely.
    // Before the fix this path existed but had no explicit test.
    assert_state!(
        "-[hello\n]>",
        |(buf, sels)| replace_selections(buf, sels, 'x'),
        "-[xxxxx\n]>"
    );
}

// ── Smart replace (pair-aware) ───────────────────────────────────────────

#[test]
fn smart_replace_opening_bracket_to_opening() {
    // Two cursors on `(` and `)`, replace with `[` → `[` and `]`.
    assert_state!(
        "-[(]>hello-[)]>\n",
        |(buf, sels)| replace_selections(buf, sels, '['),
        "-[[]>hello-[]]>\n"
    );
}

#[test]
fn smart_replace_asym_to_sym() {
    // `(` and `)` replaced with `"` → both become `"`.
    assert_state!(
        "-[(]>hello-[)]>\n",
        |(buf, sels)| replace_selections(buf, sels, '"'),
        "-[\"]>hello-[\"]>\n"
    );
}

#[test]
fn smart_replace_sym_to_asym_uses_index() {
    // Two cursors on `"` and `"`, replace with `(` → `(` and `)`.
    assert_state!(
        "-[\"]>hello-[\"]>\n",
        |(buf, sels)| replace_selections(buf, sels, '('),
        "-[(]>hello-[)]>\n"
    );
}

#[test]
fn smart_replace_sym_to_sym() {
    // Two cursors on `"` and `"`, replace with `'` → both `'`.
    assert_state!(
        "-[\"]>hello-[\"]>\n",
        |(buf, sels)| replace_selections(buf, sels, '\''),
        "-[']>hello-[']>\n"
    );
}

#[test]
fn smart_replace_non_delimiter_is_literal() {
    // Cursor on `x`, replace with `[` → literal `[` (no smart logic).
    assert_state!(
        "-[x]>hello\n",
        |(buf, sels)| replace_selections(buf, sels, '['),
        "-[[]>hello\n"
    );
}

#[test]
fn smart_replace_range_selection_no_smart_logic() {
    // Range selection (not a cursor) — all chars become `[`, no smart logic.
    assert_state!(
        "-[(he]>llo)\n",
        |(buf, sels)| replace_selections(buf, sels, '['),
        "-[[[[]>llo)\n"
    );
}

#[test]
fn smart_replace_non_pair_replacement_is_literal() {
    // Replacement is not a pair char — always literal, even on delimiters.
    assert_state!(
        "-[(]>hello-[)]>\n",
        |(buf, sels)| replace_selections(buf, sels, 'x'),
        "-[x]>hello-[x]>\n"
    );
}

// ── join_lines_select_spaces ───────────────────────────────────────────────

#[test]
fn join_lines_cursor_on_line_joins_with_next() {
    // Cursor on '2' (line 2 of 5). Single-line selection → extends to next line.
    assert_state!(
        "1\n-[2]>\n3\n4\n5\n",
        |(buf, sels)| join_lines_select_spaces(buf, sels),
        "1\n2-[ ]>3\n4\n5\n"
    );
}

#[test]
fn join_lines_range_spans_two_lines_joins_them() {
    // Forward selection spanning lines 2-3 → joins them.
    assert_state!(
        "1\n-[2\n3]>\n4\n5\n",
        |(buf, sels)| join_lines_select_spaces(buf, sels),
        "1\n2-[ ]>3\n4\n5\n"
    );
}

#[test]
fn join_lines_range_spans_three_lines_joins_all() {
    // Forward selection spanning lines 2-3-4 → joins all three.
    assert_state!(
        "1\n-[2\n3\n4]>\n5\n",
        |(buf, sels)| join_lines_select_spaces(buf, sels),
        "1\n2-[ ]>3-[ ]>4\n5\n"
    );
}

#[test]
fn join_lines_two_disjoint_cursors_each_joins_independently() {
    // Two cursors: one on '2' (line 2), one on '4' (line 4).
    // Each joins its line with the next, independently.
    assert_state!(
        "1\n-[2]>\n3\n-[4]>\n5\n",
        |(buf, sels)| join_lines_select_spaces(buf, sels),
        "1\n2-[ ]>3\n4-[ ]>5\n"
    );
}

#[test]
fn join_lines_skips_empty_line_no_space() {
    // Cursor on line 2, next line is empty → no space inserted.
    assert_state!(
        "1\n-[2]>\n\n3\n",
        |(buf, sels)| join_lines_select_spaces(buf, sels),
        "1\n-[2]>\n3\n"
    );
}

#[test]
fn join_lines_cursor_on_last_line_noop() {
    // Cursor on the last line — nothing to join, buffer and cursor unchanged.
    assert_state!(
        "1\n2\n3\n4\n-[5]>\n",
        |(buf, sels)| join_lines_select_spaces(buf, sels),
        "1\n2\n3\n4\n-[5]>\n"
    );
}

// ── align_selections ──────────────────────────────────────────────────────────

#[test]
fn align_single_selection_noop() {
    // Only the primary: always at target col, no change.
    assert_state!(
        "foo -[=]> 1\n",
        |(buf, sels)| align_selections(buf, sels),
        "foo -[=]> 1\n"
    );
}

#[test]
fn align_forward_insert_spaces() {
    // Primary '=' at col 4 ("foo ="). Secondary '=' at col 3 ("fo =").
    // One space inserted before secondary to reach col 4.
    assert_state!(
        "foo -[=]> 1\nfo -[=]> 2\n",
        |(buf, sels)| align_selections(buf, sels),
        "foo -[=]> 1\nfo  -[=]> 2\n"
    );
}

#[test]
fn align_forward_multiple_spaces_inserted() {
    // Primary '=' at col 2 (on line "ab=c"). Secondary '=' at col 0 (on line "=de").
    // Two spaces inserted before secondary to reach col 2.
    // "foo = 1" has no selection — just buffer content.
    assert_state!(
        "foo = 1\nab-[=]>c\n-[=]>de\n",
        |(buf, sels)| align_selections(buf, sels),
        "foo = 1\nab-[=]>c\n  -[=]>de\n"
    );
}

#[test]
fn align_forward_two_secondaries_insert() {
    // Primary '=' at col 6 (first sel). Both secondaries need spaces.
    // "foobar = 1"   — primary '=' at col 7
    // "foo = 2"      — secondary '=' at col 4, needs 3 spaces
    // "fo = 3"       — secondary '=' at col 3, needs 4 spaces
    assert_state!(
        "foobar -[=]> 1\nfoo -[=]> 2\nfo -[=]> 3\n",
        |(buf, sels)| align_selections(buf, sels),
        "foobar -[=]> 1\nfoo    -[=]> 2\nfo     -[=]> 3\n"
    );
}

#[test]
fn align_forward_remove_spaces() {
    // Line 0 (primary): '=' at col 3, avail=1, rem=0. Floor = 3−0 = 3.
    // Line 1:           '=' at col 6, avail=3, rem=2. Floor = 6−2 = 4.
    // target[0] = max(baseline=3, max_floor=4) = 4.
    // Line 0: insert 1 space → col 4. Line 1: remove 2 spaces → col 4.
    assert_state!(
        "fo -[=]> 1\nfoo   -[=]> 2\n",
        |(buf, sels)| align_selections(buf, sels),
        "fo  -[=]> 1\nfoo -[=]> 2\n"
    );
}

#[test]
fn align_forward_clamped_removal_one_space_left() {
    // Line 0 (primary): '=' at col 2, avail=0, rem=0. Floor = 2.
    // Line 1:           '=' at col 7, avail=2, rem=1. Floor = 7−1 = 6.
    // target[0] = max(baseline=2, max_floor=6) = 6.
    // Line 0: insert 4 spaces → col 6. Line 1: remove 1 space → col 6.
    assert_state!(
        "ab-[=]>\nabcde  -[=]>\n",
        |(buf, sels)| align_selections(buf, sels),
        "ab    -[=]>\nabcde -[=]>\n"
    );
}

#[test]
fn align_clamped_exactly_one_space_available_removes_nothing() {
    // Line 0 (primary): '=' at col 3, avail=1, rem=0. Floor = 3.
    // Line 1:           '=' at col 4, avail=1, rem=0. Floor = 4.
    // target[0] = max(baseline=3, max_floor=4) = 4.
    // Line 0: insert 1 space → col 4. Line 1: amount=0 → unchanged.
    assert_state!(
        "fo -[=]>\nfoo -[=]>\n",
        |(buf, sels)| align_selections(buf, sels),
        "fo  -[=]>\nfoo -[=]>\n"
    );
}

#[test]
fn align_bidirectional_insert_and_remove() {
    // Primary (first sel) '=' at col 4.
    // Second sel '=' at col 3 — insert 1 space → col 4.
    // Third sel '=' at col 6 with 3 spaces before it — need -2, avail=3,
    // max_remove = N-1 = 2 → remove 2, col 4.
    assert_state!(
        "foo -[=]>\nfo -[=]>\nfoo   -[=]>\n",
        |(buf, sels)| align_selections(buf, sels),
        "foo -[=]>\nfo  -[=]>\nfoo -[=]>\n"
    );
}

#[test]
fn align_direction_preserved_forward() {
    // Forward selection spans multiple chars; direction preserved after align.
    assert_state!(
        "foo -[== ]> 1\nfo -[== ]> 2\n",
        |(buf, sels)| align_selections(buf, sels),
        "foo -[== ]> 1\nfo  -[== ]> 2\n"
    );
}

#[test]
fn align_backward_selection_right_aligns() {
    // Backward selection: anchor = right edge. Primary anchor at col 5.
    // "foo  = 1" — primary, backward '=' anchor at col 5.
    // "foo = 2"  — secondary, backward '=' anchor at col 4. Insert 1 space.
    assert_state!(
        "foo  <[=]- 1\nfoo <[=]- 2\n",
        |(buf, sels)| align_selections(buf, sels),
        "foo  <[=]- 1\nfoo  <[=]- 2\n"
    );
}

#[test]
fn align_multiline_passthrough() {
    // Primary '=' at col 4. Single-line secondary '=' at col 3 — gets +1 space.
    // Multiline "bar\nbaz" spans two lines — passed through unchanged, but its
    // buffer positions shift by +1 (the space inserted for the single-line sel).
    assert_state!(
        "foo -[=]>\nfo -[=]>\nfoo -[bar\nbaz]>\n",
        |(buf, sels)| align_selections(buf, sels),
        "foo -[=]>\nfo  -[=]>\nfoo -[bar\nbaz]>\n"
    );
}

#[test]
fn align_primary_unchanged() {
    // Primary selection itself is never modified (amount == 0).
    assert_state!(
        "foo -[=]>\nfoo -[=]>\n",
        |(buf, sels)| align_selections(buf, sels),
        "foo -[=]>\nfoo -[=]>\n"
    );
}

#[test]
fn align_remove_tab_before_selection() {
    // Regression: the old avail count used `== Some(' ')`, so a tab in the
    // whitespace run before a selection broke the reverse-scan chain early,
    // yielding avail=0 and silently skipping all removal.
    //
    // Buffer " =\n  \t=\n":
    //   Line 0: ' '+'=' (primary '=' at grapheme col 1)  → target_col=1
    //   Line 1: ' ',' ','\t','=' (secondary '=' at grapheme col 3) → amount=-2
    //
    // Old code: reverse scan hits '\t' first, stops, avail=0 → remove=0 (bug).
    // New code: '\t' matches, scan continues through spaces, avail=3,
    //           remove=min(2, 3-1)=2 → removes ' '+'\t' → '=' lands at col 1. ✓
    use crate::ops::edit::align_selections;
    use hume_editing::selection::SelectionSet;
    let buf = hume_editing::text::Text::from(" =\n  \t=\n");
    let sels = SelectionSet::from_vec(
        vec![
            hume_editing::selection::Selection::collapsed(1), // primary: '=' col 1
            hume_editing::selection::Selection::collapsed(6), // secondary: '=' col 3
        ],
        0,
    );
    let (new_buf, _new_sels, _cs) = align_selections(buf, sels);
    assert_eq!(
        new_buf.to_string(),
        " =\n =\n",
        "tab should count toward alignment removal"
    );
}

#[test]
fn align_two_columns_per_line() {
    // Multi-column: primary line has 2 selections defining 2 column targets.
    //
    // Input:
    //   "a b\n"  ← primary line: 'a' at col 0 (slot 0), 'b' at col 2 (slot 1)
    //   "xy  z\n" ←             'xy' at col 0 (slot 0), 'z' at col 4 (slot 1)
    //
    // baseline = [0, 2].
    // fit_need[1]: line 0 → target[0]+(acol_b−acol_a)−rem_b = 0+(2−0)−0 = 2
    //              line 1 → 0+(4−0)−1 = 3  ('z' has 2 spaces before it, rem=1).
    // target = [0, 3].
    //
    // Line 0: 'a' amount=0 (retain); 'b' amount=3-2=+1 → insert 1 space.
    // Line 1: 'xy' amount=0 (retain); 'z' amount=3-4=-1 → remove 1 space.
    assert_state!(
        "-[a]> -[b]>\n-[xy]>  -[z]>\n",
        |(buf, sels)| align_selections(buf, sels),
        "-[a]>  -[b]>\n-[xy]> -[z]>\n"
    );
}

#[test]
fn align_two_columns_overflow_widens_primary() {
    // Multi-column: another line's wider content forces target[1] past baseline,
    // so spaces are inserted on the primary line too (primary may move).
    //
    // Input:
    //   "x y\n"      ← primary line: 'x' col 0 (slot 0), 'y' col 2 (slot 1)
    //   "loooong z\n" ←              'loooong' col 0-6 (slot 0), 'z' col 8 (slot 1)
    //
    // baseline = [0, 2].
    // fit_need[1]: line 0 → 0+(2−0)−0=2; line 1 → 0+(8−0)−0=8  ('z' has 1
    //              space before it, rem=0 since avail=1 → rem=avail−1=0).
    // target = [0, max(2,8)=8].
    //
    // Line 0 (primary): 'x' retain; 'y' amount=8-2=+6 → inserts 6 spaces.
    // Line 1:           'loooong' retain; 'z' amount=8-8=0 → retain.
    assert_state!(
        "-[x]> -[y]>\n-[loooong]> -[z]>\n",
        |(buf, sels)| align_selections(buf, sels),
        "-[x]>       -[y]>\n-[loooong]> -[z]>\n"
    );
}

#[test]
fn align_two_columns_static_text_between() {
    // Regression: static non-selected text before slot 0 and between slots must
    // set the floor, not the selection edge geometry.
    //
    // Input (each '=' and '//' selected, primary on line 0's '='):
    //   "const foo -[=]> 444; -[//]> foo\n"
    //   "const foobar -[=]> 6757383; -[//]> bar\n"
    //   "const a -[=]> 34; -[//]> a\n"
    //
    // slot 0 anchors: line 0 = col 10, line 1 = col 13, line 2 = col 8.
    //   all rem=0 (no removable whitespace before '=').
    //   fit_0 = max(10−0, 13−0, 8−0) = 13. target[0] = max(10, 13) = 13.
    //
    // slot 1 anchors: line 0 = col 17, line 1 = col 24, line 2 = col 14.
    //   all rem=0.
    //   fit_1 = max(13+(17−10)−0, 13+(24−13)−0, 13+(14−8)−0) = max(20,24,19) = 24.
    //   target[1] = max(17, 24) = 24.
    //
    // Line 0: '=' @10 → insert 3 → col 13; '//' @17+3=20 → insert 4 → col 24.
    // Line 1: '=' @13 → amount=0;          '//' @24 → amount=0.
    // Line 2: '=' @8  → insert 5 → col 13; '//' @14+5=19 → insert 5 → col 24.
    use crate::ops::edit::align_selections;
    use hume_editing::{
        selection::{Selection, SelectionSet},
        text::Text,
    };
    let buf =
        Text::from("const foo = 444; // foo\nconst foobar = 6757383; // bar\nconst a = 34; // a\n");
    // char offsets (0-based):
    //   line 0: "const foo = 444; // foo\n"
    //            0123456789...
    //     '='  at char 10; '//' starts at char 17
    //   line 1: starts at char 24; "const foobar = 6757383; // bar\n"
    //     '='  at char 24+13=37; '//' at char 24+24=48
    //   line 2: starts at char 55; "const a = 34; // a\n"
    //     '='  at char 55+8=63;  '//' at char 55+14=69
    let sels = SelectionSet::from_vec(
        vec![
            Selection::collapsed(10), // primary: '=' on line 0
            Selection::collapsed(17), // '//' on line 0
            Selection::collapsed(37), // '=' on line 1
            Selection::collapsed(48), // '//' on line 1
            Selection::collapsed(63), // '=' on line 2
            Selection::collapsed(69), // '//' on line 2
        ],
        0,
    );
    let (new_buf, _new_sels, _cs) = align_selections(buf, sels);
    assert_eq!(
        new_buf.to_string(),
        "const foo    = 444;     // foo\nconst foobar = 6757383; // bar\nconst a      = 34;      // a\n",
        "columns must widen to clear the widest line's non-removable content"
    );
}

#[test]
fn align_extras_on_same_line_pass_through() {
    // When a non-primary line has more selections than the primary line (N=1 here),
    // the extra selections (slot >= N) pass through shifted by the accumulated
    // edit delta — selection count is preserved.
    //
    // Input:
    //   "foo x\n"  ← primary: only 'x' at col 4 (N=1, target[0]=4)
    //   "a b c\n"  ← slot 0='b' col 2 (→ Align(4)), slot 1='c' col 4 (→ Passthrough)
    //
    // 'b' shifts right by +2 → 'c' (Passthrough) also shifts right by +2.
    assert_state!(
        "foo -[x]>\na -[b]> -[c]>\n",
        |(buf, sels)| align_selections(buf, sels),
        "foo -[x]>\na   -[b]> -[c]>\n"
    );
}
