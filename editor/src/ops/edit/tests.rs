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

// ── paste_after ───────────────────────────────────────────────────────────

// Helper: call paste_after and discard changeset + replaced vec for assert_state!.
fn pa(buf: Text, sels: SelectionSet, values: &[String]) -> (Text, SelectionSet) {
    let (b, s, _, _) = paste_after(buf, sels, values);
    (b, s)
}

// Helper: call paste_before and discard changeset + replaced vec for assert_state!.
fn pb(buf: Text, sels: SelectionSet, values: &[String]) -> (Text, SelectionSet) {
    let (b, s, _, _) = paste_before(buf, sels, values);
    (b, s)
}

#[test]
fn paste_after_single_cursor() {
    // Cursor on 'h' — insert "XY" after 'h'; cursor lands on 'Y'.
    assert_state!(
        "-[h]>ello\n",
        |(buf, sels)| pa(buf, sels, &["XY".to_string()]),
        "hX-[Y]>ello\n"
    );
}

#[test]
fn paste_after_mid_word() {
    // Cursor on 'e' (pos 1) — insert "XY" after 'e'.
    assert_state!(
        "h-[e]>llo\n",
        |(buf, sels)| pa(buf, sels, &["XY".to_string()]),
        "heX-[Y]>llo\n"
    );
}

#[test]
fn paste_after_cursor_on_structural_newline() {
    // Cursor on the trailing '\n' — insertion is clamped to pos 5 (before '\n').
    // "hello\n" → "helloXY\n"; cursor lands on 'Y' (pos 6).
    assert_state!(
        "hello-[\n]>",
        |(buf, sels)| pa(buf, sels, &["XY".to_string()]),
        "helloX-[Y]>\n"
    );
}

#[test]
fn paste_after_two_cursors_n_to_n() {
    // Two cursors (pos 0 and 4); two values — each cursor gets its own slot.
    assert_state!(
        "-[h]>ell-[o]>\n",
        |(buf, sels)| pa(buf, sels, &["AB".to_string(), "CD".to_string()]),
        "hA-[B]>elloC-[D]>\n"
    );
}

#[test]
fn paste_after_count_mismatch_uses_joined() {
    // 2 cursors, 1 value → both cursors get the full "XY".
    assert_state!(
        "-[h]>ell-[o]>\n",
        |(buf, sels)| pa(buf, sels, &["XY".to_string()]),
        "hX-[Y]>elloX-[Y]>\n"
    );
}

#[test]
fn paste_after_unicode() {
    // Paste a string with a combining character. Cursor lands on last char.
    assert_state!(
        "-[h]>i\n",
        |(buf, sels)| pa(buf, sels, &["e\u{0301}".to_string()]),
        "he-[\u{0301}]>i\n"
    );
}

#[test]
fn paste_after_replaces_forward_selection() {
    // Multi-char selection "hel" is replaced by "XY". Cursor on 'Y'.
    // Replaced text "hel" is returned.
    assert_state!(
        "-[hel]>lo\n",
        |(buf, sels)| pa(buf, sels, &["XY".to_string()]),
        "X-[Y]>lo\n"
    );
    let (buf, sels) = crate::testing::parse_state("-[hel]>lo\n");
    let (_, _, _, replaced) = paste_after(buf, sels, &["XY".to_string()]);
    assert_eq!(replaced, vec!["hel"]);
}

#[test]
fn paste_after_replaces_backward_selection() {
    // Direction doesn't matter for replace — same result as forward.
    assert_state!(
        "<[hel]-lo\n",
        |(buf, sels)| pa(buf, sels, &["XY".to_string()]),
        "X-[Y]>lo\n"
    );
    let (buf, sels) = crate::testing::parse_state("<[hel]-lo\n");
    let (_, _, _, replaced) = paste_after(buf, sels, &["XY".to_string()]);
    assert_eq!(replaced, vec!["hel"]);
}

#[test]
fn paste_after_replace_swap_roundtrip() {
    // Yank "foo", paste onto selection "bar" → buffer has "foo", replaced = ["bar"].
    let (buf, sels) = crate::testing::parse_state("-[bar]>\n");
    let (new_buf, _, _, replaced) = paste_after(buf, sels, &["foo".to_string()]);
    assert_eq!(new_buf.to_string(), "foo\n");
    assert_eq!(replaced, vec!["bar"]);
}

