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

/// Ctrl+w extends to the next word in kitty mode.
/// Ctrl+w has no explicit trie binding; the strip-CONTROL fallback resolves it
/// to bare 'w' → select-next-word with ctrl_extend=true.
#[test]
fn kitty_ctrl_w_extends_next_word() {
    let mut ed = editor_from_kitty("-[h]>ello world\n");
    ed.handle_key(key_ctrl('w'));
    // extend-next-word from anchor=0, head=0 ('h'): select-next-word finds
    // "world" (the next whole word past "hello"), then Extend keeps anchor=0
    // and sets head=10, extending over "hello world".
    assert_eq!(state(&ed), "-[hello world]>\n");
}

/// Ctrl+p is the pane prefix — pressing it alone waits for a second key.
/// The state is unchanged after just Ctrl+p (Interior node, not a leaf).
#[test]
fn ctrl_p_starts_pane_prefix() {
    let mut ed = editor_from_kitty("-[h]>ello world\n");
    ed.handle_key(key_ctrl('p'));
    assert_eq!(
        state(&ed),
        "-[h]>ello world\n",
        "Ctrl+p alone must not change state"
    );
}

/// Ctrl+p, w → pane-focus-next stub (not yet implemented).
#[test]
fn ctrl_p_w_is_pane_focus_next_stub() {
    let mut ed = editor_from_kitty("-[h]>ello world\n");
    ed.handle_key(key_ctrl('p'));
    ed.handle_key(key('w'));
    assert_eq!(state(&ed), "-[h]>ello world\n", "stub must not move cursor");
    assert!(
        ed.status_msg
            .as_deref()
            .unwrap_or("")
            .contains("not yet implemented"),
        "stub must report not-yet-implemented: {:?}",
        ed.status_msg,
    );
}

/// Ctrl+p, h/j/k/l → directional pane-focus stubs (M9+ placeholders).
///
/// Locks the contract that all four directional variants are stubs until
/// the `:split` feature lands — they must not mutate state or panic.
#[test]
fn ctrl_p_directional_stubs_report_not_implemented() {
    for second_key in ['h', 'j', 'k', 'l'] {
        let mut ed = editor_from_kitty("-[h]>ello world\n");
        ed.handle_key(key_ctrl('p'));
        ed.handle_key(key(second_key));
        assert_eq!(
            state(&ed),
            "-[h]>ello world\n",
            "Ctrl+p {second_key}: stub must not move cursor",
        );
        assert!(
            ed.status_msg
                .as_deref()
                .unwrap_or("")
                .contains("not yet implemented"),
            "Ctrl+p {second_key}: stub must report not-yet-implemented: {:?}",
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

/// Without kitty, Ctrl+w is a no-op. Ctrl+w has no explicit trie binding
/// (the pane prefix is Ctrl+p), so without kitty the strip-CONTROL fallback
/// is suppressed and the key is silently ignored.
#[test]
fn legacy_ctrl_w_is_noop() {
    let mut ed = editor_from("-[hello]> world foo\n");
    ed.handle_key(key_ctrl('w'));
    assert_eq!(state(&ed), "-[hello]> world foo\n");
}

/// Ctrl+u runs half-page-up (explicit leaf binding) and must not run undo.
/// Undo is bound to bare 'u'; the explicit Ctrl+u binding takes priority so
/// the strip-CONTROL fallback (which would reach 'u' = undo) is never reached.
#[test]
fn kitty_ctrl_u_is_not_undo() {
    let mut ed = editor_from_kitty("-[h]>ello\n");
    // Make an edit so undo would have something to revert.
    ed.handle_key(key('d'));
    assert_eq!(ed.doc().text().to_string(), "ello\n");
    // Ctrl+u runs half-page-up (scroll), not undo — text must be unchanged.
    ed.handle_key(key_ctrl('u'));
    assert_eq!(
        ed.doc().text().to_string(),
        "ello\n",
        "Ctrl+u must not run undo"
    );
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
    ed.handle_key(key('u')); // regular undo
    assert_eq!(ed.doc().text().to_string(), "hello\n");
    // Ctrl+U should NOT run redo.
    ed.handle_key(key_ctrl('U'));
    assert_eq!(
        ed.doc().text().to_string(),
        "hello\n",
        "Ctrl+U should be a no-op in kitty mode"
    );
}

// ── Ctrl+d / Ctrl+u — explicit leaf, must NOT extend ─────────────────────

fn scroll_test_editor_kitty() -> Editor {
    use crate::core::selection::{Selection, SelectionSet};
    use crate::core::text::Text;
    // 30 single-char lines — same shape as page_scroll tests.
    // Viewport height = 24 → half-page = 12.
    let content = "a\n".repeat(30);
    let buf = Text::from(content.as_str());
    let sels = SelectionSet::single(Selection::collapsed(0));
    let mut ed = Editor::for_testing(Buffer::new(buf, sels));
    ed.kitty_enabled = true;
    ed
}

/// Ctrl+d scrolls without extending — the selection stays collapsed (anchor == head)
/// even though half-page-down is registered with `.extendable()` for sticky-Extend mode.
/// In Normal mode, pressing an explicit Ctrl+key only extends when the binding
/// carries `force_extend = true` (e.g. Ctrl+x). Scroll commands do not.
#[test]
fn ctrl_d_does_not_extend_in_normal_mode() {
    let mut ed = scroll_test_editor_kitty();
    let before = ed.current_selections().primary();
    assert_eq!(
        before.anchor, before.head,
        "precondition: collapsed selection"
    );

    ed.handle_key(key_ctrl('d'));

    let after = ed.current_selections().primary();
    // Selection must still be collapsed — anchor == head.
    assert_eq!(
        after.anchor, after.head,
        "Ctrl+d must not extend the selection"
    );
    // The cursor must have moved (scroll actually did something).
    assert_ne!(after.head, before.head, "Ctrl+d must move the cursor");
}

/// Ctrl+u scrolls without extending (symmetric with Ctrl+d).
#[test]
fn ctrl_u_does_not_extend_in_normal_mode() {
    let mut ed = scroll_test_editor_kitty();
    // First scroll down so Ctrl+u has room to scroll back up.
    ed.handle_key(key_ctrl('d'));
    ed.handle_key(key_ctrl('u'));

    let after = ed.current_selections().primary();
    assert_eq!(
        after.anchor, after.head,
        "Ctrl+u must not extend the selection"
    );
}

/// In sticky Extend mode (`e`), Ctrl+d DOES extend — the explicit-leaf
/// non-extend rule only applies in Normal mode. Sticky Extend overrides it.
#[test]
fn extend_mode_ctrl_d_extends() {
    let mut ed = scroll_test_editor_kitty();
    let before_anchor = ed.current_selections().primary().anchor;

    ed.handle_key(key('e')); // enter sticky Extend mode
    ed.handle_key(key_ctrl('d'));

    let after = ed.current_selections().primary();
    // In Extend mode the anchor must not have moved.
    assert_eq!(
        after.anchor, before_anchor,
        "anchor must be pinned in Extend mode"
    );
    // And head must have moved (scroll happened).
    assert_ne!(after.head, before_anchor, "head must move in Extend mode");
}
