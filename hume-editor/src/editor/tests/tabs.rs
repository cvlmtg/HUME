use super::*;
use hume_editing::tab_style::TabStyle;
use pretty_assertions::assert_eq;

// ── Insert-mode Tab handling ──────────────────────────────────────────────────

/// Default (Hard) Tab in insert mode inserts a literal `\t`.
#[test]
fn insert_tab_hard_default() {
    let mut ed = editor_from("-[h]>ello\n");
    ed.handle_key(key('i')); // enter insert at 'h'
    ed.handle_key(key_tab());
    assert_eq!(state(&ed), "\t-[h]>ello\n");
}

/// `tab-style = soft` makes Tab insert spaces to the next tab stop.
#[test]
fn insert_tab_soft_setting() {
    let mut ed = editor_from("-[h]>ello\n");
    ed.state.settings.tab_style = TabStyle::Soft;
    ed.handle_key(key('i'));
    ed.handle_key(key_tab());
    assert_eq!(state(&ed), "    -[h]>ello\n");
}

/// Soft tab at column 2 inserts 2 spaces (to reach the col-4 stop), tw=4.
#[test]
fn insert_tab_soft_mid_line() {
    let mut ed = editor_from("he-[l]>lo\n");
    ed.state.settings.tab_style = TabStyle::Soft;
    ed.handle_key(key('i'));
    ed.handle_key(key_tab());
    assert_eq!(state(&ed), "he  -[l]>lo\n");
}

/// `tab-width` controls soft-tab spacing; tw=8 at col 0 → 8 spaces.
#[test]
fn insert_tab_soft_width_8() {
    let mut ed = editor_from("-[h]>i\n");
    ed.state.settings.tab_style = TabStyle::Soft;
    ed.state.settings.tab_width = 8;
    ed.handle_key(key('i'));
    ed.handle_key(key_tab());
    assert_eq!(state(&ed), "        -[h]>i\n");
}

/// Per-buffer `tab-style` override wins over the global default.
#[test]
fn insert_tab_soft_buffer_override() {
    let mut ed = editor_from("-[h]>ello\n");
    // Global stays Hard; buffer overrides to Soft.
    ed.doc_mut().overrides.tab_style = Some(TabStyle::Soft);
    ed.handle_key(key('i'));
    ed.handle_key(key_tab());
    assert_eq!(state(&ed), "    -[h]>ello\n");
}

/// Dot-repeat replays a hard Tab inserted in an insert session.
#[test]
fn dot_repeat_replays_tab() {
    let mut ed = editor_from("-[h]>ello\n");
    ed.handle_key(key('i')); // insert at 'h'
    ed.handle_key(key_tab()); // insert \t
    ed.handle_key(key_esc()); // back to normal, cursor on 'h'
    // Move right then dot-repeat: should insert another \t before 'e'.
    ed.handle_key(key('l')); // cursor on 'e'
    ed.handle_key(key('.')); // repeat last edit
    assert_eq!(state(&ed), "\th\t-[e]>llo\n");
}

// ── Auto-indent on Enter ──────────────────────────────────────────────────────

/// Enter on an indented line copies the indent to the new line.
#[test]
fn enter_copies_tab_indent() {
    let mut ed = editor_from("\t-[f]>oo\n");
    ed.handle_key(key('i'));
    ed.handle_key(key_enter());
    assert_eq!(state(&ed), "\t\n\t-[f]>oo\n");
}

/// Enter on a space-indented line copies the spaces.
#[test]
fn enter_copies_space_indent() {
    let mut ed = editor_from("    -[b]>ar\n");
    ed.handle_key(key('i'));
    ed.handle_key(key_enter());
    assert_eq!(state(&ed), "    \n    -[b]>ar\n");
}

/// Enter on a line with no indent produces a bare newline.
#[test]
fn enter_no_indent() {
    let mut ed = editor_from("fo-[o]>\n");
    ed.handle_key(key('i'));
    ed.handle_key(key_enter());
    assert_eq!(state(&ed), "fo\n-[o]>\n");
}

/// Enter in the middle of content preserves content before the cursor on the
/// old line and copies indent to the new line.
#[test]
fn enter_mid_content() {
    let mut ed = editor_from("\tfo-[o]>\n");
    ed.handle_key(key('i'));
    ed.handle_key(key_enter());
    assert_eq!(state(&ed), "\tfo\n\t-[o]>\n");
}

