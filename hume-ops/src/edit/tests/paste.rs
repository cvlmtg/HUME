use super::super::*;
use hume_test_fixtures::assert_state;
use pretty_assertions::assert_eq;

// ── paste_after ───────────────────────────────────────────────────────────

fn pa(
    text: BufferText,
    sels: SelectionSet,
    values: &[String],
) -> (BufferText, SelectionSet, ChangeSet) {
    paste_after(text, sels, values)
}

fn pb(
    text: BufferText,
    sels: SelectionSet,
    values: &[String],
) -> (BufferText, SelectionSet, ChangeSet) {
    paste_before(text, sels, values)
}

#[test]
fn paste_after_single_cursor() {
    // Cursor on 'h' — insert "XY" after 'h'; selection covers "XY".
    assert_state!(
        "-[h]>ello\n",
        |(text, sels)| pa(text, sels, &["XY".to_string()]),
        "h-[XY]>ello\n"
    );
}

#[test]
fn paste_after_mid_word() {
    // Cursor on 'e' (pos 1) — insert "XY" after 'e'; selection covers "XY".
    assert_state!(
        "h-[e]>llo\n",
        |(text, sels)| pa(text, sels, &["XY".to_string()]),
        "he-[XY]>llo\n"
    );
}

#[test]
fn paste_after_cursor_on_structural_newline() {
    // Cursor on the trailing '\n' — insertion is clamped to pos 5 (before '\n').
    // "hello\n" → "helloXY\n"; cursor lands on 'Y' (pos 6).
    assert_state!(
        "hello-[\n]>",
        |(text, sels)| pa(text, sels, &["XY".to_string()]),
        "hello-[XY]>\n"
    );
}

#[test]
fn paste_after_cursor_on_empty_line_stays_on_that_line() {
    // Cursor on an empty line (its only char is the line's own '\n'). Paste
    // must land on that line, not cross into the next one.
    assert_state!(
        "ab\n-[\n]>cd\n",
        |(text, sels)| pa(text, sels, &["XY".to_string()]),
        "ab\n-[XY]>\ncd\n"
    );
}

#[test]
fn paste_after_cursor_on_interior_newline_stays_on_that_line() {
    // Same escape via a non-empty line's own terminator — "after the cursor"
    // must not cross the line break here either.
    assert_state!(
        "ab-[\n]>cd\n",
        |(text, sels)| pa(text, sels, &["XY".to_string()]),
        "ab-[XY]>\ncd\n"
    );
}

#[test]
fn paste_before_cursor_on_empty_line_stays_on_that_line() {
    // Characterization: paste-before is already correct here (inserts at
    // sel.start(), which is the empty line's own position).
    assert_state!(
        "ab\n-[\n]>cd\n",
        |(text, sels)| pb(text, sels, &["XY".to_string()]),
        "ab\n-[XY]>\ncd\n"
    );
}

#[test]
fn paste_after_two_cursors_n_to_n() {
    // Two cursors (pos 0 and 4); two values — each cursor gets its own slot.
    assert_state!(
        "-[h]>ell-[o]>\n",
        |(text, sels)| pa(text, sels, &["AB".to_string(), "CD".to_string()]),
        "h-[AB]>ello-[CD]>\n"
    );
}

#[test]
fn paste_after_count_mismatch_uses_joined() {
    // 2 cursors, 1 value → both cursors get the full "XY".
    assert_state!(
        "-[h]>ell-[o]>\n",
        |(text, sels)| pa(text, sels, &["XY".to_string()]),
        "h-[XY]>ello-[XY]>\n"
    );
}

#[test]
fn paste_after_unicode() {
    // Paste a string with a combining character. Cursor lands on last char.
    assert_state!(
        "-[h]>i\n",
        |(text, sels)| pa(text, sels, &["e\u{0301}".to_string()]),
        "h-[e\u{0301}]>i\n"
    );
}

#[test]
fn paste_after_replaces_forward_selection() {
    // Multi-char selection "hel" is replaced by "XY". Selection covers "XY".
    assert_state!(
        "-[hel]>lo\n",
        |(text, sels)| pa(text, sels, &["XY".to_string()]),
        "-[XY]>lo\n"
    );
}

