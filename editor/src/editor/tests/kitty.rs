use super::*;
use pretty_assertions::assert_eq;

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

/// Ctrl+w, h/j/k/l → directional pane-focus stubs (M9+ placeholders).
///
/// Locks the contract that all four directional variants are stubs until
/// the `:split` feature lands — they must not mutate state or panic.
#[test]
fn ctrl_w_directional_stubs_report_not_implemented() {
    for second_key in ['h', 'j', 'k', 'l'] {
        let mut ed = editor_from_kitty("-[h]>ello world\n");
        ed.handle_key(key_ctrl('w'));
        ed.handle_key(key(second_key));
        assert_eq!(
            state(&ed), "-[h]>ello world\n",
            "Ctrl+w {second_key}: stub must not move cursor",
        );
        assert!(
            ed.status_msg.as_deref().unwrap_or("").contains("not yet implemented"),
            "Ctrl+w {second_key}: stub must report not-yet-implemented: {:?}",
            ed.status_msg,
        );
    }
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

