use super::*;
use pretty_assertions::assert_eq;

// ── Tests ─────────────────────────────────────────────────────────────────────

/// `c` must group the delete and the subsequent insert session into a single
/// undo step. One `u` should restore the original selection, not leave a
/// half-undone intermediate state.
///
/// This test feeds real key events through `handle_key` so it catches bugs
/// in the mapping itself (e.g. reverting to ungrouped `apply_edit` for the
/// delete), not just in the underlying group primitives.
#[test]
fn c_groups_delete_and_insert_into_one_undo_step() {
    let mut ed = editor_from("-[hell]>o\n");

    // `c` — delete "hell", enter Insert.
    ed.handle_key(key('c'));
    assert_eq!(ed.mode, Mode::Insert);

    // Type the replacement.
    ed.handle_key(key('h'));
    ed.handle_key(key('i'));

    // Exit Insert — commits the group.
    ed.handle_key(key_esc());
    assert_eq!(ed.mode, Mode::Normal);
    assert_eq!(ed.doc().text().to_string(), "hio\n");

    // One undo should restore the original word entirely.
    ed.handle_key(key('u'));
    assert_eq!(state(&ed), "-[hell]>o\n");

    // Only one revision was recorded.
    assert!(!ed.doc().can_undo());
}

// ── `d` pushes deleted text onto the kill ring ─────────────────────────────

/// Deleting a selection must push the deleted text onto the kill ring.
/// A bug in the mapping that removed the `yank_selections` call before
/// `delete_selection` would leave the ring empty — invisible to pure tests.
#[test]
fn d_yanks_selection_into_register_before_deleting() {
    let mut ed = editor_from("-[hell]>o\n");
    ed.handle_key(key('d'));

    assert_eq!(ed.doc().text().to_string(), "o\n", "buffer after delete");
    assert_eq!(
        ed.kill_ring.head(),
        Some(["hell".to_string()].as_slice()),
        "kill ring head after delete"
    );
}

// ── `y` yanks without modifying the buffer ─────────────────────────────────

/// `y` must write to the system clipboard (in-memory mirror) and push to the
/// kill ring, without changing the buffer or the selection.
/// This is the only way to test that `y` actually writes the correct storage —
/// pure tests of `yank_selections` never touch `Editor.registers` or `kill_ring`.
#[test]
fn y_populates_register_without_changing_buffer() {
    use crate::ops::register::CLIPBOARD_REGISTER;

    let mut ed = editor_from("-[hell]>o\n");
    ed.handle_key(key('y'));

    assert_eq!(state(&ed), "-[hell]>o\n", "buffer+selection unchanged");
    // Bare `y` writes to system clipboard (in-memory mirror in headless tests)
    // AND pushes to the kill ring.
    assert_eq!(reg(&ed, CLIPBOARD_REGISTER), &["hell"], "clipboard populated");
    assert_eq!(
        ed.kill_ring.head(),
        Some(["hell".to_string()].as_slice()),
        "kill ring head populated"
    );
}

// ── `r<char>` pending-key replace sequence ─────────────────────────────────

/// `r` sets a wait-char constructor; the following character replaces every
/// grapheme in every selection; and `Esc` after a bare `r` cancels without
/// side effects.
#[test]
fn r_then_char_replaces_every_grapheme_in_selection() {
    let mut ed = editor_from("-[hell]>o\n");

    ed.handle_key(key('r'));
    assert!(ed.wait_char.is_some(), "wait_char set after 'r'");

    ed.handle_key(key('x'));
    assert!(
        ed.wait_char.is_none(),
        "wait_char cleared after replacement char"
    );
    assert_eq!(state(&ed), "-[xxxx]>o\n");
}

#[test]
fn r_then_esc_cancels_without_side_effects() {
    let mut ed = editor_from("-[hell]>o\n");
    ed.handle_key(key('r'));
    // Esc resets wait_char (and all other pending state).
    ed.handle_key(key_esc());

    assert!(ed.wait_char.is_none());
    assert_eq!(
        state(&ed),
        "-[hell]>o\n",
        "buffer unchanged after cancelled replace"
    );
}

/// Unlike `r`, find/till has extend duality — this exercises that branch
/// being cleanly torn down on Esc.
#[test]
fn f_then_esc_cancels_without_side_effects() {
    let mut ed = editor_from("-[h]>ello a\n");
    ed.handle_key(key('f'));
    assert!(ed.wait_char.is_some(), "wait_char set after 'f'");
    ed.handle_key(key_esc());

    assert!(ed.wait_char.is_none(), "wait_char cleared after Esc");
    assert!(ed.pending_char.is_none(), "pending_char not set");
    assert_eq!(
        state(&ed),
        "-[h]>ello a\n",
        "buffer and cursor unchanged after cancelled find"
    );
}

// ── `m i w` three-key text-object sequence ─────────────────────────────────

/// The trie must advance through `m` (Interior) → `mi` (Interior) → `miw`
/// (Leaf) and dispatch the correct text-object command on the third key.
/// This exercises the entire three-key pipeline end-to-end.
#[test]
fn m_i_w_selects_inner_word() {
    let mut ed = editor_from("-[h]>ello world\n");

    ed.handle_key(key('m'));
    assert_eq!(
        ed.pending_keys.len(),
        1,
        "pending_keys has 'm' after first press"
    );

    ed.handle_key(key('i'));
    assert_eq!(
        ed.pending_keys.len(),
        2,
        "pending_keys has 'm','i' after second press"
    );

    ed.handle_key(key('w'));
    assert!(
        ed.pending_keys.is_empty(),
        "pending_keys cleared after dispatch"
    );
    assert_eq!(state(&ed), "-[hello]> world\n");
}

/// An unrecognised object char after `ma` must clear pending state without
/// modifying the buffer or the selection.
#[test]
fn m_a_unknown_char_falls_through_cleanly() {
    let mut ed = editor_from("-[h]>ello\n");

    ed.handle_key(key('m'));
    ed.handle_key(key('a'));
    // '~' is not a known text-object char — NoMatch clears pending state.
    ed.handle_key(key('~'));

    assert!(
        ed.pending_keys.is_empty(),
        "pending_keys cleared on NoMatch"
    );
    // Selection and buffer are unchanged.
    assert_eq!(state(&ed), "-[h]>ello\n");
}

// ── `e` extend-mode toggle ─────────────────────────────────────────────────

/// `e` must toggle `extend` on and off. While extend is active, motions must
/// grow the selection rather than collapse it to a cursor.
#[test]
fn e_toggles_extend_mode_and_motions_extend_selection() {
    let mut ed = editor_from("-[h]>ello\n");
    assert_eq!(ed.mode, Mode::Normal, "Normal mode initially");

    // Toggle extend on.
    ed.handle_key(key('e'));
    assert_eq!(ed.mode, Mode::Extend, "Extend mode after 'e'");

    // A motion in extend mode should grow the selection, not move a cursor.
    ed.handle_key(key('l'));
    assert_eq!(state(&ed), "-[he]>llo\n", "selection extended right by one");

    // Toggle extend off.
    ed.handle_key(key('e'));
    assert_eq!(ed.mode, Mode::Normal, "Normal mode after second 'e'");
}

// ── `x` select-line ────────────────────────────────────────────────────────

/// `x` selects the full current line including the trailing `\n`.
#[test]
fn x_selects_full_line_from_cursor() {
    let mut ed = editor_from("hello -[w]>orld\nfoo\n");
    ed.handle_key(key('x'));
    assert_eq!(state(&ed), "-[hello world\n]>foo\n");
}

/// `x` on a line that is already fully selected jumps to the next line.
#[test]
fn x_on_full_line_jumps_to_next() {
    let mut ed = editor_from("-[hello world\n]>foo\n");
    ed.handle_key(key('x'));
    assert_eq!(state(&ed), "hello world\n-[foo\n]>");
}

/// In extend mode, `x` extends the selection to include the next line.
#[test]
fn x_in_extend_mode_accumulates_lines() {
    let mut ed = editor_from("hello -[w]>orld\nfoo\nbar\n");
    // First `x` in normal mode: select current line.
    ed.handle_key(key('x'));
    assert_eq!(state(&ed), "-[hello world\n]>foo\nbar\n", "line 1 selected");
    // Toggle extend mode.
    ed.handle_key(key('e'));
    // `x` in extend mode: extend to include next line.
    ed.handle_key(key('x'));
    assert_eq!(
        state(&ed),
        "-[hello world\nfoo\n]>bar\n",
        "lines 1-2 selected"
    );
    // Another `x`: extend to line 3.
    ed.handle_key(key('x'));
    assert_eq!(
        state(&ed),
        "-[hello world\nfoo\nbar\n]>",
        "lines 1-3 selected"
    );
}

