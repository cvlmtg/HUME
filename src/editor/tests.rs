use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use pretty_assertions::assert_eq;

use crate::document::Document;
use crate::testing::{parse_state, serialize_state};
use crate::view::{compute_gutter_width, LineNumberStyle, ViewState};

use super::{Editor, Mode, PendingKey};

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
    };
    Editor {
        doc: Document::new(buf, sels),
        view,
        file_path: None,
        mode: Mode::Normal,
        extend: false,
        pending: PendingKey::None,
        registers: crate::register::RegisterSet::new(),
        colors: crate::theme::EditorColors::default(),
        should_quit: false,
    }
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

// ── 1. `d` yanks into the default register ────────────────────────────────────

/// Deleting a selection must populate the default register with the deleted
/// text. A bug in the mapping that removed the `yank_selections` call before
/// `delete_selection` would leave the register empty — invisible to pure tests.
#[test]
fn d_yanks_selection_into_register_before_deleting() {
    use crate::register::DEFAULT_REGISTER;

    let mut ed = editor_from("-[hell]>o\n");
    ed.handle_key(key('d'));

    assert_eq!(ed.doc.buf().to_string(), "o\n", "buffer after delete");
    assert_eq!(reg(&ed, DEFAULT_REGISTER), &["hell"], "register after delete");
}

// ── 2. `y` yanks without modifying the buffer ─────────────────────────────────

/// `y` must populate the register without changing the buffer or the selection.
/// This is the only way to test that `y` actually writes to the register —
/// pure tests of `yank_selections` never touch the `Editor.registers` field.
#[test]
fn y_populates_register_without_changing_buffer() {
    use crate::register::DEFAULT_REGISTER;

    let mut ed = editor_from("-[hell]>o\n");
    ed.handle_key(key('y'));

    assert_eq!(state(&ed), "-[hell]>o\n", "buffer+selection unchanged");
    assert_eq!(reg(&ed, DEFAULT_REGISTER), &["hell"], "register populated");
}

// ── 3. `p` swaps displaced selection text back into the register ──────────────

/// When `p` pastes over a non-cursor (multi-char) selection, the displaced
/// text must be written back to the default register (exchange semantics).
/// This logic lives entirely in the mapping — no pure test can see it.
#[test]
fn p_over_selection_swaps_displaced_text_into_register() {
    use crate::register::DEFAULT_REGISTER;

    let mut ed = editor_from("-[hell]>o\n");
    // Seed the register with the text we'll paste.
    ed.registers.write(DEFAULT_REGISTER, vec!["XY".to_string()]);

    ed.handle_key(key('p'));

    assert_eq!(ed.doc.buf().to_string(), "XYo\n", "pasted text in buffer");
    assert_eq!(reg(&ed, DEFAULT_REGISTER), &["hell"], "displaced text in register");
}

// ── 4. `r<char>` pending-key replace sequence ─────────────────────────────────

/// `r` alone must set `PendingKey::Replace`; the following character must
/// replace every grapheme in every selection; and `Esc` after a bare `r`
/// must cancel without side effects.
#[test]
fn r_then_char_replaces_every_grapheme_in_selection() {
    let mut ed = editor_from("-[hell]>o\n");

    ed.handle_key(key('r'));
    assert_eq!(ed.pending, PendingKey::Replace, "pending after 'r'");

    ed.handle_key(key('x'));
    assert_eq!(ed.pending, PendingKey::None, "pending cleared after replacement char");
    assert_eq!(state(&ed), "-[xxxx]>o\n");
}

#[test]
fn r_then_esc_cancels_without_side_effects() {
    let mut ed = editor_from("-[hell]>o\n");
    ed.handle_key(key('r'));
    ed.handle_key(key_esc());

    assert_eq!(ed.pending, PendingKey::None);
    assert_eq!(state(&ed), "-[hell]>o\n", "buffer unchanged after cancelled replace");
}

// ── 5. `m i w` three-key text-object sequence ─────────────────────────────────

/// The pending-key state machine must advance through `Match` → `MatchInner`
/// → `None` and dispatch the correct text-object command on the third key.
/// This exercises the entire three-key pipeline end-to-end.
#[test]
fn m_i_w_selects_inner_word() {
    let mut ed = editor_from("-[h]>ello world\n");

    ed.handle_key(key('m'));
    assert_eq!(ed.pending, PendingKey::Match);

    ed.handle_key(key('i'));
    assert_eq!(ed.pending, PendingKey::MatchInner);

    ed.handle_key(key('w'));
    assert_eq!(ed.pending, PendingKey::None);
    assert_eq!(state(&ed), "-[hello]> world\n");
}

/// An unrecognised object char after `ma` must fall through cleanly — the
/// pending state is cleared without modifying the buffer or the selection.
#[test]
fn m_a_unknown_char_falls_through_cleanly() {
    let mut ed = editor_from("-[h]>ello\n");

    ed.handle_key(key('m'));
    ed.handle_key(key('a'));
    // '~' is not a known text-object char — should fall through.
    ed.handle_key(key('~'));

    assert_eq!(ed.pending, PendingKey::None);
    // Selection and buffer are unchanged (fall-through re-dispatches '~' as a
    // normal key, which is currently a no-op).
    assert_eq!(state(&ed), "-[h]>ello\n");
}

// ── 6. `x` extend-mode toggle ─────────────────────────────────────────────────

/// `x` must toggle `extend` on and off. While extend is active, motions must
/// grow the selection rather than collapse it to a cursor.
#[test]
fn x_toggles_extend_mode_and_motions_extend_selection() {
    let mut ed = editor_from("-[h]>ello\n");
    assert!(!ed.extend, "extend off initially");

    // Toggle extend on.
    ed.handle_key(key('x'));
    assert!(ed.extend, "extend on after 'x'");

    // A motion in extend mode should grow the selection, not move a cursor.
    ed.handle_key(key('l'));
    assert_eq!(state(&ed), "-[he]>llo\n", "selection extended right by one");

    // Toggle extend off.
    ed.handle_key(key('x'));
    assert!(!ed.extend, "extend off after second 'x'");
}

// ── 7. `o` / `O` open-line variants ──────────────────────────────────────────

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

// ── 8. Insert-entry variants position the cursor correctly ────────────────────

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

// ── 9. `;` collapses selection AND clears extend mode ─────────────────────────

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

// ── 10. Plain insert session groups all chars into one undo step ──────────────

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
