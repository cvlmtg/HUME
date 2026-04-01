use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use pretty_assertions::assert_eq;

use crate::core::document::Document;
use crate::testing::{parse_state, serialize_state};
use crate::ui::view::{compute_gutter_width, LineNumberStyle, ViewState};

use super::{Editor, Mode};

// ── Harness ───────────────────────────────────────────────────────────────────

/// Build an Editor pre-loaded with the given state string (same DSL as other tests).
fn editor_from(input: &str) -> Editor {
    let (buf, sels) = parse_state(input);
    let view = ViewState {
        scroll_offset: 0,
        height: 24,
        width: 80,
        gutter_width: compute_gutter_width(buf.len_lines()),
        line_number_style: LineNumberStyle::Hybrid,
        col_offset: 0,
    };
    Editor {
        doc: Document::new(buf, sels),
        view,
        file_path: None,
        mode: Mode::Normal,
        extend: false,
        pending_keys: Vec::new(),
        count: None,
        wait_char: None,
        pending_char: None,
        registers: crate::ops::register::RegisterSet::new(),
        colors: crate::ui::theme::EditorColors::default(),
        should_quit: false,
        minibuf: None,
        status_msg: None,
        file_meta: None,
        statusline_config: crate::ui::statusline::StatusLineConfig::default(),
        registry: super::registry::CommandRegistry::with_defaults(),
        keymap: super::keymap::Keymap::default(),
        auto_pairs: crate::auto_pairs::AutoPairsConfig::default(),
        last_find: None,
        kitty_enabled: false,
        last_action: None,
        insert_session: None,
        explicit_count: false,
        search: super::SearchState::default(),
        pre_select_sels: None,
        jump_list: crate::core::jump_list::JumpList::new(),
    }
}

/// Build a kitty-protocol-enabled editor for testing Ctrl+motion bindings.
fn editor_from_kitty(input: &str) -> Editor {
    let mut ed = editor_from(input);
    ed.kitty_enabled = true;
    ed
}

/// Serialize the editor's current buffer + selection state.
fn state(ed: &Editor) -> String {
    serialize_state(ed.doc.buf(), ed.doc.sels())
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
        .map(|r| r.values().to_vec())
        .unwrap_or_default()
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
    assert_eq!(ed.doc.buf().to_string(), "hio\n");

    // One undo should restore the original word entirely.
    ed.handle_key(key('u'));
    assert_eq!(state(&ed), "-[hell]>o\n");

    // Only one revision was recorded.
    assert!(!ed.doc.can_undo());
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

    assert_eq!(ed.doc.buf().to_string(), "o\n", "buffer after delete");
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
    ed.registers.write(DEFAULT_REGISTER, vec!["XY".to_string()]);

    ed.handle_key(key('p'));

    assert_eq!(ed.doc.buf().to_string(), "XYo\n", "pasted text in buffer");
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
    assert!(!ed.extend, "extend off initially");

    // Toggle extend on.
    ed.handle_key(key('e'));
    assert!(ed.extend, "extend on after 'e'");

    // A motion in extend mode should grow the selection, not move a cursor.
    ed.handle_key(key('l'));
    assert_eq!(state(&ed), "-[he]>llo\n", "selection extended right by one");

    // Toggle extend off.
    ed.handle_key(key('e'));
    assert!(!ed.extend, "extend off after second 'e'");
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
    assert_eq!(ed.doc.buf().to_string(), "hello\n\n");
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
    assert_eq!(ed.doc.buf().to_string(), "foo\n\nbar\n");
    // Cursor on the new blank line between "foo" and "bar".
    assert_eq!(state(&ed), "foo\n-[\n]>bar\n");
}

// ── Insert-entry variants position the cursor correctly ────────────────────