/// Repeated Enter on an indented line keeps the indent on each new line.
#[test]
fn enter_repeated_preserves_indent() {
    let mut ed = editor_from("\t-[x]>\n");
    ed.handle_key(key('i'));
    ed.handle_key(key_enter()); // \t\n\tx
    ed.handle_key(key_enter()); // another line, same indent
    assert_eq!(state(&ed), "\t\n\t\n\t-[x]>\n");
}

// ── Vim autoindent parity: trim on Insert-mode exit ───────────────────────────
//
// `end_insert_session` vacates a blank line's leading whitespace on Esc, but
// only when *this* insert session auto-inserted it via Enter and nothing has
// been typed there since (`EditorState::autoindent_pending`, code review
// fix #3). These tests pin the three-way distinction vim makes: pre-existing
// blank-line whitespace and hand-typed whitespace are both left alone; only
// the session's own auto-indent gets vacated.

/// Cursor lands on a blank, already-indented line that existed before this
/// insert session touched it: `i` then immediate `Esc` must leave it as-is.
#[test]
fn i_esc_on_pre_existing_blank_line_does_not_trim() {
    let mut ed = editor_from("x\n  -[\n]>");
    ed.handle_key(key('i'));
    ed.handle_key(key_esc());
    assert_eq!(state(&ed), "x\n  -[\n]>");
}

/// Typing whitespace by hand onto an empty line, then `Esc`: vim keeps
/// hand-typed whitespace — only auto-inserted indent is vacated.
#[test]
fn i_type_spaces_esc_does_not_trim_hand_typed_whitespace() {
    let mut ed = editor_from("x\n-[\n]>");
    ed.handle_key(key('i'));
    ed.handle_key(key(' '));
    ed.handle_key(key(' '));
    ed.handle_key(key_esc());
    assert_eq!(state(&ed), "x\n  -[\n]>");
}

/// Contrast with the two tests above: when Enter itself creates the blank,
/// indented line (copying "  " from the line above), an immediate `Esc`
/// with nothing typed DOES vacate that indent — the session's own auto-indent,
/// not pre-existing or hand-typed content.
#[test]
fn enter_esc_trims_auto_inserted_blank_line() {
    let mut ed = editor_from("  x-[\n]>");
    ed.handle_key(key('i'));
    ed.handle_key(key_enter());
    assert_eq!(state(&ed), "  x\n  -[\n]>");
    ed.handle_key(key_esc());
    assert_eq!(state(&ed), "  x\n-[\n]>");
}

/// Dot-repeat replays an Enter-then-Esc insert session as a unit: the
/// replayed Enter must also be trimmed on the replayed Esc. `autoindent_
/// pending` lives on `EditorState` (not `InsertSession`, which is absent
/// during replay — see its doc comment) specifically so this holds.
#[test]
fn dot_repeat_replays_enter_esc_trim() {
    // Cursor starts on line 0's own trailing '\n' ("  x\n"); line 1 ("  y\n")
    // is shaped the same way for the replay target.
    let mut ed = editor_from("  x-[\n]>  y\n");

    ed.feed_key(key('i')); // insert-at-selection-start; cursor stays put
    ed.feed_key(key_enter()); // auto-indent creates a blank "  " line, cursor on its '\n'
    ed.feed_key(key_esc()); // trimmed: "  x\n\n  y\n", cursor on the blank line's '\n'
    assert_eq!(state(&ed), "  x\n-[\n]>  y\n");

    // Navigate to line 2's own trailing '\n' with plain motions (not a
    // selection-establishing command) — dot-repeat replays the ORIGINAL
    // entry command ('i') at whatever selection exists when '.' is pressed.
    ed.feed_key(key('j')); // down, col 0, onto "  y\n"'s first space
    ed.feed_key(key('l')); // second space
    ed.feed_key(key('l')); // 'y'
    ed.feed_key(key('l')); // onto "  y\n"'s own trailing '\n'
    ed.feed_key(key('.')); // replay: insert-at-selection-start, Enter, Esc

    // Same shape as the first site: the replayed Enter's auto-indent on
    // line 2 was trimmed by the replayed Esc.
    assert_eq!(state(&ed), "  x\n\n  y\n-[\n]>");
}

// ── Dedent on Backspace ───────────────────────────────────────────────────────

