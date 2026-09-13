//! selection_keys.rs — undo/redo boundary messages, register-affecting single-key commands, and `x` select-line.

use super::super::*;
use pretty_assertions::assert_eq;

// ── Undo/redo boundary messages ────────────────────────────────────────────

#[test]
fn undo_at_root_shows_message() {
    let mut ed = editor_from("-[h]>ello\n");
    ed.handle_key(key('i'));
    ed.handle_key(key('x'));
    ed.handle_key(key_esc());
    ed.handle_key(key('u'));
    assert!(
        ed.state.status_msg.is_none(),
        "first undo should succeed — no message"
    );
    ed.handle_key(key('u'));
    assert_eq!(
        ed.state.status_msg.as_deref(),
        Some("Already at oldest change"),
        "second undo at root should show message"
    );
}

#[test]
fn undo_message_cleared_on_next_keypress() {
    let mut ed = editor_from("-[h]>ello\n");
    ed.handle_key(key('i'));
    ed.handle_key(key('x'));
    ed.handle_key(key_esc());
    ed.handle_key(key('u'));
    ed.handle_key(key('u'));
    assert_eq!(
        ed.state.status_msg.as_deref(),
        Some("Already at oldest change")
    );
    ed.handle_key(key('l'));
    assert!(
        ed.state.status_msg.is_none(),
        "next keypress clears the undo-at-root message"
    );
}

#[test]
fn redo_at_newest_shows_message() {
    let mut ed = editor_from("-[h]>ello\n");
    ed.handle_key(key('i'));
    ed.handle_key(key('x'));
    ed.handle_key(key_esc());
    ed.handle_key(key('u'));
    ed.handle_key(key_ctrl('r'));
    assert!(
        ed.state.status_msg.is_none(),
        "first redo should succeed — no message"
    );
    ed.handle_key(key_ctrl('r'));
    assert_eq!(
        ed.state.status_msg.as_deref(),
        Some("Already at newest change"),
        "second redo at newest should show message"
    );
}

#[test]
fn undo_with_count_shows_message_on_exhaustion() {
    let mut ed = editor_from("-[h]>ello\n");
    ed.handle_key(key('i'));
    ed.handle_key(key('x'));
    ed.handle_key(key_esc());
    // Type a count prefix "2" before "u"
    ed.handle_key(key('2'));
    ed.handle_key(key('u'));
    assert_eq!(
        ed.state.status_msg.as_deref(),
        Some("Already at oldest change"),
        "count=2 with only 1 undo step should show message on final step"
    );
}

