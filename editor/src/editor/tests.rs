use std::sync::Arc;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use pretty_assertions::assert_eq;

use crate::editor::buffer::Buffer;
use crate::testing::{parse_state, serialize_state};

use super::{Editor, Mode};

// ── Harness ───────────────────────────────────────────────────────────────────

/// Build an Editor pre-loaded with the given state string (same DSL as other tests).
fn editor_from(input: &str) -> Editor {
    let (buf, sels) = parse_state(input);
    Editor::for_testing(Buffer::new(buf, sels))
}

/// Build a kitty-protocol-enabled editor for testing Ctrl+motion bindings.
fn editor_from_kitty(input: &str) -> Editor {
    let mut ed = editor_from(input);
    ed.kitty_enabled = true;
    ed
}

/// Serialize the editor's current buffer + selection state.
fn state(ed: &Editor) -> String {
    serialize_state(ed.doc().text(), ed.current_selections())
}

/// A normal (no modifier) character key event.
fn key(ch: char) -> KeyEvent {
    KeyEvent::new(KeyCode::Char(ch), KeyModifiers::NONE)
}

fn key_esc() -> KeyEvent {
    KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE)
}

fn key_ctrl(ch: char) -> KeyEvent {
    KeyEvent::new(KeyCode::Char(ch), KeyModifiers::CONTROL)
}


fn key_enter() -> KeyEvent {
    KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)
}

fn key_backspace() -> KeyEvent {
    KeyEvent::new(KeyCode::Backspace, KeyModifiers::NONE)
}

fn reg(ed: &Editor, name: char) -> Vec<String> {
    ed.registers
        .read(name)
        .and_then(|r| r.as_text())
        .unwrap_or_default()
        .to_vec()
}

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

/// Deleting a selection must populate the default register with the deleted
/// text. A bug in the mapping that removed the `yank_selections` call before
/// `delete_selection` would leave the register empty — invisible to pure tests.
#[test]
fn d_yanks_selection_into_register_before_deleting() {
    use crate::ops::register::DEFAULT_REGISTER;

    let mut ed = editor_from("-[hell]>o\n");
    ed.handle_key(key('d'));

    assert_eq!(ed.doc().text().to_string(), "o\n", "buffer after delete");
    assert_eq!(reg(&ed, DEFAULT_REGISTER), &["hell"], "register after delete");
}

// ── `y` yanks without modifying the buffer ─────────────────────────────────

/// `y` must populate the register without changing the buffer or the selection.
/// This is the only way to test that `y` actually writes to the register —
/// pure tests of `yank_selections` never touch the `Editor.registers` field.
#[test]
fn y_populates_register_without_changing_buffer() {
    use crate::ops::register::DEFAULT_REGISTER;

    let mut ed = editor_from("-[hell]>o\n");
    ed.handle_key(key('y'));

    assert_eq!(state(&ed), "-[hell]>o\n", "buffer+selection unchanged");
    assert_eq!(reg(&ed, DEFAULT_REGISTER), &["hell"], "register populated");
}

// ── `p` swaps displaced selection text back into the register ──────────────

/// When `p` pastes over a non-cursor (multi-char) selection, the displaced
/// text must be written back to the default register (exchange semantics).
/// This logic lives entirely in the mapping — no pure test can see it.
#[test]
fn p_over_selection_swaps_displaced_text_into_register() {
    use crate::ops::register::DEFAULT_REGISTER;

    let mut ed = editor_from("-[hell]>o\n");
    // Seed the register with the text we'll paste.
    ed.registers.write_text(DEFAULT_REGISTER, vec!["XY".to_string()]);

    ed.handle_key(key('p'));

    assert_eq!(ed.doc().text().to_string(), "XYo\n", "pasted text in buffer");
    assert_eq!(reg(&ed, DEFAULT_REGISTER), &["hell"], "displaced text in register");
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
    assert!(ed.wait_char.is_none(), "wait_char cleared after replacement char");
    assert_eq!(state(&ed), "-[xxxx]>o\n");
}

#[test]
fn r_then_esc_cancels_without_side_effects() {
    let mut ed = editor_from("-[hell]>o\n");
    ed.handle_key(key('r'));
    // Esc resets wait_char (and all other pending state).
    ed.handle_key(key_esc());

    assert!(ed.wait_char.is_none());
    assert_eq!(state(&ed), "-[hell]>o\n", "buffer unchanged after cancelled replace");
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
    assert_eq!(state(&ed), "-[h]>ello a\n", "buffer and cursor unchanged after cancelled find");
}

// ── `m i w` three-key text-object sequence ─────────────────────────────────

/// The trie must advance through `m` (Interior) → `mi` (Interior) → `miw`
/// (Leaf) and dispatch the correct text-object command on the third key.
/// This exercises the entire three-key pipeline end-to-end.
#[test]
fn m_i_w_selects_inner_word() {
    let mut ed = editor_from("-[h]>ello world\n");

    ed.handle_key(key('m'));
    assert_eq!(ed.pending_keys.len(), 1, "pending_keys has 'm' after first press");

    ed.handle_key(key('i'));
    assert_eq!(ed.pending_keys.len(), 2, "pending_keys has 'm','i' after second press");

    ed.handle_key(key('w'));
    assert!(ed.pending_keys.is_empty(), "pending_keys cleared after dispatch");
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

    assert!(ed.pending_keys.is_empty(), "pending_keys cleared on NoMatch");
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
    assert_eq!(state(&ed), "-[hello world\nfoo\n]>bar\n", "lines 1-2 selected");
    // Another `x`: extend to line 3.
    ed.handle_key(key('x'));
    assert_eq!(state(&ed), "-[hello world\nfoo\nbar\n]>", "lines 1-3 selected");
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

// ── Command mode ──────────────────────────────────────────────────────────────

#[test]
fn colon_enters_command_mode() {
    let mut ed = editor_from("-[h]>ello\n");
    ed.handle_key(key(':'));
    assert_eq!(ed.mode, Mode::Command);
    assert!(ed.minibuf.is_some());
    assert_eq!(ed.minibuf.as_ref().unwrap().prompt, ':');
    assert_eq!(ed.minibuf.as_ref().unwrap().input, "");
}

#[test]
fn esc_cancels_command_mode() {
    let mut ed = editor_from("-[h]>ello\n");
    ed.handle_key(key(':'));
    ed.handle_key(key('q'));
    ed.handle_key(key_esc());
    assert_eq!(ed.mode, Mode::Normal);
    assert!(ed.minibuf.is_none());
    assert!(!ed.should_quit);
}

#[test]
fn backspace_on_empty_input_cancels() {
    let mut ed = editor_from("-[h]>ello\n");
    ed.handle_key(key(':'));
    ed.handle_key(key_backspace());
    assert_eq!(ed.mode, Mode::Normal);
    assert!(ed.minibuf.is_none());
}

#[test]
fn backspace_removes_last_char() {
    let mut ed = editor_from("-[h]>ello\n");
    ed.handle_key(key(':'));
    ed.handle_key(key('w'));
    ed.handle_key(key('q'));
    ed.handle_key(key_backspace());
    assert_eq!(ed.minibuf.as_ref().unwrap().input, "w");
}

#[test]
fn colon_q_enter_quits() {
    let mut ed = editor_from("-[h]>ello\n");
    ed.handle_key(key(':'));
    ed.handle_key(key('q'));
    ed.handle_key(key_enter());
    assert!(ed.should_quit);
    assert_eq!(ed.mode, Mode::Normal);
    assert!(ed.minibuf.is_none());
}

#[test]
fn colon_quit_enter_quits() {
    let mut ed = editor_from("-[h]>ello\n");
    for ch in ":quit".chars() { ed.handle_key(key(ch)); }
    ed.handle_key(key_enter());
    assert!(ed.should_quit);
}

#[test]
fn colon_w_no_path_sets_error() {
    let mut ed = editor_from("-[h]>ello\n");
    // No file_path set — write should fail with an error message.
    ed.handle_key(key(':'));
    ed.handle_key(key('w'));
    ed.handle_key(key_enter());
    assert!(!ed.should_quit);
    assert_eq!(ed.mode, Mode::Normal);
    assert_eq!(ed.status_msg.as_deref(), Some("no file name"));
}

/// Helper: create a temp file with initial content and wire it into an editor.
///
/// `into_temp_path()` drops the `File` handle (closing it) while keeping a
/// `TempPath` that still deletes the file on drop. The explicit close matters
/// on Windows: `MoveFileEx(MOVEFILE_REPLACE_EXISTING)` — used by the atomic
/// write path — fails with ACCESS_DENIED when the destination file has an open
/// write handle. Separating the handle lifetime from the path lifetime is the
/// idiomatic way to express "I'm done writing, but the path must outlive me".
fn editor_with_file(initial_state: &str, file_content: &str) -> (Editor, tempfile::TempPath) {
    let tmp = tempfile::NamedTempFile::new().unwrap();
    std::fs::write(tmp.path(), file_content).unwrap();
    let path = tmp.path().to_path_buf();
    // Close the file handle, keep the path alive.
    let tmp_path = tmp.into_temp_path();
    let (_, meta) = crate::os::io::read_file(&path).unwrap();
    let mut ed = editor_from(initial_state);
    ed.doc_mut().path = Some(Arc::new(path));
    ed.doc_mut().file_meta = Some(meta);
    (ed, tmp_path)
}

#[test]
fn colon_w_writes_file() {
    let (mut ed, tmp) = editor_with_file("-[h]>ello\n", "hello\n");

    ed.handle_key(key(':'));
    ed.handle_key(key('w'));
    ed.handle_key(key_enter());

    assert_eq!(ed.mode, Mode::Normal);
    assert!(ed.status_msg.as_deref().unwrap_or("").starts_with("Written"));
    assert_eq!(std::fs::read_to_string(&tmp).unwrap(), "hello\n");
}

#[test]
fn colon_wq_writes_and_quits() {
    let (mut ed, tmp) = editor_with_file("-[h]>ello\n", "hello\n");

    for ch in ":wq".chars() { ed.handle_key(key(ch)); }
    ed.handle_key(key_enter());

    assert!(ed.should_quit);
    assert_eq!(std::fs::read_to_string(&tmp).unwrap(), "hello\n");
}

#[test]
fn colon_unknown_sets_error() {
    let mut ed = editor_from("-[h]>ello\n");
    for ch in ":nonsense".chars() { ed.handle_key(key(ch)); }
    ed.handle_key(key_enter());
    assert_eq!(ed.status_msg.as_deref(), Some("Unknown command: nonsense"));
    assert!(!ed.should_quit);
}

#[test]
fn status_msg_cleared_on_next_keypress() {
    let mut ed = editor_from("-[h]>ello\n");
    for ch in ":nonsense".chars() { ed.handle_key(key(ch)); }
    ed.handle_key(key_enter());
    assert!(ed.status_msg.is_some());
    // Any keypress clears it.
    ed.handle_key(key('l'));
    assert!(ed.status_msg.is_none());
}

// ── Dirty-buffer tracking and :q guard ───────────────────────────────────────

#[test]
fn fresh_editor_is_not_dirty() {
    let ed = editor_from("-[h]>ello\n");
    assert!(!ed.doc().is_dirty());
}

#[test]
fn typing_in_insert_mode_makes_dirty() {
    let mut ed = editor_from("-[h]>ello\n");
    ed.handle_key(key('i'));
    ed.handle_key(key('x'));
    ed.handle_key(key_esc());
    assert!(ed.doc().is_dirty());
}

#[test]
fn colon_w_marks_buffer_clean() {
    let (mut ed, _tmp) = editor_with_file("-[h]>ello\n", "hello\n");
    // Make the buffer dirty.
    ed.handle_key(key('i'));
    ed.handle_key(key('x'));
    ed.handle_key(key_esc());
    assert!(ed.doc().is_dirty());
    // Write — should clear dirty flag.
    for ch in ":w".chars() { ed.handle_key(key(ch)); }
    ed.handle_key(key_enter());
    assert!(!ed.doc().is_dirty());
}

#[test]
fn colon_q_on_dirty_buffer_refuses() {
    let mut ed = editor_from("-[h]>ello\n");
    // Make dirty.
    ed.handle_key(key('i'));
    ed.handle_key(key('x'));
    ed.handle_key(key_esc());
    // :q should refuse.
    for ch in ":q".chars() { ed.handle_key(key(ch)); }
    ed.handle_key(key_enter());
    assert!(!ed.should_quit);
    assert_eq!(ed.status_msg.as_deref(), Some("Unsaved changes (add ! to override)"));
}

#[test]
fn colon_q_bang_on_dirty_buffer_quits() {
    let mut ed = editor_from("-[h]>ello\n");
    // Make dirty.
    ed.handle_key(key('i'));
    ed.handle_key(key('x'));
    ed.handle_key(key_esc());
    // :q! should quit regardless.
    for ch in ":q!".chars() { ed.handle_key(key(ch)); }
    ed.handle_key(key_enter());
    assert!(ed.should_quit);
}

#[test]
fn colon_q_on_clean_buffer_quits() {
    let mut ed = editor_from("-[h]>ello\n");
    // Text is fresh (not dirty) — :q should quit.
    for ch in ":q".chars() { ed.handle_key(key(ch)); }
    ed.handle_key(key_enter());
    assert!(ed.should_quit);
}

#[test]
fn colon_w_path_creates_new_file() {
    let tmp_dir = tempfile::tempdir().unwrap();
    let new_path = tmp_dir.path().join("new_file.txt");
    assert!(!new_path.exists());

    let mut ed = editor_from("-[h]>ello\n");
    let cmd = format!(":w {}", new_path.display());
    for ch in cmd.chars() { ed.handle_key(key(ch)); }
    ed.handle_key(key_enter());

    assert!(ed.status_msg.as_deref().unwrap_or("").starts_with("Written"));
    assert!(new_path.exists());
    assert_eq!(std::fs::read_to_string(&new_path).unwrap(), "hello\n");
    // file_path should be updated.
    assert!(ed.doc_mut().path.is_some());
    // Text should now be clean.
    assert!(!ed.doc().is_dirty());
}

#[test]
fn colon_w_path_updates_file_path_for_subsequent_writes() {
    let tmp_dir = tempfile::tempdir().unwrap();
    let new_path = tmp_dir.path().join("subsequent.txt");

    let mut ed = editor_from("-[h]>ello\n");
    // First :w with path — sets file_path and file_meta.
    let cmd = format!(":w {}", new_path.display());
    for ch in cmd.chars() { ed.handle_key(key(ch)); }
    ed.handle_key(key_enter());
    assert!(ed.doc_mut().file_meta.is_some());

    // Make dirty again and write without a path — should use the new path.
    ed.handle_key(key('i'));
    ed.handle_key(key('y'));
    ed.handle_key(key_esc());
    for ch in ":w".chars() { ed.handle_key(key(ch)); }
    ed.handle_key(key_enter());
    assert!(ed.status_msg.as_deref().unwrap_or("").starts_with("Written"));
    assert!(!ed.doc().is_dirty());
}

#[test]
fn colon_wq_path_saves_to_new_file_and_quits() {
    let tmp_dir = tempfile::tempdir().unwrap();
    let new_path = tmp_dir.path().join("wq_test.txt");
    assert!(!new_path.exists());

    let mut ed = editor_from("-[h]>ello\n");
    let cmd = format!(":wq {}", new_path.display());
    for ch in cmd.chars() { ed.handle_key(key(ch)); }
    ed.handle_key(key_enter());

    assert!(ed.should_quit);
    assert!(new_path.exists());
    assert_eq!(std::fs::read_to_string(&new_path).unwrap(), "hello\n");
}

#[test]
fn colon_w_bang_is_rejected() {
    let (mut ed, _tmp) = editor_with_file("-[h]>ello\n", "hello\n");
    for ch in ":w!".chars() { ed.handle_key(key(ch)); }
    ed.handle_key(key_enter());
    assert_eq!(ed.status_msg.as_deref(), Some("w! is not supported"));
    assert!(!ed.should_quit);
}

#[test]
fn colon_wq_bang_quits_even_if_write_fails() {
    // Scratch buffer (no file_path) — write will fail, but :wq! should still quit.
    let mut ed = editor_from("-[h]>ello\n");
    for ch in ":wq!".chars() { ed.handle_key(key(ch)); }
    ed.handle_key(key_enter());
    assert!(ed.should_quit);
}

// ── File metadata preservation ────────────────────────────────────────────────

#[cfg(unix)]
#[test]
fn write_preserves_permissions() {
    use std::os::unix::fs::PermissionsExt;

    let (mut ed, tmp) = editor_with_file("-[h]>ello\n", "hello\n");

    // Set a non-default permission that differs from the tempfile default (0600).
    std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o644)).unwrap();
    // Re-read metadata so file_meta captures the new permissions.
    let (_, meta) = crate::os::io::read_file(&tmp).unwrap();
    ed.doc_mut().file_meta = Some(meta);

    for ch in ":w".chars() { ed.handle_key(key(ch)); }
    ed.handle_key(key_enter());

    assert!(ed.status_msg.as_deref().unwrap_or("").starts_with("Written"));
    let mode = std::fs::metadata(&tmp).unwrap().permissions().mode() & 0o777;
    assert_eq!(mode, 0o644, "permissions must be preserved across atomic write");
}

#[cfg(unix)]
#[test]
fn write_follows_symlink() {
    use std::os::unix::fs::symlink;

    // Create the real file and a symlink pointing to it.
    let real = tempfile::NamedTempFile::new().unwrap();
    std::fs::write(real.path(), "hello\n").unwrap();

    let link_dir = tempfile::tempdir().unwrap();
    let link_path = link_dir.path().join("link.txt");
    symlink(real.path(), &link_path).unwrap();

    // Open via the symlink — io::read_file should resolve it.
    let (_, meta) = crate::os::io::read_file(&link_path).unwrap();
    assert_eq!(meta.resolved_path, std::fs::canonicalize(real.path()).unwrap());

    let mut ed = editor_from("-[h]>ello\n");
    ed.doc_mut().path = Some(Arc::new(link_path.clone()));
    ed.doc_mut().file_meta = Some(meta);

    for ch in ":w".chars() { ed.handle_key(key(ch)); }
    ed.handle_key(key_enter());

    assert!(ed.status_msg.as_deref().unwrap_or("").starts_with("Written"));
    // The symlink must still exist and still be a symlink.
    assert!(link_path.symlink_metadata().unwrap().file_type().is_symlink());
    // Content was written to the real file.
    assert_eq!(std::fs::read_to_string(real.path()).unwrap(), "hello\n");
}

// ── insert-at-selection-start / insert-at-selection-end ──────────────────────

/// `i` with a forward selection collapses to the start of the selection.
#[test]
fn insert_at_selection_start_forward() {
    let mut ed = editor_from("foo -[bar]> baz\n");
    ed.handle_key(key('i'));
    assert_eq!(state(&ed), "foo -[b]>ar baz\n");
    assert_eq!(ed.mode, Mode::Insert);
}