/// `x` repeated in normal mode walks downward: each press moves to the next line.
#[test]
fn x_repeated_walks_lines_down() {
    let mut ed = editor_from("hello -[w]>orld\nfoo\nbar\n");
    ed.handle_key(key('x'));
    assert_eq!(state(&ed), "-[hello world\n]>foo\nbar\n", "line 1");
    ed.handle_key(key('x'));
    assert_eq!(state(&ed), "hello world\n-[foo\n]>bar\n", "line 2");
    ed.handle_key(key('x'));
    assert_eq!(state(&ed), "hello world\nfoo\n-[bar\n]>", "line 3");
}

/// `x` at the last line stays put (no panic).
#[test]
fn x_clamps_at_last_line() {
    let mut ed = editor_from("hello\n-[world\n]>");
    ed.handle_key(key('x'));
    assert_eq!(state(&ed), "hello\n-[world\n]>");
}

/// `X` selects the current line with a backward selection (anchor=`\n`, head=start).
#[test]
fn shift_x_selects_line_backward() {
    let mut ed = editor_from("hello -[w]>orld\nfoo\n");
    ed.handle_key(key('X'));
    assert_eq!(state(&ed), "<[hello world\n]-foo\n");
}

/// `X` repeated in normal mode walks upward: each press moves to the previous line.
#[test]
fn shift_x_repeated_walks_lines_up() {
    let mut ed = editor_from("aaa\nbbb\nhello -[w]>orld\n");
    ed.handle_key(key('X'));
    assert_eq!(state(&ed), "aaa\nbbb\n<[hello world\n]-", "line 3");
    ed.handle_key(key('X'));
    assert_eq!(state(&ed), "aaa\n<[bbb\n]-hello world\n", "line 2");
    ed.handle_key(key('X'));
    assert_eq!(state(&ed), "<[aaa\n]-bbb\nhello world\n", "line 1");
}

/// `X` at the first line stays put (no panic).
#[test]
fn shift_x_clamps_at_first_line() {
    let mut ed = editor_from("<[hello world\n]-foo\n");
    ed.handle_key(key('X'));
    assert_eq!(state(&ed), "<[hello world\n]-foo\n");
}

/// Ctrl+x accumulates lines downward (extend behavior).
#[test]
fn ctrl_x_extends_selection_down() {
    let mut ed = editor_from("hello -[w]>orld\nfoo\nbar\n");
    ed.handle_key(key('x'));
    assert_eq!(state(&ed), "-[hello world\n]>foo\nbar\n", "line 1 selected");
    ed.handle_key(key_ctrl('x'));
    assert_eq!(state(&ed), "-[hello world\nfoo\n]>bar\n", "lines 1-2");
    ed.handle_key(key_ctrl('x'));
    assert_eq!(state(&ed), "-[hello world\nfoo\nbar\n]>", "lines 1-3");
}

/// Ctrl+X accumulates lines upward (extend behavior).
#[test]
fn ctrl_shift_x_extends_selection_up() {
    let mut ed = editor_from("aaa\nbbb\nhello -[w]>orld\n");
    ed.handle_key(key('X'));
    assert_eq!(state(&ed), "aaa\nbbb\n<[hello world\n]-", "line 3 selected");
    ed.handle_key(key_ctrl('X'));
    assert_eq!(state(&ed), "aaa\n<[bbb\nhello world\n]-", "lines 2-3");
    ed.handle_key(key_ctrl('X'));
    assert_eq!(state(&ed), "<[aaa\nbbb\nhello world\n]-", "lines 1-3");
}

/// `x` (forward line) then `X` (backward line): flips direction, stays on same line
/// when already at the first line (no line to jump back to).
#[test]
fn x_then_shift_x_flips_direction() {
    let mut ed = editor_from("hello -[w]>orld\nfoo\n");
    ed.handle_key(key('x'));
    assert_eq!(state(&ed), "-[hello world\n]>foo\n");
    // sel.start() == line_start AND top_line == 0 → can't jump, just flips to backward.
    ed.handle_key(key('X'));
    assert_eq!(state(&ed), "<[hello world\n]-foo\n");
}

/// `X` (backward line) then `x` (forward line): jumps to next line (flips direction).
#[test]
fn shift_x_then_x_flips_direction() {
    let mut ed = editor_from("aaa\nhello -[w]>orld\nfoo\n");
    ed.handle_key(key('X'));
    assert_eq!(state(&ed), "aaa\n<[hello world\n]-foo\n");
    // sel.end() is at `\n` of line 1 → x jumps to next line (forward selection).
    ed.handle_key(key('x'));
    assert_eq!(state(&ed), "aaa\nhello world\n-[foo\n]>");
}

/// Ctrl+x after `X` (backward selection): extends forward, flipping direction.
#[test]
fn ctrl_x_after_shift_x() {
    // Cursor mid-line so `X` selects the current line (doesn't jump back).
    let mut ed = editor_from("aaa\nfoo -[b]>ar\nbaz\n");
    ed.handle_key(key('X'));
    assert_eq!(state(&ed), "aaa\n<[foo bar\n]-baz\n");
    // Ctrl+x extends forward (adds next line, switches to forward selection).
    ed.handle_key(key_ctrl('x'));
    assert_eq!(state(&ed), "aaa\n-[foo bar\nbaz\n]>");
}

/// Ctrl+X after `x` (forward selection): extends backward, flipping direction.
#[test]
fn ctrl_shift_x_after_x() {
    let mut ed = editor_from("aaa\nbbb\n-[f]>oo\n");
    ed.handle_key(key('x'));
    assert_eq!(state(&ed), "aaa\nbbb\n-[foo\n]>");
    // Ctrl+X extends backward (adds previous line, switches to backward selection).
    ed.handle_key(key_ctrl('X'));
    assert_eq!(state(&ed), "aaa\n<[bbb\nfoo\n]-");
}

// ── `o` / `O` open-line variants ──────────────────────────────────────────

/// `o` must insert a blank line *below* the current line, position the cursor
/// on it, and enter Insert mode — all as a single composed operation.
#[test]
fn o_opens_line_below_and_enters_insert() {
    let mut ed = editor_from("-[h]>ello\n");
    ed.handle_key(key('o'));

    assert_eq!(ed.mode, Mode::Insert);
    assert_eq!(ed.doc().text().to_string(), "hello\n\n");
    // Cursor should be on the new blank line (the second '\n').
    assert_eq!(state(&ed), "hello\n-[\n]>");
}

/// `o` on a blank line must open a new blank line *below* it, not overshoot
/// into the line after.
/// Regression: `goto_line_end + move_right` advanced past the `\n` on empty
/// lines, inserting the new `\n` one line too low.
#[test]
fn o_on_empty_line_places_cursor_on_new_blank_line() {
    let mut ed = editor_from("AAA\nBBB\n-[\n]>CCC\n");
    ed.handle_key(key('o'));

    assert_eq!(ed.mode, Mode::Insert);
    assert_eq!(ed.doc().text().to_string(), "AAA\nBBB\n\n\nCCC\n");
    assert_eq!(state(&ed), "AAA\nBBB\n\n-[\n]>CCC\n");
}

/// `O` must insert a blank line *above* the current line, position the cursor
/// on it, and enter Insert mode.
#[test]
fn capital_o_opens_line_above_and_enters_insert() {
    let mut ed = editor_from("foo\n-[b]>ar\n");
    ed.handle_key(key('O'));

    assert_eq!(ed.mode, Mode::Insert);
    assert_eq!(ed.doc().text().to_string(), "foo\n\nbar\n");
    // Cursor on the new blank line between "foo" and "bar".
    assert_eq!(state(&ed), "foo\n-[\n]>bar\n");
}

// ── Insert-entry variants position the cursor correctly ────────────────────

/// `a` collapses to one past the end of the selection and enters Insert mode.
/// On a collapsed cursor this is identical to the old "append after cursor".
#[test]
fn a_enters_insert_after_selection_end() {
    let mut ed = editor_from("-[h]>ello\n");
    ed.handle_key(key('a'));

    assert_eq!(ed.mode, Mode::Insert);
    assert_eq!(state(&ed), "h-[e]>llo\n");
}

/// `A` must jump to the end of the line and then step one right (onto the
/// newline), then enter Insert mode — "append at end of line".
#[test]
fn capital_a_enters_insert_after_end_of_line() {
    let mut ed = editor_from("-[h]>ello\n");
    ed.handle_key(key('A'));

    assert_eq!(ed.mode, Mode::Insert);
    assert_eq!(state(&ed), "hello-[\n]>");
}

/// `I` jumps to the first non-blank character on the line and enters Insert mode.
#[test]
fn capital_i_enters_insert_at_line_start() {
    let mut ed = editor_from("  -[hello]>\n");
    ed.handle_key(key('I'));

    assert_eq!(ed.mode, Mode::Insert);
    assert_eq!(state(&ed), "  -[h]>ello\n");
}

