use super::super::*;
use hume_editing::tab_style::TabStyle;
use hume_test_fixtures::assert_state;

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

// ── insert_str ────────────────────────────────────────────────────────────
//
// Bulk-string counterpart of insert_char, used for terminal paste. Mirrors
// insert_char's cases plus multi-char-specific ones (multi-line text,
// grapheme clusters spanning the inserted text, identity on empty string).

#[test]
fn insert_str_at_cursor_start() {
    assert_state!(
        "-[h]>ello\n",
        |(buf, sels)| insert_str(buf, sels, "xyz"),
        "xyz-[h]>ello\n"
    );
}

#[test]
fn insert_str_at_cursor_eof() {
    assert_state!(
        "hello-[\n]>",
        |(buf, sels)| insert_str(buf, sels, "xyz"),
        "helloxyz-[\n]>"
    );
}

#[test]
fn insert_str_replaces_forward_selection() {
    // Selection covers "hell" (4 chars); replaced by "xyz".
    assert_state!(
        "-[hell]>o\n",
        |(buf, sels)| insert_str(buf, sels, "xyz"),
        "xyz-[o]>\n"
    );
}

#[test]
fn insert_str_replaces_backward_selection() {
    assert_state!(
        "<[hell]-o\n",
        |(buf, sels)| insert_str(buf, sels, "xyz"),
        "xyz-[o]>\n"
    );
}

#[test]
fn insert_str_two_cursors() {
    // Cursors at 0 and 3; "xy" inserted at both.
    assert_state!(
        "-[f]>oo-[ ]>bar\n",
        |(buf, sels)| insert_str(buf, sels, "xy"),
        "xy-[f]>ooxy-[ ]>bar\n"
    );
}

#[test]
fn insert_str_unicode_grapheme() {
    // Pasted text itself contains a combining sequence (é = e + combining
    // acute). Cursor lands after the whole pasted span via new_pos(), never
    // mid-cluster.
    assert_state!(
        "caf-[é]>\n",
        |(buf, sels)| insert_str(buf, sels, "e\u{0301}"),
        "cafe\u{0301}-[é]>\n"
    );
}

#[test]
fn insert_str_multiline() {
    // Pasted text contains an embedded newline; buffer's structural trailing
    // \n is untouched.
    assert_state!(
        "h-[e]>llo\n",
        |(buf, sels)| insert_str(buf, sels, "X\nY"),
        "hX\nY-[e]>llo\n"
    );
}

#[test]
fn insert_str_empty_is_identity() {
    assert_state!(
        "-[h]>ello\n",
        |(buf, sels)| insert_str(buf, sels, ""),
        "-[h]>ello\n"
    );
}

// ── insert_tab ────────────────────────────────────────────────────────────

#[test]
fn insert_tab_hard_at_cursor() {
    // Hard tab at col 0 → inserts '\t', cursor stays on the original char.
    assert_state!(
        "-[h]>ello\n",
        |(buf, sels)| insert_tab(buf, sels, TabStyle::Hard, 4),
        "\t-[h]>ello\n"
    );
}

#[test]
fn insert_tab_hard_mid_line() {
    assert_state!(
        "hel-[l]>o\n",
        |(buf, sels)| insert_tab(buf, sels, TabStyle::Hard, 4),
        "hel\t-[l]>o\n"
    );
}

#[test]
fn insert_tab_hard_replaces_selection() {
    // Tab over a selection replaces it, same as typing any char.
    assert_state!(
        "-[hell]>o\n",
        |(buf, sels)| insert_tab(buf, sels, TabStyle::Hard, 4),
        "\t-[o]>\n"
    );
}

#[test]
fn insert_tab_hard_two_cursors() {
    assert_state!(
        "-[f]>oo-[ ]>bar\n",
        |(buf, sels)| insert_tab(buf, sels, TabStyle::Hard, 4),
        "\t-[f]>oo\t-[ ]>bar\n"
    );
}

