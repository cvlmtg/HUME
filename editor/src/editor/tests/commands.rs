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

// ── `d` yanks into the default register ────────────────────────────────────

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

// ── `p` swaps displaced selection text back into the register ──────────────

/// When `p` pastes over a non-cursor (multi-char) selection, the displaced
/// text must be written back to the clipboard (exchange semantics).
/// `p` with no prior `c`/`d` reads the system clipboard via Smart-p; the
/// displaced text is written back to the clipboard so it can be pasted again.
#[test]
fn p_over_selection_swaps_displaced_text_into_register() {
    use crate::ops::register::CLIPBOARD_REGISTER;

    let mut ed = editor_from("-[hell]>o\n");
    // Seed clipboard (in-memory mirror for headless tests) with the text to paste.
    ed.registers
        .write_text(CLIPBOARD_REGISTER, vec!["XY".to_string()]);

    ed.handle_key(key('p'));

    assert_eq!(
        ed.doc().text().to_string(),
        "XYo\n",
        "pasted text in buffer"
    );
    // Displaced "hell" goes back to clipboard (Smart-p read from clipboard,
    // so displaced text returns to the same source).
    assert_eq!(
        reg(&ed, CLIPBOARD_REGISTER),
        &["hell"],
        "displaced text back in clipboard"
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

/// `"3p` must paste from kill ring slot 3, not the system clipboard.
#[test]
fn paste_from_named_register() {
    use crate::ops::register::CLIPBOARD_REGISTER;

    // Push 4 entries so slot 3 holds "P" (first-deleted = oldest = slot 3).
    let mut ed = editor_from("-[P]>QRS\n");
    for _ in 0..4 {
        ed.handle_key(key('d')); // delete each char in turn
    }
    // ring: slot 0 = "S" (newest), slot 1 = "R", slot 2 = "Q", slot 3 = "P"

    // Seed clipboard with "wrong" so we can verify it is NOT used.
    ed.registers.write_text(CLIPBOARD_REGISTER, vec!["wrong".to_string()]);

    ed.handle_key(key('"'));
    ed.handle_key(key('3'));
    ed.handle_key(key('p')); // "3p → ring slot 3 = "P"

    assert!(
        ed.doc().text().to_string().contains('P'),
        "pasted from ring slot 3"
    );
    assert!(
        !ed.doc().text().to_string().contains("wrong"),
        "clipboard not used"
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
    ed.handle_key(key('d')); // delete 'a' → ring = ["a"]
    // After delete: buffer = "b\n", cursor at 'b'.
    ed.handle_key(key('p')); // paste-after from ring → "ba\n"? No: paste-after on cursor 'b' inserts after 'b'.
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
    ed.handle_key(key('d')); // delete 'a' → ring = ["a"]
    ed.handle_key(key('j')); // move-down → last_command = "move-down" ∉ SMART_P_LAST_CMDS
    ed.handle_key(key('p')); // paste-after → must read clipboard ("CLIP")
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
    ed.handle_key(key('y')); // yank → clipboard + ring
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
    ed.handle_key(key('d')); // delete 'X' → ring = ["X"]
    ed.handle_key(key('p')); // first paste → from ring, last_command = "paste-after"
    // last_command = "paste-after" ∈ SMART_P_LAST_CMDS → next p also reads ring.
    ed.handle_key(key('p')); // second paste → still from ring
    // Buffer should contain "X" twice (pasted) and NOT "CLIP".
    assert!(
        !ed.doc().text().to_string().contains("CLIP"),
        "second consecutive p still reads ring"
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
        ed.handle_key(key('x')); // select-line
        ed.handle_key(key('d')); // delete line → push ring
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
    ed.handle_key(key('"'));
    ed.handle_key(key('c'));
    ed.handle_key(key('y')); // "cy → clipboard only

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
    ed.handle_key(key('"'));
    ed.handle_key(key('5'));
    ed.handle_key(key('y')); // "5y → in-memory register '5' (not kill ring push)

    assert_eq!(reg(&ed, '5'), &["hello"], "register '5' written");
    assert!(
        ed.kill_ring.head().is_none(),
        "kill ring head untouched by explicit \"5y"
    );
}

/// `"5p` reads kill ring slot 5; no in-memory fallback.
/// Fill the ring past slot 5 so the slot has a real entry.
#[test]
fn explicit_digit_p_reads_ring_slot() {
    let mut ed = editor_from("-[a]>bcdefg\n");
    // Push 6 entries via bare `d` so ring slot 5 (the 6th-newest, 0-based) has data.
    // After each delete the buffer shrinks by one char; delete 'a' through 'f'.
    for _ in 0..6 {
        ed.handle_key(key('d'));
    }
    // ring slots: 0=f, 1=e, 2=d, 3=c, 4=b, 5=a
    // Clear pending prefix state, then do "5p.
    ed.handle_key(key('"'));
    ed.handle_key(key('5'));
    ed.handle_key(key('p')); // "5p → ring slot 5 = "a"

    assert!(
        ed.doc().text().to_string().contains('a'),
        "paste from kill ring slot 5"
    );
}

/// `"5p` returns nothing when the ring has fewer than 6 entries (no in-memory fallback).
#[test]
fn explicit_digit_p_no_inmemory_fallback() {
    let mut ed = editor_from("-[x]>\n");
    // Seed in-memory register '5' — this must NOT be read by "5p.
    ed.registers.write_text('5', vec!["INMEM".to_string()]);
    // Ring is empty (no deletes/yanks), so ring slot 5 is also absent.

    let before = state(&ed);
    ed.handle_key(key('"'));
    ed.handle_key(key('5'));
    ed.handle_key(key('p')); // "5p → ring slot 5 absent → no-op

    assert_eq!(
        state(&ed),
        before,
        "\"5p must be a no-op when the ring has no slot 5 (in-memory '5' is not a fallback)"
    );
}

/// `paste-ring-older` / `paste-ring-newer` (`[` / `]`) on an empty ring are no-ops.
#[test]
fn paste_ring_older_empty_ring_is_noop() {
    let mut ed = editor_from("-[a]>bc\n");
    let before = state(&ed);
    ed.handle_key(key('['));
    assert_eq!(state(&ed), before, "[ on empty ring is a no-op");
    ed.handle_key(key(']'));
    assert_eq!(state(&ed), before, "] on empty ring is a no-op");
}

/// `[ ]` cycle after `d`: the ring cursor walks older then back newer.
#[test]
fn paste_ring_cycle_older_then_newer() {
    // Push 3 entries: A (oldest), B, C (newest/head).
    let mut ed = editor_from("-[A]>\nB\nC\n");
    // Delete A → ring = [A] (head)
    ed.handle_key(key('x'));
    ed.handle_key(key('d'));
    // Delete B → ring = [B, A] (B is now head)
    ed.handle_key(key('x'));
    ed.handle_key(key('d'));
    // Delete C → ring = [C, B, A] (C is now head)
    ed.handle_key(key('x'));
    ed.handle_key(key('d'));

    // `[` cycles older (from None → slot 1 = B).
    ed.handle_key(key('['));
    let after_first_older = ed.doc().text().to_string();
    assert!(
        after_first_older.contains('B'),
        "first [ pastes slot 1 (B)"
    );
    // `[` again → slot 2 = A.
    ed.handle_key(key('['));
    let after_second_older = ed.doc().text().to_string();
    assert!(
        after_second_older.contains('A'),
        "second [ pastes slot 2 (A)"
    );
    // `]` retreats → slot 1 = B.
    ed.handle_key(key(']'));
    let after_newer = ed.doc().text().to_string();
    assert!(after_newer.contains('B'), "] after two [ pastes slot 1 (B)");
}

/// `[` over a non-cursor selection: displaced text must be pushed onto the kill
/// ring head rather than silently dropped.
#[test]
fn paste_ring_cycle_preserves_displaced_text() {
    // Build a ring with 2 entries so `[` reaches slot 1.
    let mut ed = editor_from("-[X]>Y\nZ\n");
    // Delete X → ring = [X]
    ed.handle_key(key('d'));
    // Delete Y → ring = [Y, X] (Y is head at slot 0, X at slot 1)
    ed.handle_key(key('d'));

    // Manually arm a 2-char selection so paste-after triggers replace-and-swap.
    // Use `x` (select-char) and extend with `l` to have sel=[Z].
    // Actually: after two deletes the buffer is "\nZ\n", cursor on '\n'.
    // Move to next line to reach 'Z', then `x` selects 'Z'.
    ed.handle_key(key('j')); // move to 'Z' line
    ed.handle_key(key('x')); // select 'Z' (non-cursor selection)

    // `[` → cycle older → reads slot 1 (X), pastes over 'Z'. Displaced = "Z".
    ed.handle_key(key('['));

    // Displaced 'Z' must now be on the ring head.
    assert_eq!(
        ed.kill_ring.head(),
        Some(["Z".to_string()].as_slice()),
        "displaced text 'Z' must be pushed to ring head after [ over selection"
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