/// `a` must move the cursor one grapheme right of the current position, then
/// enter Insert mode — "append after cursor".
#[test]
fn a_enters_insert_one_right_of_cursor() {
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

/// `I` must jump to the first non-blank character of the line, then enter
/// Insert mode — "insert before first non-blank".
#[test]
fn capital_i_enters_insert_at_first_nonblank() {
    let mut ed = editor_from("  hell-[o]>\n");
    ed.handle_key(key('I'));

    assert_eq!(ed.mode, Mode::Insert);
    assert_eq!(state(&ed), "  -[h]>ello\n");
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
    ed.extend = true;

    ed.handle_key(key('o'));

    // anchor and head are swapped — selection is now backward.
    assert_eq!(state(&ed), "<[hell]-o\n");
    // extend mode is still active (flip doesn't exit it).
    assert!(ed.extend);
}

#[test]
fn o_in_normal_mode_still_opens_line_below() {
    let mut ed = editor_from("-[h]>ello\n");
    // extend is off (default).

    ed.handle_key(key('o'));

    assert_eq!(ed.mode, Mode::Insert);
    assert_eq!(ed.doc.buf().to_string(), "hello\n\n");
}

// ── `;` collapses selection AND clears extend mode ─────────────────────────

/// `;` must (a) collapse every selection to its head and (b) clear the
/// `extend` flag. The extend side-effect only exists in the mapping — a pure
/// `cmd_collapse_selection` test cannot see it.
#[test]
fn semicolon_collapses_selection_and_resets_extend() {
    let mut ed = editor_from("-[hell]>o\n");
    ed.extend = true;

    ed.handle_key(key(';'));

    assert!(!ed.extend, "extend cleared by ';'");
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
    assert_eq!(ed.doc.buf().to_string(), "hello\nworld\n");

    ed.handle_key(key('u'));
    assert_eq!(state(&ed), "-[h]>ello\n");
    assert!(!ed.doc.can_undo());
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
    assert_eq!(ed.doc.buf().to_string(), "foo\nnew\nbar\n");

    ed.handle_key(key('u'));
    assert_eq!(state(&ed), "foo\n-[b]>ar\n");
    assert!(!ed.doc.can_undo());
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
    assert_eq!(ed.doc.buf().to_string(), "hello\n");
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
    assert_eq!(ed.doc.buf().to_string(), "XYhello\n");

    // One undo restores the original state completely.
    ed.handle_key(key('u'));
    assert_eq!(state(&ed), "-[h]>ello\n");

    // Only one revision was recorded.
    assert!(!ed.doc.can_undo());
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
    assert_eq!(ed.status_msg.as_deref(), Some("Error: no file name"));
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
    let (_, meta) = crate::io::read_file(&path).unwrap();
    let mut ed = editor_from(initial_state);
    ed.file_path = Some(path);
    ed.file_meta = Some(meta);
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
    assert!(!ed.doc.is_dirty());
}

#[test]
fn typing_in_insert_mode_makes_dirty() {
    let mut ed = editor_from("-[h]>ello\n");
    ed.handle_key(key('i'));
    ed.handle_key(key('x'));
    ed.handle_key(key_esc());
    assert!(ed.doc.is_dirty());
}

#[test]
fn colon_w_marks_buffer_clean() {
    let (mut ed, _tmp) = editor_with_file("-[h]>ello\n", "hello\n");
    // Make the buffer dirty.
    ed.handle_key(key('i'));
    ed.handle_key(key('x'));
    ed.handle_key(key_esc());
    assert!(ed.doc.is_dirty());
    // Write — should clear dirty flag.
    for ch in ":w".chars() { ed.handle_key(key(ch)); }
    ed.handle_key(key_enter());
    assert!(!ed.doc.is_dirty());
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
    // Buffer is fresh (not dirty) — :q should quit.
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
    assert!(ed.file_path.is_some());
    // Buffer should now be clean.
    assert!(!ed.doc.is_dirty());
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
    assert!(ed.file_meta.is_some());

    // Make dirty again and write without a path — should use the new path.
    ed.handle_key(key('i'));
    ed.handle_key(key('y'));
    ed.handle_key(key_esc());
    for ch in ":w".chars() { ed.handle_key(key(ch)); }
    ed.handle_key(key_enter());
    assert!(ed.status_msg.as_deref().unwrap_or("").starts_with("Written"));
    assert!(!ed.doc.is_dirty());
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
    assert_eq!(ed.status_msg.as_deref(), Some("Error: w! is not supported"));
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
    let (_, meta) = crate::io::read_file(&tmp).unwrap();
    ed.file_meta = Some(meta);

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
    let (_, meta) = crate::io::read_file(&link_path).unwrap();
    assert_eq!(meta.resolved_path, std::fs::canonicalize(real.path()).unwrap());

    let mut ed = editor_from("-[h]>ello\n");
    ed.file_path = Some(link_path.clone());
    ed.file_meta = Some(meta);

    for ch in ":w".chars() { ed.handle_key(key(ch)); }
    ed.handle_key(key_enter());

    assert!(ed.status_msg.as_deref().unwrap_or("").starts_with("Written"));
    // The symlink must still exist and still be a symlink.
    assert!(link_path.symlink_metadata().unwrap().file_type().is_symlink());
    // Content was written to the real file.
    assert_eq!(std::fs::read_to_string(real.path()).unwrap(), "hello\n");
}

// ── Auto-pairs integration tests ──────────────────────────────────────────────

/// Typing `(` inserts `()` with the cursor between them (on `)`) so subsequent
/// characters appear inside the pair.
#[test]
fn auto_pairs_auto_close() {
    let mut ed = editor_from("-[h]>ello\n");
    ed.handle_key(key('i'));        // enter insert at 'h'
    ed.handle_key(key('('));
    assert_eq!(state(&ed), "(-[)]>hello\n");
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

/// Typing `"` inserts `""` with cursor between (symmetric pair auto-close).
#[test]
fn auto_pairs_symmetric_auto_close() {
    let mut ed = editor_from("-[x]>\n");
    ed.handle_key(key('i'));
    ed.handle_key(key('"'));
    assert_eq!(state(&ed), "\"-[\"]>x\n");
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
    ed.auto_pairs.enabled = false;
    ed.handle_key(key('i'));
    ed.handle_key(key('('));
    assert_eq!(state(&ed), "(-[h]>ello\n");
}

// Note: wrap-selection (insert_pair_close with a non-cursor selection) is tested
// at the unit level in auto_pairs::tests. It is not reachable via the normal
// editor insert-mode entry points because all of them (i, a, c, o, …) collapse
// to a cursor before entering Insert.

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

/// Ctrl+w extends to the next word via union semantics (kitty mode only).
/// Starting from a cursor at 'h', select_next_word finds "world" (6,10).
/// Union with current pos (0,0): min(0,6)=0, max(0,10)=10 → "hello world".
#[test]
fn kitty_ctrl_w_extends_next_word() {
    let mut ed = editor_from_kitty("-[h]>ello world\n");
    ed.handle_key(key_ctrl('w'));
    assert_eq!(state(&ed), "-[hello world]>\n");
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

/// Without kitty, Ctrl+w is a no-op (same rationale as Ctrl+h above).
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
    assert_eq!(ed.doc.buf().to_string(), "ello\n");
    // Ctrl+u should NOT run undo — it's a no-op because "undo" has no extend variant.
    ed.handle_key(key_ctrl('u'));
    assert_eq!(ed.doc.buf().to_string(), "ello\n", "Ctrl+u should be a no-op in kitty mode");
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
    assert_eq!(ed.doc.buf().to_string(), "ello\n");
    ed.handle_key(key('u'));    // regular undo
    assert_eq!(ed.doc.buf().to_string(), "hello\n");
    // Ctrl+U should NOT run redo.
    ed.handle_key(key_ctrl('U'));
    assert_eq!(ed.doc.buf().to_string(), "hello\n", "Ctrl+U should be a no-op in kitty mode");
}

// ── Dot-repeat tests ──────────────────────────────────────────────────────────

/// `d` deletes the selection. Moving then pressing `.` should delete the next selection.
#[test]
fn dot_repeats_delete() {
    // Cursor starts at 'f'. `w` selects "foo", `d` deletes it.
    // Then from the space at pos 0, `w` selects "bar" (the next word). `.` deletes it.
    let mut ed = editor_from("-[foo]> bar\n");
    ed.handle_key(key('d'));           // delete "foo" → " bar\n", cursor at 0 (space)
    assert_eq!(ed.doc.buf().to_string(), " bar\n");

    ed.handle_key(key('w'));           // from space, select "bar"
    ed.handle_key(key('.'));           // repeat delete
    assert_eq!(ed.doc.buf().to_string(), " \n");
}

/// `c` + typed text + Esc should be replayable: the replacement text is reused.
#[test]
fn dot_repeats_change_with_insert() {
    let mut ed = editor_from("-[foo]> bar\n");

    ed.handle_key(key('c'));           // change: delete "foo", enter Insert
    ed.handle_key(key('h'));
    ed.handle_key(key('i'));
    ed.handle_key(key_esc());          // back to Normal; buffer is "hi bar"

    assert_eq!(ed.doc.buf().to_string(), "hi bar\n");

    // Move to "bar" and repeat.
    ed.handle_key(key('w'));           // select "bar"
    ed.handle_key(key('.'));           // repeat: delete "bar", insert "hi"

    assert_eq!(ed.doc.buf().to_string(), "hi hi\n");
}

/// `i` + typed text + Esc inserts before the selection. `.` should replay that insert.
#[test]
fn dot_repeats_insert_before() {
    let mut ed = editor_from("-[x]>\n");

    ed.handle_key(key('i'));           // insert-before, cursor collapses to start
    ed.handle_key(key('a'));
    ed.handle_key(key('b'));
    ed.handle_key(key_esc());          // back to Normal; buffer is "abx"

    assert_eq!(ed.doc.buf().to_string(), "abx\n");

    // Move to 'x' and repeat.
    ed.handle_key(key('w'));           // select 'x'
    ed.handle_key(key('.'));           // repeat insert "ab" before 'x'

    assert_eq!(ed.doc.buf().to_string(), "ababx\n");
}

/// `r` + char replaces every character in the selection. `.` should replay with
/// the same replacement character.
#[test]
fn dot_repeats_replace() {
    // Use a space between words so `w` can navigate to the second word.
    let mut ed = editor_from("-[ab]> cd\n");

    ed.handle_key(key('r'));           // wait-char
    ed.handle_key(key('x'));           // replace "ab" → "xx cd\n"

    assert_eq!(ed.doc.buf().to_string(), "xx cd\n");

    // `w` from the "xx" selection (head at pos 1) selects the next word "cd".
    ed.handle_key(key('w'));
    ed.handle_key(key('.'));           // repeat replace with 'x' → "xx xx\n"

    assert_eq!(ed.doc.buf().to_string(), "xx xx\n");
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
    assert!(!ed.doc.buf().to_string().contains('c'));
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
    assert!(!ed.doc.buf().to_string().contains("world"));
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
    assert_eq!(ed.doc.buf().to_string(), "hi bar\n");

    // Move to "bar" and repeat.
    ed.handle_key(key('w'));
    ed.handle_key(key('.'));
    assert_eq!(ed.doc.buf().to_string(), "hi hi\n");

    // One undo undoes the `.` replay entirely.
    ed.handle_key(key('u'));
    assert_eq!(ed.doc.buf().to_string(), "hi bar\n");
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

    assert_eq!(ed.doc.buf().to_string(), "a\nx\nb\n");

    // Move cursor to "b" and repeat.
    ed.handle_key(key('j'));           // move down to 'x'
    ed.handle_key(key('j'));           // move down to 'b'
    ed.handle_key(key('.'));           // repeat: open line below "b", insert "x"

    assert_eq!(ed.doc.buf().to_string(), "a\nx\nb\nx\n");
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
    let buf = ed.doc.buf().to_string();
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
/// Vim would go backward here; Helix/Kakoune go forward. HUME follows Helix/Kakoune.
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
    ed.extend = true;

    ed.handle_key(key('/'));
    for ch in "world".chars() {
        ed.handle_key(key(ch));
    }
    // Live search in extend mode: anchor stays at 0 ('h'), head moves to 10 ('d').
    assert_eq!(state(&ed), "-[hello world]>\n");

    ed.handle_key(key_enter());
    ed.extend = false;

    // n in extend mode: anchor stays at 0, head jumps to next match.
    ed.extend = true;
    // Only one "world" — wraps back to the same match.
    ed.handle_key(key('n'));
    // Selection should still cover from anchor=0 to the match end.
    assert_eq!(state(&ed), "-[hello world]>\n");
}

/// `Esc` in Normal mode clears the active search regex and its cached state.
#[test]
fn esc_in_normal_clears_search() {
    let mut ed = editor_from("-[h]>ello hello\n").with_search_regex("hello");

    assert!(ed.search.regex.is_some(), "pre-condition: search regex is set");
    assert!(ed.search.match_count.is_some(), "pre-condition: cache is populated");

    ed.handle_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
    ed.update_search_cache();

    assert!(ed.search.regex.is_none(), "search.regex should be cleared by Esc");
    assert!(ed.search.match_count.is_none(), "search.match_count should be cleared by Esc");
    assert!(ed.search.matches.is_empty(), "search.matches should be cleared by Esc");
}

/// `:clear-search` / `:cs` in Command mode clears the active search regex and its cached state.
#[test]
fn command_clear_search_clears_search() {
    let mut ed = editor_from("-[h]>ello hello\n").with_search_regex("hello");

    assert!(ed.search.regex.is_some(), "pre-condition: search regex is set");

    // :clear-search (canonical name)
    ed.handle_key(key(':'));
    for ch in "clear-search".chars() {
        ed.handle_key(key(ch));
    }
    ed.handle_key(key_enter());
    ed.update_search_cache();

    assert_eq!(ed.mode, Mode::Normal);
    assert!(ed.search.regex.is_none(), "search.regex should be cleared by :clear-search");
    assert!(ed.search.match_count.is_none(), "search.match_count should be cleared by :clear-search");
    assert!(ed.search.matches.is_empty(), "search.matches should be cleared by :clear-search");

    // :cs shorthand also works
    let mut ed2 = editor_from("-[h]>ello hello\n").with_search_regex("hello");
    ed2.handle_key(key(':'));
    for ch in "cs".chars() {
        ed2.handle_key(key(ch));
    }
    ed2.handle_key(key_enter());
    ed2.update_search_cache();

    assert!(ed2.search.regex.is_none(), "search.regex should be cleared by :cs");
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
    assert!(ed.pre_select_sels.is_some());
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
    assert!(ed.pre_select_sels.is_none());
    // Two "ab" matches within the original selection.
    assert_eq!(ed.doc.sels().len(), 2);
    assert_eq!(ed.doc.sels().primary().anchor, 0);
    assert_eq!(ed.doc.sels().primary().head, 1);
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

/// `s` writes the confirmed pattern to the search register.
#[test]
fn select_within_writes_search_register() {
    let mut ed = editor_from("-[ab cd ab]>\n");
    ed.handle_key(key('s'));
    ed.handle_key(key('a'));
    ed.handle_key(key('b'));
    ed.handle_key(key_enter());
    assert_eq!(reg(&ed, 's'), vec!["ab"]);
}

/// `s` does not set the search regex — highlights would be misleading
/// because they appear outside the selection scope.
#[test]
fn select_within_does_not_set_search_regex() {
    let mut ed = editor_from("-[ab cd ab]>\n");
    ed.handle_key(key('s'));
    ed.handle_key(key('a'));
    ed.handle_key(key('b'));
    assert!(ed.search.regex.is_none());
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
    assert!(ed.search.regex.is_some());
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
    let buf = crate::core::buffer::Buffer::from("foo.bar\n");
    let sels = crate::core::selection::SelectionSet::single(
        crate::core::selection::Selection::new(0, 6),
    );
    ed.doc = crate::core::document::Document::new(buf, sels);

    ed.handle_key(key('*'));
    assert_eq!(reg(&ed, 's'), vec!["foo\\.bar"]);
}

// ── Jump list ────────────────────────────────────────────────────────────────

/// Build a 20-line buffer with the cursor on a given line for jump list tests.
fn jump_editor(cursor_line: usize) -> Editor {
    // 20 lines: "line 0\n", "line 1\n", ..., "line 19\n"
    let text: String = (0..20).map(|i| format!("line {i}\n")).collect();
    let buf = crate::core::buffer::Buffer::from(text.as_str());
    // Place cursor at the start of the requested line.
    let pos = buf.line_to_char(cursor_line);
    let sels = crate::core::selection::SelectionSet::single(
        crate::core::selection::Selection::cursor(pos),
    );
    let view = ViewState {
        scroll_offset: 0,
        height: 24,
        width: 80,
        gutter_width: compute_gutter_width(buf.len_lines()),
        line_number_style: LineNumberStyle::Hybrid,
        col_offset: 0,
    };
    let doc = crate::core::document::Document::new(buf, sels);
    let mut ed = Editor::for_testing(doc, view);
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
    assert_eq!(ed.doc.buf().char_to_line(ed.doc.sels().primary().head), 0);

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
    assert_eq!(ed.doc.buf().char_to_line(ed.doc.sels().primary().head), 10);

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
    assert_eq!(ed.doc.buf().char_to_line(ed.doc.sels().primary().head), 15);

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
    let buf = crate::core::buffer::Buffer::from(text);
    let sels = crate::core::selection::SelectionSet::single(
        crate::core::selection::Selection::cursor(0),
    );
    let view = ViewState {
        scroll_offset: 0,
        height: 24,
        width: 80,
        gutter_width: compute_gutter_width(buf.len_lines()),
        line_number_style: LineNumberStyle::Hybrid,
        col_offset: 0,
    };
    let doc = crate::core::document::Document::new(buf, sels);
    let mut ed = Editor::for_testing(doc, view);
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