/// `i` on a multi-char selection collapses to the selection start (not just the
/// cursor head) and enters Insert mode.
#[test]
fn i_on_wide_selection_collapses_to_start() {
    // Backward selection: head=0 (h), anchor=3 (last l) → start=0.
    let mut ed = editor_from("<[hell]-o\n");
    ed.handle_key(key('i'));

    assert_eq!(ed.mode, Mode::Insert);
    assert_eq!(state(&ed), "-[h]>ello\n");
}

/// `a` on a multi-char selection collapses to one past the selection end and
/// enters Insert mode — the cursor lands after the last selected character.
#[test]
fn a_on_wide_selection_collapses_after_end() {
    // Forward selection: anchor=0 (h), head=3 (l) → end=3, one past = 4.
    let mut ed = editor_from("-[hel]>lo\n");
    ed.handle_key(key('a'));

    assert_eq!(ed.mode, Mode::Insert);
    assert_eq!(state(&ed), "hel-[l]>o\n");
}

// ── `S` splits selection on newlines ──────────────────────────────────────────

/// `S` must split a multi-line selection into one cursor per line, which is
/// the primary way to turn a block selection into a multi-cursor.
#[test]
fn capital_s_splits_selection_on_newlines() {
    let mut ed = editor_from("-[foo\nbar\nbaz]>\n");

    ed.handle_key(key('S'));

    assert_eq!(state(&ed), "-[foo]>\n-[bar]>\n-[baz]>\n");
}

// ── `ctrl+,` removes the primary selection ────────────────────────────────────

/// `ctrl+,` must drop the primary selection and promote one of the secondaries,
/// leaving all other cursors intact. Plain `,` must still keep only the primary.
#[test]
fn ctrl_comma_removes_primary_selection() {
    let mut ed = editor_from("-[h]>ello -[w]>orld\n");

    ed.handle_key(key_ctrl(','));

    // Primary ('h') is dropped; 'w' becomes the new (only) primary.
    assert_eq!(state(&ed), "hello -[w]>orld\n");
}

#[test]
fn plain_comma_still_keeps_primary_selection() {
    let mut ed = editor_from("-[h]>ello -[w]>orld\n");

    ed.handle_key(key(','));

    // Only the primary ('h') survives.
    assert_eq!(state(&ed), "-[h]>ello world\n");
}

// ── `o` in extend mode flips the selection ────────────────────────────────────

/// In extend mode `o` must swap anchor and head (Vim visual-mode `o`), letting
/// the user extend the selection in the opposite direction. In normal mode `o`
/// must still open a line below — the extend branch must not shadow it.
#[test]
fn o_in_extend_mode_flips_selection() {
    let mut ed = editor_from("-[hell]>o\n");
    ed.mode = Mode::Extend;

    ed.handle_key(key('o'));

    // anchor and head are swapped — selection is now backward.
    assert_eq!(state(&ed), "<[hell]-o\n");
    // extend mode is still active (flip doesn't exit it).
    assert_eq!(ed.mode, Mode::Extend);
}

#[test]
fn o_in_normal_mode_still_opens_line_below() {
    let mut ed = editor_from("-[h]>ello\n");
    // extend is off (default).

    ed.handle_key(key('o'));

    assert_eq!(ed.mode, Mode::Insert);
    assert_eq!(ed.doc().text().to_string(), "hello\n\n");
}

// ── `;` collapses selection AND clears extend mode ─────────────────────────

/// `;` must (a) collapse every selection to its head and (b) clear the
/// `extend` flag. The extend side-effect only exists in the mapping — a pure
/// `cmd_collapse_selection` test cannot see it.
#[test]
fn semicolon_collapses_selection_and_resets_extend() {
    let mut ed = editor_from("-[hell]>o\n");
    ed.mode = Mode::Extend;

    ed.handle_key(key(';'));

    assert_eq!(ed.mode, Mode::Normal, "extend cleared by ';'");
    // head of the original selection was 'l' (last char of "hell").
    assert_eq!(state(&ed), "hel-[l]>o\n");
}

// ── `o`/`O` undo grouping ─────────────────────────────────────────────────────

/// `o` must group the structural newline insertion and the subsequent insert
/// session into one undo step. Without the fix, the newline would be a
/// separate `apply_edit` revision, so `u` would only undo the typed text and
/// leave behind an empty line.
#[test]
fn o_groups_newline_and_insert_session_into_one_undo_step() {
    let mut ed = editor_from("-[h]>ello\n");

    ed.handle_key(key('o'));
    assert_eq!(ed.mode, Mode::Insert);

    ed.handle_key(key('w'));
    ed.handle_key(key('o'));
    ed.handle_key(key('r'));
    ed.handle_key(key('l'));
    ed.handle_key(key('d'));

    ed.handle_key(key_esc());
    assert_eq!(ed.doc().text().to_string(), "hello\nworld\n");

    ed.handle_key(key('u'));
    assert_eq!(state(&ed), "-[h]>ello\n");
    assert!(!ed.doc().can_undo());
}

/// Same undo-grouping invariant for `O` (open line above).
#[test]
fn capital_o_groups_newline_and_insert_session_into_one_undo_step() {
    let mut ed = editor_from("foo\n-[b]>ar\n");

    ed.handle_key(key('O'));
    assert_eq!(ed.mode, Mode::Insert);

    ed.handle_key(key('n'));
    ed.handle_key(key('e'));
    ed.handle_key(key('w'));

    ed.handle_key(key_esc());
    assert_eq!(ed.doc().text().to_string(), "foo\nnew\nbar\n");

    ed.handle_key(key('u'));
    assert_eq!(state(&ed), "foo\n-[b]>ar\n");
    assert!(!ed.doc().can_undo());
}

// ── Plain insert session groups all chars into one undo step ──────────────

/// `i` with a non-collapsed selection must collapse to the start of the
/// selection and enter Insert — it must NOT replace the selected text.
#[test]
fn i_collapses_selection_to_start() {
    let mut ed = editor_from("-[hell]>o\n");
    ed.handle_key(key('i'));

    assert_eq!(ed.mode, Mode::Insert);
    // Cursor collapsed to 'h' — nothing deleted.
    assert_eq!(state(&ed), "-[h]>ello\n");
    assert_eq!(ed.doc().text().to_string(), "hello\n");
}

/// `i` + typing + `Esc` must commit as one undo step, just like `c`. A single
/// `u` should restore the original buffer — not leave partial edits behind.
#[test]
fn i_groups_insert_session_into_one_undo_step() {
    let mut ed = editor_from("-[h]>ello\n");

    ed.handle_key(key('i'));
    assert_eq!(ed.mode, Mode::Insert);

    ed.handle_key(key('X'));
    ed.handle_key(key('Y'));

    ed.handle_key(key_esc());
    assert_eq!(ed.mode, Mode::Normal);
    assert_eq!(ed.doc().text().to_string(), "XYhello\n");

    // One undo restores the original state completely.
    ed.handle_key(key('u'));
    assert_eq!(state(&ed), "-[h]>ello\n");

    // Only one revision was recorded.
    assert!(!ed.doc().can_undo());
}

// ── Line text objects (mil / mal) ─────────────────────────────────────────────

#[test]
fn mil_selects_line_content_excluding_newline() {
    let mut ed = editor_from("hell-[o]> world\nsecond\n");
    ed.handle_key(key('m'));
    ed.handle_key(key('i'));
    ed.handle_key(key('l'));
    assert_eq!(state(&ed), "-[hello world]>\nsecond\n");
}

#[test]
fn mal_selects_line_including_newline() {
    let mut ed = editor_from("hell-[o]> world\nsecond\n");
    ed.handle_key(key('m'));
    ed.handle_key(key('a'));
    ed.handle_key(key('l'));
    assert_eq!(state(&ed), "-[hello world\n]>second\n");
}

#[test]
fn mil_on_empty_line_is_noop() {
    // An empty line has no content — selection should not change.
    let mut ed = editor_from("foo\n-[\n]>bar\n");
    ed.handle_key(key('m'));
    ed.handle_key(key('i'));
    ed.handle_key(key('l'));
    assert_eq!(state(&ed), "foo\n-[\n]>bar\n");
}

// ── Register prefix `"<reg>` ────────────────────────────────────────────────

/// `"5y` must write text into register '5', leaving `'"'` empty.
#[test]
fn register_prefix_routes_yank_to_named_register() {
    let mut ed = editor_from("-[hell]>o\n");
    ed.handle_key(key('"'));
    ed.handle_key(key('5'));
    ed.handle_key(key('y'));

    assert_eq!(state(&ed), "-[hell]>o\n", "buffer unchanged");
    assert_eq!(reg(&ed, '5'), &["hell"], "register '5' populated");
    assert!(reg(&ed, '"').is_empty(), "'\"' register untouched");
}