#[test]
fn paste_after_replace_multi_cursor_n_to_n() {
    // Two non-cursor selections; two values — each replaced independently.
    // "-[he]>l-[lo]>\n": "he" replaced by "AB", "lo" replaced by "CD".
    // Text: h(0)e(1)l(2)l(3)o(4)\n(5)
    // After: AB + l + CD + \n = "ABlCD\n"
    assert_state!(
        "-[he]>l-[lo]>\n",
        |(buf, sels)| pa(buf, sels, &["AB".to_string(), "CD".to_string()]),
        "A-[B]>lC-[D]>\n"
    );
    let (buf, sels) = crate::testing::parse_state("-[he]>l-[lo]>\n");
    let (_, _, _, replaced) = paste_after(buf, sels, &["AB".to_string(), "CD".to_string()]);
    assert_eq!(replaced, vec!["he", "lo"]);
}

#[test]
fn paste_after_mixed_cursor_and_selection() {
    // One cursor (inserts) + one multi-char selection (replaces).
    // "-[h]>el-[lo]>\n": cursor at 'h' inserts "AB" after it; "lo" is replaced by "CD".
    // Text: h + AB + el + CD + \n = "hABelCD\n"
    // Cursors land on 'B' (pos 2) and 'D' (pos 6).
    assert_state!(
        "-[h]>el-[lo]>\n",
        |(buf, sels)| pa(buf, sels, &["AB".to_string(), "CD".to_string()]),
        "hA-[B]>elC-[D]>\n"
    );
    let (buf, sels) = crate::testing::parse_state("-[h]>el-[lo]>\n");
    let (_, _, _, replaced) = paste_after(buf, sels, &["AB".to_string(), "CD".to_string()]);
    // Cursor replaced nothing; selection replaced "lo".
    assert_eq!(replaced, vec!["", "lo"]);
}

#[test]
fn paste_after_empty_string_cursor_is_noop() {
    // B4 regression: b.new_pos() - 1 underflows when text is "".
    // For a cursor selection with empty text, buffer and cursor must be unchanged.
    assert_state!(
        "-[h]>ello\n",
        |(buf, sels)| {
            let (b, s, _, _) = paste_after(buf, sels, &["".to_string()]);
            (b, s)
        },
        "-[h]>ello\n"
    );
}

#[test]
fn paste_after_empty_string_over_selection_deletes_and_lands_at_start() {
    // Empty text with a multi-char selection: the selection is deleted,
    // cursor lands at the start of the deleted region (not new_pos() - 1).
    assert_state!(
        "-[hel]>lo\n",
        |(buf, sels)| {
            let (b, s, _, _) = paste_after(buf, sels, &["".to_string()]);
            (b, s)
        },
        "-[l]>o\n"
    );
}

// ── paste_before ──────────────────────────────────────────────────────────

#[test]
fn paste_before_single_cursor() {
    // Cursor on 'h' — insert "XY" before 'h'; cursor lands on 'Y'.
    assert_state!(
        "-[h]>ello\n",
        |(buf, sels)| pb(buf, sels, &["XY".to_string()]),
        "X-[Y]>hello\n"
    );
}

#[test]
fn paste_before_mid_word() {
    // Cursor on 'e' (pos 1) — insert "XY" before 'e'.
    assert_state!(
        "h-[e]>llo\n",
        |(buf, sels)| pb(buf, sels, &["XY".to_string()]),
        "hX-[Y]>ello\n"
    );
}

#[test]
fn paste_before_two_cursors_n_to_n() {
    // Two cursors; two values — each cursor gets its own slot.
    // Text after: AB + hell + CD + o + \n
    assert_state!(
        "-[h]>ell-[o]>\n",
        |(buf, sels)| pb(buf, sels, &["AB".to_string(), "CD".to_string()]),
        "A-[B]>hellC-[D]>o\n"
    );
}