#[test]
fn insert_tab_soft_at_col0_inserts_full_width() {
    // Soft tab at col 0, tw=4 → 4 spaces.
    assert_state!(
        "-[h]>ello\n",
        |(buf, sels)| insert_tab(buf, sels, TabStyle::Soft, 4),
        "    -[h]>ello\n"
    );
}

#[test]
fn insert_tab_soft_at_col2_inserts_two_spaces() {
    // Soft tab at col 2, tw=4 → 2 spaces (to reach next stop at col 4).
    assert_state!(
        "he-[l]>lo\n",
        |(buf, sels)| insert_tab(buf, sels, TabStyle::Soft, 4),
        "he  -[l]>lo\n"
    );
}

#[test]
fn insert_tab_soft_at_col4_inserts_full_width() {
    // Already on a tab stop (col 4) → full tab-width of spaces.
    assert_state!(
        "abcd-[e]>\n",
        |(buf, sels)| insert_tab(buf, sels, TabStyle::Soft, 4),
        "abcd    -[e]>\n"
    );
}

#[test]
fn insert_tab_soft_after_tab_uses_current_col() {
    // "\tx" → cursor after 'x' is at display col 5, tw=4 → 3 spaces to col 8.
    assert_state!(
        "\tx-[y]>\n",
        |(buf, sels)| insert_tab(buf, sels, TabStyle::Soft, 4),
        "\tx   -[y]>\n"
    );
}

#[test]
fn insert_tab_soft_tab_width_8() {
    // tw=8 at col 0 → 8 spaces.
    assert_state!(
        "-[h]>i\n",
        |(buf, sels)| insert_tab(buf, sels, TabStyle::Soft, 8),
        "        -[h]>i\n"
    );
}

#[test]
fn insert_tab_soft_replaces_selection() {
    // Soft tab over a selection: delete selection, insert spaces for the
    // cursor's column (which is the selection start).
    assert_state!(
        "-[hell]>o\n",
        |(buf, sels)| insert_tab(buf, sels, TabStyle::Soft, 4),
        "    -[o]>\n"
    );
}

#[test]
fn insert_tab_soft_two_cursors_different_lines() {
    // Two cursors on different lines: each column is independent.
    // Line 0: cursor on 'c' (col 2) → 2 spaces to reach col 4.
    // Line 1: cursor on 'z' (col 2) → 2 spaces to reach col 4.
    assert_state!(
        "ab-[c]>\nxy-[z]>\n",
        |(buf, sels)| insert_tab(buf, sels, TabStyle::Soft, 4),
        "ab  -[c]>\nxy  -[z]>\n"
    );
}

#[test]
fn insert_tab_soft_two_cursors_same_line() {
    // Two cursors on the SAME line: cursor 1's effective column accounts for
    // the spaces cursor 0 already inserted, so it aligns to the correct stop.
    //
    // "abc xyz\n": cursor 0 on 'c' (col 2), cursor 1 on 'z' (col 6). tw=4.
    // Cursor 0: col 2 → 2 spaces to reach col 4. col_shift = +2.
    // Cursor 1: original col 6 + shift 2 = effective col 8. 8 is a tab stop,
    //           so a full tw=4 spaces to reach col 12.
    // Independent oracle: after cursor 0's 2 spaces, 'z' sits at col 8;
    //                     next stop = 12; spaces needed = 4.
    assert_state!(
        "ab-[c]> xy-[z]>\n",
        |(buf, sels)| insert_tab(buf, sels, TabStyle::Soft, 4),
        "ab  -[c]> xy    -[z]>\n"
    );
}