/// Backspace in leading whitespace snaps to the previous tab stop.
#[test]
fn backspace_dedents_spaces_to_zero() {
    // "    x" cursor on 'x' (col 4) → delete all 4 spaces.
    let mut ed = editor_from("    -[x]>\n");
    ed.handle_key(key('i'));
    ed.handle_key(key_backspace());
    assert_eq!(state(&ed), "-[x]>\n");
}

#[test]
fn backspace_dedents_six_spaces_to_four() {
    // "      x" (6 spaces) cursor on 'x' (col 6) → delete 2 spaces (to col 4).
    let mut ed = editor_from("      -[x]>\n");
    ed.handle_key(key('i'));
    ed.handle_key(key_backspace());
    assert_eq!(state(&ed), "    -[x]>\n");
}

#[test]
fn backspace_dedents_hard_tab() {
    // "\t\tx" cursor on 'x' (col 8) → delete 1 tab (to col 4).
    let mut ed = editor_from("\t\t-[x]>\n");
    ed.handle_key(key('i'));
    ed.handle_key(key_backspace());
    assert_eq!(state(&ed), "\t-[x]>\n");
}

#[test]
fn backspace_dedents_soft_tab_indent() {
    // Soft-tab workflow: type 4 spaces, then Backspace should clear all 4.
    let mut ed = editor_from("-[ ]>x\n"); // cursor on a space at col 0
    ed.state.settings.tab_style = TabStyle::Soft;
    ed.handle_key(key('i'));
    ed.handle_key(key_tab()); // insert 4 spaces before the space → "    | x"
    // Cursor now on the original space at col 4. Backspace dedents to col 0.
    ed.handle_key(key_backspace());
    assert_eq!(state(&ed), "-[ ]>x\n");
}

#[test]
fn backspace_in_content_falls_back_to_plain() {
    // "    x\n" cursor on '\n' (end of line) — chars before cursor include
    // 'x', so not all-whitespace → plain backspace deletes 'x'.
    let mut ed = editor_from("    x-[\n]>");
    ed.handle_key(key('i'));
    ed.handle_key(key_backspace());
    assert_eq!(state(&ed), "    -[\n]>");
}

#[test]
fn backspace_on_first_content_char_dedents() {
    // "    x" cursor on 'x' (first content char) — all chars before it are ws,
    // so dedent applies (matches modern editor behaviour).
    let mut ed = editor_from("    -[x]>\n");
    ed.handle_key(key('i'));
    ed.handle_key(key_backspace());
    assert_eq!(state(&ed), "-[x]>\n");
}

#[test]
fn backspace_at_col_zero_plain_delete() {
    // "foo" cursor on 'f' (col 0) — no leading ws → plain backspace is a no-op
    // at buffer start (nothing to delete to the left).
    let mut ed = editor_from("-[f]>oo\n");
    ed.handle_key(key('i'));
    ed.handle_key(key_backspace());
    assert_eq!(state(&ed), "-[f]>oo\n");
}

#[test]
fn backspace_dedent_two_cursors() {
    // Two lines, each "  x", cursor on 'x' (col 2) → dedent each to col 0.
    let mut ed = editor_from("  -[x]>\n  -[y]>\n");
    ed.handle_key(key('i'));
    ed.handle_key(key_backspace());
    assert_eq!(state(&ed), "-[x]>\n-[y]>\n");
}

#[test]
fn backspace_dedent_all_or_nothing() {
    // One cursor in leading ws, one in content → all fall back to plain
    // backspace (the leading-ws cursor does NOT dedent).
    let mut ed = editor_from("  -[x]>\nab-[c]>\n");
    ed.handle_key(key('i'));
    ed.handle_key(key_backspace());
    // Cursor 0 (on 'x', col 2): plain backspace deletes ' ' before it → " x".
    // Cursor 1 (on 'c', col 2): plain backspace deletes 'b' before it → "ac".
    assert_eq!(state(&ed), " -[x]>\na-[c]>\n");
}

#[test]
fn backspace_dedent_mid_indent_with_content() {
    // "  x\n" with the cursor on the second space (col 1): the cursor sits in
    // leading whitespace, content after it is irrelevant. Backspace snaps to
    // the previous tab stop (col 0), deleting one space.
    let mut ed = editor_from(" -[ ]>x\n");
    ed.handle_key(key('i'));
    ed.handle_key(key_backspace());
    assert_eq!(state(&ed), "-[ ]>x\n");
}