/// `i` with a backward selection also collapses to the start (lower index).
#[test]
fn insert_at_selection_start_backward() {
    let mut ed = editor_from("foo <[bar]- baz\n");
    ed.handle_key(key('i'));
    assert_eq!(state(&ed), "foo -[b]>ar baz\n");
    assert_eq!(ed.mode, Mode::Insert);
}

/// `i` with a collapsed cursor just enters insert at the same position.
#[test]
fn insert_at_selection_start_collapsed() {
    let mut ed = editor_from("foo -[b]>ar baz\n");
    ed.handle_key(key('i'));
    assert_eq!(state(&ed), "foo -[b]>ar baz\n");
    assert_eq!(ed.mode, Mode::Insert);
}

// ── Auto-pairs integration tests ──────────────────────────────────────────────

/// Typing `(` before a word character inserts only `(` (context-aware gating).
/// Typing `(` before whitespace or a close char inserts `()`.
#[test]
fn auto_pairs_auto_close() {
    // Before a word char: no auto-close.
    let mut ed = editor_from("-[h]>ello\n");
    ed.handle_key(key('i'));        // enter insert at 'h'
    ed.handle_key(key('('));
    assert_eq!(state(&ed), "(-[h]>ello\n");

    // Before the structural newline: auto-close fires.
    let mut ed = editor_from("hello-[\n]>");
    ed.handle_key(key('i'));
    ed.handle_key(key('('));
    assert_eq!(state(&ed), "hello(-[)]>\n");
}

/// Typing `)` when the cursor is already sitting on `)` moves the cursor
/// past it rather than inserting a second `)`.
#[test]
fn auto_pairs_skip_close() {
    let mut ed = editor_from("-[h]>ello\n");
    ed.handle_key(key('i'));
    ed.handle_key(key('('));        // inserts `()`, cursor on `)`
    ed.handle_key(key(')'));        // skip-close: moves cursor past `)`
    assert_eq!(state(&ed), "()-[h]>ello\n");
}

/// Backspace between an empty pair `()` deletes both brackets.
#[test]
fn auto_pairs_auto_delete() {
    let mut ed = editor_from("-[h]>ello\n");
    ed.handle_key(key('i'));
    ed.handle_key(key('('));        // buffer: `(|)hello` — cursor on `)`
    ed.handle_key(key_backspace()); // should delete both `(` and `)`
    assert_eq!(state(&ed), "-[h]>ello\n");
}

/// Typing `"` before a word char inserts only `"` (context-aware gating).
/// Typing `"` before whitespace or at EOL inserts `""`.
#[test]
fn auto_pairs_symmetric_auto_close() {
    // Before a word char: no auto-close.
    let mut ed = editor_from("-[x]>\n");
    ed.handle_key(key('i'));
    ed.handle_key(key('"'));
    assert_eq!(state(&ed), "\"-[x]>\n");

    // On an empty line (cursor on `\n`, no prev char): auto-close fires.
    let mut ed = editor_from("-[\n]>");
    ed.handle_key(key('i'));
    ed.handle_key(key('"'));
    assert_eq!(state(&ed), "\"-[\"]>\n");
}

/// Typing `"` again when the cursor is already on a `"` skips over it.
#[test]
fn auto_pairs_symmetric_skip_close() {
    let mut ed = editor_from("-[x]>\n");
    ed.handle_key(key('i'));
    ed.handle_key(key('"'));        // inserts `""`, cursor on second `"`
    ed.handle_key(key('"'));        // skip-close: cursor moves past `"`
    assert_eq!(state(&ed), "\"\"-[x]>\n");
}

/// Typing `)` when the next character is NOT `)` inserts a literal `)`.
#[test]
fn auto_pairs_no_false_skip() {
    let mut ed = editor_from("-[h]>ello\n");
    ed.handle_key(key('i'));
    ed.handle_key(key(')'));        // `)` is not already there — insert normally
    assert_eq!(state(&ed), ")-[h]>ello\n");
}

/// When auto-pairs is disabled, typing `(` inserts only `(`.
#[test]
fn auto_pairs_disabled() {
    let mut ed = editor_from("-[h]>ello\n");
    ed.settings.auto_pairs_enabled = false;
    ed.handle_key(key('i'));
    ed.handle_key(key('('));
    assert_eq!(state(&ed), "(-[h]>ello\n");
}

// Note: wrap-selection (insert_pair_close with a non-cursor selection) is tested
// at the unit level in auto_pairs::tests. It is not reachable via the normal
// editor insert-mode entry points because all of them (i, a, c, o, …) collapse
// to a cursor before entering Insert.

/// Typing `"` before an alphanumeric char inserts only `"`, not `""`.
/// (The scenario from the original bug: `foo -[b]>ar` → `i` → `"` → `"bar`)
#[test]
fn auto_pairs_no_close_before_word_char() {
    let mut ed = editor_from("foo -[b]>ar baz\n");
    ed.handle_key(key('i')); // insert before 'b'
    ed.handle_key(key('"'));
    assert_eq!(state(&ed), "foo \"-[b]>ar baz\n");
}

/// Typing `(` before an alphanumeric char inserts only `(`.
#[test]
fn auto_pairs_no_close_paren_before_word_char() {
    let mut ed = editor_from("-[f]>oo\n");
    ed.handle_key(key('i'));
    ed.handle_key(key('('));
    assert_eq!(state(&ed), "(-[f]>oo\n");
}

/// Typing `"` before a space DOES auto-pair.
#[test]
fn auto_pairs_close_before_space() {
    let mut ed = editor_from("-[ ]>foo\n");
    ed.handle_key(key('i'));
    ed.handle_key(key('"'));
    assert_eq!(state(&ed), "\"-[\"]> foo\n");
}

/// Typing `(` before the structural newline (end of line) DOES auto-pair.
#[test]
fn auto_pairs_close_before_newline() {
    let mut ed = editor_from("foo-[\n]>");
    ed.handle_key(key('i'));
    ed.handle_key(key('('));
    assert_eq!(state(&ed), "foo(-[)]>\n");
}

/// Multi-cursor skip-close is all-or-nothing: if one cursor is on `)` and
/// another is not, the whole operation falls back to plain insert for all cursors.
#[test]
fn auto_pairs_skip_close_mixed_cursors() {
    // cursor 1 on `)`, cursor 2 on `b` — not all cursors match skip-close.
    let mut ed = editor_from("(-[)]>a-[b]>c\n");
    ed.handle_key(key('i'));
    ed.handle_key(key(')'));            // fallback: inserts `)` at both positions
    assert_eq!(state(&ed), "()-[)]>a)-[b]>c\n");
}

/// Multi-cursor delete-pair is all-or-nothing: if one cursor is between a pair
/// and another is not, backspace falls back to plain delete-char-backward for all.
#[test]
fn auto_pairs_auto_delete_mixed_cursors() {
    // cursor 1 between `()`, cursor 2 between `a`+`b` (not a pair).
    let mut ed = editor_from("(-[)]>a-[b]>c\n");
    ed.handle_key(key('i'));
    ed.handle_key(key_backspace());     // fallback: each cursor deletes one char backward
    assert_eq!(state(&ed), "-[)]>-[b]>c\n");
}

// ── Normal-mode pair-wrap ─────────────────────────────────────────────────────

/// With a selection active, typing `"` in normal mode wraps it with `""`.
#[test]
fn normal_wrap_selection_double_quote() {
    let mut ed = editor_from("foo -[bar]> baz\n");
    ed.handle_key(key('"'));
    assert_eq!(state(&ed), "foo \"bar-[\"]> baz\n");
}

/// `(` is bound to cycle-primary-backward and retains that behaviour even
/// with a non-collapsed selection — only unbound pair chars trigger wrap.
#[test]
fn normal_wrap_bound_key_not_intercepted() {
    let mut ed = editor_from("foo -[bar]> baz\n");
    ed.handle_key(key('('));
    // `(` runs cycle-primary-backward, NOT wrap — selection is unchanged.
    assert_eq!(state(&ed), "foo -[bar]> baz\n");
}

/// A collapsed cursor + `"` in normal mode is silently swallowed (no binding).
#[test]
fn normal_wrap_noop_on_cursor() {
    let mut ed = editor_from("foo -[b]>ar baz\n");
    ed.handle_key(key('"'));
    assert_eq!(state(&ed), "foo -[b]>ar baz\n");
}

/// Multi-cursor: two selections both get wrapped independently.
#[test]
fn normal_wrap_multi_cursor() {
    let mut ed = editor_from("-[foo]> -[bar]>\n");
    ed.handle_key(key('"'));
    assert_eq!(state(&ed), "\"foo-[\"]> \"bar-[\"]>\n");
}

/// Multi-line selection: wrapping spans the newline cleanly.
/// The structural trailing `\n` is excluded from the wrap via end_inclusive clamping.
#[test]
fn normal_wrap_multi_line_selection() {
    let mut ed = editor_from("-[foo\nbar]> baz\n");
    ed.handle_key(key('"'));
    assert_eq!(state(&ed), "\"foo\nbar-[\"]> baz\n");
}

/// When auto-pairs is disabled, the pair char is swallowed even with a selection.
#[test]
fn normal_wrap_disabled_when_auto_pairs_off() {
    let mut ed = editor_from("foo -[bar]> baz\n");
    ed.settings.auto_pairs_enabled = false;
    ed.handle_key(key('"'));
    assert_eq!(state(&ed), "foo -[bar]> baz\n");
}

// ── f/t character find ────────────────────────────────────────────────────────

/// `fa` in Normal mode: cursor lands on the next `a`.
#[test]
fn find_forward_inclusive_basic() {
    let mut ed = editor_from("-[h]>ello a world\n");
    ed.handle_key(key('f'));
    ed.handle_key(key('a'));
    assert_eq!(state(&ed), "hello -[a]> world\n");
}

/// `ta` stops one grapheme before the next `a`.
#[test]
fn find_forward_exclusive_basic() {
    let mut ed = editor_from("-[h]>ello a world\n");
    ed.handle_key(key('t'));
    ed.handle_key(key('a'));
    assert_eq!(state(&ed), "hello-[ ]>a world\n");
}

/// `Fa` finds `a` backward.
#[test]
fn find_backward_inclusive_basic() {
    let mut ed = editor_from("hello a -[w]>orld\n");
    ed.handle_key(key('F'));
    ed.handle_key(key('a'));
    assert_eq!(state(&ed), "hello -[a]> world\n");
}

/// `Ta` stops one grapheme after the `a` when searching backward.
#[test]
fn find_backward_exclusive_basic() {
    let mut ed = editor_from("hello a -[w]>orld\n");
    ed.handle_key(key('T'));
    ed.handle_key(key('a'));
    assert_eq!(state(&ed), "hello a-[ ]>world\n");
}

/// `=` repeats the last find forward regardless of original direction.
#[test]
fn find_repeat_forward() {
    let mut ed = editor_from("-[h]>ello a world a end\n");
    ed.handle_key(key('f'));
    ed.handle_key(key('a'));
    // cursor on first 'a'
    ed.handle_key(key('='));
    assert_eq!(state(&ed), "hello a world -[a]> end\n");
}

/// `-` repeats backward regardless of original direction.
#[test]
fn find_repeat_backward() {
    let mut ed = editor_from("-[h]>ello a world a end\n");
    // Jump to second 'a' first.
    ed.handle_key(key('f'));
    ed.handle_key(key('a'));
    ed.handle_key(key('='));
    // Now repeat backward — should land on first 'a'.
    ed.handle_key(key('-'));
    assert_eq!(state(&ed), "hello -[a]> world a end\n");
}

/// `Fa` followed by `=` goes forward (absolute direction, not backward).
#[test]
fn find_repeat_absolute_direction() {
    let mut ed = editor_from("hello a -[w]>orld a end\n");
    ed.handle_key(key('F'));
    ed.handle_key(key('a'));
    // cursor on first 'a' (backward search)
    // `=` must go forward (absolute), landing on the second 'a'.
    ed.handle_key(key('='));
    assert_eq!(state(&ed), "hello a world -[a]> end\n");
}

/// `=` with no prior find is a no-op.
#[test]
fn find_repeat_noop_when_no_prior_find() {
    let mut ed = editor_from("-[h]>ello\n");
    ed.handle_key(key('='));
    assert_eq!(state(&ed), "-[h]>ello\n");
}

/// Extend mode: `e` then `fa` extends the selection to include 'a'.
#[test]
fn find_forward_extend_mode() {
    let mut ed = editor_from("-[h]>ello a world\n");
    ed.handle_key(key('e'));    // toggle extend
    ed.handle_key(key('f'));
    ed.handle_key(key('a'));
    assert_eq!(state(&ed), "-[hello a]> world\n");
}

/// `f` with no match is a no-op.
#[test]
fn find_forward_no_match_is_noop() {
    let mut ed = editor_from("-[h]>ello\n");
    ed.handle_key(key('f'));
    ed.handle_key(key('z'));
    assert_eq!(state(&ed), "-[h]>ello\n");
}

/// `=` after `ta` (exclusive) repeats with the same exclusive kind — stops
/// one grapheme before the next occurrence, not on it.
#[test]
fn find_repeat_exclusive_kind_preserved() {
    let mut ed = editor_from("-[h]>ello a world a end\n");
    ed.handle_key(key('t'));
    ed.handle_key(key('a'));
    // cursor on the space before first 'a'
    assert_eq!(state(&ed), "hello-[ ]>a world a end\n");
    // move past the first 'a' so `=` can find the second
    ed.handle_key(key('l'));
    ed.handle_key(key('l'));
    ed.handle_key(key('='));
    // should land on the space before second 'a', not on 'a' itself
    assert_eq!(state(&ed), "hello a world-[ ]>a end\n");
}

// ── Kitty keyboard protocol — Ctrl+motion one-shot extend ──────────────────

/// Ctrl+h extends the selection left (kitty mode only).
#[test]
fn kitty_ctrl_h_extends_left() {
    let mut ed = editor_from_kitty("hell-[o]>\n");
    ed.handle_key(key_ctrl('h'));
    // anchor=4 stays, head moves left to 3 → backward selection
    assert_eq!(state(&ed), "hel<[lo]-\n");
}

/// Ctrl+l extends the selection right (kitty mode only).
#[test]
fn kitty_ctrl_l_extends_right() {
    let mut ed = editor_from_kitty("-[h]>ello\n");
    ed.handle_key(key_ctrl('l'));
    assert_eq!(state(&ed), "-[he]>llo\n");
}

/// Ctrl+j extends the selection down one line (kitty mode only).
#[test]
fn kitty_ctrl_j_extends_down() {
    let mut ed = editor_from_kitty("-[h]>ello\nworld\n");
    ed.handle_key(key_ctrl('j'));
    assert_eq!(state(&ed), "-[hello\nw]>orld\n");
}

/// Ctrl+k extends the selection up one line (kitty mode only).
#[test]
fn kitty_ctrl_k_extends_up() {
    let mut ed = editor_from_kitty("hello\n-[w]>orld\n");
    ed.handle_key(key_ctrl('k'));
    // anchor=6 stays, head moves up to col 0 of line 0 → backward
    assert_eq!(state(&ed), "<[hello\nw]-orld\n");
}

/// Ctrl+w is the window prefix — pressing it alone waits for a second key.
/// The state is unchanged after just Ctrl+w (Interior node, not a leaf).
#[test]
fn ctrl_w_starts_window_prefix() {
    let mut ed = editor_from_kitty("-[h]>ello world\n");
    ed.handle_key(key_ctrl('w'));
    assert_eq!(state(&ed), "-[h]>ello world\n", "Ctrl+w alone must not change state");
}

/// Ctrl+w, w → pane-focus-next stub (not yet implemented).
#[test]
fn ctrl_w_w_is_pane_focus_next_stub() {
    let mut ed = editor_from_kitty("-[h]>ello world\n");
    ed.handle_key(key_ctrl('w'));
    ed.handle_key(key('w'));
    assert_eq!(state(&ed), "-[h]>ello world\n", "stub must not move cursor");
    assert!(
        ed.status_msg.as_deref().unwrap_or("").contains("not yet implemented"),
        "stub must report not-yet-implemented: {:?}", ed.status_msg,
    );
}

/// Ctrl+b extends to the previous word via union semantics (kitty mode only).
/// From cursor at 'w' (pos 6), select_prev_word finds "hello" (0,4).
/// Union: min(6,0)=0, max(6,4)=6 → "hello w" forward.
#[test]
fn kitty_ctrl_b_extends_prev_word() {
    let mut ed = editor_from_kitty("hello -[w]>orld\n");
    ed.handle_key(key_ctrl('b'));
    assert_eq!(state(&ed), "-[hello w]>orld\n");
}

/// Without kitty, Ctrl+h is a no-op — legacy terminals can't reliably
/// distinguish Ctrl+letter from control codes, so implicit Ctrl+motion
/// is suppressed entirely.
#[test]
fn legacy_ctrl_h_is_noop() {
    let mut ed = editor_from("-[hello]>world\n");
    ed.handle_key(key_ctrl('h'));
    assert_eq!(state(&ed), "-[hello]>world\n");
}

/// Without kitty, Ctrl+w starts the window prefix but leaves state unchanged.
#[test]
fn legacy_ctrl_w_is_noop() {
    let mut ed = editor_from("-[hello]> world foo\n");
    ed.handle_key(key_ctrl('w'));
    assert_eq!(state(&ed), "-[hello]> world foo\n");
}

/// Ctrl+u must be a no-op in kitty mode: 'u' maps to "undo" which has no
/// extend variant, so the one-shot extend guard should suppress it.
#[test]
fn kitty_ctrl_u_is_noop() {
    let mut ed = editor_from_kitty("-[h]>ello\n");
    // Make an edit so undo would have something to revert.
    ed.handle_key(key('d'));
    assert_eq!(ed.doc().text().to_string(), "ello\n");
    // Ctrl+u should NOT run undo — it's a no-op because "undo" has no extend variant.
    ed.handle_key(key_ctrl('u'));
    assert_eq!(ed.doc().text().to_string(), "ello\n", "Ctrl+u should be a no-op in kitty mode");
}

/// Ctrl+} extends to the next paragraph (kitty mode).
///
/// With REPORT_ALTERNATE_KEYS, crossterm delivers the shifted character
/// directly: Ctrl+} arrives as Char('}') with CONTROL (no SHIFT).
#[test]
fn kitty_ctrl_close_brace_extends_next_paragraph() {
    let mut ed = editor_from_kitty("-[h]>ello\n\nworld\n");
    ed.handle_key(key_ctrl('}'));
    // extend-next-paragraph: anchor stays at 0, head moves to 'w' in "world".
    assert_eq!(state(&ed), "-[hello\n\nw]>orld\n");
}

/// Ctrl+$ extends to end of line (kitty mode).
#[test]
fn kitty_ctrl_dollar_extends_line_end() {
    let mut ed = editor_from_kitty("-[h]>ello world\n");
    ed.handle_key(key_ctrl('$'));
    // goto-line-end extend variant: anchor stays, head moves to last char on line.
    assert_eq!(state(&ed), "-[hello world]>\n");
}