#[test]
fn insert_tab_soft_two_cursors_same_line_not_on_stop() {
    // Both cursors on the same line; cursor 0 is NOT at col 0, so cursor 1's
    // effective col is not a multiple of tw — verifies the arithmetic mid-stop.
    //
    // "abcde fgh\n": cursor 0 on 'd' (col 3), cursor 1 on 'h' (col 8). tw=4.
    // Cursor 0: col 3 → 1 space to reach col 4. col_shift = +1.
    // Cursor 1: original col 8 + shift 1 = effective col 9. Next stop = 12.
    //           Spaces = 12 - 9 = 3.
    // Independent oracle: after cursor 0's 1 space, 'h' is at col 9; next stop
    //                     at col 12; spaces needed = 3.
    assert_state!(
        "abc-[d]>e fg-[h]>\n",
        |(buf, sels)| insert_tab(buf, sels, TabStyle::Soft, 4),
        "abc -[d]>e fg   -[h]>\n"
    );
}

#[test]
fn insert_tab_soft_tab_width_1() {
    // Minimum tab width: every column is a stop, so exactly one space is
    // inserted regardless of the cursor's column.
    assert_state!(
        "-[h]>i\n",
        |(buf, sels)| insert_tab(buf, sels, TabStyle::Soft, 1),
        " -[h]>i\n"
    );
    assert_state!(
        "he-[l]>lo\n",
        |(buf, sels)| insert_tab(buf, sels, TabStyle::Soft, 1),
        "he -[l]>lo\n"
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

// ── insert_newline_indent (auto-indent on Enter) ──────────────────────────

#[test]
fn newline_indent_copies_tab_indent() {
    // "\tfoo" cursor on 'f' → new line gets "\t", cursor on 'f' (new line).
    assert_state!(
        "\t-[f]>oo\n",
        |(buf, sels)| insert_newline_indent(buf, sels, true),
        "\t\n\t-[f]>oo\n"
    );
}

#[test]
fn newline_indent_copies_space_indent() {
    // "    bar" cursor on 'b' → new line gets "    ".
    assert_state!(
        "    -[b]>ar\n",
        |(buf, sels)| insert_newline_indent(buf, sels, true),
        "    \n    -[b]>ar\n"
    );
}

#[test]
fn newline_indent_no_indent_on_bare_line() {
    // "foo" cursor on 'o' (last char) → new line bare.
    assert_state!(
        "fo-[o]>\n",
        |(buf, sels)| insert_newline_indent(buf, sels, true),
        "fo\n-[o]>\n"
    );
}

#[test]
fn newline_indent_at_line_start_no_indent_before_cursor() {
    // "foo\n" cursor on 'f' (line start, no indent before cursor) → new line
    // is bare, original 'f' moves to new line.
    assert_state!(
        "-[f]>oo\n",
        |(buf, sels)| insert_newline_indent(buf, sels, true),
        "\n-[f]>oo\n"
    );
}

#[test]
fn newline_indent_mid_line_preserves_content_before_cursor() {
    // "\tfoo" cursor on 'o' (mid content) → content before cursor stays on
    // old line; new line gets indent; cursor on 'o'.
    assert_state!(
        "\tfo-[o]>\n",
        |(buf, sels)| insert_newline_indent(buf, sels, true),
        "\tfo\n\t-[o]>\n"
    );
}

#[test]
fn newline_indent_mixed_indent() {
    // "\t  x" cursor on 'x' → new line gets "\t  ".
    assert_state!(
        "\t  -[x]>\n",
        |(buf, sels)| insert_newline_indent(buf, sels, true),
        "\t  \n\t  -[x]>\n"
    );
}

#[test]
fn newline_indent_replaces_selection() {
    // Selection over "foo" in "\tfoo\n": delete selection, insert "\n" + indent.
    // Cursor lands on the structural trailing '\n' (the retained original).
    assert_state!(
        "\t-[foo]>\n",
        |(buf, sels)| insert_newline_indent(buf, sels, true),
        "\t\n\t-[\n]>"
    );
}

#[test]
fn newline_indent_second_line() {
    // "a\n\tb\n" cursor on 'b' (line 1, indented) → new line gets "\t".
    assert_state!(
        "a\n\t-[b]>\n",
        |(buf, sels)| insert_newline_indent(buf, sels, true),
        "a\n\t\n\t-[b]>\n"
    );
}

#[test]
fn newline_indent_two_cursors_different_indents() {
    // Two cursors on differently-indented lines get their own indent.
    // Line 0 "  a" (cursor on 'a'), line 1 "\tb" (cursor on 'b').
    assert_state!(
        "  -[a]>\n\t-[b]>\n",
        |(buf, sels)| insert_newline_indent(buf, sels, true),
        "  \n  -[a]>\n\t\n\t-[b]>\n"
    );
}

#[test]
fn newline_indent_cursor_on_structural_newline() {
    // Cursor on the structural '\n' of an indented line ("  x\n"): the '\n'
    // is retained, a new line is opened after it with the line's indent
    // copied across, and the cursor lands on the new line's '\n'. Mirrors
    // `insert_char` behaviour for a cursor on the structural newline.
    assert_state!(
        "  x-[\n]>",
        |(buf, sels)| insert_newline_indent(buf, sels, true),
        "  x\n  -[\n]>"
    );
}

#[test]
fn newline_indent_replaces_multi_line_selection() {
    // Selection spans "ab\n\txy" in "\tab\n\txy\n": the whole span is deleted,
    // then a '\n' + the source line's indent ("\t") is inserted at the
    // selection's start. With the retained leading '\t' and the trailing
    // structural '\n', the buffer collapses to "\t\n\t\n" and the cursor ends
    // on that final '\n' — the same "land on the structural '\n'" rule as the
    // single-line selection case.
    assert_state!(
        "\t-[ab\n\txy]>\n",
        |(buf, sels)| insert_newline_indent(buf, sels, true),
        "\t\n\t-[\n]>"
    );
}

#[test]
fn newline_indent_trims_blank_line_on_second_enter() {
    // Cursor on the structural '\n' of a blank, auto-indented line ("  \n"):
    // vim autoindent parity — that whitespace is vacated (not carried
    // forward) before opening a fresh indented line below it.
    assert_state!(
        "x\n  -[\n]>",
        |(buf, sels)| insert_newline_indent(buf, sels, true),
        "x\n\n  -[\n]>"
    );
}

#[test]
fn newline_indent_trims_blank_line_cursor_mid_whitespace() {
    // Collapsed cursor anywhere within a blank line's whitespace (not just
    // at its end) still triggers the trim — the whole line is judged blank,
    // not just the region before the cursor.
    assert_state!(
        "x\n -[ ]> \n",
        |(buf, sels)| insert_newline_indent(buf, sels, true),
        "x\n\n   -[\n]>"
    );
}

#[test]
fn newline_indent_trim_blank_false_preserves_pre_existing_blank_line() {
    // `trim_blank = false`: the first Enter on a line that was already blank
    // before this insert session touched it leaves the pre-existing
    // whitespace alone; only the *new* line gets a copied indent, same as
    // the non-blank-line case.
    assert_state!(
        "x\n  -[\n]>",
        |(buf, sels)| insert_newline_indent(buf, sels, false),
        "x\n  \n  -[\n]>"
    );
}

#[test]
fn newline_indent_two_cursors_same_blank_line_merge() {
    // Two collapsed cursors on the same whitespace-only line: the first
    // vacates the line; the second lands at the same spot instead of
    // retaining backwards past what the builder already consumed.
    // `SelectionSet::from_vec` then merges the coincident cursors — no
    // panic, no duplicate newline.
    assert_state!(
        "-[ ]> -[ ]>\n",
        |(buf, sels)| insert_newline_indent(buf, sels, true),
        "\n   -[\n]>"
    );
}

#[test]
fn newline_indent_two_cursors_second_on_blank_line_newline_no_underflow() {
    // Regression: the first cursor (head 0) trims the blank line "  \n",
    // advancing the builder's `old_pos()` to 2 — exactly the position of
    // the second cursor's head, which sits on the line's structural '\n'.
    // The `pos < b.old_pos()` "already consumed" guard requires strict
    // inequality, so `2 < 2` is false and the second cursor is NOT treated
    // as consumed; its own `line_start` (0) is what has actually been
    // passed. Before the fix, `try_trim_blank_line` didn't check
    // `line_start` against `old_pos()` and computed `b.retain(0 - 2)`,
    // underflowing. The fix falls back to the non-blank arm instead: both
    // cursors independently run their own Enter-with-copied-indent, each
    // landing on its own freshly opened line — no crash.
    assert_state!(
        "-[ ]> -[\n]>",
        |(buf, sels)| insert_newline_indent(buf, sels, true),
        "\n  -[\n]>  -[\n]>"
    );
}

// ── clear_blank_line_indent (vim autoindent parity on Insert-mode exit) ───

#[test]
fn clear_blank_line_indent_clears_whitespace_only_line() {
    // Cursor on a blank, auto-indented line: its whitespace is cleared,
    // cursor lands on the resulting (now truly empty) line's '\n'.
    assert_state!(
        "-[ ]> \n",
        |(buf, sels)| clear_blank_line_indent(buf, sels),
        "-[\n]>"
    );
}

#[test]
fn clear_blank_line_indent_no_op_on_content_line() {
    // Cursor on a line with real content: identity edit, nothing cleared.
    assert_state!(
        "-[f]>oo\n",
        |(buf, sels)| clear_blank_line_indent(buf, sels),
        "-[f]>oo\n"
    );
}

#[test]
fn clear_blank_line_indent_multi_cursor_only_clears_blank_line() {
    // One cursor on a content line, one on a blank auto-indented line: only
    // the blank line's whitespace is cleared; the content-line cursor is an
    // identity edit.
    assert_state!(
        "-[f]>oo\n -[ ]>\n",
        |(buf, sels)| clear_blank_line_indent(buf, sels),
        "-[f]>oo\n-[\n]>"
    );
}

#[test]
fn clear_blank_line_indent_two_cursors_same_line_merge() {
    // Two cursors on the same blank line: the first clears it; the second
    // lands at the same spot instead of retaining backwards. Selections
    // merge into one, no panic.
    assert_state!(
        "-[ ]> -[ ]>\n",
        |(buf, sels)| clear_blank_line_indent(buf, sels),
        "-[\n]>"
    );
}

#[test]
fn clear_blank_line_indent_second_cursor_on_blank_line_newline_no_underflow() {
    // Same regression as `newline_indent_two_cursors_second_on_blank_line_
    // newline_no_underflow`, for the Esc/exit-insert path: the second
    // cursor sits exactly on the blank line's structural '\n', at a
    // position equal to (not less than) `old_pos()` after the first
    // cursor's trim — the "already consumed" guard's strict `<` doesn't
    // catch it, so `try_trim_blank_line`'s own `line_start >= old_pos()`
    // check is what prevents the underflow. Both cursors land on the same
    // final position and merge, same as the mid-whitespace case above.
    assert_state!(
        "-[ ]> -[\n]>",
        |(buf, sels)| clear_blank_line_indent(buf, sels),
        "-[\n]>"
    );
}

#[test]
fn clear_blank_line_indent_preserves_non_collapsed_selection() {
    // One collapsed cursor on a blank indented line, plus a non-collapsed
    // selection elsewhere: the blank line is trimmed as usual, but the
    // other selection's anchor and head must both survive, not collapse to
    // its head. "foo\n \nbar\n" (line1 is a single-space blank line);
    // selection B covers "ar" in "bar" on line2.
    assert_state!(
        "foo\n-[ ]>\nb-[ar]>\n",
        |(buf, sels)| clear_blank_line_indent(buf, sels),
        "foo\n-[\n]>b-[ar]>\n"
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
