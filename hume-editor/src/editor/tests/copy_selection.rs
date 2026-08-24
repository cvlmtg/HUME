//! `copy-selection-on-next-line`/`-prev-line` (`C`) — duplicate each
//! selection onto the lines above/below it.
//!
//! Relocated from `hume-ops`'s pure-fn suite: the command's placement now
//! needs a `RowMap` (`hume-editor::editor::visual_move::
//! copy_selection_vertically`) to land on a *display* column rather than a
//! char offset — the same fix `9j`/`9k` already got (see
//! `hume-rope::lines::place_char_column`'s doc for why it's no longer this
//! command's caller). `WrapMode::None` throughout — these pin the buffer-line
//! column model itself, not its interaction with wrapping.

use super::*;
use crate::editor::dispatch::ArgSource;
use pretty_assertions::assert_eq;

/// `editor_from` plus `WrapMode::None` — mirrors `visual_move.rs`'s
/// `buffer_line_editor` for the same reason: `9j`/`9k`-style buffer-line
/// placement, not a display-row walk.
fn copy_test_editor(initial: &str) -> Editor {
    let mut ed = editor_from(initial);
    ed.view.panes[ed.state.focused_pane_id].set_wrap(hume_engine::pane::WrapOverride {
        mode: Some(hume_engine::pane::WrapMode::None),
        saved: None,
    });
    ed
}

fn run_copy(ed: &mut Editor, down: bool, count: usize) {
    let name = if down {
        "copy-selection-on-next-line"
    } else {
        "copy-selection-on-prev-line"
    };
    ed.execute_keymap_command(name.into(), Some(count), false, ArgSource::Keymap);
}

/// Build an editor from `initial`, run the copy command, and compare the
/// resulting buffer+selections against `expected` — the `Editor`-driven
/// counterpart of `assert_state!` for a command whose `RowMap` dependency
/// keeps it out of `hume-test-fixtures`' pure-fn signature. Used for cases
/// that don't need to distinguish which selection ends up primary.
fn assert_copy_state(initial: &str, down: bool, count: usize, expected: &str) {
    let mut ed = copy_test_editor(initial);
    run_copy(&mut ed, down, count);

    let (expected_text, expected_sels) = parse_state(expected);
    assert_eq!(
        serialize_state(ed.doc().text(), ed.current_selections()),
        serialize_state(&expected_text, &expected_sels),
    );
}

// ── copy-selection-on-next-line ────────────────────────────────────────────

#[test]
fn copy_cursor_to_next_line() {
    // "foo\nbar\n" — cursor at column 1 of line 0 ('o').
    // Copy should land at column 1 of line 1 ('a').
    let mut ed = copy_test_editor("f-[o]>o\nbar\n");
    run_copy(&mut ed, true, 1);
    assert_eq!(ed.doc().text().to_string(), "foo\nbar\n"); // buffer unchanged
    assert_eq!(ed.current_selections().len(), 2);
    // "foo\n" = offsets 0-3, "bar\n" = offsets 4-7. Col 1 = offset 5.
    let heads: Vec<usize> = ed
        .current_selections()
        .iter_sorted()
        .map(|s| s.head())
        .collect();
    assert!(
        heads.contains(&1),
        "original cursor should remain at col 1 of line 0"
    );
    assert!(
        heads.contains(&5),
        "new cursor should be at col 1 of line 1"
    );
    // Primary should be the new copy (the one on line 1).
    assert_eq!(ed.current_selections().primary().head(), 5);
}

#[test]
fn copy_to_next_line_on_last_line_is_noop() {
    // Cursor on the last real line — nothing to copy to.
    let mut ed = copy_test_editor("foo\nb-[a]>r\n");
    run_copy(&mut ed, true, 1);
    assert_eq!(ed.current_selections().len(), 1); // no copy added
    assert_eq!(ed.current_selections().primary().head(), 5); // cursor unchanged
}