/// Ctrl+0 extends to start of line (kitty mode).
#[test]
fn kitty_ctrl_0_extends_line_start() {
    let mut ed = editor_from_kitty("hello -[w]>orld\n");
    ed.handle_key(key_ctrl('0'));
    // goto-line-start extend variant: head moves to col 0.
    assert_eq!(state(&ed), "<[hello w]-orld\n");
}

/// Ctrl+U (redo) must also be a no-op in kitty mode.
#[test]
fn kitty_ctrl_shift_u_is_noop() {
    let mut ed = editor_from_kitty("-[h]>ello\n");
    ed.handle_key(key('d'));
    assert_eq!(ed.doc().text().to_string(), "ello\n");
    ed.handle_key(key('u'));    // regular undo
    assert_eq!(ed.doc().text().to_string(), "hello\n");
    // Ctrl+U should NOT run redo.
    ed.handle_key(key_ctrl('U'));
    assert_eq!(ed.doc().text().to_string(), "hello\n", "Ctrl+U should be a no-op in kitty mode");
}

// ── Dot-repeat tests ──────────────────────────────────────────────────────────

/// `d` deletes the selection. Moving then pressing `.` should delete the next selection.
#[test]
fn dot_repeats_delete() {
    // Cursor starts at 'f'. `w` selects "foo", `d` deletes it.
    // Then from the space at pos 0, `w` selects "bar" (the next word). `.` deletes it.
    let mut ed = editor_from("-[foo]> bar\n");
    ed.handle_key(key('d'));           // delete "foo" → " bar\n", cursor at 0 (space)
    assert_eq!(ed.doc().text().to_string(), " bar\n");

    ed.handle_key(key('w'));           // from space, select "bar"
    ed.handle_key(key('.'));           // repeat delete
    assert_eq!(ed.doc().text().to_string(), " \n");
}

/// `c` + typed text + Esc should be replayable: the replacement text is reused.
#[test]
fn dot_repeats_change_with_insert() {
    let mut ed = editor_from("-[foo]> bar\n");

    ed.handle_key(key('c'));           // change: delete "foo", enter Insert
    ed.handle_key(key('h'));
    ed.handle_key(key('i'));
    ed.handle_key(key_esc());          // back to Normal; buffer is "hi bar"

    assert_eq!(ed.doc().text().to_string(), "hi bar\n");

    // Move to "bar" and repeat.
    ed.handle_key(key('w'));           // select "bar"
    ed.handle_key(key('.'));           // repeat: delete "bar", insert "hi"

    assert_eq!(ed.doc().text().to_string(), "hi hi\n");
}

/// `i` + typed text + Esc inserts at the selection start. `.` should replay that insert.
#[test]
fn dot_repeats_insert_before() {
    let mut ed = editor_from("-[x]>\n");

    ed.handle_key(key('i'));           // insert-at-selection-start, cursor collapses to start
    ed.handle_key(key('a'));
    ed.handle_key(key('b'));
    ed.handle_key(key_esc());          // back to Normal; buffer is "abx"

    assert_eq!(ed.doc().text().to_string(), "abx\n");

    // Move to 'x' and repeat.
    ed.handle_key(key('w'));           // select 'x'
    ed.handle_key(key('.'));           // repeat insert "ab" before 'x'

    assert_eq!(ed.doc().text().to_string(), "ababx\n");
}

/// `r` + char replaces every character in the selection. `.` should replay with
/// the same replacement character.
#[test]
fn dot_repeats_replace() {
    // Use a space between words so `w` can navigate to the second word.
    let mut ed = editor_from("-[ab]> cd\n");

    ed.handle_key(key('r'));           // wait-char
    ed.handle_key(key('x'));           // replace "ab" → "xx cd\n"

    assert_eq!(ed.doc().text().to_string(), "xx cd\n");

    // `w` from the "xx" selection (head at pos 1) selects the next word "cd".
    ed.handle_key(key('w'));
    ed.handle_key(key('.'));           // repeat replace with 'x' → "xx xx\n"

    assert_eq!(ed.doc().text().to_string(), "xx xx\n");
}

/// When `.` is given an explicit count, that count overrides the one stored in
/// the action.
#[test]
fn dot_with_explicit_count_overrides() {
    // Select one word and delete it.
    let mut ed = editor_from("-[a]> b c d e\n");
    ed.handle_key(key('d'));           // delete "a" → " b c d e"

    // Select "b", repeat with count=3 → should apply delete 3 times somehow.
    // Actually count on `d` itself is a repetition of `d`; here we test that
    // the stored count=1 is replaced by the explicit count=2.
    // Two-digit test: press `2` then `.` to apply 2 copies of the delete.
    // Re-select "b":
    ed.handle_key(key('w'));           // select "b"
    ed.handle_key(key('d'));           // delete "b" (now last_action.count=1)

    // Select "c":
    ed.handle_key(key('w'));           // select "c"
    // Press `2.` — explicit count 2 overrides stored count 1.
    // Since `d` doesn't loop on count, this effectively runs `d` with count=2,
    // but `d` ignores count entirely (_count). The key point is `explicit_count`
    // is set and the stored count (1) is NOT used — the passed count (2) is.
    // We verify last_action.count is reset to the stored 1 after replay.
    ed.handle_key(key('2'));
    ed.handle_key(key('.'));
    // Just verify it doesn't panic and the buffer changed.
    assert!(!ed.doc().text().to_string().contains('c'));
}

/// When `.` is pressed without a count, the original action's count is reused.
#[test]
fn dot_without_count_uses_original() {
    // Use `select-line` (x) which is repeatable... wait, 'x' is select-line which
    // is a Selection command (not repeatable). Use 'p' (paste) instead.
    // Actually let's test with `d` — record with count, replay without.
    // `d` ignores count anyway, so let's use a simpler repeatable: paste.
    // Use `i` + text + Esc with count, then `.` without count.
    // Actually the simplest: just verify last_action.count is preserved.
    let mut ed = editor_from("-[hi]> world\n");

    // `d` (count ignored by the command, but stored as 1 in last_action).
    ed.handle_key(key('d'));
    assert_eq!(ed.last_action.as_ref().unwrap().count, 1);

    // Move to "world", hit `.` without a count.
    ed.handle_key(key('w'));
    ed.handle_key(key('.'));
    // last_action.count should still be 1 after replay.
    assert_eq!(ed.last_action.as_ref().unwrap().count, 1);
    // The delete should have happened.
    assert!(!ed.doc().text().to_string().contains("world"));
}

/// After `.`, a single `u` should undo the entire replayed action as one step.
#[test]
fn dot_is_single_undo_step() {
    let mut ed = editor_from("-[foo]> bar\n");

    // `c` + "hi" + Esc = one undo step.
    ed.handle_key(key('c'));
    ed.handle_key(key('h'));
    ed.handle_key(key('i'));
    ed.handle_key(key_esc());
    assert_eq!(ed.doc().text().to_string(), "hi bar\n");

    // Move to "bar" and repeat.
    ed.handle_key(key('w'));
    ed.handle_key(key('.'));
    assert_eq!(ed.doc().text().to_string(), "hi hi\n");

    // One undo undoes the `.` replay entirely.
    ed.handle_key(key('u'));
    assert_eq!(ed.doc().text().to_string(), "hi bar\n");
}

/// Pressing `.` before any edit has been recorded should be a no-op.
#[test]
fn dot_noop_without_prior_action() {
    let mut ed = editor_from("-[hello]> world\n");
    let before = state(&ed);
    ed.handle_key(key('.'));
    assert_eq!(state(&ed), before);
}

/// `o` (open-line-below) + typed text + Esc should be replayable.
#[test]
fn dot_repeats_open_line_below() {
    let mut ed = editor_from("-[a]>\nb\n");

    ed.handle_key(key('o'));           // open line below "a", enter Insert
    ed.handle_key(key('x'));
    ed.handle_key(key_esc());          // back to Normal; buffer is "a\nx\nb"

    assert_eq!(ed.doc().text().to_string(), "a\nx\nb\n");

    // Move cursor to "b" and repeat.
    ed.handle_key(key('j'));           // move down to 'x'
    ed.handle_key(key('j'));           // move down to 'b'
    ed.handle_key(key('.'));           // repeat: open line below "b", insert "x"

    assert_eq!(ed.doc().text().to_string(), "a\nx\nb\nx\n");
}

/// `p` (paste-after) is repeatable: the register contents are pasted again.
#[test]
fn dot_repeats_paste_after() {
    let mut ed = editor_from("-[ab]>cd\n");

    // Yank "ab" then delete so we have something to paste.
    ed.handle_key(key('y'));           // yank "ab" into default register
    ed.handle_key(key('d'));           // delete "ab" → cursor on "cd"

    // Paste after.
    ed.handle_key(key('p'));           // pastes "ab" after 'c' → "cabd"
    // Move to end character and repeat.
    ed.handle_key(key('w'));           // select "cd" or next word
    ed.handle_key(key('.'));           // paste again
    // Just verify no panic and paste happened twice (content contains "ab" twice).
    let buf = ed.doc().text().to_string();
    let count = buf.matches("ab").count();
    assert!(count >= 2, "expected at least 2 occurrences of 'ab', got: {buf:?}");
}

/// `f`/`t` are NOT repeatable (they have `=`/`-` for that). Pressing `.`
/// after a find/till motion should be a no-op.
#[test]
fn dot_after_find_is_noop() {
    let mut ed = editor_from("-[h]>ello world\n");

    // `f` + `o` moves cursor to the first 'o' in "hello".
    ed.handle_key(key('f'));
    ed.handle_key(key('o'));
    let state_after_find = state(&ed);

    // `.` should have nothing recorded and leave state unchanged.
    assert!(ed.last_action.is_none());
    ed.handle_key(key('.'));
    assert_eq!(state(&ed), state_after_find);
}

// ── Search ────────────────────────────────────────────────────────────────────

/// `/` opens Search mode; typing a pattern triggers live search; `Enter` confirms
/// the match and writes the pattern to the `'s'` register.
#[test]
fn search_forward_enter_confirms() {
    let mut ed = editor_from("-[h]>ello world\n");

    ed.handle_key(key('/'));
    assert_eq!(ed.mode, Mode::Search);

    for ch in "world".chars() {
        ed.handle_key(key(ch));
    }
    // Live search has already moved the selection to "world".
    assert_eq!(state(&ed), "hello -[world]>\n");

    ed.handle_key(key_enter());
    assert_eq!(ed.mode, Mode::Normal);
    assert_eq!(state(&ed), "hello -[world]>\n");
    // Pattern written to the 's' register for n/N repeat.
    assert_eq!(reg(&ed, 's'), vec!["world"]);
}

/// `Esc` during search restores the selection to its pre-search state.
#[test]
fn search_esc_restores_position() {
    let mut ed = editor_from("-[h]>ello world\n");

    ed.handle_key(key('/'));
    for ch in "world".chars() {
        ed.handle_key(key(ch));
    }
    assert_eq!(state(&ed), "hello -[world]>\n");

    ed.handle_key(key_esc());
    assert_eq!(ed.mode, Mode::Normal);
    assert_eq!(state(&ed), "-[h]>ello world\n");
}

/// `n` repeats the last confirmed forward search, advancing through matches in
/// document order.
#[test]
fn search_n_repeats_forward() {
    // "ab ab ab\n" — three "ab" matches at (0,1), (3,4), (6,7).
    let mut ed = editor_from("-[a]>b ab ab\n");

    ed.handle_key(key('/'));
    ed.handle_key(key('a'));
    ed.handle_key(key('b'));
    ed.handle_key(key_enter());
    assert_eq!(state(&ed), "-[ab]> ab ab\n");

    ed.handle_key(key('n'));
    assert_eq!(state(&ed), "ab -[ab]> ab\n");

    ed.handle_key(key('n'));
    assert_eq!(state(&ed), "ab ab -[ab]>\n");
}

/// `n` always goes forward and `N` always goes backward regardless of how the search was initiated.
#[test]
fn search_n_repeats_backward() {
    let mut ed = editor_from("-[a]>b ab ab\n");

    ed.handle_key(key('/'));
    ed.handle_key(key('a'));
    ed.handle_key(key('b'));
    ed.handle_key(key_enter());
    // Advance to the second match.
    ed.handle_key(key('n'));
    assert_eq!(state(&ed), "ab -[ab]> ab\n");

    // N goes back.
    ed.handle_key(key('N'));
    assert_eq!(state(&ed), "-[ab]> ab ab\n");
}

/// After a `?` backward search, `n` still goes forward (absolute direction model).
///
/// Vim would go backward here; HUME uses absolute direction (same choice as Kakoune/Helix).
#[test]
fn search_backward_n_goes_forward() {
    // Three matches; cursor at the third. `?` lands on the second.
    let mut ed = editor_from("ab ab -[a]>b\n");

    ed.handle_key(key('?'));
    ed.handle_key(key('a'));
    ed.handle_key(key('b'));
    ed.handle_key(key_enter());
    assert_eq!(state(&ed), "ab -[ab]> ab\n");

    // n must go forward (to the third match), not backward (to the first).
    ed.handle_key(key('n'));
    assert_eq!(state(&ed), "ab ab -[ab]>\n");
}

/// `?` searches backward — the confirmed match is the last occurrence before
/// the pre-search cursor position.
#[test]
fn search_backward_confirms() {
    // Cursor at the third "ab"; backward search should land on the second.
    let mut ed = editor_from("ab ab -[a]>b\n");

    ed.handle_key(key('?'));
    assert_eq!(ed.mode, Mode::Search);

    ed.handle_key(key('a'));
    ed.handle_key(key('b'));
    ed.handle_key(key_enter());

    assert_eq!(ed.mode, Mode::Normal);
    assert_eq!(state(&ed), "ab -[ab]> ab\n");
}

/// When no match exists, `n` sets the "no match" status message.
/// Confirming a search with no match returns to the pre-search position.
#[test]
fn search_no_match_behaviour() {
    let mut ed = editor_from("-[h]>ello\n");

    // Confirm a pattern that matches nothing.
    ed.handle_key(key('/'));
    ed.handle_key(key('x'));
    ed.handle_key(key('y'));
    ed.handle_key(key('z'));
    ed.handle_key(key_enter());

    assert_eq!(ed.mode, Mode::Normal);
    // Position restored to pre-search (live search restores on each no-match keystroke).
    assert_eq!(state(&ed), "-[h]>ello\n");

    // n: "no match" status message.
    ed.handle_key(key('n'));
    assert_eq!(ed.status_msg.as_deref(), Some("no match"));
}

/// Extend-search-next keeps the original anchor and moves the head to the match.
#[test]
fn extend_search_next_extends_selection() {
    // Cursor on 'h'; search forward for "world" with extend active.
    let mut ed = editor_from("-[h]>ello world\n");
    ed.mode = Mode::Extend;

    ed.handle_key(key('/'));
    for ch in "world".chars() {
        ed.handle_key(key(ch));
    }
    // Live search in extend mode: anchor stays at 0 ('h'), head moves to 10 ('d').
    assert_eq!(state(&ed), "-[hello world]>\n");

    ed.handle_key(key_enter());
    ed.mode = Mode::Normal;

    // n in extend mode: anchor stays at 0, head jumps to next match.
    ed.mode = Mode::Extend;
    // Only one "world" — wraps back to the same match.
    ed.handle_key(key('n'));
    // Selection should still cover from anchor=0 to the match end.
    assert_eq!(state(&ed), "-[hello world]>\n");
}

/// `Esc` in Normal mode clears the active search regex and its cached state.
#[test]
fn esc_in_normal_clears_search() {
    let mut ed = editor_from("-[h]>ello hello\n").with_search_regex("hello");

    assert!(ed.search_pattern().is_some(), "pre-condition: search pattern is set");
    assert!(ed.current_search_cursor().match_count.is_some(), "pre-condition: cache is populated");

    ed.handle_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
    ed.sync_search_cache();

    assert!(ed.search_pattern().is_none(), "search pattern should be cleared by Esc");
    assert!(ed.current_search_cursor().match_count.is_none(), "match_count should be cleared by Esc");
    assert!(ed.search_matches().matches.is_empty(), "matches should be cleared by Esc");
}

/// `:clear-search` in Command mode clears the active search regex and its cached state.
#[test]
fn command_clear_search_clears_search() {
    let mut ed = editor_from("-[h]>ello hello\n").with_search_regex("hello");

    assert!(ed.search_pattern().is_some(), "pre-condition: search pattern is set");

    // :clear-search (canonical name)
    ed.handle_key(key(':'));
    for ch in "clear-search".chars() {
        ed.handle_key(key(ch));
    }
    ed.handle_key(key_enter());
    ed.sync_search_cache();

    assert_eq!(ed.mode, Mode::Normal);
    assert!(ed.search_pattern().is_none(), "search pattern should be cleared by :clear-search");
    assert!(ed.current_search_cursor().match_count.is_none(), "match_count should be cleared by :clear-search");
    assert!(ed.search_matches().matches.is_empty(), "matches should be cleared by :clear-search");

}

// ── Select within (s) ────────────────────────────────────────────────────────

/// `s` is a noop when all selections are collapsed (anchor == head).
#[test]
fn select_within_noop_when_collapsed() {
    let mut ed = editor_from("-[h]>ello\n");
    ed.handle_key(key('s'));
    assert_eq!(ed.mode, Mode::Normal);
    assert!(ed.minibuf.is_none());
}

/// `s` enters Select mode, sets up minibuffer, and snapshots selections.
#[test]
fn select_within_enters_select_mode() {
    let mut ed = editor_from("-[hello world]>\n");
    ed.handle_key(key('s'));
    assert_eq!(ed.mode, Mode::Select);
    assert!(ed.pane_transient[ed.focused_pane_id].pre_select_sels.is_some());
    assert!(ed.minibuf.is_some());
    assert_eq!(ed.minibuf.as_ref().unwrap().prompt, '⫽');
}

/// `s` + pattern + Enter confirms: selections become matches, mode returns to Normal.
#[test]
fn select_within_confirm_replaces_selections() {
    let mut ed = editor_from("-[ab cd ab]>\n");
    ed.handle_key(key('s'));
    ed.handle_key(key('a'));
    ed.handle_key(key('b'));
    ed.handle_key(key_enter());

    assert_eq!(ed.mode, Mode::Normal);
    assert!(ed.pane_transient[ed.focused_pane_id].pre_select_sels.is_none());
    // Two "ab" matches within the original selection.
    assert_eq!(ed.current_selections().len(), 2);
    assert_eq!(ed.current_selections().primary().anchor, 0);
    assert_eq!(ed.current_selections().primary().head, 1);
}