/// After `"5y`, the prefix is consumed. The next bare `y` writes to clipboard
/// and the kill ring (not to register '5').
#[test]
fn register_prefix_clears_after_one_operation() {
    use crate::ops::register::CLIPBOARD_REGISTER;

    let mut ed = editor_from("-[hell]>o\n");
    ed.handle_key(key('"'));
    ed.handle_key(key('5'));
    ed.handle_key(key('y'));

    // Now the prefix is cleared — move right to get a different selection,
    // then yank again without a prefix.
    ed.handle_key(key('l')); // move right
    ed.handle_key(key('y')); // bare yank — writes clipboard + kill ring

    // The second yank updated the clipboard, not register '5'.
    assert!(!reg(&ed, CLIPBOARD_REGISTER).is_empty(), "clipboard written by bare y");
    // Kill ring head holds the latest bare yank.
    assert!(ed.kill_ring.head().is_some(), "kill ring head set by bare y");
    // '5' is unchanged from the first yank.
    assert_eq!(reg(&ed, '5'), &["hell"], "register '5' unchanged");
}

/// `Esc` after `"` cancels the prefix — the next `y` writes to clipboard + ring.
#[test]
fn esc_cancels_register_prefix() {
    use crate::ops::register::CLIPBOARD_REGISTER;

    let mut ed = editor_from("-[hell]>o\n");
    ed.handle_key(key('"'));
    ed.handle_key(key_esc()); // cancel
    ed.handle_key(key('y'));

    assert_eq!(reg(&ed, CLIPBOARD_REGISTER), &["hell"], "clipboard populated");
    assert_eq!(
        ed.kill_ring.head(),
        Some(["hell".to_string()].as_slice()),
        "kill ring head populated"
    );
    assert!(reg(&ed, '5').is_empty(), "register '5' untouched");
}

/// `"3y` then `"3p` must round-trip through in-memory register '3'.
/// Digit registers are symmetric: yank writes RegisterSet['3'], paste reads it.
#[test]
fn paste_from_named_register() {
    use crate::ops::register::CLIPBOARD_REGISTER;

    let mut ed = editor_from("-[hello]>world\n");

    // "3y: yank "hello" into in-memory register '3'.
    ed.handle_key(key('"'));
    ed.handle_key(key('3'));
    ed.handle_key(key('y'));

    assert_eq!(reg(&ed, '3'), &["hello"], "register '3' populated by yank");

    // Move to a fresh position to make the paste visible.
    ed.handle_key(key('w')); // move word → selection on 'w' of "world"

    // Seed clipboard with "wrong" to verify "3p doesn't read it.
    ed.registers.write_text(CLIPBOARD_REGISTER, vec!["wrong".to_string()]);

    ed.handle_key(key('"'));
    ed.handle_key(key('3'));
    ed.handle_key(key('p')); // "3p → in-memory register '3' = "hello"

    assert!(
        ed.doc().text().to_string().contains("hello"),
        "pasted from in-memory register '3'"
    );
    assert!(
        !ed.doc().text().to_string().contains("wrong"),
        "clipboard not used by \"3p"
    );
}

/// `d` pushes to the kill ring; `"3p` reads in-memory register '3' (empty),
/// NOT ring slot 3. Digit registers are decoupled from the ring.
#[test]
fn digit_register_decoupled_from_kill_ring() {
    // Push 4 deletes so ring slot 3 = "P" (oldest).
    let mut ed = editor_from("-[P]>QRS\n");
    for _ in 0..4 {
        ed.handle_key(key('d'));
    }
    // ring: slot 0 = "S", slot 1 = "R", slot 2 = "Q", slot 3 = "P"
    // in-memory register '3' is empty (nothing yanked into it).
    assert!(
        reg(&ed, '3').is_empty(),
        "register '3' is empty — d never writes named registers"
    );

    // "3p reads in-memory register '3' which is empty → paste is a no-op.
    let text_before = ed.doc().text().to_string();
    ed.handle_key(key('"'));
    ed.handle_key(key('3'));
    ed.handle_key(key('p'));

    assert_eq!(
        ed.doc().text().to_string(),
        text_before,
        "\"3p is a no-op when register '3' is empty (not ring slot 3)"
    );
}

/// `"by` discards the yank — `'"'` must remain empty.
#[test]
fn black_hole_register_via_prefix() {
    use crate::ops::register::BLACK_HOLE_REGISTER;

    let mut ed = editor_from("-[hell]>o\n");
    ed.handle_key(key('"'));
    ed.handle_key(key('b'));
    ed.handle_key(key('y'));

    assert_eq!(state(&ed), "-[hell]>o\n", "buffer unchanged");
    assert!(reg(&ed, '"').is_empty(), "'\"' register untouched");
    assert!(
        ed.registers.read(BLACK_HOLE_REGISTER).is_none(),
        "black hole register returns None"
    );
}

// ── Clipboard register fallback (in-memory mirror) ─────────────────────────

/// When the system clipboard is unavailable, `"cy` falls back to the in-memory
/// mirror and logs a Warning. The mirror is then used by `"cp`.
#[test]
fn clipboard_register_falls_back_to_memory_when_unavailable() {
    use crate::editor::Severity;
    use crate::ops::register::CLIPBOARD_REGISTER;

    let mut ed = editor_from("-[hello]>\n");
    // Simulate a headless environment with no clipboard server.
    ed.clipboard.force_unavailable();

    ed.handle_key(key('"'));
    ed.handle_key(key('c'));
    ed.handle_key(key('y'));

    // A Warning must have been logged.
    assert!(
        ed.message_log
            .entries()
            .any(|e| e.severity == Severity::Warning),
        "expected a Warning for clipboard unavailable"
    );

    // In-memory mirror must hold the yanked text.
    assert_eq!(
        reg(&ed, CLIPBOARD_REGISTER),
        &["hello"],
        "in-memory mirror populated"
    );

    // Move right so cursor is now on 'o', giving a distinct selection.
    ed.handle_key(key('l'));

    // `"cp` should read from the in-memory mirror and paste "hello".
    ed.handle_key(key('"'));
    ed.handle_key(key('c'));
    ed.handle_key(key('p'));

    assert!(
        ed.doc().text().to_string().contains("hello"),
        "pasted from in-memory mirror"
    );
}

// ── Kill-ring register (`"k`) ─────────────────────────────────────────────────

/// `"kp` must paste the kill-ring head, not the clipboard.
#[test]
fn kill_ring_register_pastes_ring_head() {
    use crate::ops::register::CLIPBOARD_REGISTER;

    let mut ed = editor_from("-[hello]>world\n");
    ed.handle_key(key('d')); // delete "hello" → ring head = ["hello"]

    // Seed clipboard with "wrong" to confirm "kp doesn't read it.
    ed.registers.write_text(CLIPBOARD_REGISTER, vec!["wrong".to_string()]);

    ed.handle_key(key('"'));
    ed.handle_key(key('k'));
    ed.handle_key(key('p'));

    assert!(
        ed.doc().text().to_string().contains("hello"),
        "\"kp pasted ring head"
    );
    assert!(
        !ed.doc().text().to_string().contains("wrong"),
        "clipboard not used by \"kp"
    );
}

/// `"kp` seeds the `[`/`]` cycle so pressing `[` after cycles to the older entry.
#[test]
fn kill_ring_register_paste_seeds_cycle() {
    // Build a ring with 2 entries: push "first" then "second" (head).
    let mut ed = editor_from("-[second]>X\n");
    ed.handle_key(key('d')); // ring head = ["second"]

    // Manually push an older entry.
    ed.kill_ring.push(vec!["first".to_string()]);
    // ring: head = ["first"] (newest push), slot 1 = ["second"]

    // "kp: paste ring head ("first"). This should open a paste session
    // seeded at the head so [ can cycle to the older entry.
    ed.handle_key(key('"'));
    ed.handle_key(key('k'));
    ed.handle_key(key('p'));

    assert!(
        ed.doc().text().to_string().contains("first"),
        "\"kp pasted ring head"
    );

    // [ should cycle to the next-older entry ("second").
    ed.handle_key(key('['));
    assert!(
        ed.doc().text().to_string().contains("second"),
        "[ after \"kp cycled to older ring entry"
    );
}

/// `"ky` pushes the yank onto the kill ring and does NOT touch the clipboard.
#[test]
fn kill_ring_register_yank_pushes_ring_only() {
    use crate::ops::register::CLIPBOARD_REGISTER;

    let mut ed = editor_from("-[hello]>world\n");
    // Ensure clipboard starts empty.
    assert!(reg(&ed, CLIPBOARD_REGISTER).is_empty(), "clipboard starts empty");

    ed.handle_key(key('"'));
    ed.handle_key(key('k'));
    ed.handle_key(key('y')); // "ky → ring push, no clipboard

    assert_eq!(
        ed.kill_ring.head(),
        Some(["hello".to_string()].as_slice()),
        "ring head set by \"ky"
    );
    assert!(
        reg(&ed, CLIPBOARD_REGISTER).is_empty(),
        "\"ky must not write the clipboard"
    );
}