#[test]
fn copy_to_next_line_clamps_to_shorter_target_line() {
    // "hello\nhi\n" — cursor at column 4 of line 0.
    // Line 1 is "hi\n" (only 2 real chars). Should clamp to last char 'i'.
    let mut ed = copy_test_editor("hell-[o]>\nhi\n");
    run_copy(&mut ed, true, 1);
    assert_eq!(ed.current_selections().len(), 2);
    // "hello\n" = offsets 0-5, "hi\n" = offsets 6-8.
    // Last non-\n char = 'i' at offset 7.
    assert_eq!(ed.current_selections().primary().head(), 7);
}

#[test]
fn copy_next_backward_selection() {
    // Backward selection on line 0: anchor=2('o'), head=0('f') — selects "foo" (3 chars).
    // Copy down: both endpoints shift to line 1 preserving column.
    // "foo\nbar\n": f(0),o(1),o(2),\n(3),b(4),a(5),r(6),\n(7).
    // anchor col=2 → line 1 col 2 = offset 6 ('r'). head col=0 → offset 4 ('b').
    let mut ed = copy_test_editor("<[foo]-\nbar\n");
    run_copy(&mut ed, true, 1);
    assert_eq!(ed.current_selections().len(), 2);
    // The copy (primary) should be backward: anchor=6, head=4.
    let copy = ed.current_selections().primary();
    assert!(
        copy.anchor() > copy.head(),
        "copy should preserve backward direction"
    );
    assert_eq!(copy.head(), 4); // 'b' at col 0 of line 1
    assert_eq!(copy.anchor(), 6); // 'r' at col 2 of line 1
}

#[test]
fn copy_next_multiple_cursors() {
    // Two cursors on line 0 at cols 1 and 2. Both get copied to line 1.
    // "foo\nbar\n": f(0),o(1),o(2),\n(3),b(4),a(5),r(6),\n(7).
    // Col 1 → offset 5 ('a'), col 2 → offset 6 ('r').
    assert_copy_state("f-[o]>-[o]>\nbar\n", true, 1, "f-[o]>-[o]>\nb-[a]>-[r]>\n");
}

#[test]
fn copy_next_line_count_3() {
    // count=3 copies the cursor onto each of the 3 lines below natively —
    // no external repeat loop needed.
    assert_copy_state(
        "-[a]>\nb\nc\nd\ne\n",
        true,
        3,
        "-[a]>\n-[b]>\n-[c]>\n-[d]>\ne\n",
    );
}

#[test]
fn copy_next_line_count_exceeds_buffer_clamps() {
    // Only 2 lines exist below the cursor's line — a count of 10 clamps at
    // the last real line instead of erroring.
    assert_copy_state("-[a]>\nb\nc\n", true, 10, "-[a]>\n-[b]>\n-[c]>\n");
}

#[test]
fn copy_next_line_count_3_range_selection() {
    // Forward range selection covering "hello". count=3 duplicates it onto
    // each of the next 3 lines, each preserving the same column span.
    assert_copy_state(
        "-[hello]>\nworld\nfoo!!\nbar!!\n",
        true,
        3,
        "-[hello]>\n-[world]>\n-[foo!!]>\n-[bar!!]>\n",
    );
}

#[test]
fn copy_next_line_count_3_multiple_cursors() {
    // Two cursors on line 0 at cols 1 and 2; count=3 gives each 3 copies (6
    // new selections total), landing on the correct columns of lines 1-3.
    // "f-[o]>-[o]>\nbar\nbaz\nqux\n": f(0) o(1) o(2) \n(3) bar(4-6) \n(7)
    // baz(8-10) \n(11) qux(12-14) \n(15).
    // Col 1 → offsets 5, 9, 13 on lines 1-3. Col 2 → offsets 6, 10, 14.
    let mut ed = copy_test_editor("f-[o]>-[o]>\nbar\nbaz\nqux\n");
    run_copy(&mut ed, true, 3);
    assert_eq!(ed.current_selections().len(), 8); // 2 originals + 2*3 copies
    let heads: Vec<usize> = ed
        .current_selections()
        .iter_sorted()
        .map(|s| s.head())
        .collect();
    assert!(heads.contains(&1), "original col-1 cursor");
    assert!(heads.contains(&2), "original col-2 cursor");
    assert!(heads.contains(&5), "col-1 copy on line 1");
    assert!(heads.contains(&6), "col-2 copy on line 1");
    assert!(heads.contains(&9), "col-1 copy on line 2");
    assert!(heads.contains(&10), "col-2 copy on line 2");
    assert!(heads.contains(&13), "col-1 copy on line 3");
    assert!(heads.contains(&14), "col-2 copy on line 3");
}