#[test]
fn paste_after_replaces_backward_selection() {
    // Direction doesn't matter for replace — same result as forward.
    assert_state!(
        "<[hel]-lo\n",
        |(text, sels)| pa(text, sels, &["XY".to_string()]),
        "-[XY]>lo\n"
    );
}

#[test]
fn paste_after_replace_multi_cursor_n_to_n() {
    // Two non-cursor selections; two values — each replaced independently.
    // "-[he]>l-[lo]>\n": "he" replaced by "AB", "lo" replaced by "CD".
    assert_state!(
        "-[he]>l-[lo]>\n",
        |(text, sels)| pa(text, sels, &["AB".to_string(), "CD".to_string()]),
        "-[AB]>l-[CD]>\n"
    );
}

#[test]
fn paste_after_mixed_cursor_and_selection() {
    // One cursor (inserts) + one multi-char selection (replaces).
    // "-[h]>el-[lo]>\n": cursor at 'h' inserts "AB" after it; "lo" is replaced by "CD".
    assert_state!(
        "-[h]>el-[lo]>\n",
        |(text, sels)| pa(text, sels, &["AB".to_string(), "CD".to_string()]),
        "h-[AB]>el-[CD]>\n"
    );
}

#[test]
fn paste_after_empty_string_cursor_is_noop() {
    assert_state!(
        "-[h]>ello\n",
        |(text, sels)| pa(text, sels, &["".to_string()]),
        "-[h]>ello\n"
    );
}

#[test]
fn paste_after_empty_string_over_selection_deletes_and_lands_at_start() {
    // Empty text with a multi-char selection: the selection is deleted,
    // cursor lands at the start of the deleted region.
    assert_state!(
        "-[hel]>lo\n",
        |(text, sels)| pa(text, sels, &["".to_string()]),
        "-[l]>o\n"
    );
}

// ── paste_before ──────────────────────────────────────────────────────────

#[test]
fn paste_before_single_cursor() {
    // Cursor on 'h' — insert "XY" before 'h'; selection covers "XY".
    assert_state!(
        "-[h]>ello\n",
        |(text, sels)| pb(text, sels, &["XY".to_string()]),
        "-[XY]>hello\n"
    );
}

#[test]
fn paste_before_mid_word() {
    // Cursor on 'e' (pos 1) — insert "XY" before 'e'; selection covers "XY".
    assert_state!(
        "h-[e]>llo\n",
        |(text, sels)| pb(text, sels, &["XY".to_string()]),
        "h-[XY]>ello\n"
    );
}

#[test]
fn paste_before_two_cursors_n_to_n() {
    // Two cursors; two values — each cursor gets its own slot.
    // Text after: AB + hell + CD + o + \n; each selection covers its value.
    assert_state!(
        "-[h]>ell-[o]>\n",
        |(text, sels)| pb(text, sels, &["AB".to_string(), "CD".to_string()]),
        "-[AB]>hell-[CD]>o\n"
    );
}

#[test]
fn paste_before_count_mismatch_uses_joined() {
    // 2 cursors, 1 value → both cursors get the full "XY".
    assert_state!(
        "-[h]>ell-[o]>\n",
        |(text, sels)| pb(text, sels, &["XY".to_string()]),
        "-[XY]>hell-[XY]>o\n"
    );
}

#[test]
fn paste_before_replaces_selection() {
    // Multi-char selection — paste_before also replaces (same as paste_after for selections).
    assert_state!(
        "-[hel]>lo\n",
        |(text, sels)| pb(text, sels, &["XY".to_string()]),
        "-[XY]>lo\n"
    );
}

// ── paste empty-values (no-op path) ──────────────────────────────────────

#[test]
fn paste_after_empty_values_is_noop() {
    let (text, sels) = hume_test_fixtures::testing::parse_state("-[h]>ello\n");
    let buf_str = text.to_string();
    let (new_text, new_sels, _cs) = paste_after(text, sels.clone(), &[]);
    assert_eq!(new_text.to_string(), buf_str);
    assert_eq!(new_sels, sels);
}