/// `"kd` deletes and pushes to the ring, identical to bare `d`.
#[test]
fn kill_ring_register_delete_pushes_ring() {
    let mut ed = editor_from("-[hello]>world\n");

    ed.handle_key(key('"'));
    ed.handle_key(key('k'));
    ed.handle_key(key('d')); // "kd → delete + push ring

    assert_eq!(ed.doc().text().to_string(), "world\n", "buffer after delete");
    assert_eq!(
        ed.kill_ring.head(),
        Some(["hello".to_string()].as_slice()),
        "ring head set by \"kd"
    );
}

// ── surround-add (`mw`) ───────────────────────────────────────────────────────

#[test]
fn mw_wraps_with_bracket() {
    let mut ed = editor_from("-[bar]>\n");
    ed.handle_key(key('m'));
    ed.handle_key(key('w'));
    ed.handle_key(key('['));
    assert_eq!(state(&ed), "[bar-[]]>\n");
}

#[test]
fn mw_wraps_with_brace_via_close_char() {
    // `mw}` should normalize to the pair `{` `}`.
    let mut ed = editor_from("-[bar]>\n");
    ed.handle_key(key('m'));
    ed.handle_key(key('w'));
    ed.handle_key(key('}'));
    assert_eq!(state(&ed), "{bar-[}]>\n");
}

#[test]
fn mw_wraps_symmetric_quote() {
    let mut ed = editor_from("-[bar]>\n");
    ed.handle_key(key('m'));
    ed.handle_key(key('w'));
    ed.handle_key(key('"'));
    assert_eq!(state(&ed), "\"bar-[\"]>\n");
}

#[test]
fn mw_wraps_unknown_char_symmetric() {
    // `*` is not a configured pair — wraps symmetrically open == close == `*`.
    let mut ed = editor_from("-[bar]>\n");
    ed.handle_key(key('m'));
    ed.handle_key(key('w'));
    ed.handle_key(key('*'));
    assert_eq!(state(&ed), "*bar-[*]>\n");
}

#[test]
fn mw_wraps_multi_cursor() {
    let mut ed = editor_from("-[ab]>c-[de]>f\n");
    ed.handle_key(key('m'));
    ed.handle_key(key('w'));
    ed.handle_key(key('('));
    assert_eq!(state(&ed), "(ab-[)]>c(de-[)]>f\n");
}

#[test]
fn mw_wraps_cursor_one_char() {
    let mut ed = editor_from("-[h]>ello\n");
    ed.handle_key(key('m'));
    ed.handle_key(key('w'));
    ed.handle_key(key('['));
    assert_eq!(state(&ed), "[h-[]]>ello\n");
}

#[test]
fn mw_esc_cancels() {
    let mut ed = editor_from("-[bar]>\n");
    ed.handle_key(key('m'));
    ed.handle_key(key('w'));
    ed.handle_key(key_esc()); // cancel before typing the delimiter
    assert_eq!(state(&ed), "-[bar]>\n");
}

#[test]
fn mw_wraps_when_auto_pairs_disabled() {
    // surround-add uses the pairs table only as a lookup; it ignores the
    // auto-pairs-enabled flag. `mw[` must still wrap even when auto-pairs are off.
    let mut ed = editor_from("-[bar]>\n");
    ed.settings.auto_pairs_enabled = false;
    ed.handle_key(key('m'));
    ed.handle_key(key('w'));
    ed.handle_key(key('['));
    assert_eq!(state(&ed), "[bar-[]]>\n");
}

// ── Smart-p heuristic and kill ring ──────────────────────────────────────────

/// `d` then `p` reads from the kill ring (char-swap / dp pattern).
/// `last_command` after `d` is "delete" ∈ `SMART_P_LAST_CMDS`, so `p` reads ring.
#[test]
fn smart_p_dp_reads_ring() {
    // Buffer: "ab\n", cursor on 'a'.
    let mut ed = editor_from("-[a]>b\n");
    ed.feed_key(key('d')); // delete 'a' → ring = ["a"]
    // After delete: buffer = "b\n", cursor at 'b'.
    ed.feed_key(key('p')); // paste-after from ring → "ba\n"? No: paste-after on cursor 'b' inserts after 'b'.
    // Actually: after 'd', cursor is on 'b'. paste-after inserts "a" after 'b'. Buffer = "ba\n".
    assert!(
        ed.doc().text().to_string().contains('a'),
        "ring content pasted after delete"
    );
    // Clipboard is not written by bare 'd', so the pasted value came from ring.
    assert!(
        ed.kill_ring.head().is_some(),
        "kill ring still has an entry after paste"
    );
}

/// `d` then `j` (motion) then `p` reads from clipboard, not ring.
/// Motion is NOT in `SMART_P_LAST_CMDS`, so `p` falls back to clipboard.
#[test]
fn smart_p_motion_resets_to_clipboard() {
    use crate::ops::register::CLIPBOARD_REGISTER;

    // Two-line buffer; cursor on line 0.
    let mut ed = editor_from("-[a]>b\ncd\n");
    // Seed clipboard with something distinct from what 'd' would yank.
    ed.registers
        .write_text(CLIPBOARD_REGISTER, vec!["CLIP".to_string()]);
    ed.feed_key(key('d')); // delete 'a' → ring = ["a"]
    ed.feed_key(key('j')); // move-down → last_command = "move-down" ∉ SMART_P_LAST_CMDS
    ed.feed_key(key('p')); // paste-after → must read clipboard ("CLIP")
    assert!(
        ed.doc().text().to_string().contains("CLIP"),
        "p after motion reads clipboard"
    );
}

/// Bare `y` writes to both the clipboard AND the kill ring.
/// A subsequent `p` (no preceding `c`/`d`) reads from the clipboard.
#[test]
fn smart_p_after_yank_reads_clipboard() {
    use crate::ops::register::CLIPBOARD_REGISTER;

    let mut ed = editor_from("-[hello]> world\n");
    ed.feed_key(key('y')); // yank → clipboard + ring
    // Clipboard and ring both get "hello".
    assert_eq!(reg(&ed, CLIPBOARD_REGISTER), &["hello"], "clipboard written");
    assert!(ed.kill_ring.head().is_some(), "ring written");
    // Now move right and paste — last_command = "yank" ∉ SMART_P_LAST_CMDS → clipboard.
    // (Both paths yield the same "hello" since y wrote both, but we verify
    // last_command is reset by checking the heuristic does NOT pick ring-only.)
    assert!(
        !ed.last_command.as_deref().is_some_and(|c| [
            "change", "delete", "paste-after", "paste-before",
            "paste-ring-older", "paste-ring-newer"
        ].contains(&c)),
        "last_command after bare y is not in SMART_P_LAST_CMDS"
    );
}

/// Consecutive `p p` after `d` keeps reading the ring (last_command stays in set).
#[test]
fn smart_p_consecutive_paste_stays_in_ring() {
    use crate::ops::register::CLIPBOARD_REGISTER;

    let mut ed = editor_from("-[X]>abc\n");
    // Seed clipboard with something distinct.
    ed.registers
        .write_text(CLIPBOARD_REGISTER, vec!["CLIP".to_string()]);
    ed.feed_key(key('d')); // delete 'X' → ring = ["X"]
    ed.feed_key(key('p')); // first paste → from ring, last_command = "paste-after"
    // last_command = "paste-after" ∈ PASTE_FAMILY_CMDS → is_append = true → appends from last_paste.
    ed.feed_key(key('p')); // second paste → still from ring
    // Buffer should contain "X" twice (pasted) and NOT "CLIP".
    assert!(
        !ed.doc().text().to_string().contains("CLIP"),
        "second consecutive p still reads ring"
    );
}

/// `x d p` pastes the kill-ring head, not the clipboard (PASTERING.md rule 17).
///
/// `last_command = "delete"` is in `SMART_P_LAST_CMDS`, so bare `p` reads the
/// ring even when the clipboard holds different content.
#[test]
fn xdp_pastes_ring_head_not_clipboard() {
    use crate::ops::register::CLIPBOARD_REGISTER;

    let mut ed = editor_from("-[A]>\nB\n");
    ed.clipboard.force_unavailable();
    ed.registers.write_text(CLIPBOARD_REGISTER, vec!["CLIP".to_string()]);

    ed.feed_key(key('x')); // select "A\n"
    ed.feed_key(key('d')); // delete → ring = ["A\n"], last_command = "delete"
    ed.feed_key(key('p')); // prefer_ring = true → ring head

    assert_eq!(
        state(&ed),
        "B\n-[A\n]>",
        "xdp must paste the deleted line (ring head), not the clipboard sentinel"
    );
}