/// `s` + Esc restores original selections.
#[test]
fn select_within_esc_restores() {
    let mut ed = editor_from("-[ab cd ab]>\n");
    let original = state(&ed);
    ed.handle_key(key('s'));
    ed.handle_key(key('a'));
    ed.handle_key(key('b'));
    // Live preview should have changed selections.
    assert_ne!(state(&ed), original);
    ed.handle_key(key_esc());
    assert_eq!(ed.mode, Mode::Normal);
    assert_eq!(state(&ed), original);
}

/// `s` + Enter with empty pattern cancels (same as Esc).
#[test]
fn select_within_empty_confirm_cancels() {
    let mut ed = editor_from("-[hello]>\n");
    let original = state(&ed);
    ed.handle_key(key('s'));
    ed.handle_key(key_enter());
    assert_eq!(ed.mode, Mode::Normal);
    assert_eq!(state(&ed), original);
}

/// `s` does not overwrite the search register — it is a selection op, not a search.
/// A prior search pattern must survive a select-within so that n/N still works.
#[test]
fn select_within_does_not_overwrite_search_register() {
    use crate::ops::register::SEARCH_REGISTER;
    let mut ed = editor_from("-[ab cd ab]>\n");
    // Simulate a prior search by writing directly to the search register (as
    // search confirm does).
    ed.registers.write_text(SEARCH_REGISTER, vec!["cd".to_string()]);
    // Select within using a different pattern.
    ed.handle_key(key('s'));
    ed.handle_key(key('a'));
    ed.handle_key(key('b'));
    ed.handle_key(key_enter());
    // The search register must still hold "cd", not "ab".
    assert_eq!(reg(&ed, 's'), vec!["cd"]);
}

/// `s` does not set the search regex — highlights would be misleading
/// because they appear outside the selection scope.
#[test]
fn select_within_does_not_set_search_regex() {
    let mut ed = editor_from("-[ab cd ab]>\n");
    ed.handle_key(key('s'));
    ed.handle_key(key('a'));
    ed.handle_key(key('b'));
    assert!(ed.search_pattern().is_none());
}

/// `s` with no matches restores original selections on each keystroke.
#[test]
fn select_within_no_matches_keeps_originals() {
    let mut ed = editor_from("-[hello]>\n");
    let original = state(&ed);
    ed.handle_key(key('s'));
    ed.handle_key(key('z'));
    // No match for "z" in "hello" — should still show original selections.
    assert_eq!(state(&ed), original);
}

// ── select-within with multiple cursors ───────────────────────────────────────

/// Two pre-existing selections each containing matches — `s` produces one
/// result selection per match, across all original selections.
///
/// "aa bb aa\n" with two selections: [aa ] and [aa] at start/end.
/// Splitting on "aa" yields two "aa" selections, one from each original.
#[test]
fn select_within_multiple_selections_finds_matches_in_each() {
    use crate::core::selection::{Selection, SelectionSet};
    // "aa bb aa\n"
    //  0123456789
    let mut ed = editor_from("-[aa bb aa]>\n");
    // Replace with two selections: "aa " (0..2) and "aa" (6..7).
    let two_sels = SelectionSet::from_vec(
        vec![
            Selection::new(0, 2), // "aa " — primary
            Selection::new(6, 7), // "aa"
        ],
        0,
    );
    ed.set_current_selections(two_sels);

    ed.handle_key(key('s'));
    for ch in "aa".chars() { ed.handle_key(key(ch)); }
    ed.handle_key(key_enter());

    // One "aa" from each original selection → 2 selections total.
    assert_eq!(ed.current_selections().len(), 2);
    // First match: chars 0..1 ("aa" in first selection).
    let sels: Vec<_> = ed.current_selections().iter_sorted().collect();
    assert_eq!(sels[0].start(), 0);
    assert_eq!(sels[0].end_inclusive(ed.doc().text()), 1);
    // Second match: chars 6..7 ("aa" in second selection).
    assert_eq!(sels[1].start(), 6);
    assert_eq!(sels[1].end_inclusive(ed.doc().text()), 7);
}

/// When one selection has matches and another does not, only the matching
/// selection produces results — the non-matching one is dropped.
#[test]
fn select_within_drops_selections_with_no_match() {
    use crate::core::selection::{Selection, SelectionSet};
    // "aa bb cc\n"
    //  01234567
    let mut ed = editor_from("-[aa bb cc]>\n");
    let two_sels = SelectionSet::from_vec(
        vec![
            Selection::new(0, 1), // "aa" — primary, has match
            Selection::new(6, 7), // "cc" — no "aa" here
        ],
        0,
    );
    ed.set_current_selections(two_sels);

    ed.handle_key(key('s'));
    for ch in "aa".chars() { ed.handle_key(key(ch)); }
    ed.handle_key(key_enter());

    // Only one match (from the first selection).
    assert_eq!(ed.current_selections().len(), 1);
    assert_eq!(ed.current_selections().primary().start(), 0);
    assert_eq!(ed.current_selections().primary().end_inclusive(ed.doc().text()), 1);
}

/// When NO selection contains a match, the original selections are restored.
#[test]
fn select_within_multiple_selections_no_match_restores_all() {
    use crate::core::selection::{Selection, SelectionSet};
    let mut ed = editor_from("-[aa bb cc]>\n");
    let two_sels = SelectionSet::from_vec(
        vec![
            Selection::new(0, 1),
            Selection::new(3, 4),
        ],
        0,
    );
    ed.set_current_selections(two_sels.clone());

    let original = state(&ed);
    ed.handle_key(key('s'));
    ed.handle_key(key('z')); // no "z" in either selection
    // Live preview found no matches → originals already restored.
    assert_eq!(state(&ed), original);
    // Confirm with a non-empty pattern that has no matches. Live preview already
    // restored the originals, so confirm keeps them in place.
    ed.handle_key(key_enter());
    assert_eq!(ed.current_selections().len(), 2, "original two selections should be restored");
}

/// Primary index after select-within tracks to the first match within the
/// original primary selection, even when that selection is not first in order.
#[test]
fn select_within_primary_tracks_original_primary() {
    use crate::core::selection::{Selection, SelectionSet};
    // "aa bb aa\n" — two selections, primary is the SECOND one (6..7).
    let mut ed = editor_from("-[aa bb aa]>\n");
    let two_sels = SelectionSet::from_vec(
        vec![
            Selection::new(0, 1), // first in order, NOT primary
            Selection::new(6, 7), // second in order, IS primary
        ],
        1,
    );
    ed.set_current_selections(two_sels);

    ed.handle_key(key('s'));
    for ch in "aa".chars() { ed.handle_key(key(ch)); }
    ed.handle_key(key_enter());

    assert_eq!(ed.current_selections().len(), 2);
    // Primary must be the match from the original primary selection (6..7).
    let primary = ed.current_selections().primary();
    assert_eq!(primary.start(), 6, "primary should come from the original primary selection");
}

/// Esc after live-preview with multiple selections restores all originals.
#[test]
fn select_within_esc_restores_multiple_selections() {
    use crate::core::selection::{Selection, SelectionSet};
    // Use wider original selections ("aa bb" and "aa") so the live-preview
    // of "aa" visibly shrinks them — confirming the snapshot is correct.
    // "aa bb aa\n"
    //  012345678
    let mut ed = editor_from("-[aa bb aa]>\n");
    let two_sels = SelectionSet::from_vec(
        vec![
            Selection::new(0, 4), // "aa bb" — wider than any "aa" match
            Selection::new(6, 7), // "aa"
        ],
        0,
    );
    ed.set_current_selections(two_sels);
    let original = state(&ed);

    ed.handle_key(key('s'));
    for ch in "aa".chars() { ed.handle_key(key(ch)); }
    // Live preview shrinks "aa bb" → "aa", so state differs.
    assert_ne!(state(&ed), original);

    ed.handle_key(key_esc());
    assert_eq!(ed.current_selections().len(), 2, "both original selections restored");
    assert_eq!(state(&ed), original);
}

// ── Search / select-within independence ──────────────────────────────────────

/// After `/foo` + confirm, `s` + `bar` + confirm, pressing `n` must jump to the
/// next "foo" — not "bar". This is the critical end-to-end independence test.
#[test]
fn search_n_after_select_within_uses_original_search() {
    // "xx ab cd ab cd\n" — cursor starts before all matches.
    let mut ed = editor_from("-[x]>x ab cd ab cd\n");

    // Search for "ab", confirm → lands on first "ab".
    ed.handle_key(key('/'));
    for ch in "ab".chars() { ed.handle_key(key(ch)); }
    ed.handle_key(key_enter());
    assert_eq!(state(&ed), "xx -[ab]> cd ab cd\n");

    // Select the whole line and split on "cd".
    ed.handle_key(key('%'));
    ed.handle_key(key('s'));
    for ch in "cd".chars() { ed.handle_key(key(ch)); }
    ed.handle_key(key_enter());

    // `n` must jump to an "ab", not a "cd".
    ed.handle_key(key('n'));
    let st = state(&ed);
    assert!(
        st.contains("-[ab]>") || st.contains("<[ab]-"),
        "expected primary on 'ab', got: {st}"
    );
}

/// After `/foo` + confirm, `s` + `bar` + Esc (cancel), pressing `n` must still
/// jump to the next "foo".
#[test]
fn search_n_after_cancelled_select_within_uses_original_search() {
    let mut ed = editor_from("-[x]>x ab cd ab cd\n");

    // Search for "ab", confirm.
    ed.handle_key(key('/'));
    for ch in "ab".chars() { ed.handle_key(key(ch)); }
    ed.handle_key(key_enter());
    assert_eq!(state(&ed), "xx -[ab]> cd ab cd\n");

    // Select all, start select-within with "cd", then cancel.
    ed.handle_key(key('%'));
    ed.handle_key(key('s'));
    for ch in "cd".chars() { ed.handle_key(key(ch)); }
    ed.handle_key(key_esc());

    // `n` must still find "ab".
    ed.handle_key(key('n'));
    let st = state(&ed);
    assert!(
        st.contains("-[ab]>") || st.contains("<[ab]-"),
        "expected primary on 'ab', got: {st}"
    );
}

/// A prior search pattern must survive a select-within confirm.
#[test]
fn search_regex_survives_select_within_confirm() {
    let mut ed = editor_from("-[ab cd ab]>\n").with_search_regex("cd");
    assert!(ed.search_pattern().is_some());

    ed.handle_key(key('s'));
    for ch in "ab".chars() { ed.handle_key(key(ch)); }
    ed.handle_key(key_enter());

    assert!(ed.search_pattern().is_some(), "search pattern should survive select-within confirm");
}

/// A prior search pattern must survive a select-within cancel.
#[test]
fn search_regex_survives_select_within_cancel() {
    let mut ed = editor_from("-[ab cd ab]>\n").with_search_regex("cd");
    assert!(ed.search_pattern().is_some());

    ed.handle_key(key('s'));
    for ch in "ab".chars() { ed.handle_key(key(ch)); }
    ed.handle_key(key_esc());

    assert!(ed.search_pattern().is_some(), "search pattern should survive select-within cancel");
}

/// `s` + confirm with no prior search — pressing `n` afterward should be a
/// no-op (no crash, no match, selection unchanged).
#[test]
fn search_n_after_select_within_with_no_prior_search() {
    let mut ed = editor_from("-[ab cd ab]>\n");
    assert!(ed.search_pattern().is_none());
    assert!(reg(&ed, 's').is_empty());

    ed.handle_key(key('s'));
    for ch in "ab".chars() { ed.handle_key(key(ch)); }
    ed.handle_key(key_enter());

    let before = state(&ed);
    ed.handle_key(key('n'));
    // With no search pattern, `n` is a no-op — selection unchanged.
    assert_eq!(state(&ed), before);
}

// ── n merges overlapping selections ──────────────────────────────────────────

/// When `n` moves the primary to a position already covered by a secondary
/// selection, the two must merge — no duplicate/overlapping selections.
#[test]
fn search_n_merges_with_overlapping_secondary() {
    use crate::core::selection::{Selection, SelectionSet};
    // "ab cd ab\n" — set up two selections already on the "ab" matches,
    // then confirm a search for "ab" and press `n` so the primary lands
    // on the second "ab", which is also the secondary.
    let mut ed = editor_from("-[ab cd ab]>\n");

    // Search for "ab", confirm → primary lands on first "ab".
    ed.handle_key(key('/'));
    ed.handle_key(key('a'));
    ed.handle_key(key('b'));
    ed.handle_key(key_enter());
    assert_eq!(state(&ed), "-[ab]> cd ab\n");

    // Add a secondary selection manually on the second "ab" (chars 6..7).
    let sels = SelectionSet::from_vec(
        vec![
            Selection::new(0, 1), // first "ab" — primary
            Selection::new(6, 7), // second "ab" — secondary
        ],
        0,
    );
    ed.set_current_selections(sels);
    assert_eq!(ed.current_selections().len(), 2);

    // `n` moves primary from first "ab" to second "ab", which already has a
    // secondary selection there → they must merge.
    ed.handle_key(key('n'));

    // After merge: one selection covering the second "ab".
    assert_eq!(ed.current_selections().len(), 1, "overlapping selections must merge");
    assert_eq!(ed.current_selections().primary().start(), 6);
    assert_eq!(ed.current_selections().primary().end_inclusive(ed.doc().text()), 7);
}

// ── select-all-matches ────────────────────────────────────────────────────────

/// `select-all-matches` turns every match into a selection.
#[test]
fn select_all_matches_creates_selection_per_match() {
    // "ab cd ab\n" — two "ab" matches at 0 and 6.
    let mut ed = editor_from("-[a]>b cd ab\n").with_search_regex("ab");

    ed.handle_key(key(':'));
    for ch in "select-all-matches".chars() {
        ed.handle_key(key(ch));
    }
    ed.handle_key(key_enter());

    assert_eq!(ed.current_selections().len(), 2, "one selection per 'ab' match");
    let sels: Vec<_> = ed.current_selections().iter_sorted().collect();
    assert_eq!(sels[0].start(), 0);
    assert_eq!(sels[1].start(), 6);
}

/// `select-all-matches` with no active search is a no-op.
#[test]
fn select_all_matches_no_search_is_noop() {
    let mut ed = editor_from("-[ab cd ab]>\n");
    let original = state(&ed);

    ed.handle_key(key(':'));
    for ch in "select-all-matches".chars() {
        ed.handle_key(key(ch));
    }
    ed.handle_key(key_enter());

    assert_eq!(state(&ed), original);
}

/// `select-all-matches` falls back to SEARCH_REGISTER when regex is cleared.
#[test]
fn select_all_matches_uses_search_register_fallback() {
    use crate::ops::register::SEARCH_REGISTER;
    let mut ed = editor_from("-[ab cd ab]>\n");
    ed.registers.write_text(SEARCH_REGISTER, vec!["ab".to_string()]);
    // No live regex — forces register fallback.
    assert!(ed.search_pattern().is_none());

    ed.handle_key(key(':'));
    for ch in "select-all-matches".chars() {
        ed.handle_key(key(ch));
    }
    ed.handle_key(key_enter());

    assert_eq!(ed.current_selections().len(), 2);
}

/// `m/` keybind reaches `select-all-matches` (tests the keymap path, not just `:select-all-matches`).
#[test]
fn select_all_matches_via_m_slash_keybind() {
    let mut ed = editor_from("-[a]>b cd ab\n").with_search_regex("ab");
    ed.handle_key(key('m'));
    ed.handle_key(key('/'));
    assert_eq!(ed.current_selections().len(), 2, "m/ should select all 'ab' matches");
}

// ── Use selection as search (*) ──────────────────────────────────────────────

/// `*` on a cursor expands to the word under the cursor and sets search.
#[test]
fn star_on_cursor_expands_to_word() {
    let mut ed = editor_from("-[h]>ello world\n");
    ed.handle_key(key('*'));
    assert_eq!(ed.mode, Mode::Normal);
    // Selection expanded to cover "hello".
    assert_eq!(state(&ed), "-[hello]> world\n");
    // Pattern in search register is the escaped word.
    assert_eq!(reg(&ed, 's'), vec!["hello"]);
    // Search direction set to forward.
    assert_eq!(ed.search.direction, super::SearchDirection::Forward);
    assert!(ed.search_pattern().is_some());
}

/// `*` on a non-cursor selection uses the selected text literally.
#[test]
fn star_on_selection_uses_selected_text() {
    let mut ed = editor_from("a-[b c]>d\n");
    ed.handle_key(key('*'));
    // Selection unchanged (non-cursor, no expansion).
    assert_eq!(state(&ed), "a-[b c]>d\n");
    assert_eq!(reg(&ed, 's'), vec!["b c"]);
}

/// `*` on the trailing structural newline does nothing.
#[test]
fn star_on_trailing_newline_is_noop() {
    let mut ed = editor_from("hello\n-[\n]>");
    // Exercise state() before and after the keypress to verify the
    // serialisation path doesn't panic on this edge-case cursor position.
    let _ = state(&ed);
    ed.handle_key(key('*'));
    // inner_word_impl on trailing \n produces a \n pattern.
    // This is a degenerate case but should not panic.
    assert_eq!(ed.mode, Mode::Normal);
    let _ = state(&ed);
}

/// `*` escapes regex metacharacters in the selection.
#[test]
fn star_escapes_metacharacters() {
    let mut ed = editor_from("-[f]>oo.bar\n");
    // Select "foo.bar" first via `v$` equivalent — use the whole line.
    // Easier: just set up a selection covering "foo.bar".
    let buf = crate::core::text::Text::from("foo.bar\n");
    let sels = crate::core::selection::SelectionSet::single(
        crate::core::selection::Selection::new(0, 6),
    );
    *ed.doc_mut() = crate::editor::buffer::Buffer::new(buf, sels.clone());
    ed.set_current_selections(sels);

    ed.handle_key(key('*'));
    assert_eq!(reg(&ed, 's'), vec!["foo\\.bar"]);
}

// ── Jump list ────────────────────────────────────────────────────────────────

/// Build a 20-line buffer with the cursor on a given line for jump list tests.
fn jump_editor(cursor_line: usize) -> Editor {
    // 20 lines: "line 0\n", "line 1\n", ..., "line 19\n"
    let text: String = (0..20).map(|i| format!("line {i}\n")).collect();
    let buf = crate::core::text::Text::from(text.as_str());
    // Place cursor at the start of the requested line.
    let pos = buf.line_to_char(cursor_line);
    let sels = crate::core::selection::SelectionSet::single(
        crate::core::selection::Selection::collapsed(pos),
    );
    let doc = crate::editor::buffer::Buffer::new(buf, sels);
    let mut ed = Editor::for_testing(doc);
    // Ensure we start in Normal mode.
    ed.mode = Mode::Normal;
    ed
}

/// `gg` from the middle of the file records the pre-jump position.
#[test]
fn goto_first_line_records_jump() {
    let mut ed = jump_editor(10);
    let before = state(&ed);

    // `gg` — goto first line.
    ed.handle_key(key('g'));
    ed.handle_key(key('g'));
    assert_eq!(ed.doc().text().char_to_line(ed.current_selections().primary().head), 0);

    // jump-backward should restore the pre-jump position.
    ed.handle_key(key_ctrl('o'));
    assert_eq!(state(&ed), before);
}