#[test]
fn redo_with_count_shows_message_on_exhaustion() {
    let mut ed = editor_from("-[h]>ello\n");
    ed.handle_key(key('i'));
    ed.handle_key(key('x'));
    ed.handle_key(key_esc());
    ed.handle_key(key('u'));
    // Type a count prefix "2" before Ctrl+r
    ed.handle_key(key('2'));
    ed.handle_key(key_ctrl('r'));
    assert_eq!(
        ed.state.status_msg.as_deref(),
        Some("Already at newest change"),
        "count=2 with only 1 redo step should show message on final step"
    );
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
        ed.state.kill_ring.head(),
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
    use hume_ops::register::CLIPBOARD_REGISTER;

    let mut ed = editor_from("-[hell]>o\n");
    ed.handle_key(key('y'));

    assert_eq!(state(&ed), "-[hell]>o\n", "buffer+selection unchanged");
    // Bare `y` writes to system clipboard (in-memory mirror in headless tests)
    // AND pushes to the kill ring.
    assert_eq!(
        reg(&ed, CLIPBOARD_REGISTER),
        &["hell"],
        "clipboard populated"
    );
    assert_eq!(
        ed.state.kill_ring.head(),
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
    assert!(ed.state.wait_char.is_some(), "wait_char set after 'r'");

    ed.handle_key(key('x'));
    assert!(
        ed.state.wait_char.is_none(),
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

    assert!(ed.state.wait_char.is_none());
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
    assert!(ed.state.wait_char.is_some(), "wait_char set after 'f'");
    ed.handle_key(key_esc());

    assert!(ed.state.wait_char.is_none(), "wait_char cleared after Esc");
    assert!(ed.state.pending_char.is_none(), "pending_char not set");
    assert_eq!(
        state(&ed),
        "-[h]>ello a\n",
        "buffer and cursor unchanged after cancelled find"
    );
}

// ── WaitChar accepts Enter and Tab (Kakoune parity) ────────────────────────────

/// `r<ret>` replaces the char under the cursor with a literal newline. The
/// wait-char consumer translates `KeyCode::Enter` to `'\n'` before dispatch.
#[test]
fn r_then_enter_replaces_with_newline() {
    let mut ed = editor_from("-[h]>ello\n");
    ed.handle_key(key('r'));
    ed.handle_key(key_enter());
    assert!(
        ed.state.wait_char.is_none(),
        "wait_char cleared after Enter"
    );
    assert_eq!(state(&ed), "-[\n]>ello\n");
}

/// Multi-char selection: every grapheme becomes '\n', except a grapheme that
/// already was '\n' — `replace_selections` never replaces an existing newline,
/// it is retained as-is (see `replace_multiline_selection_skips_newline` in
/// `hume-ops/src/edit/tests/replace.rs`). This exercises that rule with the new Enter argument.
#[test]
fn r_then_enter_multi_char_selection_replaces_each_grapheme() {
    let mut ed = editor_from("-[ab\ncd]>\n");
    ed.handle_key(key('r'));
    ed.handle_key(key_enter());
    assert_eq!(state(&ed), "-[\n\n\n\n\n]>\n");
}

/// `r<tab>` replaces with a literal tab character.
#[test]
fn r_then_tab_replaces_with_tab() {
    let mut ed = editor_from("-[h]>ello\n");
    ed.handle_key(key('r'));
    ed.handle_key(key_tab());
    assert!(ed.state.wait_char.is_none(), "wait_char cleared after Tab");
    assert_eq!(state(&ed), "-[\t]>ello\n");
}

/// `f<ret>` is accepted as a wait-char argument (the wait clears, unlike Esc)
/// but never matches: `find_char_on_line_forward` (hume-ops/src/motion/find.rs)
/// explicitly excludes '\n' as a structural line boundary, not content — by
/// design, not a bug. This documents that "accepted argument" and "found on
/// line" are separate questions, exactly like `fz` on a line with no 'z'.
#[test]
fn f_then_enter_is_accepted_but_never_matches() {
    let mut ed = editor_from("-[h]>ello\n");
    ed.handle_key(key('f'));
    ed.handle_key(key_enter());
    assert!(
        ed.state.wait_char.is_none(),
        "wait_char cleared after Enter"
    );
    assert_eq!(
        state(&ed),
        "-[h]>ello\n",
        "'\\n' is never a match target for find — cursor unchanged"
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
        ed.state.pending_keys.len(),
        1,
        "pending_keys has 'm' after first press"
    );

    ed.handle_key(key('i'));
    assert_eq!(
        ed.state.pending_keys.len(),
        2,
        "pending_keys has 'm','i' after second press"
    );

    ed.handle_key(key('w'));
    assert!(
        ed.state.pending_keys.is_empty(),
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
        ed.state.pending_keys.is_empty(),
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
    assert_eq!(ed.state.mode, Mode::Normal, "Normal mode initially");

    // Toggle extend on.
    ed.handle_key(key('e'));
    assert_eq!(ed.state.mode, Mode::Extend, "Extend mode after 'e'");

    // A motion in extend mode should grow the selection, not move a cursor.
    ed.handle_key(key('l'));
    assert_eq!(state(&ed), "-[he]>llo\n", "selection extended right by one");

    // Toggle extend off.
    ed.handle_key(key('e'));
    assert_eq!(ed.state.mode, Mode::Normal, "Normal mode after second 'e'");
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