/// Regression: `drain_replay_queue` ran unconditionally after every key, writing the
/// `"macro-replay"` sentinel even when the queue was empty. A bare `p` after `x d`
/// must still read the ring head — the idle drain must not clobber `last_command`
/// (pre-432c24f bug: pasted the clipboard instead). `feed_key` / `feed_keys` include
/// the idle drain so this invariant is checked automatically by all paste tests now.
#[test]
fn smart_p_survives_idle_replay_drain() {
    use crate::ops::register::CLIPBOARD_REGISTER;

    let mut ed = editor_from("-[A]>\nB\n");
    ed.clipboard.force_unavailable();
    ed.registers.write_text(CLIPBOARD_REGISTER, vec!["CLIP".to_string()]);

    ed.feed_keys([key('x'), key('d'), key('p')]);

    assert_eq!(
        state(&ed),
        "B\n-[A\n]>",
        "idle replay-queue drain must not reset last_command; p reads the ring head"
    );
}

/// Kill ring depth: after >10 pushes (via `d`), `len() == 10` and the oldest entry
/// is evicted.  The 11th push displaces the 1st.
#[test]
fn kill_ring_depth_capped_at_ten() {
    // 11 one-char lines: A through K.
    let mut ed = editor_from("-[A]>\nB\nC\nD\nE\nF\nG\nH\nI\nJ\nK\n");
    // Delete each line by repeatedly pressing x then d.
    for _ in 0..11 {
        ed.feed_key(key('x')); // select-line
        ed.feed_key(key('d')); // delete line → push ring
        // After delete, cursor lands on next line automatically.
    }
    assert_eq!(ed.kill_ring.len(), 10, "kill ring capped at depth 10");
}

/// `"cy` writes clipboard only — no kill-ring push.
#[test]
fn explicit_cy_writes_clipboard_only() {
    use crate::ops::register::CLIPBOARD_REGISTER;

    let mut ed = editor_from("-[hello]>\n");
    // Kill the ring beforehand so we can detect any erroneous push.
    ed.feed_key(key('"'));
    ed.feed_key(key('c'));
    ed.feed_key(key('y')); // "cy → clipboard only

    assert_eq!(reg(&ed, CLIPBOARD_REGISTER), &["hello"], "clipboard written");
    assert!(
        ed.kill_ring.head().is_none(),
        "kill ring NOT pushed by explicit \"cy"
    );
}

/// `"5y` writes the in-memory named register '5'; kill ring is not touched.
///
/// Digit-register writes route through `write_register` → `registers.write_text`,
/// not through `kill_ring.push`. The in-memory and ring storage are orthogonal.
#[test]
fn explicit_digit_y_writes_in_memory_only() {
    let mut ed = editor_from("-[hello]>\n");
    ed.feed_key(key('"'));
    ed.feed_key(key('5'));
    ed.feed_key(key('y')); // "5y → in-memory register '5' (not kill ring push)

    assert_eq!(reg(&ed, '5'), &["hello"], "register '5' written");
    assert!(
        ed.kill_ring.head().is_none(),
        "kill ring head untouched by explicit \"5y"
    );
}

/// `"5p` reads in-memory register '5', not the kill ring.
/// Push 6 entries to the ring via bare `d`; `"5p` must be a no-op (register '5' empty).
#[test]
fn explicit_digit_p_reads_inmemory_not_ring() {
    let mut ed = editor_from("-[a]>bcdefg\n");
    for _ in 0..6 {
        ed.feed_key(key('d'));
    }
    // ring has 6 entries; in-memory register '5' was never written
    let before = state(&ed);
    ed.feed_key(key('"'));
    ed.feed_key(key('5'));
    ed.feed_key(key('p')); // "5p → in-memory register '5' is empty → no-op

    assert_eq!(state(&ed), before, "\"5p must be a no-op when in-memory register '5' is empty");
}

/// `"5y` then `"5p` round-trips via in-memory storage, regardless of kill-ring contents.
#[test]
fn digit_register_roundtrip_inmemory() {
    let mut ed = editor_from("-[INMEM]>\n");
    ed.feed_key(key('"')); ed.feed_key(key('5')); ed.feed_key(key('y'));
    // Ring: empty (no d/c). In-memory register '5' = "INMEM".
    // "5p must paste from in-memory, not clipboard or ring.
    ed.feed_key(key(';')); // collapse selection
    ed.feed_key(key('"')); ed.feed_key(key('5')); ed.feed_key(key('p'));

    assert!(
        ed.doc().text().to_string().contains("INMEM"),
        "\"5p must paste what \"5y wrote (in-memory round-trip)"
    );
}

/// `paste-ring-older` / `paste-ring-newer` (`[` / `]`) on an empty ring are no-ops.
#[test]
fn paste_ring_older_empty_ring_is_noop() {
    let mut ed = editor_from("-[a]>bc\n");
    let before = state(&ed);
    ed.feed_key(key('['));
    assert_eq!(state(&ed), before, "[ on empty ring is a no-op");
    ed.feed_key(key(']'));
    assert_eq!(state(&ed), before, "] on empty ring is a no-op");
}

/// `[ ]` cycle within a paste session: the ring cursor walks older then back newer.
#[test]
fn paste_ring_cycle_older_then_newer() {
    // Push 3 entries: A\n (oldest), B\n, C\n (newest/head at slot 0).
    let mut ed = editor_from("-[A]>\nB\nC\n");
    ed.feed_key(key('x')); ed.feed_key(key('d')); // ring = [A\n]
    ed.feed_key(key('x')); ed.feed_key(key('d')); // ring = [B\n, A\n]
    ed.feed_key(key('x')); ed.feed_key(key('d')); // ring = [C\n, B\n, A\n]

    // Open paste session: `p` reads ring head (C\n) since last_command ∈ SMART_P_LAST_CMDS.
    ed.feed_key(key('p')); // seeds cycle at Some(0) = C\n

    // `[` cycles older: Some(0) → Some(1) = B\n, re-pastes from session snapshot.
    ed.feed_key(key('['));
    let after_first_older = ed.doc().text().to_string();
    assert!(after_first_older.contains('B'), "first [ pastes slot 1 (B)");
    // `[` again → Some(1) → Some(2) = A\n.
    ed.feed_key(key('['));
    let after_second_older = ed.doc().text().to_string();
    assert!(after_second_older.contains('A'), "second [ pastes slot 2 (A)");
    // `]` retreats → Some(2) → Some(1) = B\n.
    ed.feed_key(key(']'));
    let after_newer = ed.doc().text().to_string();
    assert!(after_newer.contains('B'), "] after two [ pastes slot 1 (B)");
}


/// Select a line with `x`, delete with `d`, move with `j`, then paste via
/// explicit ring head (`"kp`) — the deleted line must appear as its own line *below*
/// the cursor, not embedded inside the current line.
#[test]
fn paste_ring_linewise_pastes_below_not_inline() {
    // Buffer: "A\nB\nC\n". Delete line A (x+d), move to C (j), paste via "kp.
    let mut ed = editor_from("-[A]>\nB\nC\n");
    ed.feed_key(key('x')); // select line "A\n"
    ed.feed_key(key('d')); // push "A\n" to ring head, buffer → "B\nC\n"
    ed.feed_key(key('j')); // cursor → 'C'
    // "kp reads ring head (="A\n") directly — avoids smart-p clipboard routing
    // after the intervening motion cleared last_command.
    ed.feed_key(key('"'));
    ed.feed_key(key('k'));
    ed.feed_key(key('p')); // paste ring head (A\n) linewise below C

    // "A\n" must land as its own line below C — not inside C's text.
    assert_eq!(
        state(&ed),
        "B\nC\n-[A\n]>",
        "\"kp on a linewise ring entry must paste as a new line, not inline"
    );
}

/// `[`/`]` cycle within a paste session REPLACES the previous paste — never
/// accumulates a second copy.
#[test]
fn paste_ring_warm_cycle_replaces_not_accumulates() {
    // Two ring entries: A\n (older, slot 1), B\n (head, slot 0).
    let mut ed = editor_from("-[A]>\nB\nC\n");
    ed.feed_key(key('x')); ed.feed_key(key('d')); // ring = [A\n]; buffer = "B\nC\n"
    ed.feed_key(key('x')); ed.feed_key(key('d')); // ring = [B\n, A\n]; buffer = "C\n"

    // p: smart-p reads ring head B\n (last_command = "delete"), opens session.
    ed.feed_key(key('p'));
    assert_eq!(
        ed.doc().text().to_string().matches("B\n").count(),
        1,
        "p pastes B once"
    );

    // [: cycle older (slot 0 → slot 1 = A\n) — must REPLACE B, not add another.
    ed.feed_key(key('['));
    let after_older = ed.doc().text().to_string();
    assert_eq!(after_older.matches("A\n").count(), 1, "[ replaces paste with A");

    // ]: cycle newer (slot 1 → slot 0 = B\n) — must REPLACE A.
    ed.feed_key(key(']'));
    let after_newer = ed.doc().text().to_string();
    assert_eq!(after_newer.matches("B\n").count(), 1, "] replaces back with B");
    assert_eq!(after_newer.matches("A\n").count(), 0, "] removes A");
}