#[test]
fn paste_before_empty_values_is_noop() {
    let (text, sels) = hume_test_fixtures::testing::parse_state("-[h]>ello\n");
    let buf_str = text.to_string();
    let (new_text, new_sels, _cs) = paste_before(text, sels.clone(), &[]);
    assert_eq!(new_text.to_string(), buf_str);
    assert_eq!(new_sels, sels);
}

// ── paste with multiline text ─────────────────────────────────────────────

#[test]
fn paste_after_multiline_text() {
    // Paste "foo\nbar" after 'h'. Text: "h" + "foo\nbar" + "ello\n".
    // Selection covers the entire pasted span "foo\nbar".
    assert_state!(
        "-[h]>ello\n",
        |(text, sels)| pa(text, sels, &["foo\nbar".to_string()]),
        "h-[foo\nbar]>ello\n"
    );
}

#[test]
fn paste_before_multiline_text() {
    // Paste "foo\nbar" before 'h'. Text: "foo\nbar" + "hello\n".
    // Selection covers the entire pasted span "foo\nbar".
    assert_state!(
        "-[h]>ello\n",
        |(text, sels)| pb(text, sels, &["foo\nbar".to_string()]),
        "-[foo\nbar]>hello\n"
    );
}

// ── register content normalization ────────────────────────────────────────

#[test]
fn paste_after_normalizes_crlf_in_register_content() {
    assert_state!(
        "-[h]>ello\n",
        |(text, sels)| pa(text, sels, &["foo\r\nbar".to_string()]),
        "h-[foo\nbar]>ello\n"
    );
}

#[test]
fn paste_after_normalizes_bare_cr_in_register_content() {
    assert_state!(
        "-[h]>ello\n",
        |(text, sels)| pa(text, sels, &["foo\rbar".to_string()]),
        "h-[foo\nbar]>ello\n"
    );
}

#[test]
fn paste_over_selection_normalizes_bare_cr_in_register_content() {
    // Non-collapsed (selection-replace) path — a separate code path from the
    // collapsed-cursor insert above, and its own `b.insert` site.
    assert_state!(
        "-[hel]>lo\n",
        |(text, sels)| pa(text, sels, &["foo\rbar".to_string()]),
        "-[foo\nbar]>lo\n"
    );
}

// ── linewise paste (content ending in '\n') ───────────────────────────────

#[test]
fn paste_after_linewise_cursor_inserts_below() {
    // Cursor on 'e' (line 0 of "hello\nworld\n"). Paste "X\n" → new line
    // below line 0. Buffer becomes "hello\nX\nworld\n"; selection covers "X\n".
    assert_state!(
        "h-[e]>llo\nworld\n",
        |(text, sels)| pa(text, sels, &["X\n".to_string()]),
        "hello\n-[X\n]>world\n"
    );
}

#[test]
fn paste_before_linewise_cursor_inserts_above() {
    // Cursor on 'e' (line 0). Paste "X\n" before → new line above line 0.
    // Buffer becomes "X\nhello\nworld\n"; selection covers "X\n".
    assert_state!(
        "h-[e]>llo\nworld\n",
        |(text, sels)| pb(text, sels, &["X\n".to_string()]),
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
        |(text, sels)| pa(text, sels, &["X\n".to_string()]),
        "hello\nworld\n-[X\n]>"
    );
}

#[test]
fn paste_after_linewise_multiline_content() {
    // Paste "a\nb\n" (two lines) after cursor on line 0 → two new lines below.
    // Buffer becomes "hello\na\nb\nworld\n"; selection covers "a\nb\n".
    assert_state!(
        "h-[e]>llo\nworld\n",
        |(text, sels)| pa(text, sels, &["a\nb\n".to_string()]),
        "hello\n-[a\nb\n]>world\n"
    );
}

#[test]
fn paste_after_linewise_over_full_line_selection() {
    // Full-line selection (head on '\n') — before and after both empty →
    // three-way collapses to just the pasted line.
    assert_state!(
        "-[hello\n]>world\n",
        |(text, sels)| pa(text, sels, &["X\n".to_string()]),
        "-[X\n]>world\n"
    );
}

#[test]
fn paste_after_linewise_over_content_selection_only() {
    // Selection covers all content but NOT the '\n' — both before and after empty →
    // three-way collapses to just the pasted line.
    assert_state!(
        "-[hello]>\nworld\n",
        |(text, sels)| pa(text, sels, &["X\n".to_string()]),
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
        |(text, sels)| pa(text, sels, &["X\n".to_string()]),
        "h\n-[X\n]>o\nworld\n"
    );
}