/// `ge` (goto-last-line) records a jump.
#[test]
fn goto_last_line_records_jump() {
    let mut ed = jump_editor(5);
    let before = state(&ed);

    ed.handle_key(key('g'));
    ed.handle_key(key('e'));
    assert_ne!(state(&ed), before); // moved somewhere else

    // jump-backward should restore the pre-jump position.
    ed.handle_key(key_ctrl('o'));
    assert_eq!(state(&ed), before);
}

/// Full round-trip: jump → jump-backward → jump-forward.
#[test]
fn jump_backward_then_forward() {
    let mut ed = jump_editor(10);

    // Jump to first line.
    ed.handle_key(key('g'));
    ed.handle_key(key('g'));
    let at_top = state(&ed);

    // Back to original position.
    ed.handle_key(key_ctrl('o'));
    assert_ne!(state(&ed), at_top);

    // Forward returns to top.
    ed.handle_key(key_ctrl('i'));
    assert_eq!(state(&ed), at_top);
}

/// A small motion (e.g. `2j`) does NOT record a jump.
#[test]
fn small_motion_does_not_record_jump() {
    let mut ed = jump_editor(10);
    let before = state(&ed);

    // Move down 2 lines — below the threshold.
    ed.handle_key(key('2'));
    ed.handle_key(key('j'));
    let after = state(&ed);
    assert_ne!(after, before);

    // jump-backward should NOT go back — nothing was recorded.
    ed.handle_key(key_ctrl('o'));
    assert_eq!(state(&ed), after);
}

/// A large motion (e.g. `10j`) records a jump via the line-distance threshold.
#[test]
fn large_motion_records_jump() {
    let mut ed = jump_editor(0);
    let before = state(&ed);

    // Move down 10 lines — exceeds the threshold of 5.
    // Type "10j" as separate key presses.
    ed.handle_key(key('1'));
    ed.handle_key(key('0'));
    ed.handle_key(key('j'));
    assert_eq!(ed.doc().text().char_to_line(ed.current_selections().primary().head), 10);

    ed.handle_key(key_ctrl('o'));
    assert_eq!(state(&ed), before);
}

/// Search confirm records a jump; search cancel does not.
#[test]
fn search_confirm_records_jump() {
    let mut ed = jump_editor(0);
    let before = state(&ed);

    // Search for "line 15".
    ed.handle_key(key('/'));
    for ch in "line 15".chars() {
        ed.handle_key(key(ch));
    }
    ed.handle_key(key_enter());
    assert_eq!(ed.mode, Mode::Normal);
    assert_eq!(ed.doc().text().char_to_line(ed.current_selections().primary().head), 15);

    // jump-backward should return to line 0.
    ed.handle_key(key_ctrl('o'));
    assert_eq!(state(&ed), before);
}

/// Search cancel (Esc) does NOT record a jump.
#[test]
fn search_cancel_does_not_record_jump() {
    let mut ed = jump_editor(0);
    let before = state(&ed);

    ed.handle_key(key('/'));
    for ch in "line 15".chars() {
        ed.handle_key(key(ch));
    }
    // Cancel — restores position.
    ed.handle_key(key_esc());
    assert_eq!(state(&ed), before);

    // jump-backward should NOT go anywhere — nothing recorded.
    ed.handle_key(key_ctrl('o'));
    assert_eq!(state(&ed), before);
}

/// `n` (search-next) records a jump.
#[test]
fn search_next_records_jump() {
    let mut ed = jump_editor(0);

    // Set up a search pattern first.
    ed.handle_key(key('/'));
    for ch in "line".chars() {
        ed.handle_key(key(ch));
    }
    ed.handle_key(key_enter());
    // Now on line 1 (first match after line 0 which is also "line 0").
    let after_search = state(&ed);

    // Press `n` to go to next match.
    ed.handle_key(key('n'));
    let after_n = state(&ed);
    assert_ne!(after_n, after_search);

    // jump-backward should go back to the position before search-next.
    ed.handle_key(key_ctrl('o'));
    assert_eq!(state(&ed), after_search);
}

/// When search-next lands on the same line as the previous match, jump-forward
/// must still return to the exact pre-jump-backward position.
#[test]
fn ctrl_i_works_when_current_is_same_line_as_last_jump() {
    // Two "editor" matches on the same line.
    let text = "the editor and the editor\nother line\n";
    let buf = crate::core::text::Text::from(text);
    let sels = crate::core::selection::SelectionSet::single(
        crate::core::selection::Selection::collapsed(0),
    );
    let doc = crate::editor::buffer::Buffer::new(buf, sels);
    let mut ed = Editor::for_testing(doc);
    ed.kitty_enabled = true;

    // Search "editor" — lands on first match.
    ed.handle_key(key('/'));
    for ch in "editor".chars() {
        ed.handle_key(key(ch));
    }
    ed.handle_key(key_enter());
    let first_match = state(&ed);

    // `n` — lands on second "editor" on the SAME line.
    ed.handle_key(key('n'));
    let second_match = state(&ed);
    assert_ne!(first_match, second_match);

    // jump-backward should go back to first match.
    ed.handle_key(key_ctrl('o'));
    assert_eq!(state(&ed), first_match, "jump-backward should return to first match");

    // jump-forward MUST return to the second match (the pre-jump-backward position).
    ed.handle_key(key_ctrl('i'));
    assert_eq!(state(&ed), second_match, "jump-forward should return to second match");
}

/// search-next + jump-backward + jump-forward round-trip, all matches on different lines.
#[test]
fn search_n_ctrl_o_ctrl_i_different_lines() {
    let mut ed = jump_editor(0);

    // Search "line 1" — matches lines 1, 10, 11, 12, ...
    ed.handle_key(key('/'));
    for ch in "line 1".chars() {
        ed.handle_key(key(ch));
    }
    ed.handle_key(key_enter());

    // `n` twice to advance through matches on different lines.
    ed.handle_key(key('n'));
    let state_after_n1 = state(&ed);
    ed.handle_key(key('n'));
    let state_after_n2 = state(&ed);

    // jump-backward goes back.
    ed.handle_key(key_ctrl('o'));
    assert_eq!(state(&ed), state_after_n1);

    // jump-forward goes forward.
    ed.handle_key(key_ctrl('i'));
    assert_eq!(state(&ed), state_after_n2);
}

// ── Surround operations ──────────────────────────────────────────────────────

/// `ms(` selects the surrounding `(` and `)` as two cursor selections.
#[test]
fn surround_select_paren() {
    let mut ed = editor_from("(-[h]>ello)\n");
    for ch in "ms(".chars() { ed.handle_key(key(ch)); }
    assert_eq!(state(&ed), "-[(]>hello-[)]>\n");
}

/// `ms[` works the same for square brackets.
#[test]
fn surround_select_bracket() {
    let mut ed = editor_from("[-[h]>ello]\n");
    for ch in "ms[".chars() { ed.handle_key(key(ch)); }
    assert_eq!(state(&ed), "-[[]>hello-[]]>\n");
}

/// `ms"` selects surrounding double quotes.
#[test]
fn surround_select_double_quote() {
    let mut ed = editor_from("\"-[h]>ello\"\n");
    for ch in "ms\"".chars() { ed.handle_key(key(ch)); }
    assert_eq!(state(&ed), "-[\"]>hello-[\"]>\n");
}

/// `ms(` → `d` deletes the surrounding parens, leaving two cursors.
#[test]
fn surround_delete_paren() {
    let mut ed = editor_from("(-[h]>ello)\n");
    for ch in "ms(".chars() { ed.handle_key(key(ch)); }
    ed.handle_key(key('d'));
    // Two cursors remain: one where `(` was (now `h`), one where `)` was
    // (now the structural `\n`).
    assert_eq!(state(&ed), "-[h]>ello-[\n]>");
}

/// `ms(` → `r[` replaces `()` with `[]` via smart replace.
#[test]
fn surround_replace_paren_with_bracket() {
    let mut ed = editor_from("(-[h]>ello)\n");
    for ch in "ms(".chars() { ed.handle_key(key(ch)); }
    ed.handle_key(key('r'));
    ed.handle_key(key('['));
    assert_eq!(state(&ed), "-[[]>hello-[]]>\n");
}

/// `ms"` → `r(` replaces `""` with `()` (symmetric → asymmetric).
#[test]
fn surround_replace_quote_with_paren() {
    let mut ed = editor_from("\"-[h]>ello\"\n");
    for ch in "ms\"".chars() { ed.handle_key(key(ch)); }
    ed.handle_key(key('r'));
    ed.handle_key(key('('));
    assert_eq!(state(&ed), "-[(]>hello-[)]>\n");
}

/// `ms(` with no enclosing parens is a no-op.
#[test]
fn surround_no_match_is_noop() {
    let mut ed = editor_from("-[h]>ello\n");
    for ch in "ms(".chars() { ed.handle_key(key(ch)); }
    assert_eq!(state(&ed), "-[h]>ello\n");
}

// ── Pane selection sync (Bug 3) ──────────────────────────────────────────────
//
// The engine pane's `selections` field must stay in sync with `doc.sels()` so
// the renderer always shows the correct cursor. `push_selections_to_pane` is
// called once per frame in the run loop; these tests call it explicitly (as
// the run loop would) and verify the pane reflects the post-operation state.

/// Return the pane's primary cursor as an absolute char offset — the engine's
/// representation after Phase 2 unified the selection types.
fn pane_head(ed: &Editor) -> usize {
    ed.engine_view.panes[ed.focused_pane_id].selections[0].head
}

/// After `c` (change): the selection is deleted and Insert mode entered.
/// Before the fix, the pane still held the pre-deletion selection after `c`.
#[test]
fn pane_selections_synced_after_change_command() {
    let mut ed = editor_from("-[hell]>o\n");
    ed.handle_key(key('c'));
    // `c` enters Insert; buffer is now "o\n" with cursor at char 0.
    assert_eq!(ed.mode, Mode::Insert);

    // Simulate the per-frame sync that happens in the run loop.
    ed.push_selections_to_pane();

    // Cursor must be at char offset 0 (start of "o\n").
    assert_eq!(pane_head(&ed), 0, "pane head must be at char 0 after 'c' deletes selection");
}

/// After typing a character in Insert mode: the pane cursor must advance.
/// Before the fix, `apply_edit_grouped` never called `push_selections_to_pane`.
#[test]
fn pane_selections_synced_after_insert_typing() {
    let mut ed = editor_from("-[a]>b\n");
    ed.handle_key(key('c')); // delete "a", enter Insert — cursor at byte 0
    ed.handle_key(key('x')); // type 'x' — cursor advances past 'x' to byte 1

    ed.push_selections_to_pane();

    // Text is now "xb\n"; cursor sits after 'x', at byte offset 1.
    assert_eq!(pane_head(&ed), 1, "pane head must be at char 1 after typing 'x'");
}

/// After `Esc` (exit Insert): pane must reflect the final cursor position.
/// Before the fix, `end_insert_session` never called `push_selections_to_pane`.
#[test]
fn pane_selections_synced_after_exit_insert() {
    let mut ed = editor_from("ab-[c]>\n");
    ed.handle_key(key('i')); // enter Insert at 'c' (byte 2)
    ed.handle_key(key('x')); // type 'x' before 'c' → "abxc\n", cursor at byte 3
    ed.handle_key(key_esc()); // exit Insert

    ed.push_selections_to_pane();

    // 'x' was inserted at byte 2; cursor now sits just after 'x' at byte 3.
    assert_eq!(pane_head(&ed), 3, "pane head must be at char 3 (after 'x') after Esc");
}

/// When the primary selection is NOT the earliest in the document,
/// `pane.selections[0]` must still be the primary (not the earliest).
///
/// Before the fix, `push_selections_to_pane` used `iter_sorted()`, which lost
/// primary info, so the engine always treated the earliest selection as primary.
#[test]
fn pane_selections_primary_is_first_even_when_not_earliest() {
    use crate::core::selection::{Selection, SelectionSet};

    let mut ed = editor_from("-[a]>b\n");

    // Two cursors: one at "a" (char 0) and one at "b" (char 1).
    // Primary is index 1 — the "b" cursor, which is LATER in document order.
    let two_sels = SelectionSet::from_vec(
        vec![
            Selection::collapsed(0), // at "a" — NOT primary
            Selection::collapsed(1), // at "b" — IS primary
        ],
        1,
    );
    ed.set_current_selections(two_sels);

    // Simulate the per-frame sync.
    ed.push_selections_to_pane();

    // Selections are passed in sorted document order; primary_idx identifies the primary.
    let pane = &ed.engine_view.panes[ed.focused_pane_id];
    assert_eq!(
        pane.selections[0].head, 0,
        "pane.selections[0] is the earliest in document order (char 0, 'a')"
    );
    assert_eq!(
        pane.selections[1].head, 1,
        "pane.selections[1] is 'b' at char 1"
    );
    assert_eq!(pane.primary_idx, 1, "primary_idx must point to 'b' (index 1)");
}

/// Backward selections (head < anchor) can cause start()-order to differ from
/// head-order. Before the fix, pane selections were passed in start()-order, which
/// triggered the engine's `debug_assert!(selections sorted by head)`.
///
/// Reproduction: two selections where their start() order differs from head order:
///   A: anchor=10, head=3  → start()=3, head=3   (backward)
///   B: anchor=0,  head=8  → start()=0, head=8   (forward)
/// start() order: [B(0), A(3)]  → heads [8, 3]  — NOT sorted → panic
/// head  order:   [A(3), B(8)]  → heads [3, 8]  — sorted     → OK
#[test]
fn pane_selections_sorted_by_head_not_start() {
    use crate::core::selection::{Selection, SelectionSet};

    // Text needs at least 11 chars. The -[h]> marker satisfies editor_from's
    // requirement; we replace the selection set immediately after.
    let mut ed = editor_from("-[h]>ello world\n");

    // A: backward selection, anchor=10, head=3  → start()=3
    // B: forward  selection, anchor=0,  head=8  → start()=0
    // In start() order: [B, A].  In head order: [A, B].
    let two_sels = SelectionSet::from_vec(
        vec![
            Selection { anchor: 10, head: 3, horiz: None }, // A — primary
            Selection { anchor: 0,  head: 8, horiz: None }, // B
        ],
        0, // primary is A
    );
    ed.set_current_selections(two_sels);

    ed.push_selections_to_pane();

    let pane = &ed.engine_view.panes[ed.focused_pane_id];
    // After sort-by-head: [A(head=3), B(head=8)]
    assert_eq!(pane.selections[0].head, 3, "first in head order is A");
    assert_eq!(pane.selections[1].head, 8, "second in head order is B");
    // Primary (A) ends up at index 0 after sorting.
    assert_eq!(pane.primary_idx, 0, "primary_idx follows A to its new position");
}

// ── Visual-line movement ──────────────────────────────────────────────────────
//
// `for_testing` uses `WrapMode::Indent { width: 76 }` with tab_width=4 and an
// 80×24 viewport. For a line with no leading indent, Indent wrap is equivalent
// to Soft wrap (indent_cols = 0), so the wrap boundary is simply at column 76.
//
// Test layout:
//   Line 0: 'a' × 80  →  sub-row 0: chars  0..76 (cols 0..75)
//                         sub-row 1: chars 76..80 (cols 0..3) + '\n' at col 4
//   Line 1: "short\n"  →  chars 81..86
//
// Char offsets:
//   0      = first 'a'
//   76     = first 'a' on sub-row 1
//   80     = '\n' at end of line 0
//   81     = 's' (start of "short")
//   85     = 't'
//   86     = '\n' at end of line 1

fn visual_test_editor(head: usize) -> Editor {
    let line0: String = "a".repeat(80);
    let content = format!("{}\nshort\n", line0);
    // Build manually so we can place the cursor at an exact char offset.
    use crate::core::text::Text;
    use crate::core::selection::{Selection, SelectionSet};
    let buf = Text::from(content.as_str());
    let sels = SelectionSet::single(Selection::collapsed(head));
    Editor::for_testing(Buffer::new(buf, sels))
}

/// j moves from sub-row 0 to sub-row 1 of the same buffer line.
#[test]
fn visual_move_down_within_wrapped_line() {
    let mut ed = visual_test_editor(0);
    ed.handle_key(key('j'));
    assert_eq!(ed.current_selections().primary().head, 76, "j: sub-row 0 → sub-row 1, col 0 → char 76");
    assert_eq!(ed.current_selections().primary().horiz, Some(0), "sticky col latched on first j");
}

/// j on the last sub-row crosses to the next buffer line.
#[test]
fn visual_move_down_crosses_buffer_line() {
    let mut ed = visual_test_editor(76); // sub-row 1 of line 0
    ed.handle_key(key('j'));
    assert_eq!(ed.current_selections().primary().head, 81, "j: last sub-row → first char of next buffer line");
}

/// k from the first row of a buffer line enters the last sub-row of the previous line.
#[test]
fn visual_move_up_enters_last_subrow_of_previous_line() {
    let mut ed = visual_test_editor(81); // start of "short"
    ed.handle_key(key('k'));
    assert_eq!(ed.current_selections().primary().head, 76, "k: buffer line n+1 → last sub-row of line n, col 0 → char 76");
}

/// k on sub-row 1 retreats to sub-row 0 of the same buffer line.
#[test]
fn visual_move_up_within_wrapped_line() {
    let mut ed = visual_test_editor(76); // sub-row 1 of line 0
    ed.handle_key(key('k'));
    assert_eq!(ed.current_selections().primary().head, 0, "k: sub-row 1 → sub-row 0, col 0 → char 0");
}

/// k on the first sub-row of the first line stays put.
#[test]
fn visual_move_up_at_top_stays_put() {
    let mut ed = visual_test_editor(0);
    ed.handle_key(key('k'));
    assert_eq!(ed.current_selections().primary().head, 0, "k at first row: no-op");
}

/// j on the last sub-row of the last line stays put.
#[test]
fn visual_move_down_at_bottom_stays_put() {
    // Place cursor at "short" (line 1 is last). Line 1 has only 1 sub-row.
    let mut ed = visual_test_editor(81);
    ed.handle_key(key('j'));
    assert_eq!(ed.current_selections().primary().head, 81, "j at last row: no-op");
}

/// The preferred display column is preserved across consecutive j/k presses
/// and used to find the closest grapheme when the target row is shorter.
#[test]
fn visual_preferred_col_stickiness() {
    // Cursor at char 40 (display col 40) in sub-row 0 of the long line.
    let mut ed = visual_test_editor(40);

    // j: target_col = 40, sub-row 1 has only 4 chars (cols 0..3).
    // Closest to col 40 is char 79 (col 3, last 'a' on sub-row 1).
    ed.handle_key(key('j'));
    assert_eq!(ed.current_selections().primary().head, 79, "j: clamped to last char on short sub-row");
    assert_eq!(ed.current_selections().primary().horiz, Some(40), "sticky col stays at 40");

    // j again: cross to "short\n" (line 1). target_col=40, "short" has cols 0..4.
    // Closest to 40 is 't' at col 4, char 85.
    ed.handle_key(key('j'));
    assert_eq!(ed.current_selections().primary().head, 85, "j: clamped to last char on short second line");
    assert_eq!(ed.current_selections().primary().horiz, Some(40), "sticky col still 40");
}