#[test]
fn copy_next_line_count_3_does_not_equal_three_presses() {
    // `copy_selection_vertically`'s doc comment: each copy re-derives its
    // column from the *original* selection, so a short intermediate line
    // only clamps that one copy instead of collapsing every copy after it.
    //
    // "hell-[o]>\nhi\nworld\n": cursor at col 4 of "hello". Only 2 real lines
    // exist below it ("hi", "world"), so count=3 clamps to 2 copies, same as
    // `copy_next_line_count_exceeds_buffer_clamps`.
    //
    // A press-by-press repeat would clamp col 4 down to col 1 on "hi" (only 2
    // chars), then carry that clamped col 1 forward onto "world" — landing on
    // 'o' (offset 10). Re-deriving from the original instead lands the
    // "world" copy on col 4 directly — 'd' (offset 13).
    let mut ed = copy_test_editor("hell-[o]>\nhi\nworld\n");
    run_copy(&mut ed, true, 3);
    assert_eq!(ed.current_selections().len(), 3); // original + one copy per real line below
    let heads: Vec<usize> = ed
        .current_selections()
        .iter_sorted()
        .map(|s| s.head())
        .collect();
    assert!(heads.contains(&4), "original cursor at col 4 of line 0");
    assert!(
        heads.contains(&7),
        "line 1 copy clamped to 'i', last char of \"hi\""
    );
    assert!(
        heads.contains(&13),
        "line 2 copy re-derives col 4 from the original, landing on 'd' of \"world\" \
         (offset 13) rather than the col-1-carried-forward offset 10 a press-by-press \
         repeat would produce"
    );
}

#[test]
fn copy_next_line_count_3_primary_lands_on_furthest_copy() {
    // Primary should end on the copy 3 lines down, not the first copy.
    let mut ed = copy_test_editor("-[a]>\nb\nc\nd\ne\n");
    run_copy(&mut ed, true, 3);
    // "a\nb\nc\nd\ne\n": a(0) b(2) c(4) d(6) e(8). Furthest copy is on 'd'.
    assert_eq!(ed.current_selections().primary().head(), 6);
}

#[test]
fn copy_next_line_count_usize_max_returns_instantly() {
    // A naive `0..count` loop would hang forever on `usize::MAX`; the target
    // line runs off the buffer after 4 steps, so this must return instantly.
    let mut ed = copy_test_editor("-[a]>\nb\nc\nd\ne\n");
    run_copy(&mut ed, true, usize::MAX);
    assert_eq!(ed.current_selections().len(), 5); // original + one copy per remaining line
}

#[test]
fn copy_next_line_range_selection() {
    // Forward range selection covering "hello" (0..4). Copy to next line:
    // anchor=6 ('w'), head=10 ('d') — selecting "world". Both selections exist.
    assert_copy_state("-[hello]>\nworld\n", true, 1, "-[hello]>\n-[world]>\n");
}

#[test]
fn copy_next_line_preserves_display_column_across_a_tab() {
    // "\tworld" (tab_width 4): 'o' (char 2) sits at display column 5 (tab
    // expands to 4, 'w' is 1 more). The copy on "abcdefgh" must land on
    // display column 5 — 'f' (char offset 5) — not char-offset column 2,
    // which would be 'c'. Same fixture as `visual_move.rs`'s
    // `explicit_count_move_down_preserves_display_column_across_a_tab`.
    let mut ed = copy_test_editor("\tw-[o]>rld\nabcdefgh\n");
    run_copy(&mut ed, true, 1);
    assert_eq!(ed.current_selections().len(), 2);
    let heads: Vec<usize> = ed
        .current_selections()
        .iter_sorted()
        .map(|s| s.head())
        .collect();
    assert!(heads.contains(&2), "original cursor unchanged");
    assert!(heads.contains(&12), "copy lands on display col 5 ('f')");
    assert_eq!(ed.current_selections().primary().head(), 12);
}