#[test]
fn paste_after_linewise_three_way_split_empty_before() {
    // Selection starts at column 0 — before is empty, after="o".
    // Two-part: X / o
    assert_state!(
        "-[ell]>o\nworld\n",
        |(text, sels)| pa(text, sels, &["X\n".to_string()]),
        "-[X\n]>o\nworld\n"
    );
}

#[test]
fn paste_after_linewise_three_way_split_empty_after() {
    // Selection ends just before the '\n' — before="h", after empty.
    // Two-part: h / X
    assert_state!(
        "h-[ello]>\nworld\n",
        |(text, sels)| pa(text, sels, &["X\n".to_string()]),
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
        |(text, sels)| pa(text, sels, &["X\n".to_string()]),
        "-[X\n]>l\n-[X\n]>"
    );
    // Two-line buffer: same invariant; "world" line untouched.
    assert_state!(
        "-[he]>l-[lo]>\nworld\n",
        |(text, sels)| pa(text, sels, &["X\n".to_string()]),
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
        |(text, sels)| pb(text, sels, &["X\n".to_string()]),
        "hello\n-[X\n]>world\n"
    );
}

#[test]
fn paste_before_linewise_multiline_content() {
    // Paste "a\nb\n" (two lines) before cursor on line 0 → two new lines above.
    // Buffer becomes "a\nb\nhello\nworld\n"; selection covers "a\nb\n".
    assert_state!(
        "h-[e]>llo\nworld\n",
        |(text, sels)| pb(text, sels, &["a\nb\n".to_string()]),
        "-[a\nb\n]>hello\nworld\n"
    );
}

#[test]
fn paste_before_linewise_over_full_line_selection() {
    // Full-line selection — three-way split with both sides empty; identical
    // to paste_after for non-collapsed selections.
    assert_state!(
        "-[hello\n]>world\n",
        |(text, sels)| pb(text, sels, &["X\n".to_string()]),
        "-[X\n]>world\n"
    );
}

#[test]
fn paste_before_linewise_three_way_split_both_sides() {
    // Partial selection "ell" within "hello" — before="h", after="o".
    assert_state!(
        "h-[ell]>o\nworld\n",
        |(text, sels)| pb(text, sels, &["X\n".to_string()]),
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
        |(text, sels)| pb(text, sels, &["X\n".to_string()]),
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
    let (text, _) = hume_test_fixtures::testing::parse_state("-[a]>bc\nxyz\nfoo\n");
    let sels = SelectionSet::from_vec(
        vec![
            Selection::new(2, 4), // "c\nx" — first_line=0, last_line=1
            Selection::new(6, 8), // "z\nf" — first_line=1, last_line=2
        ],
        0,
    );
    let (new_text, _new_sels, _cs) = pa(text, sels, &["X\n".to_string()]);
    let result = new_text.to_string();
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

// ── yank → paste round-trip ───────────────────────────────────────────────

#[test]
fn yank_then_paste_after_round_trip() {
    use crate::register::yank_selections;
    let (text, sels) = hume_test_fixtures::testing::parse_state("-[h]>ello\n");
    let yanked = yank_selections(&text, &sels);
    assert_eq!(yanked, vec!["h"], "yank captures the cursor char");

    assert_state!(
        "-[h]>ello\n",
        |(text, sels)| {
            let values = yank_selections(&text, &sels);
            pa(text, sels, &values)
        },
        "h-[h]>ello\n"
    );
}

#[test]
fn yank_multi_cursor_then_paste_after_n_to_n() {
    use crate::register::yank_selections;
    let (text, sels) = hume_test_fixtures::testing::parse_state("-[h]>ell-[o]>\n");
    let yanked = yank_selections(&text, &sels);
    assert_eq!(yanked, vec!["h", "o"]);

    assert_state!(
        "-[h]>ell-[o]>\n",
        |(text, sels)| {
            let values = yank_selections(&text, &sels);
            pa(text, sels, &values)
        },
        "h-[h]>ello-[o]>\n"
    );
}