/// Single-char cycle: `[` within a session pastes the older entry, `]` replaces
/// it back with the head — collapsed selection is not an obstacle.
#[test]
fn paste_ring_warm_cycle_replaces_single_char_paste() {
    let mut ed = editor_from("-[X]>Y\n");
    ed.feed_key(key('d')); // kill "X"; ring = [X], buffer = "-[Y]>\n"
    ed.feed_key(key('d')); // kill "Y"; ring = [Y, X], buffer = "-[\n]>"

    // p: reads ring head Y (last_command = "delete"), opens session, seeds cycle at 0.
    ed.feed_key(key('p'));
    assert!(ed.doc().text().to_string().contains('Y'), "p pastes Y (ring head)");

    // [: cycle older (slot 0 → slot 1 = X), replaces Y.
    ed.feed_key(key('['));
    assert!(ed.doc().text().to_string().contains('X'), "[ pastes X (slot 1)");

    // ]: cycle newer (slot 1 → slot 0 = Y), replacing X.
    ed.feed_key(key(']'));
    let buf = ed.doc().text().to_string();
    assert!(buf.contains('Y'), "] pastes Y (slot 0)");
    assert!(!buf.contains('X'), "] replaces X — no 'X' remains");
}

/// `P` (paste-before) opens a before-session; `[`/`]` must re-paste BEFORE the
/// cursor, not after it.
#[test]
fn paste_before_cycle_stays_before_charwise() {
    let mut ed = editor_from("-[c]>d\n"); // cursor on 'c' at index 0
    ed.kill_ring.push(vec!["X".to_string()]); // slot 1 after next push
    ed.kill_ring.push(vec!["Y".to_string()]); // ring=[Y, X]; head=Y, slot 1=X

    // "kP: paste-before ring head ("Y") before cursor 'c'.
    ed.feed_key(key('"')); ed.feed_key(key('k')); ed.feed_key(key('P'));
    assert_eq!(state(&ed), "-[Y]>cd\n", "P pastes before the cursor");

    // [: cycle to slot 1 ("X"); must re-paste BEFORE the cursor snapshot (at 0).
    ed.feed_key(key('['));
    assert_eq!(state(&ed), "-[X]>cd\n", "[ after P re-pastes before the cursor (would be c-[X]>d if it used paste_after)");
}

/// `p` (paste-after) opens an after-session; cycling stays after (regression).
#[test]
fn paste_after_cycle_stays_after_charwise() {
    let mut ed = editor_from("-[c]>d\n");
    ed.kill_ring.push(vec!["X".to_string()]);
    ed.kill_ring.push(vec!["Y".to_string()]); // ring=[Y, X]

    ed.feed_key(key('"')); ed.feed_key(key('k')); ed.feed_key(key('p'));
    assert_eq!(state(&ed), "c-[Y]>d\n", "p pastes after the cursor");

    ed.feed_key(key('['));
    assert_eq!(state(&ed), "c-[X]>d\n", "[ after p stays paste-after");
}

/// `P` on a linewise entry opens a before-session; `[` must re-paste ABOVE the
/// cursor line, not below it.
#[test]
fn paste_before_cycle_stays_above_linewise() {
    let mut ed = editor_from("-[B]>\nC\n"); // cursor on 'B', line 0
    ed.kill_ring.push(vec!["X\n".to_string()]); // slot 1
    ed.kill_ring.push(vec!["Y\n".to_string()]); // ring=[Y\n, X\n]; head=Y\n

    // "kP: linewise paste-before ring head ("Y\n") — inserts above line 0.
    ed.feed_key(key('"')); ed.feed_key(key('k')); ed.feed_key(key('P'));
    assert_eq!(ed.doc().text().to_string(), "Y\nB\nC\n", "P pastes above current line");

    // [: cycle to slot 1 ("X\n"); must re-paste ABOVE line 0 (not below).
    ed.feed_key(key('['));
    assert_eq!(
        ed.doc().text().to_string(),
        "X\nB\nC\n",
        "[ after linewise P re-pastes above (would be B\\nX\\nC\\n if it used paste_after)"
    );
}

/// `p [ p` duplicates the currently-cycled entry — never does a fresh clipboard
/// paste (PASTERING.md rule 19).
///
/// After `[` swaps the paste to the ring head, `last_command = "paste-ring-older"`
/// is in `PASTE_FAMILY_CMDS`, so the next `p` must append (not replace).
#[test]
fn paste_after_cycle_appends_cycled_entry() {
    use crate::ops::register::CLIPBOARD_REGISTER;

    let mut ed = editor_from("-[x]>\n");
    ed.clipboard.force_unavailable();
    ed.registers.write_text(CLIPBOARD_REGISTER, vec!["CLIP".to_string()]);
    ed.kill_ring.push(vec!["RING".to_string()]); // ring head = "RING"

    // p: last_command=None → clipboard "CLIP"; seed_cycle(None).
    ed.feed_key(key('p'));
    // [: cycle_older None→0="RING"; replaces paste; last_paste=["RING"].
    ed.feed_key(key('['));
    // p: is_append (last_command="paste-ring-older" ∈ PASTE_FAMILY) → append last_paste.
    ed.feed_key(key('p'));

    let buf = ed.doc().text().to_string();
    assert_eq!(
        buf.matches("RING").count(),
        2,
        "p after [ must duplicate the cycled entry (rule 19)"
    );
    assert!(
        !buf.contains("CLIP"),
        "clipboard content must not appear — p after [ appends ring entry, not clipboard"
    );
}

/// Consecutive `p` presses append copies rather than replacing the selected paste.
#[test]
fn consecutive_paste_appends_copies() {
    let mut ed = editor_from("-[ab]>\n");
    ed.feed_key(key('y')); // yank "ab" to clipboard + ring
    ed.feed_key(key('d')); // delete, buffer = "\n"; ring head = "ab"
    ed.feed_key(key('p')); // paste "ab" (from ring, prev=delete); "ab" selected
    ed.feed_key(key('p')); // prev=paste-after → APPEND another copy
    let buf = ed.doc().text().to_string();
    assert_eq!(
        buf.matches("ab").count(),
        2,
        "two consecutive p presses must stack two copies of 'ab'"
    );
}

/// Consecutive `p` presses append when the previous paste came from the CLIPBOARD
/// and the kill ring is empty — the second `p` must not be a no-op.
#[test]
fn consecutive_clipboard_paste_appends() {
    use crate::ops::register::CLIPBOARD_REGISTER;

    let mut ed = editor_from("-[x]>\n");
    ed.clipboard.force_unavailable(); // headless: reads fall back to in-memory mirror
    ed.registers.write_text(CLIPBOARD_REGISTER, vec!["XY".to_string()]);
    // ring is empty — this is the regression case

    ed.feed_key(key('p')); // last_command=None → clipboard → pastes "XY"; last_paste=["XY"]
    ed.feed_key(key('p')); // last_command="paste-after" ∈ PASTE_FAMILY → repeat last_paste
    let buf = ed.doc().text().to_string();
    assert_eq!(
        buf.matches("XY").count(),
        2,
        "two consecutive p presses must stack two copies even with an empty kill ring"
    );
}

/// Consecutive `p` after a clipboard paste must repeat the clipboard value, not
/// whatever happens to be at the ring head.
#[test]
fn consecutive_paste_repeats_last_not_ring_head() {
    use crate::ops::register::CLIPBOARD_REGISTER;

    let mut ed = editor_from("-[x]>\n");
    ed.clipboard.force_unavailable();
    ed.registers.write_text(CLIPBOARD_REGISTER, vec!["XY".to_string()]);
    ed.kill_ring.push(vec!["ZZ".to_string()]); // ring has different content

    ed.feed_key(key('p')); // clipboard → "XY"; last_paste=["XY"]
    ed.feed_key(key('p')); // append → repeats "XY", not ring head "ZZ"
    let buf = ed.doc().text().to_string();
    assert_eq!(buf.matches("XY").count(), 2, "clipboard value repeated");
    assert!(!buf.contains("ZZ"), "ring head must not appear — append repeats last paste verbatim");
}

/// After paste-after, the pasted text is selected (covers the full inserted span).
#[test]
fn paste_leaves_output_selected() {
    // Delete "ab" → ring head = "ab". Then paste: selection must cover "ab".
    let mut ed = editor_from("-[ab]>cd\n");
    ed.feed_key(key('d')); // kill "ab"; buffer = "-[c]>d\n"
    ed.feed_key(key('p')); // smart-p reads ring head "ab" (charwise); paste after 'c'
    assert_eq!(
        state(&ed),
        "c-[ab]>d\n",
        "paste must leave the pasted text selected"
    );
}


// ── Register prefix persistence across non-register commands ────────────────

