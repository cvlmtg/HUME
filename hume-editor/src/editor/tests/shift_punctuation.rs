use super::*;
use pretty_assertions::assert_eq;
use termina::event::{KeyCode, Modifiers};

// ── Shifted-punctuation bindings under partial kitty support ──────────────────
//
// On terminals that enable DISAMBIGUATE_ESCAPE_CODES but do not fully honour
// REPORT_ALTERNATE_KEYS (e.g. older WezTerm builds), shifted punctuation like
// `:`, `$`, `?` arrives as Char(x) + SHIFT. Every keymap binding for a printable
// is stored as Char(x) + NONE, so without normalization these keys miss the trie
// and are silently swallowed in Normal/Extend mode.
//
// `handle_normal` strips the redundant SHIFT bit for any Char when SHIFT is the
// only modifier. These tests lock that behaviour for the regression class
// reported when running HUME on WezTerm (where `:` did nothing but `i`/`a`
// worked and `:` typed fine in Insert mode).

/// A shifted printable: Char(ch) + SHIFT, mirroring what the decoder delivers
/// on terminals with DISAMBIGUATE but incomplete REPORT_ALTERNATE_KEYS.
fn key_shift(ch: char) -> KeyEvent {
    KeyEvent::new(KeyCode::Char(ch), Modifiers::SHIFT)
}

/// `:` delivered as Char(':') + SHIFT must still enter command mode.
#[test]
fn colon_enters_command_mode_when_shift_set() {
    let mut ed = editor_from("-[h]>ello\n");
    ed.handle_key(key_shift(':'));
    assert_eq!(ed.state.mode, Mode::Command);
    assert!(ed.state.minibuf.is_some());
    assert_eq!(ed.state.minibuf.as_ref().unwrap().prompt, ":");
    assert_eq!(ed.state.minibuf.as_ref().unwrap().input, "");
}

/// Broadening the SHIFT strip to all chars must not break the alphabetic
/// case. `A` delivered as Char('A') + SHIFT must resolve to the `A` binding
/// (insert-at-line-end), entering Insert mode.
#[test]
fn shift_a_still_inserts_at_line_end() {
    let mut ed = editor_from("-[h]>ello world\n");
    ed.handle_key(key_shift('A'));
    assert_eq!(ed.state.mode, Mode::Insert);
    // Mirrors `capital_a_enters_insert_after_end_of_line` in commands.rs: `A`
    // lands on the newline at end of line.
    assert_eq!(state(&ed), "hello world-[\n]>");
}

/// `}` delivered as Char('}') + SHIFT must still run next-paragraph.
#[test]
fn close_brace_next_paragraph_when_shift_set() {
    let mut ed = editor_from("-[h]>ello\n\nworld\n");
    ed.handle_key(key_shift('}'));
    // next-paragraph moves head to the first char of the next paragraph.
    assert_eq!(state(&ed), "hello\n\n-[w]>orld\n");
}

/// `?` delivered as Char('?') + SHIFT must still enter backward search.
#[test]
fn question_enters_search_when_shift_set() {
    let mut ed = editor_from("ab ab -[a]>b\n");
    ed.handle_key(key_shift('?'));
    assert_eq!(ed.state.mode, Mode::Search);
    for ch in "ab".chars() {
        ed.handle_key(key(ch));
    }
    ed.handle_key(key_enter());
    assert_eq!(ed.state.mode, Mode::Normal);
    assert_eq!(state(&ed), "ab -[ab]> ab\n");
}

/// Regression guard: Shift+Tab arrives as KeyCode::BackTab + SHIFT (not a
/// Char), so the SHIFT strip must leave it untouched and the completion
/// back-cycle stays intact.
#[test]
fn shift_tab_still_backtab() {
    let mut ed = editor_from("-[h]>ello\n");
    ed.handle_key(key(':'));
    ed.handle_key(key('w'));
    // Open popup (first candidate), then Tab forward once.
    ed.handle_key(KeyEvent::new(KeyCode::Tab, Modifiers::NONE));
    ed.handle_key(KeyEvent::new(KeyCode::Tab, Modifiers::NONE));
    // Shift-Tab back to candidate 0.
    ed.handle_key(KeyEvent::new(KeyCode::BackTab, Modifiers::SHIFT));
    let state = ed.state.minibuf_completion.as_ref().unwrap();
    assert_eq!(state.selected, 0);
}