#[test]
fn copy_next_line_preserves_display_column_across_a_wide_cjk_char() {
    // 漢 (East Asian Wide) is 2 display columns but 1 char, so 'b' (char 1)
    // sits at display column 2. The copy on "abcdefgh" must land on display
    // column 2 — 'c' (char offset 2) — not char-offset column 1, which
    // would be 'b'. Same fixture as `visual_move.rs`'s
    // `explicit_count_move_down_preserves_display_column_across_a_wide_cjk_char`.
    let mut ed = copy_test_editor("\u{6F22}-[b]>c\nabcdefgh\n");
    run_copy(&mut ed, true, 1);
    assert_eq!(ed.current_selections().len(), 2);
    let heads: Vec<usize> = ed
        .current_selections()
        .iter_sorted()
        .map(|s| s.head())
        .collect();
    assert!(heads.contains(&1), "original cursor unchanged");
    assert!(heads.contains(&6), "copy lands on display col 2 ('c')");
    assert_eq!(ed.current_selections().primary().head(), 6);
}

// ── copy-selection-on-prev-line ────────────────────────────────────────────

#[test]
fn copy_cursor_to_prev_line() {
    // Cursor at column 1 of line 1 ('a' in "bar"). Copy goes to line 0.
    let mut ed = copy_test_editor("foo\nb-[a]>r\n");
    run_copy(&mut ed, false, 1);
    assert_eq!(ed.current_selections().len(), 2);
    // Original at offset 5 (line 1, col 1). New at offset 1 (line 0, col 1).
    let heads: Vec<usize> = ed
        .current_selections()
        .iter_sorted()
        .map(|s| s.head())
        .collect();
    assert!(heads.contains(&5), "original cursor should remain");
    assert!(
        heads.contains(&1),
        "new cursor should be at col 1 of line 0"
    );
    // Primary is the new copy (on line 0).
    assert_eq!(ed.current_selections().primary().head(), 1);
}

#[test]
fn copy_to_prev_line_on_first_line_is_noop() {
    let mut ed = copy_test_editor("f-[o]>o\nbar\n");
    run_copy(&mut ed, false, 1);
    assert_eq!(ed.current_selections().len(), 1); // no copy added
}

#[test]
fn copy_to_prev_line_clamps_to_shorter_target_line() {
    // "hi\nhello\n" — cursor at column 4 of line 1 ('o').
    // Line 0 is "hi\n" (only 2 real chars). Should clamp to last char 'i'.
    let mut ed = copy_test_editor("hi\nhell-[o]>\n");
    run_copy(&mut ed, false, 1);
    assert_eq!(ed.current_selections().len(), 2);
    // Copy should land at last char of "hi" = 'i' at offset 1.
    assert_eq!(ed.current_selections().primary().head(), 1);
}

#[test]
fn copy_prev_line_count_3() {
    // Mirror of copy_next_line_count_3, shifting up instead of down.
    assert_copy_state(
        "a\nb\nc\nd\n-[e]>\n",
        false,
        3,
        "a\n-[b]>\n-[c]>\n-[d]>\n-[e]>\n",
    );
}

#[test]
fn copy_prev_line_count_exceeds_buffer_clamps() {
    // Mirror: only 2 lines exist above the cursor's line.
    assert_copy_state("a\nb\n-[c]>\n", false, 10, "-[a]>\n-[b]>\n-[c]>\n");
}

#[test]
fn copy_prev_line_count_3_primary_lands_on_furthest_copy() {
    let mut ed = copy_test_editor("a\nb\nc\nd\n-[e]>\n");
    run_copy(&mut ed, false, 3);
    // Furthest copy (3 lines up from 'e') is on 'b'.
    assert_eq!(ed.current_selections().primary().head(), 2);
}

#[test]
fn copy_prev_line_count_usize_max_returns_instantly() {
    let mut ed = copy_test_editor("a\nb\nc\nd\n-[e]>\n");
    run_copy(&mut ed, false, usize::MAX);
    assert_eq!(ed.current_selections().len(), 5); // original + one copy per remaining line
}