/// `"5` arms the prefix; `l` (a motion) does not consume it; the next `y` writes
/// to register 5. This is the intended sticky behaviour — the prefix persists
/// until a register-consuming command runs or Esc cancels it.
#[test]
fn register_prefix_persists_across_motion() {
    let mut ed = editor_from("-[hell]>o\n");
    ed.handle_key(key('"'));
    ed.handle_key(key('5'));
    ed.handle_key(key('l')); // motion — does not consume the prefix
    ed.handle_key(key('y')); // yank targets register 5, not '"'

    assert!(!reg(&ed, '5').is_empty(), "register '5' written after motion");
    assert!(reg(&ed, '"').is_empty(), "'\"' register untouched");
}

/// An explicit `"Xp` while in the append state must paste from register X,
/// not silently re-paste the previous value.  Before the fix, the append path
/// returned without calling `take_register_prefix()`, so the named register was
/// ignored AND the prefix leaked into the next command.
#[test]
fn register_prefix_overrides_append_path() {
    let mut ed = editor_from("-[x]>\n");
    ed.registers.write_text('5', vec!["REG5".to_string()]);
    ed.kill_ring.push(vec!["RING".to_string()]);

    // Delete 'x' so the ring has "x" at head; RING is at slot 1.
    ed.feed_key(key('d'));
    // Paste via kill register to get into the append state with last_paste=[ring head].
    ed.handle_key(key('"'));
    ed.handle_key(key('k'));
    ed.feed_key(key('p'));

    // Now try to paste from named register '5' — must NOT take the append path.
    ed.handle_key(key('"'));
    ed.handle_key(key('5'));
    ed.feed_key(key('p'));

    let buf = ed.doc().text().to_string();
    assert!(
        buf.contains("REG5"),
        "explicit \"5p must paste from register 5, not re-paste last_paste; buf={buf:?}"
    );
}

/// After an explicit `"Xp` the register prefix must be consumed (not leaked).
/// Before the fix the prefix persisted and the NEXT command accidentally used it.
#[test]
fn register_prefix_consumed_by_paste() {
    let mut ed = editor_from("-[x]>\n");
    ed.registers.write_text('5', vec!["REG5".to_string()]);
    ed.kill_ring.push(vec!["RING".to_string()]);

    // Get into append state via a paste.
    ed.feed_key(key('d'));           // delete x; ring head = "x"
    ed.handle_key(key('"'));
    ed.handle_key(key('k'));         // select kill register
    ed.feed_key(key('p'));           // paste ring head

    // Now type "5p — explicit register paste.
    ed.handle_key(key('"'));
    ed.handle_key(key('5'));
    ed.feed_key(key('p'));           // should consume the '5' prefix

    // The prefix must be gone — the next 'd' must NOT route to register 5.
    ed.feed_key(key('d'));           // delete; should push to kill ring, not register 5
    // Register 5 must still hold "REG5" — if the prefix leaked into 'd', it
    // would be overwritten with the deleted char.
    let reg5 = ed.registers.read('5').and_then(|r| r.as_text()).map(|v| v.to_vec());
    assert_eq!(
        reg5,
        Some(vec!["REG5".to_string()]),
        "register 5 must be unchanged after d — prefix leaked if it differs"
    );
}

// ── Bundled theme loading (end-to-end wiring) ─────────────────────────────────

/// Smoke-test all three bundled themes through the full loader → bake → resolve
/// pipeline. Catches wiring regressions (bad paths, parse errors, missing palette
/// entries) without needing a running editor.
#[test]
fn bundled_themes_load_and_resolve() {
    use std::path::PathBuf;
    let themes_dir =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../runtime/themes");
    let paths = vec![themes_dir];

    for name in ["dark", "light", "gruvbox"] {
        let mut theme = engine::theme::loader::load_theme(name, &paths)
            .unwrap_or_else(|e| panic!("bundled theme '{name}' failed to load: {e}"));
        let mut reg = engine::theme::ScopeRegistry::new();
        reg.intern("ui.cursor.primary");
        reg.intern("ui.selection");
        theme.bake(&reg);
        let style = theme.resolve_by_name(engine::types::Scope("ui.cursor.primary"));
        assert!(
            style.fg.is_some() || style.bg.is_some(),
            "bundled theme '{name}': ui.cursor.primary has neither fg nor bg"
        );
    }
}

/// `load_theme_by_name` reports failure via the message log and returns `false`;
/// the theme stays unchanged.
#[test]
fn load_theme_by_name_fails_gracefully() {
    let mut ed = editor_from("-[a]>b\n");
    let ok = ed.load_theme_by_name("no_such_theme_xyz");
    assert!(!ok, "expected false for nonexistent theme");
    // Failure warning ends up in the message log, not as an error result.
    assert!(ed.message_log.has_unseen(), "expected a warning message");
}

// ── Minibuffer arity-rule for Steel commands ──────────────────────────────

/// Wire up a Steel command and return the editor + scripting host ready for use.
///
/// `eval_source` processes the lambda into the scripting engine, but discards the
/// `SteelCmdDef` (returning `()`).  We must call `register_steel_cmds` separately
/// to make the command reachable via `:name` in the minibuffer.
fn setup_arity_test(
    src: &str,
    name: &str,
    arity: u16,
    is_variadic: bool,
) -> Editor {
    use crate::scripting::{ScriptingHost, SteelCmdDef};
    use crate::editor::keymap::Keymap;
    use crate::settings::EditorSettings;

    let mut ed = editor_from("-[a]>b\n");
    let mut host = ScriptingHost::new();
    let mut s = EditorSettings::default();
    let mut km = Keymap::default();
    host.eval_source(src, &mut s, &mut km).unwrap();
    ed.register_steel_cmds(vec![SteelCmdDef {
        name: name.to_string(),
        doc: String::new(),
        steel_proc: format!("%hume-cmd-{name}"),
        extendable: false,
        arity,
        is_variadic,
        inline_output: false,
    }]);
    ed.scripting = Some(host);
    ed
}

/// arity-1 + string arg: the rule converts the typed string to `StringV` and the
/// lambda receives it, queuing it as a command name, which runs `move-right`.
/// Oracle: state changes → cursor moved → arg was forwarded.
/// Verification: changing "move-right" in the assert to something else → fails.
#[test]
fn minibuffer_arity_rule_forwards_string_arg_to_arity_1() {
    let mut ed = setup_arity_test(
        r#"(define-command! "echo-cmd" "" (lambda (x) (when (string? x) (call! x))))"#,
        "echo-cmd",
        1,
        false,
    );

    let before = state(&ed);
    // `:echo-cmd move-right<Enter>` — arity-1 rule passes "move-right" as StringV.
    ed.handle_key(key(':'));
    for ch in "echo-cmd move-right".chars() {
        ed.handle_key(key(ch));
    }
    ed.handle_key(key_enter());

    assert_ne!(state(&ed), before, "arity-1 rule must forward arg as StringV; cursor must have moved");
}

/// arity-1 + no arg: the rule passes `BoolV(false)`.  The lambda checks
/// `(string? x)`, gets `#f`, and does nothing — cursor stays put.
#[test]
fn minibuffer_arity_rule_passes_false_when_no_arg() {
    let mut ed = setup_arity_test(
        r#"(define-command! "echo-cmd" "" (lambda (x) (when (string? x) (call! x))))"#,
        "echo-cmd",
        1,
        false,
    );

    let before = state(&ed);
    // `:echo-cmd<Enter>` — no arg → arity-1 rule passes #f.
    ed.handle_key(key(':'));
    for ch in "echo-cmd".chars() {
        ed.handle_key(key(ch));
    }
    ed.handle_key(key_enter());

    assert_eq!(state(&ed), before, "arity-1 with no arg must pass #f (not crash or move)");
}

/// arity-2 + one arg (the most the minibuffer can supply): the rule reports an
/// error and never invokes the command.  Cursor stays; error is logged.
/// The command needs no real lambda — the early return fires before call_steel_cmd.
#[test]
fn minibuffer_arity_rule_errors_on_arity_2() {
    use crate::scripting::SteelCmdDef;

    let mut ed = editor_from("-[a]>b\n");
    ed.register_steel_cmds(vec![SteelCmdDef {
        name: "needs-two".to_string(),
        doc: String::new(),
        steel_proc: "%hume-cmd-needs-two".to_string(),
        extendable: false,
        arity: 2,
        is_variadic: false,
        inline_output: false,
    }]);

    let before = state(&ed);
    // `:needs-two<Enter>` — arity-2 command, minibuffer can only supply 1 arg.
    ed.handle_key(key(':'));
    for ch in "needs-two".chars() {
        ed.handle_key(key(ch));
    }
    ed.handle_key(key_enter());

    assert_eq!(state(&ed), before, "arity rule must not dispatch the command");
    assert!(
        ed.message_log.entries().any(|e| e.text.contains("requires 2 args")),
        "arity rule must log a user-facing error"
    );
}