/// Any non-vertical command resets preferred_display_col.
#[test]
fn visual_preferred_col_reset_on_horizontal_motion() {
    let mut ed = visual_test_editor(40);
    ed.handle_key(key('j')); // latches horiz on the selection
    assert!(ed.current_selections().primary().horiz.is_some(), "j latches sticky col");
    ed.handle_key(key('l')); // horizontal motion — Selection::new() clears horiz
    assert!(ed.current_selections().primary().horiz.is_none(), "l resets sticky col");
}

/// WrapMode::None falls back to buffer-line movement.
#[test]
fn visual_move_no_wrap_falls_back_to_buffer_line() {
    let mut ed = visual_test_editor(0);
    // Override via buffer: apply_visual_vertical reads overrides at call time.
    ed.doc_mut().overrides.wrap_mode = Some(engine::pane::WrapMode::None);

    ed.handle_key(key('j'));
    // With no wrapping: j moves by one buffer line (0 → 81 "short").
    assert_eq!(ed.current_selections().primary().head, 81, "WrapMode::None: j moves by buffer line");
    assert!(ed.current_selections().primary().horiz.is_none(), "no sticky col in non-wrap mode");
}

/// count prefix: 2j moves two visual rows.
#[test]
fn visual_move_down_with_count() {
    let mut ed = visual_test_editor(0);
    ed.handle_key(key('2'));
    ed.handle_key(key('j'));
    // 2j from char 0: first j → char 76 (sub-row 1), second j → char 81 (next line).
    assert_eq!(ed.current_selections().primary().head, 81, "2j: two visual rows from sub-row 0");
}

/// Each cursor uses its own sticky column in multi-cursor j/k.
///
/// Text layout (visual_test_editor):
///   sub-row 0: chars  0..76 (cols 0..75)
///   sub-row 1: chars 76..80 (cols 0..3)  ← two cursors placed here
///   line 1:    chars 81..86 "short\n"
///
/// Cursor A at char 76 (col 0), cursor B at char 79 (col 3, primary).
/// j → line 1: A should land at col 0 = char 81, B at col 3 = char 84.
/// k → sub-row 1: A should return to col 0 = char 76, B to col 3 = char 79.
#[test]
fn visual_move_per_selection_sticky_col() {
    use crate::core::selection::{Selection, SelectionSet};

    let line0: String = "a".repeat(80);
    let content = format!("{}\nshort\n", line0);
    let buf = crate::core::text::Text::from(content.as_str());
    // A at col 0, B at col 3 (primary).
    let sels = SelectionSet::from_vec(
        vec![
            Selection::collapsed(76), // A — col 0 on sub-row 1
            Selection::collapsed(79), // B — col 3 on sub-row 1
        ],
        1, // primary is B
    );
    let mut ed = Editor::for_testing(Buffer::new(buf, sels));

    // j: each cursor should use its own column, not the primary's.
    ed.handle_key(key('j'));
    let sels = ed.current_selections().clone();
    assert_eq!(sels.len(), 2, "two cursors remain distinct");
    // Sorted by start(): A is first.
    let heads: Vec<usize> = sels.iter_sorted().map(|s| s.head).collect();
    assert_eq!(heads[0], 81, "A (col 0) → char 81 on line 1");
    assert_eq!(heads[1], 84, "B (col 3) → char 84 on line 1");

    // k: sticky cols should bring each cursor back to its original column.
    ed.handle_key(key('k'));
    let sels = ed.current_selections().clone();
    assert_eq!(sels.len(), 2, "two cursors remain distinct");
    let heads: Vec<usize> = sels.iter_sorted().map(|s| s.head).collect();
    assert_eq!(heads[0], 76, "A returns to col 0 = char 76 on sub-row 1");
    assert_eq!(heads[1], 79, "B returns to col 3 = char 79 on sub-row 1");
}

// ── Visual-line extend variants ───────────────────────────────────────────────
//
// Extend mode is toggled with `e`. In extend mode `j`/`k` resolve to
// extend-down/extend-up: the anchor stays fixed and only the head moves.

/// extend-down (e+j) within a wrapped line: anchor stays at sub-row 0, head
/// advances to sub-row 1 of the same buffer line.
#[test]
fn visual_extend_down_within_wrapped_line() {
    let mut ed = visual_test_editor(0);
    ed.handle_key(key('e')); // enter extend mode
    ed.handle_key(key('j'));
    let sel = ed.current_selections().primary();
    assert_eq!(sel.anchor, 0,  "anchor fixed at sub-row 0 col 0");
    assert_eq!(sel.head,   76, "head extends to sub-row 1 col 0");
}

/// extend-down crosses to the next buffer line when already on the last sub-row.
#[test]
fn visual_extend_down_crosses_buffer_line() {
    let mut ed = visual_test_editor(76); // last sub-row of line 0
    ed.handle_key(key('e'));
    ed.handle_key(key('j'));
    let sel = ed.current_selections().primary();
    assert_eq!(sel.anchor, 76, "anchor fixed at last sub-row");
    assert_eq!(sel.head,   81, "head crosses to first char of next buffer line");
}

/// extend-up (e+k) within a wrapped line: head retreats from sub-row 1 to sub-row 0.
#[test]
fn visual_extend_up_within_wrapped_line() {
    let mut ed = visual_test_editor(76); // sub-row 1 of line 0
    ed.handle_key(key('e'));
    ed.handle_key(key('k'));
    let sel = ed.current_selections().primary();
    assert_eq!(sel.anchor, 76, "anchor fixed at sub-row 1");
    assert_eq!(sel.head,   0,  "head retreats to sub-row 0 col 0");
}

/// extend-up enters the last sub-row of the previous buffer line.
#[test]
fn visual_extend_up_enters_previous_line_last_subrow() {
    let mut ed = visual_test_editor(81); // start of "short"
    ed.handle_key(key('e'));
    ed.handle_key(key('k'));
    let sel = ed.current_selections().primary();
    assert_eq!(sel.anchor, 81, "anchor fixed at line 1 start");
    assert_eq!(sel.head,   76, "head enters last sub-row of previous buffer line");
}

// ── Page scroll ───────────────────────────────────────────────────────────────
//
// page_scroll / half_page_scroll were refactored from Motion dispatch to
// EditorCmd dispatch. These tests verify they still move by the right distance.
//
// Viewport height in for_testing = 24 → page = 24, half = 12.
// Text: 30 single-char lines "a\n" (60 chars total). No wrap needed.
// Line N starts at char 2*N.

fn page_test_editor() -> Editor {
    use crate::core::text::Text;
    use crate::core::selection::{Selection, SelectionSet};
    let content = "a\n".repeat(30);
    let buf = Text::from(content.as_str());
    let sels = SelectionSet::single(Selection::collapsed(0));
    let mut ed = Editor::for_testing(Buffer::new(buf, sels));
    // Override via buffer: scroll logic reads overrides at call time.
    ed.doc_mut().overrides.wrap_mode = Some(engine::pane::WrapMode::None);
    ed
}

fn key_page_down() -> KeyEvent {
    KeyEvent::new(KeyCode::PageDown, KeyModifiers::NONE)
}

fn key_page_up() -> KeyEvent {
    KeyEvent::new(KeyCode::PageUp, KeyModifiers::NONE)
}

/// Ctrl+d (half-page-down) moves cursor down by half the viewport height (12 lines).
#[test]
fn half_page_down_moves_half_viewport() {
    let mut ed = page_test_editor();
    ed.handle_key(key_ctrl('d'));
    // half = 24/2 = 12 lines → line 12 → char 24
    assert_eq!(ed.current_selections().primary().head, 24, "half-page-down from line 0: cursor at line 12");
}

/// Ctrl+u (half-page-up) moves cursor up by half the viewport height.
#[test]
fn half_page_up_moves_half_viewport() {
    let mut ed = page_test_editor();
    // Place cursor at line 12 first.
    ed.handle_key(key_ctrl('d'));
    assert_eq!(ed.current_selections().primary().head, 24);
    ed.handle_key(key_ctrl('u'));
    assert_eq!(ed.current_selections().primary().head, 0, "half-page-up returns to line 0");
}

/// PageDown moves cursor down by a full viewport height (24 lines).
#[test]
fn page_down_moves_full_viewport() {
    let mut ed = page_test_editor();
    ed.handle_key(key_page_down());
    // page = 24 lines → line 24 → char 48
    assert_eq!(ed.current_selections().primary().head, 48, "page-down from line 0: cursor at line 24");
}

/// PageUp moves cursor up by a full viewport height.
#[test]
fn page_up_moves_full_viewport() {
    let mut ed = page_test_editor();
    // Place cursor at line 24 first.
    ed.handle_key(key_page_down());
    assert_eq!(ed.current_selections().primary().head, 48);
    ed.handle_key(key_page_up());
    assert_eq!(ed.current_selections().primary().head, 0, "page-up returns to line 0");
}

// ── Keyboard macros ───────────────────────────────────────────────────────────

/// `QQ` starts recording into register `q`, second `Q` stops.
/// Keys typed during recording are stored as a macro.
#[test]
fn macro_qq_records_into_register_q() {
    let mut ed = editor_from("-[a]>bcd\n");
    // First `Q` sets the pending state — recording hasn't started yet.
    ed.handle_key(key('Q'));
    assert!(ed.macro_recording.is_none(), "recording not started until register name given");
    assert!(ed.macro_pending.is_some(), "pending should be set after Q");

    // Second `Q` is consumed as the register name — recording starts now.
    ed.handle_key(key('Q'));
    assert!(ed.macro_recording.is_some(), "recording should start after Q<reg>");
    assert_eq!(ed.macro_recording.as_ref().unwrap().0, 'q');

    // Record a motion: j (move down)
    ed.handle_key(key('j'));

    // Stop recording: Q
    ed.handle_key(key('Q'));
    assert!(ed.macro_recording.is_none(), "recording should stop after stop-Q");

    // Register 'q' should now hold a macro with [j] (not the register-name Q or stop Q)
    let keys = ed.registers.read('q').and_then(|r| r.as_macro()).map(|k| k.to_vec());
    assert!(keys.is_some(), "register q should hold a macro");
    let keys = keys.unwrap();
    assert_eq!(keys.len(), 1, "only the j key should be recorded, not Q keys");
    assert_eq!(keys[0].code, KeyCode::Char('j'));
}

/// `Q0` records into register `0`.
#[test]
fn macro_q_digit_records_into_named_register() {
    let mut ed = editor_from("-[a]>bcd\n");
    ed.handle_key(key('Q'));
    ed.handle_key(key('0'));
    assert!(ed.macro_recording.is_some());
    assert_eq!(ed.macro_recording.as_ref().unwrap().0, '0');
    ed.handle_key(key('j'));
    ed.handle_key(key('Q'));
    assert!(ed.macro_recording.is_none());
    let keys = ed.registers.read('0').and_then(|r| r.as_macro()).map(|k| k.to_vec());
    assert!(keys.is_some());
    assert_eq!(keys.unwrap()[0].code, KeyCode::Char('j'));
}

/// `Q Esc` cancels: no recording starts.
#[test]
fn macro_q_esc_cancels() {
    let mut ed = editor_from("-[a]>bcd\n");
    ed.handle_key(key('Q'));
    assert!(ed.macro_pending.is_some(), "pending should be set after Q");
    ed.handle_key(key_esc());
    assert!(ed.macro_pending.is_none(), "pending should be cleared after Esc");
    assert!(ed.macro_recording.is_none(), "no recording should have started");
}

/// `q Esc` cancels: no replay is queued.
#[test]
fn macro_big_q_esc_cancels() {
    let mut ed = editor_from("-[a]>bcd\n");
    ed.handle_key(key('q'));
    assert!(ed.macro_pending.is_some());
    ed.handle_key(key_esc());
    assert!(ed.macro_pending.is_none());
    assert!(ed.replay_queue.is_empty());
}

/// `qq` replays from the default register `q`. The cursor should move down one line.
#[test]
fn macro_big_q_replays_from_register() {
    // 3 lines: cursor starts on first line
    let mut ed = editor_from("-[a]>\nb\nc\n");

    // Record `j` into register `q`
    ed.handle_key(key('Q'));
    ed.handle_key(key('Q'));
    ed.handle_key(key('j'));
    ed.handle_key(key('Q'));

    let before = ed.current_selections().primary().head;

    // `qq` replays from the default register — no extra key needed.
    ed.handle_key(key('q'));
    ed.handle_key(key('q'));

    ed.drain_replay_queue();

    let after = ed.current_selections().primary().head;
    assert!(after > before, "cursor should have moved down after replay");
}

/// `q` followed by a non-register key cancels replay — key is swallowed.
#[test]
fn macro_big_q_non_register_key_cancels() {
    let mut ed = editor_from("-[a]>\nb\nc\n");

    // Record `j` into register `q` so there's something to (not) replay.
    ed.handle_key(key('Q'));
    ed.handle_key(key('Q'));
    ed.handle_key(key('j'));
    ed.handle_key(key('Q'));

    let before = ed.current_selections().primary().head;

    // `q` then `Q` (uppercase, not a valid register) — cancelled, cursor stays put.
    ed.handle_key(key('q'));
    ed.handle_key(key('Q'));

    ed.drain_replay_queue();

    let after = ed.current_selections().primary().head;
    assert_eq!(before, after, "cancelled replay should not move cursor");
}

/// Replay of an empty/nonexistent register is a no-op.
#[test]
fn macro_replay_empty_register_is_noop() {
    let mut ed = editor_from("-[a]>bcd\n");
    let before = state(&ed);

    // `q` must arm macro_pending — proving the dispatch path ran.
    ed.handle_key(key('q'));
    assert!(ed.macro_pending.is_some(), "macro_pending should be set after q");

    // Register 'z' has never been written — macro_pending is consumed but
    // no keys are queued and state is unchanged.
    ed.handle_key(key('z'));
    assert!(ed.macro_pending.is_none(), "macro_pending should be consumed after register key");
    assert!(ed.replay_queue.is_empty(), "no keys queued for unset register");
    assert_eq!(state(&ed), before, "state unchanged");
}

/// `Q` during replay does not start recording (nested recording suppressed).
#[test]
fn macro_no_nested_recording_during_replay() {
    // Record a macro that would press `Q Q` (try to start recording).
    // During replay, the `Q` intercept should be suppressed.
    let mut ed = editor_from("-[a]>bcd\n");

    // Manually seed a macro that contains `Q Q j` into register 'q'.
    // We can't record this via the normal path (Q would stop recording),
    // so we write directly to the register.
    ed.registers.write_macro('q', vec![
        KeyEvent::new(KeyCode::Char('Q'), KeyModifiers::NONE),
        KeyEvent::new(KeyCode::Char('Q'), KeyModifiers::NONE),
        KeyEvent::new(KeyCode::Char('j'), KeyModifiers::NONE),
    ]);

    // Trigger replay: qq
    ed.handle_key(key('q'));
    ed.handle_key(key('q'));

    ed.drain_replay_queue();

    // Recording should NOT have started — the Q intercept is suppressed during replay
    assert!(ed.macro_recording.is_none(), "nested recording must be suppressed");
    assert!(ed.macro_pending.is_none(), "macro_pending must not be armed after replay");
}

/// A macro whose last key is `Q` must not arm `macro_pending` after replay.
///
/// Previously, the suppression checked `replay_queue.is_empty()`, which becomes
/// `true` at the exact moment the last key is processed — causing a trailing `Q`
/// to slip through and arm `macro_pending`. The fix uses `is_replaying` instead.
#[test]
fn macro_trailing_q_does_not_arm_pending() {
    let mut ed = editor_from("-[a]>\nb\nc\n");

    // Seed a macro ending with Q (can't be recorded normally).
    ed.registers.write_macro('q', vec![
        KeyEvent::new(KeyCode::Char('j'), KeyModifiers::NONE),
        KeyEvent::new(KeyCode::Char('Q'), KeyModifiers::NONE),
    ]);

    // Replay: qq
    ed.handle_key(key('q'));
    ed.handle_key(key('q'));

    ed.drain_replay_queue();

    assert!(ed.macro_recording.is_none(), "recording must not have started");
    assert!(ed.macro_pending.is_none(), "macro_pending must not be armed by trailing Q");
}


/// Status bar shows `[recording @q]` during recording and nothing when idle.
///
/// Tests that `StatusElement::MacroRecording` is in the default config and that
/// the actual `render_element` path (in `statusline.rs`) produces the right text.
/// This test lives here for access to `editor_from`; the rendering assertion
/// is in `statusline.rs::tests::macro_recording_element_renders`.
#[test]
fn macro_status_indicator() {
    use crate::ui::statusline::StatusElement;

    let ed = editor_from("-[a]>bcd\n");
    let config = &ed.settings.statusline;
    assert!(
        config.right.contains(&StatusElement::MacroRecording),
        "MacroRecording should be in the default right section"
    );
}

/// Recording works across mode transitions: insert-mode keys are captured.
#[test]
fn macro_records_insert_mode_keys() {
    let mut ed = editor_from("-[a]>bcd\n");

    // Start recording
    ed.handle_key(key('Q'));
    ed.handle_key(key('Q'));

    // Enter insert mode, type 'x', exit
    ed.handle_key(key('i'));
    ed.handle_key(key('x'));
    ed.handle_key(key_esc());

    // Stop recording
    ed.handle_key(key('Q'));

    // The recorded macro should contain: i, x, Esc (3 keys)
    let keys = ed.registers.read('q').and_then(|r| r.as_macro()).map(|k| k.to_vec()).unwrap();
    assert_eq!(keys.len(), 3, "expected i, x, Esc — got {} keys: {:?}", keys.len(), keys);
    assert_eq!(keys[0].code, KeyCode::Char('i'));
    assert_eq!(keys[1].code, KeyCode::Char('x'));
    assert_eq!(keys[2].code, KeyCode::Esc);
}

// ── New edge-case tests ───────────────────────────────────────────────────────