#[test]
fn paste_before_count_mismatch_uses_joined() {
    // 2 cursors, 1 value → both cursors get the full "XY".
    assert_state!(
        "-[h]>ell-[o]>\n",
        |(buf, sels)| pb(buf, sels, &["XY".to_string()]),
        "X-[Y]>hellX-[Y]>o\n"
    );
}

#[test]
fn paste_before_replaces_selection() {
    // Multi-char selection — paste_before also replaces (same as paste_after for selections).
    assert_state!(
        "-[hel]>lo\n",
        |(buf, sels)| pb(buf, sels, &["XY".to_string()]),
        "X-[Y]>lo\n"
    );
    let (buf, sels) = crate::testing::parse_state("-[hel]>lo\n");
    let (_, _, _, replaced) = paste_before(buf, sels, &["XY".to_string()]);
    assert_eq!(replaced, vec!["hel"]);
}

// ── paste empty-values (no-op path) ──────────────────────────────────────

#[test]
fn paste_after_empty_values_is_noop() {
    let (buf, sels) = crate::testing::parse_state("-[h]>ello\n");
    let buf_str = buf.to_string();
    let (new_buf, new_sels, _, replaced) = paste_after(buf, sels.clone(), &[]);
    assert_eq!(new_buf.to_string(), buf_str);
    assert_eq!(new_sels, sels);
    assert!(replaced.is_empty());
}

#[test]
fn paste_before_empty_values_is_noop() {
    let (buf, sels) = crate::testing::parse_state("-[h]>ello\n");
    let buf_str = buf.to_string();
    let (new_buf, new_sels, _, replaced) = paste_before(buf, sels.clone(), &[]);
    assert_eq!(new_buf.to_string(), buf_str);
    assert_eq!(new_sels, sels);
    assert!(replaced.is_empty());
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
    // Cursor lands on the last pasted char 'r'(7).
    assert_state!(
        "-[h]>ello\n",
        |(buf, sels)| {
            let (b, s, cs, _) = paste_after(buf, sels, &["foo\nbar".to_string()]);
            (b, s, cs)
        },
        "hfoo\nba-[r]>ello\n"
    );
}

#[test]
fn paste_before_multiline_text() {
    // Paste "foo\nbar" before 'h'. Text: "foo\nbar" + "hello\n".
    // Cursor lands on the last pasted char 'r'(6).
    assert_state!(
        "-[h]>ello\n",
        |(buf, sels)| {
            let (b, s, cs, _) = paste_before(buf, sels, &["foo\nbar".to_string()]);
            (b, s, cs)
        },
        "foo\nba-[r]>hello\n"
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
    // Yank "ello" from selection, then paste it after the cursor.
    // Initial: cursor on 'h', selection covers "ello".
    // After yank: yanked = ["ello"]
    // After paste_after: "h" + "ello" + "\n" — cursor on last pasted 'o'.
    let (buf, sels) = crate::testing::parse_state("-[h]>ello\n");
    let yanked = yank_selections(&buf, &sels);
    assert_eq!(yanked, vec!["h"], "yank captures the cursor char");

    // Now paste the yanked text after the cursor (which is on 'h').
    // paste_after inserts "h" after 'h': "hh|ello\n"
    assert_state!(
        "-[h]>ello\n",
        |(buf, sels)| {
            let values = yank_selections(&buf, &sels);
            let (b, s, cs, _) = paste_after(buf, sels, &values);
            (b, s, cs)
        },
        "h-[h]>ello\n"
    );
}

#[test]
fn yank_multi_cursor_then_paste_after_n_to_n() {
    use crate::ops::register::yank_selections;
    // Two cursors: one on 'h', one on 'o'. Yank both, paste after each.
    // Expected yanked: ["h", "o"]
    // After paste: "hh" at pos 0-1, "oo" at pos 4-5 (with shift).
    let (buf, sels) = crate::testing::parse_state("-[h]>ell-[o]>\n");
    let yanked = yank_selections(&buf, &sels);
    assert_eq!(yanked, vec!["h", "o"]);

    assert_state!(
        "-[h]>ell-[o]>\n",
        |(buf, sels)| {
            let values = yank_selections(&buf, &sels);
            let (b, s, cs, _) = paste_after(buf, sels, &values);
            (b, s, cs)
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