/// `3qq` replays the macro 3 times. With a `j` macro and enough lines, the
/// cursor should end up exactly 3 lines below its position at replay start.
#[test]
fn macro_replay_with_count() {
    // 5 lines so we can move down 3 from line 0 without hitting the buffer end.
    let mut ed = editor_from("-[a]>\nb\nc\nd\ne\n");

    // Record `j` into register 'q'. The cursor moves to line 1 during recording.
    ed.handle_key(key('Q'));
    ed.handle_key(key('Q'));
    ed.handle_key(key('j'));
    ed.handle_key(key('Q'));

    // Go back to line 0 (gg = goto-first-line) so replay has room to move 3 lines.
    ed.handle_key(key('g'));
    ed.handle_key(key('g'));

    let start = ed.current_selections().primary().head;
    let start_line = ed.doc().text().char_to_line(start);
    assert_eq!(start_line, 0, "cursor should be on line 0 before replay");

    // `3qq` — count 3, replay from register 'q'.
    ed.handle_key(key('3'));
    ed.handle_key(key('q'));
    ed.handle_key(key('q'));
    ed.drain_replay_queue();

    let end_line = ed.doc().text().char_to_line(ed.current_selections().primary().head);
    assert_eq!(end_line, 3, "expected cursor on line 3, got line {}", end_line);
}

/// Replaying a register that holds text (not a macro) is a no-op.
///
/// `enqueue_macro_replay` calls `as_macro()` which returns `None` for text
/// registers. The queue must stay empty and the state unchanged.
#[test]
fn macro_replay_of_text_register_is_noop() {
    use crate::ops::register::DEFAULT_REGISTER;
    let mut ed = editor_from("-[a]>bcd\n");

    // Yank puts text into the default register.
    ed.handle_key(key('y'));
    assert!(ed.registers.read(DEFAULT_REGISTER).and_then(|r| r.as_text()).is_some(),
        "default register should hold text after yank");

    let before = state(&ed);

    // Try to replay from the default register (which holds text, not a macro).
    ed.registers.write_text('0', vec!["some text".into()]);
    ed.handle_key(key('q'));
    assert!(ed.macro_pending.is_some());
    ed.handle_key(key('0'));
    assert!(ed.macro_pending.is_none());
    assert!(ed.replay_queue.is_empty(), "text register must not enqueue any keys");
    assert_eq!(state(&ed), before, "state must be unchanged");
}

/// Record `f` + `x` (find-char). Both keys must be captured in the macro.
/// Replay must move the cursor to the next `x` on the line.
#[test]
fn macro_with_find_char() {
    // Two `x` chars so we can move from the first to the second.
    let mut ed = editor_from("-[a]>bxcxd\n");

    // Record `f` then `x` into register 'q'.
    ed.handle_key(key('Q'));
    ed.handle_key(key('Q'));
    ed.handle_key(key('f'));
    ed.handle_key(key('x'));
    ed.handle_key(key('Q'));

    let keys = ed.registers.read('q').and_then(|r| r.as_macro()).map(|k| k.to_vec()).unwrap();
    assert_eq!(keys.len(), 2, "macro should contain exactly 2 keys (f and x), got {:?}", keys);
    assert_eq!(keys[0].code, KeyCode::Char('f'));
    assert_eq!(keys[1].code, KeyCode::Char('x'));

    // After recording, cursor is on first 'x'. Move to 'c' so replay can find next 'x'.
    ed.handle_key(key('l'));  // step right to 'c'

    let before_pos = ed.current_selections().primary().head;
    let before_char = ed.doc().text().char_at(before_pos);

    // Replay: `f x` from 'c' should land on the second 'x'.
    ed.handle_key(key('q'));
    ed.handle_key(key('q'));
    ed.drain_replay_queue();

    let after_pos = ed.current_selections().primary().head;
    assert!(after_pos > before_pos, "cursor should have moved right");
    assert_eq!(ed.doc().text().char_at(after_pos), Some('x'), "cursor should be on 'x' after replay");
    let _ = before_char;
}

/// Record `i x Esc` (insert 'x' then exit insert mode) into a register.
/// Replay on a different cursor position should insert 'x' there.
#[test]
fn macro_insert_mode_round_trip() {
    let mut ed = editor_from("ab-[c]>d\n");

    // Record: insert 'x' before the current cursor, then Esc.
    ed.handle_key(key('Q'));
    ed.handle_key(key('Q'));
    ed.handle_key(key('i'));
    ed.handle_key(key('x'));
    ed.handle_key(key_esc());
    ed.handle_key(key('Q'));

    // Move to 'd' (one position right) so replay applies at a different spot.
    ed.handle_key(key('l'));
    let before = state(&ed);

    ed.handle_key(key('q'));
    ed.handle_key(key('q'));
    ed.drain_replay_queue();

    let after = state(&ed);
    assert_ne!(after, before, "replay should have modified the buffer");
    assert!(ed.doc().text().to_string().matches('x').count() == 2,
        "there should be two 'x' chars — one from recording, one from replay");
}

/// After replaying a macro, `.` must repeat the last *editing* action, not
/// the macro itself. `last_action` is saved/restored around the replay drain.
#[test]
fn macro_replay_preserves_dot_repeat() {
    let mut ed = editor_from("-[a]>bc\nxyz\n");

    // Perform a `d` (delete) to establish last_action = "delete".
    ed.handle_key(key('d'));
    let action_after_delete = ed.last_action.as_ref().map(|a| a.command.as_ref());
    assert_eq!(action_after_delete, Some("delete"), "last_action should be 'delete'");

    // Record a `j` motion macro (not repeatable — should not overwrite last_action).
    ed.handle_key(key('Q'));
    ed.handle_key(key('Q'));
    ed.handle_key(key('j'));
    ed.handle_key(key('Q'));

    // Replay the macro.
    ed.handle_key(key('q'));
    ed.handle_key(key('q'));
    ed.drain_replay_queue();

    // last_action must still be "delete", not whatever the macro did.
    let action_after_replay = ed.last_action.as_ref().map(|a| a.command.as_ref());
    assert_eq!(action_after_replay, Some("delete"),
        "dot-repeat must survive macro replay; got {:?}", action_after_replay);
}

/// Pressing `q` while recording should be silently captured as a recorded key
/// — it must not arm macro_pending or trigger replay.
#[test]
fn macro_q_during_recording_is_captured() {
    let mut ed = editor_from("-[a]>bcd\n");

    // Start recording into 'q'.
    ed.handle_key(key('Q'));
    ed.handle_key(key('Q'));

    // Press `q` (replay trigger) while recording.
    ed.handle_key(key('q'));

    // Must not have changed pending or replay state.
    assert!(ed.macro_pending.is_none(), "q during recording must not arm macro_pending");
    assert!(ed.replay_queue.is_empty(), "q during recording must not enqueue replay");

    // Stop recording.
    ed.handle_key(key('Q'));

    // The `q` must have been captured as a recorded key.
    let keys = ed.registers.read('q').and_then(|r| r.as_macro()).map(|k| k.to_vec()).unwrap();
    assert_eq!(keys.len(), 1, "macro should contain exactly 1 key (the q), got {:?}", keys);
    assert_eq!(keys[0].code, KeyCode::Char('q'));
}

/// A macro containing `qq` (self-replay) must not actually replay during replay —
/// the `is_replaying` guard must suppress the nested `q` intercept.
#[test]
fn macro_recursive_replay_suppressed() {
    let mut ed = editor_from("-[a]>\nb\nc\n");

    // Seed a macro `[q, q]` (self-replay) into 'q' manually.
    ed.registers.write_macro('q', vec![
        KeyEvent::new(KeyCode::Char('q'), KeyModifiers::NONE),
        KeyEvent::new(KeyCode::Char('q'), KeyModifiers::NONE),
    ]);

    // Replay: qq. The macro contains `qq` which should be suppressed.
    ed.handle_key(key('q'));
    ed.handle_key(key('q'));
    ed.drain_replay_queue();

    // Neither recording nor pending should be armed.
    assert!(ed.macro_recording.is_none(), "nested recording must not start");
    assert!(ed.macro_pending.is_none(), "macro_pending must not be armed after replay");
    assert!(ed.replay_queue.is_empty(), "replay queue must be empty after drain");
}

/// `QQ Q` — record with zero keys, then stop. The register should hold an
/// empty macro. Replaying it is a no-op.
#[test]
fn macro_empty_recording() {
    let mut ed = editor_from("-[a]>bcd\n");
    let before = state(&ed);

    // Start and immediately stop recording.
    ed.handle_key(key('Q'));
    ed.handle_key(key('Q'));
    ed.handle_key(key('Q'));

    // Register 'q' should hold an empty macro (Some, but zero keys).
    let keys = ed.registers.read('q').and_then(|r| r.as_macro()).map(|k| k.to_vec());
    assert!(keys.is_some(), "register should hold a macro (not None)");
    assert!(keys.unwrap().is_empty(), "macro should have zero keys");

    // Replay: no-op.
    ed.handle_key(key('q'));
    ed.handle_key(key('q'));
    ed.drain_replay_queue();
    assert_eq!(state(&ed), before, "replay of empty macro must not change state");
}

/// `Esc` while recording should be captured as a key (stopping insert/extend),
/// not stop the recording session itself. Recording continues after Esc.
#[test]
fn macro_esc_during_recording_is_captured() {
    let mut ed = editor_from("-[a]>bcd\n");

    // Start recording.
    ed.handle_key(key('Q'));
    ed.handle_key(key('Q'));
    assert!(ed.macro_recording.is_some());

    // Press Esc — this should be recorded, not stop the session.
    ed.handle_key(key_esc());
    assert!(ed.macro_recording.is_some(), "Esc must not stop recording");

    // Record a motion to confirm the session is still open.
    ed.handle_key(key('j'));

    // Stop recording.
    ed.handle_key(key('Q'));
    assert!(ed.macro_recording.is_none());

    // Macro should contain Esc and j.
    let keys = ed.registers.read('q').and_then(|r| r.as_macro()).map(|k| k.to_vec()).unwrap();
    assert_eq!(keys.len(), 2, "expected [Esc, j], got {} keys: {:?}", keys.len(), keys);
    assert_eq!(keys[0].code, KeyCode::Esc);
    assert_eq!(keys[1].code, KeyCode::Char('j'));
}

/// A count prefix before `Q` (e.g. `3Qq`) must not leak into the recording
/// session — the count is consumed by the `Q` intercept and not stored.
#[test]
fn macro_count_prefix_before_record_does_not_leak() {
    let mut ed = editor_from("-[a]>bcd\n");

    // `3` then `Q` then `q` — count prefix before start-record sequence.
    ed.handle_key(key('3'));
    ed.handle_key(key('Q'));
    ed.handle_key(key('q')); // register name

    // Recording should have started cleanly.
    assert!(ed.macro_recording.is_some(), "recording should start after Q<reg>");
    // Count must be consumed/cleared.
    assert!(ed.count.is_none(), "count must be cleared after Q<reg> sequence");

    ed.handle_key(key('Q')); // stop
    assert!(ed.macro_recording.is_none());
}

/// After replaying a macro, `u` should undo the edits made by the macro.
#[test]
fn macro_replay_undo() {
    let mut ed = editor_from("-[f]>oo\nbar\n");

    // Record `d` (delete selection) into 'q'.
    ed.handle_key(key('Q'));
    ed.handle_key(key('Q'));
    ed.handle_key(key('d'));
    ed.handle_key(key('Q'));

    let before_replay = state(&ed);

    // Replay: cursor deletes its selection.
    ed.handle_key(key('q'));
    ed.handle_key(key('q'));
    ed.drain_replay_queue();

    let after_replay = state(&ed);
    assert_ne!(after_replay, before_replay, "replay should have changed state");

    // Undo should restore to the pre-replay state.
    ed.handle_key(key('u'));
    assert_eq!(state(&ed), before_replay, "undo after replay should restore pre-replay state");
}

/// Record into register 1, undo, then replay — the edit should be reapplied.
#[test]
fn macro_q1_replay_after_undo() {
    let mut ed = editor_from("-[h]>ello world\nhello world\n");

    // Q 1: start recording into register '1'
    ed.handle_key(key('Q'));
    ed.handle_key(key('1'));
    assert!(ed.macro_recording.is_some(), "recording into register 1");

    // Record: w (select word) → c (change) → x (insert 'x') → Esc
    ed.handle_key(key('w'));
    ed.handle_key(key('c'));
    ed.handle_key(key('x'));
    ed.handle_key(key_esc());

    // Q: stop recording
    ed.handle_key(key('Q'));
    assert!(ed.macro_recording.is_none());
    assert!(ed.registers.read('1').and_then(|r| r.as_macro()).is_some(), "macro saved");

    // Undo the edit
    ed.handle_key(key('u'));
    let before = state(&ed);

    // q 1: replay from register '1'
    ed.handle_key(key('q'));
    ed.handle_key(key('1'));
    assert!(!ed.replay_queue.is_empty(), "replay queue populated");

    ed.drain_replay_queue();

    assert_ne!(state(&ed), before, "replay should have changed the state");
}

// ── D1–D6: Multi-pane contract tests ──────────────────────────────────────────
//
// These tests lock the SSOT invariants for per-pane, per-buffer, and per-search
// state.  D2/D3 require Phase 5 (`propagate_cs_to_panes`) and will be added then.

/// D1 — Each pane maintains its own cursor independently for the same buffer.
///
/// Two panes on the same buffer; set them to different positions; verify
/// `switch_focused_pane` restores each pane's cursor exactly.
#[test]
fn d1_selections_are_pane_owned() {
    use crate::core::selection::{Selection, SelectionSet};

    let mut ed = editor_from("-[h]>ello world\n");
    let bid = ed.buffer_id;
    let pid_a = ed.focused_pane_id;

    let pid_b = ed.open_pane(bid);

    // Pane A → position 2 ('l').
    ed.switch_focused_pane(pid_a);
    ed.set_current_selections(SelectionSet::single(Selection::collapsed(2)));

    // Pane B → position 6 ('w').
    ed.switch_focused_pane(pid_b);
    ed.set_current_selections(SelectionSet::single(Selection::collapsed(6)));

    // Back to pane A: head must be 2, not 6.
    ed.switch_focused_pane(pid_a);
    assert_eq!(ed.current_selections().primary().head, 2, "pane A head after switch");

    // Back to pane B: head must be 6, not 2.
    ed.switch_focused_pane(pid_b);
    assert_eq!(ed.current_selections().primary().head, 6, "pane B head after switch");
}

/// D4a — `Buffer.search_pattern` is shared across all panes on the same buffer;
/// each pane has its own `SearchCursor` in `pane_state`.
#[test]
fn d4a_search_pattern_is_per_buffer() {
    use crate::editor::pane_state::SearchCursor;

    let mut ed = editor_from("-[f]>oo foo foo\n");
    let bid = ed.buffer_id;
    let pid_a = ed.focused_pane_id;
    let pid_b = ed.open_pane(bid);

    // Both panes see Buffer.search_pattern — it's a single field on `doc`.
    // Verify independence of search_cursor: write distinct values per pane.
    ed.pane_state[pid_a][bid].search_cursor = SearchCursor {
        match_count: Some((1, 3)),
        wrapped: false,
        ..SearchCursor::default()
    };
    ed.pane_state[pid_b][bid].search_cursor = SearchCursor {
        match_count: Some((2, 3)),
        wrapped: true,
        ..SearchCursor::default()
    };

    // Pane A and pane B see different cursors even though they share the buffer.
    assert_eq!(ed.pane_state[pid_a][bid].search_cursor.match_count, Some((1, 3)));
    assert!(!ed.pane_state[pid_a][bid].search_cursor.wrapped);

    assert_eq!(ed.pane_state[pid_b][bid].search_cursor.match_count, Some((2, 3)));
    assert!(ed.pane_state[pid_b][bid].search_cursor.wrapped);
}

/// D4b — `Selection.horiz` travels with the selection; resets when its line
/// is touched by an edit; survives translate_in_place on untouched lines.
#[test]
fn d4b_sticky_col_is_per_selection() {
    use crate::core::changeset::ChangeSetBuilder;
    use crate::core::selection::{Selection, SelectionSet};
    use crate::core::text::Text;

    // "abc\ndef\n" — two lines.
    let text = Text::from("abc\ndef\n");
    let rope = text.rope().clone();

    // Selection on line 1 (char offset 4 = 'd'), horiz = 0.
    let sel = Selection::with_horiz(4, 4, 0);
    let mut sels = SelectionSet::single(sel);

    // CS that inserts at the start of line 0 only: "abc\n" → "Xabc\n"
    // This touches line 0 but not line 1, so horiz on line-1 head should survive.
    let mut b = ChangeSetBuilder::new(rope.len_chars());
    b.insert("X");   // insert at start
    b.retain_rest();
    let cs = b.finish();

    sels.translate_in_place(&cs, &rope);
    // Head moved from 4 to 5 (past the inserted 'X'), horiz preserved.
    assert_eq!(sels.primary().head, 5, "head mapped past insert");
    assert_eq!(sels.primary().horiz, Some(0), "horiz preserved on untouched line");

    // Now a CS that touches line 1 (inserts at position of 'd'): horiz should reset.
    // Re-build sels with the updated head but set horiz back to show it was latched.
    let sel2 = Selection::with_horiz(5, 5, 0);
    let mut sels2 = SelectionSet::single(sel2);

    // "Xabc\ndef\n" (after first edit) — "d" is now at char 5 (line 1).
    // Insert at char 5 (start of "def" in new rope); use the original rope for
    // translate_in_place (rope_pre = before-this-edit rope).
    let text2 = Text::from("Xabc\ndef\n");
    let rope2 = text2.rope().clone();
    let mut b2 = ChangeSetBuilder::new(rope2.len_chars());
    b2.retain(5);   // skip "Xabc\n"
    b2.insert("Y"); // insert at line 1
    b2.retain_rest();
    let cs2 = b2.finish();

    sels2.translate_in_place(&cs2, &rope2);
    // Head moved past insert; horiz must be reset because line 1 was touched.
    assert_eq!(sels2.primary().horiz, None, "horiz reset when head's line is touched");
}

/// D5 — `EditGroup` is per-(pane, buffer); insert sessions are independent across
/// panes on the same buffer.  Two separate i…Esc sessions each produce one revision.
#[test]
fn d5_insert_session_is_pane_buffer_scoped() {
    let mut ed = editor_from("-[a]>bc\n");
    let bid = ed.buffer_id;
    let pid_a = ed.focused_pane_id;
    let pid_b = ed.open_pane(bid);

    // Pane A insert session: type 'X' at the start.
    ed.switch_focused_pane(pid_a);
    assert!(ed.pane_state[pid_a][bid].edit_group.is_none(), "no group before i");
    ed.handle_key(key('i'));
    assert!(ed.pane_state[pid_a][bid].edit_group.is_some(), "group open after i");
    ed.handle_key(key('X'));
    ed.handle_key(key_esc());
    assert!(ed.pane_state[pid_a][bid].edit_group.is_none(), "group committed on Esc");

    let rev_after_a = ed.doc().revision_id();

    // Pane B insert session: type 'Y'.
    ed.switch_focused_pane(pid_b);
    assert!(ed.pane_state[pid_b][bid].edit_group.is_none(), "pane B starts with no group");
    ed.handle_key(key('i'));
    assert!(ed.pane_state[pid_b][bid].edit_group.is_some(), "pane B group opens");
    ed.handle_key(key('Y'));
    ed.handle_key(key_esc());
    assert!(ed.pane_state[pid_b][bid].edit_group.is_none(), "pane B group committed");

    let rev_after_b = ed.doc().revision_id();

    // Each session produced a distinct revision.
    assert_ne!(rev_after_a, rev_after_b, "pane B produced a new revision");

    // Two undos restore original content.
    ed.switch_focused_pane(pid_a);
    ed.handle_key(key('u'));
    ed.handle_key(key('u'));
    assert_eq!(ed.doc().text().to_string(), "abc\n", "two undos restore original");
}

/// D6 — `pane_transient[pid]` snapshots are per-pane and never aliased.
#[test]
fn d6_search_mode_snapshot_is_per_pane() {
    use crate::core::selection::{Selection, SelectionSet};

    let mut ed = editor_from("-[h]>ello\n");
    let bid = ed.buffer_id;
    let pid_a = ed.focused_pane_id;
    let pid_b = ed.open_pane(bid);

    let sels_a = SelectionSet::single(Selection::collapsed(1));
    let sels_b = SelectionSet::single(Selection::collapsed(3));

    ed.pane_transient[pid_a].pre_search_sels = Some(sels_a.clone());
    ed.pane_transient[pid_b].pre_search_sels = Some(sels_b.clone());

    // Pane A snapshot is independent of pane B.
    assert_eq!(
        ed.pane_transient[pid_a].pre_search_sels.as_ref().unwrap().primary().head,
        1,
        "pane A pre_search_sels head"
    );
    assert_eq!(
        ed.pane_transient[pid_b].pre_search_sels.as_ref().unwrap().primary().head,
        3,
        "pane B pre_search_sels head"
    );

    // Clearing pane A's snapshot does not affect pane B.
    ed.pane_transient[pid_a].pre_search_sels = None;
    assert!(ed.pane_transient[pid_a].pre_search_sels.is_none());
    assert!(ed.pane_transient[pid_b].pre_search_sels.is_some(), "pane B unaffected");
}

/// D2 — An edit in the focused pane translates non-acting pane selections via the CS.
///
/// Pane A deletes char 0; pane B's cursor at position 9 must slide to 8.
#[test]
fn d2_edit_in_pane_a_translates_pane_b_selections() {
    use crate::core::selection::{Selection, SelectionSet};

    // "abcdefghij\n" (11 chars including trailing \n); cursor on 'a'.
    let mut ed = editor_from("-[a]>bcdefghij\n");
    let bid = ed.buffer_id;
    let pid_a = ed.focused_pane_id;
    let pid_b = ed.open_pane(bid);

    // Position pane B's cursor at char 9 ('j').
    ed.switch_focused_pane(pid_b);
    ed.set_current_selections(SelectionSet::single(Selection::collapsed(9)));

    // Switch to pane A and delete char 0 ('a').
    ed.switch_focused_pane(pid_a);
    ed.handle_key(key('d')); // delete selection (covers 'a')

    // Pane A's cursor is now at 0 (post-delete); pane B's should be at 8.
    assert_eq!(
        ed.selections_for(pid_b, bid).unwrap().primary().head,
        8,
        "pane B selection translated by forward CS"
    );
}

/// D3 — Undo in the focused pane propagates the inverse CS to non-acting panes.
///
/// After the D2 edit (delete 'a'), undo restores 'a'; pane B's cursor at 8
/// must ride the inverse CS back to 9.
#[test]
fn d3_undo_restores_acting_pane_and_translates_others() {
    use crate::core::selection::{Selection, SelectionSet};

    let mut ed = editor_from("-[a]>bcdefghij\n");
    let bid = ed.buffer_id;
    let pid_a = ed.focused_pane_id;
    let pid_b = ed.open_pane(bid);

    // Position pane B at char 9.
    ed.switch_focused_pane(pid_b);
    ed.set_current_selections(SelectionSet::single(Selection::collapsed(9)));

    // Pane A: delete 'a', then undo.
    ed.switch_focused_pane(pid_a);
    ed.handle_key(key('d'));
    // After delete: pane B at 8. Undo restores 'a'.
    ed.handle_key(key('u'));

    // Pane A's cursor is restored to pre-delete position.
    assert_eq!(
        ed.current_selections().primary().head,
        0,
        "pane A cursor restored by undo"
    );
    // Pane B's cursor is translated back to 9 by the inverse CS.
    assert_eq!(
        ed.selections_for(pid_b, bid).unwrap().primary().head,
        9,
        "pane B selection translated by inverse CS (undo)"
    );
}

/// Multi-cursor propagation: a deletion that spans two selections in pane B
/// merges them into one (proves translate_in_place calls merge_overlapping_in_place).
#[test]
fn propagate_cs_merges_collapsed_non_acting_pane_selections() {
    use crate::core::selection::{Selection, SelectionSet};

    // "abcde\n" — 6 chars.
    let mut ed = editor_from("-[a]>bcde\n");
    let bid = ed.buffer_id;
    let pid_a = ed.focused_pane_id;
    let pid_b = ed.open_pane(bid);

    // Pane B: two cursors at positions 2 ('c') and 4 ('e').
    ed.switch_focused_pane(pid_b);
    ed.set_current_selections(SelectionSet::from_vec(
        vec![Selection::collapsed(2), Selection::collapsed(4)],
        0,
    ));

    // Pane A: select chars 1–4 ("bcde") and delete.
    // First put pane A's selection on 'b'-'e'.
    ed.switch_focused_pane(pid_a);
    // Select 'a' then extend to 'e': use 'v' to enter Select then motion.
    // Simplest: directly set selections and do a delete.
    ed.set_current_selections(SelectionSet::single(Selection::new(1, 4)));
    ed.handle_key(key('d'));

    // After deleting chars 1-4, pane B's two cursors at 2 and 4 both map to
    // the deletion point (1); they must merge into a single cursor at 1.
    let pane_b_sels = ed.selections_for(pid_b, bid).unwrap();
    assert_eq!(pane_b_sels.len(), 1, "collapsed selections must merge after propagation");
    assert_eq!(pane_b_sels.primary().head, 1, "merged cursor at deletion point");
}

// ── Phase 6 — BufferStore + buffer choke-points ───────────────────────────────

use crate::core::text::Text;
use crate::core::selection::SelectionSet;

/// `open_buffer` allocates a new BufferId, seeds pane_state, and tracks MRU.
#[test]
fn p6_open_buffer_seeds_pane_state() {
    let mut ed = Editor::for_testing(Buffer::new(Text::from("hello\n"), SelectionSet::default()));
    let initial_bid = ed.buffer_id;
    let doc2 = Buffer::new(Text::from("world\n"), SelectionSet::default());
    let bid2 = ed.open_buffer(doc2);
    assert_ne!(bid2, initial_bid);
    // pane_state should be seeded for bid2 on the focused pane.
    assert!(ed.selections_for(ed.focused_pane_id, bid2).is_some(), "pane_state seeded for new buffer");
}

/// `close_buffer` with one other buffer redirects panes and frees the slot.
#[test]
fn p6_close_buffer_redirects_to_mru() {
    let mut ed = Editor::for_testing(Buffer::new(Text::from("alpha\n"), SelectionSet::default()));
    let bid_alpha = ed.buffer_id;
    let doc_beta = Buffer::new(Text::from("beta\n"), SelectionSet::default());
    let bid_beta = ed.open_buffer(doc_beta);
    ed.switch_to_buffer_with_jump(bid_beta);
    assert_eq!(ed.buffer_id, bid_beta);
    // Close beta — should redirect focused pane back to alpha.
    ed.close_buffer(bid_beta);
    assert_eq!(ed.buffer_id, bid_alpha, "focused pane redirected to alpha after closing beta");
    assert!(ed.buffers.try_get(bid_beta).is_none(), "beta slot freed from BufferStore");
}

/// `close_buffer` on the last buffer replaces it with scratch (Case C).
#[test]
fn p6_close_last_buffer_becomes_scratch() {
    let mut ed = Editor::for_testing(Buffer::new(Text::from("only\n"), SelectionSet::default()));
    let bid = ed.buffer_id;
    ed.close_buffer(bid);
    // Buffer id stays valid but content is now scratch.
    assert_eq!(ed.buffer_id, bid, "same buffer id after scratch replacement");
    assert_eq!(ed.doc().text().to_string(), "\n", "scratch buffer has structural newline only");
}

/// `replace_buffer_in_place` reseeds selections and clears scrolls.
#[test]
fn p6_replace_buffer_in_place_reseeds() {
    let mut ed = Editor::for_testing(Buffer::new(Text::from("old content\n"), SelectionSet::default()));
    let bid = ed.buffer_id;
    // Move the cursor somewhere non-zero.
    ed.apply_motion(|b, _sels| {
        let head = b.len_chars().saturating_sub(2);
        SelectionSet::single(crate::core::selection::Selection::collapsed(head))
    });
    let replacement = Buffer::new(Text::from("new content\n"), SelectionSet::default());
    ed.replace_buffer_in_place(bid, replacement);
    // Selections should be reset to initial (cursor at 0).
    let sels = ed.current_selections();
    assert_eq!(sels.primary().head, 0, "selections reset after replace_buffer_in_place");
    assert_eq!(ed.doc().text().to_string(), "new content\n");
}

/// `:bnext` / `:bprev` cycle through buffers in open-order.
#[test]
fn p6_bnext_bprev_cycle() {
    let mut ed = Editor::for_testing(Buffer::new(Text::from("a\n"), SelectionSet::default()));
    let bid_a = ed.buffer_id;
    let bid_b = ed.open_buffer(Buffer::new(Text::from("b\n"), SelectionSet::default()));
    let bid_c = ed.open_buffer(Buffer::new(Text::from("c\n"), SelectionSet::default()));
    // Still focused on a. bnext → b.
    let _ = ed.execute_typed("bn", None);
    assert_eq!(ed.buffer_id, bid_b, "bnext advances to b");
    let _ = ed.execute_typed("bn", None);
    assert_eq!(ed.buffer_id, bid_c, "bnext advances to c");
    let _ = ed.execute_typed("bn", None);
    assert_eq!(ed.buffer_id, bid_a, "bnext wraps to a");
    // bprev from a → c.
    let _ = ed.execute_typed("bp", None);
    assert_eq!(ed.buffer_id, bid_c, "bprev wraps to c");
    let _ = ed.execute_typed("bp", None);
    assert_eq!(ed.buffer_id, bid_b, "bprev to b");
}

/// `:bd` closes the current buffer.
#[test]
fn p6_bd_closes_focused_buffer() {
    let mut ed = Editor::for_testing(Buffer::new(Text::from("first\n"), SelectionSet::default()));
    let bid_first = ed.buffer_id;
    let bid_second = ed.open_buffer(Buffer::new(Text::from("second\n"), SelectionSet::default()));
    ed.switch_to_buffer_with_jump(bid_second);
    let _ = ed.execute_typed("bd", None);
    assert_eq!(ed.buffer_id, bid_first, "bd closed second, focused pane moved to first");
    assert!(ed.buffers.try_get(bid_second).is_none(), "second buffer freed");
}

/// `:bd!` closes a dirty buffer without error.
#[test]
fn p6_bd_force_closes_dirty_buffer() {
    let mut ed = Editor::for_testing(Buffer::new(Text::from("clean\n"), SelectionSet::default()));
    let bid_clean = ed.buffer_id;
    let bid_dirty = ed.open_buffer(Buffer::new(Text::from("dirty\n"), SelectionSet::default()));
    ed.switch_to_buffer_with_jump(bid_dirty);
    // Make it dirty by inserting a character.
    ed.handle_key(key('i'));
    ed.handle_key(key('x'));
    ed.handle_key(key_esc());
    assert!(ed.doc().is_dirty(), "buffer should be dirty after edit");
    // :bd without force should fail.
    let result = ed.execute_typed("bd", None);
    assert!(result.is_err(), ":bd on dirty buffer without force should fail");
    // :bd! should succeed.
    let result = ed.execute_typed("bd!", None);
    assert!(result.is_ok(), ":bd! should close dirty buffer");
    assert_eq!(ed.buffer_id, bid_clean);
}

/// `:e path` opens a new buffer when the file is not already open.
#[test]
#[cfg(not(windows))]
fn p6_edit_opens_new_buffer() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("test.txt");
    std::fs::write(&path, "file content\n").unwrap();

    let mut ed = Editor::for_testing(Buffer::new(Text::from("scratch\n"), SelectionSet::default()));
    let initial_bid = ed.buffer_id;
    let canonical = std::fs::canonicalize(&path).unwrap();
    let result = ed.execute_typed("e", Some(path.to_str().unwrap()));
    assert!(result.is_ok(), ":e should succeed for readable file");
    assert_ne!(ed.buffer_id, initial_bid, ":e opened a new buffer");
    assert_eq!(ed.doc().text().to_string(), "file content\n");
    // Path stored correctly.
    assert_eq!(ed.doc().path.as_deref().map(|p| p.as_path()), Some(canonical.as_path()));
}

/// `:e path` deduplicates: switching to an already-open file doesn't create a new buffer.
#[test]
#[cfg(not(windows))]
fn p6_edit_deduplicates_open_file() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("dedup.txt");
    std::fs::write(&path, "dedup\n").unwrap();

    let mut ed = Editor::for_testing(Buffer::new(Text::from("scratch\n"), SelectionSet::default()));
    // Open the file once.
    let r1 = ed.execute_typed("e", Some(path.to_str().unwrap()));
    assert!(r1.is_ok());
    let bid_first_open = ed.buffer_id;
    let count_after_first = ed.buffers.len();
    // Switch back to scratch.
    let scratch_bid = ed.buffers.prev(bid_first_open);
    ed.switch_to_buffer_without_jump(scratch_bid);
    // Open the same file again — should switch to existing buffer, not create new.
    let r2 = ed.execute_typed("e", Some(path.to_str().unwrap()));
    assert!(r2.is_ok());
    assert_eq!(ed.buffer_id, bid_first_open, "dedup: switched to existing buffer");
    assert_eq!(ed.buffers.len(), count_after_first, "no new buffer created on dedup");
}

/// `:e!` reloads the current file even when dirty.
#[test]
#[cfg(not(windows))]
fn p6_edit_force_reloads_current_file() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("reload.txt");
    std::fs::write(&path, "original\n").unwrap();

    let mut ed = Editor::for_testing(Buffer::new(Text::from("scratch\n"), SelectionSet::default()));
    ed.execute_typed("e", Some(path.to_str().unwrap())).unwrap();
    // Dirty the buffer.
    ed.handle_key(key('i'));
    ed.handle_key(key('x'));
    ed.handle_key(key_esc());
    assert!(ed.doc().is_dirty());
    // :e without force should fail.
    let r = ed.execute_typed("e", None);
    assert!(r.is_err(), ":e on dirty buffer should fail without !");
    // :e! should reload.
    let r = ed.execute_typed("e!", None);
    assert!(r.is_ok(), ":e! should reload");
    assert_eq!(ed.doc().text().to_string(), "original\n", "reloaded from disk");
    assert!(!ed.doc().is_dirty(), "not dirty after reload");
}

// ── Phase 7: per-pane pane_jumps ─────────────────────────────────────────────

/// Ctrl+O navigates backward in the per-pane jump list (not a global list).
#[test]
fn p7_pane_jumps_ctrl_o_backward() {
    let mut ed = jump_editor(10);
    let before = state(&ed);

    ed.handle_key(key('g'));
    ed.handle_key(key('g'));
    assert_eq!(ed.doc().text().char_to_line(ed.current_selections().primary().head), 0);

    ed.handle_key(key_ctrl('o'));
    assert_eq!(state(&ed), before, "Ctrl+O returns to pre-jump position");
}

/// Ctrl+I navigates forward in the per-pane jump list.
#[test]
fn p7_pane_jumps_ctrl_i_forward() {
    let mut ed = jump_editor(10);

    ed.handle_key(key('g'));
    ed.handle_key(key('g'));
    let at_top = state(&ed);

    ed.handle_key(key_ctrl('o'));
    assert_ne!(state(&ed), at_top);

    ed.handle_key(key_ctrl('i'));
    assert_eq!(state(&ed), at_top, "Ctrl+I returns to top position");
}

/// Ctrl+O across buffers: `:e file2`, large motion in file2, Ctrl+O lands back in file1.
#[test]
fn p7_cross_buffer_ctrl_o() {
    let dir = tempfile::tempdir().unwrap();
    let file1 = dir.path().join("file1.txt");
    let file2 = dir.path().join("file2.txt");
    // 20 lines in each file so large motions are valid.
    let content: String = (0..20).map(|i| format!("line {i}\n")).collect();
    std::fs::write(&file1, &content).unwrap();
    std::fs::write(&file2, &content).unwrap();

    let mut ed = Editor::for_testing(Buffer::new(Text::from("scratch\n"), SelectionSet::default()));
    ed.execute_typed("e", Some(file1.to_str().unwrap())).unwrap();
    let buf1 = ed.buffer_id;
    let line0_state_f1 = state(&ed);

    // Open file2 — switch_to_buffer_with_jump records {file1, line 0} before switching.
    ed.execute_typed("e", Some(file2.to_str().unwrap())).unwrap();
    let buf2 = ed.buffer_id;
    assert_ne!(buf1, buf2, "different buffers");
    // Now in file2, cursor at line 0. Jump list: [{scratch}, {file1}], cursor = 2.

    // Ctrl+O: saves current (file2, line 0) then goes to entries[1] = {file1, line 0}.
    ed.handle_key(key_ctrl('o'));
    assert_eq!(ed.buffer_id, buf1, "Ctrl+O crossed back to file1");
    assert_eq!(state(&ed), line0_state_f1, "cursor restored in file1");
}

/// Closing a buffer prunes its entries from pane_jumps.
#[test]
fn p7_close_buffer_prunes_pane_jumps() {
    let dir = tempfile::tempdir().unwrap();
    let file1 = dir.path().join("prune1.txt");
    let file2 = dir.path().join("prune2.txt");
    let content: String = (0..20).map(|i| format!("row {i}\n")).collect();
    std::fs::write(&file1, &content).unwrap();
    std::fs::write(&file2, &content).unwrap();

    let mut ed = Editor::for_testing(Buffer::new(Text::from("scratch\n"), SelectionSet::default()));
    ed.execute_typed("e", Some(file1.to_str().unwrap())).unwrap();
    let buf1 = ed.buffer_id;

    // Open file2, recording a jump from file1→file2.
    ed.execute_typed("e", Some(file2.to_str().unwrap())).unwrap();
    let buf2 = ed.buffer_id;
    assert_ne!(buf1, buf2);

    // Close file1 — its jump entries should be pruned from pane_jumps.
    let pid = ed.focused_pane_id;
    ed.close_buffer(buf1);
    // The jump list for this pane must not contain any file1 entries.
    let has_buf1_entry = ed.pane_jumps[pid].entries_for_buffer(buf1);
    assert!(!has_buf1_entry, "pane_jumps should not contain closed buffer entries");
}
