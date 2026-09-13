//! line_rules.rs — trailing-newline content rules for a/A/c/d and the small single-key selection commands around them.

use super::super::*;
use pretty_assertions::assert_eq;

// ── `a` / `A` trailing-newline content rule ────────────────────────────────────

/// `a` on an empty line must stay on that line (≡ `i`) — not jump to the next.
///
/// An empty line is just a `\n`; the selection ends on that `\n`. Under the
/// content rule `a` does not step past a trailing `\n`.
#[test]
fn a_on_empty_line_stays_on_same_line() {
    // Buffer: "foo\n\nbar\n" — the middle line is empty (char index 4 = '\n').
    let mut ed = editor_from("foo\n-[\n]>bar\n");
    ed.handle_key(key('a'));

    assert_eq!(ed.state.mode, Mode::Insert);
    // Cursor must remain on the \n at position 4, not jump to 'b'.
    assert_eq!(state(&ed), "foo\n-[\n]>bar\n");
}

/// `a` after `x` (select-line) on an interior non-last line must place the
/// cursor on the line's trailing `\n`, not at the start of the next line.
#[test]
fn a_after_select_line_stays_on_same_line() {
    // select-line on 'b' → anchor=4 ('b'), head=7 ('\n').
    // `a`: sel.end()=7, char_at(7)='\n' → stay at 7.
    let mut ed = editor_from("foo\n-[b]>ar\nbaz\n");
    ed.handle_key(key('x')); // select "bar\n" — head on '\n'
    ed.handle_key(key('a'));

    assert_eq!(ed.state.mode, Mode::Insert);
    // Cursor on the trailing '\n' of the line — same line, not on 'b' of next line.
    assert_eq!(state(&ed), "foo\nbar-[\n]>baz\n");
}

/// `A` on an empty line must stay on the `\n` of that line — not step onto
/// the next line. An unconditional `move_right` after `goto_line_end` would
/// advance past the `\n` on empty lines.
#[test]
fn capital_a_on_empty_line_stays_on_same_line() {
    let mut ed = editor_from("foo\n-[\n]>bar\n");
    ed.handle_key(key('A'));

    assert_eq!(ed.state.mode, Mode::Insert);
    assert_eq!(state(&ed), "foo\n-[\n]>bar\n");
}

/// `A` on a non-empty line must still position after the last content character
/// (on the trailing `\n` slot).
#[test]
fn capital_a_on_nonempty_line_is_unchanged() {
    let mut ed = editor_from("-[h]>ello\nworld\n");
    ed.handle_key(key('A'));

    assert_eq!(ed.state.mode, Mode::Insert);
    // Cursor on the \n at position 5 (between "hello" and "world").
    assert_eq!(state(&ed), "hello-[\n]>world\n");
}

// ── `c` trailing-newline content rule ─────────────────────────────────────────

/// `c` on an empty line must not delete anything — the line stays, cursor stays.
/// Equivalent to pressing `i` on an empty line.
#[test]
fn change_on_empty_line_is_noop() {
    let mut ed = editor_from("foo\n-[\n]>bar\n");
    ed.handle_key(key('c'));

    assert_eq!(ed.state.mode, Mode::Insert);
    assert_eq!(
        ed.doc().text().to_string(),
        "foo\n\nbar\n",
        "empty line must survive"
    );
    // Cursor on the \n (empty line).
    assert_eq!(state(&ed), "foo\n-[\n]>bar\n");
}

/// `c` after `x` (select-line) on an interior line clears the content but keeps
/// the line — `c` rewrites a line, not deletes it.
#[test]
fn change_after_select_line_keeps_line() {
    let mut ed = editor_from("foo\n-[b]>ar\nbaz\n");
    ed.handle_key(key('x')); // selects "bar\n" (head on \n)
    ed.handle_key(key('c'));

    assert_eq!(ed.state.mode, Mode::Insert);
    // "bar" deleted, \n kept → line 1 is now empty; cursor at line start.
    assert_eq!(
        ed.doc().text().to_string(),
        "foo\n\nbaz\n",
        "line must be kept"
    );
    assert_eq!(state(&ed), "foo\n-[\n]>baz\n");
}

/// Multi-line `c`: interior `\n`s are deleted normally; only the final `\n` is
/// kept, collapsing the selection to a single empty line.
#[test]
fn change_multi_line_collapses_to_one_empty_line() {
    // Selection covers "bar\nbaz\n" (anchor=4, head=11 on the last '\n').
    // change_span: sel.end()=11, char_at(11)='\n' → stop=11.
    // Deletes chars 4..11 = "bar\nbaz". Buffer → "foo\n\n".
    let mut ed = editor_from("foo\n-[bar\nbaz\n]>");
    ed.handle_key(key('c'));

    assert_eq!(ed.state.mode, Mode::Insert);
    assert_eq!(
        ed.doc().text().to_string(),
        "foo\n\n",
        "two lines become one empty line"
    );
    assert_eq!(state(&ed), "foo\n-[\n]>");
}

/// `c` on a plain (non-`\n`) char still deletes it — regression guard.
#[test]
fn change_on_content_char_still_deletes() {
    let mut ed = editor_from("-[h]>ello\n");
    ed.handle_key(key('c'));

    assert_eq!(ed.state.mode, Mode::Insert);
    assert_eq!(ed.doc().text().to_string(), "ello\n");
    assert_eq!(state(&ed), "-[e]>llo\n");
}

/// After `c` of a line-selected region, the kill ring must contain only the
/// line content — no trailing `\n`.
#[test]
fn change_kill_ring_excludes_trailing_newline() {
    let mut ed = editor_from("-[b]>ar\n");
    ed.handle_key(key('x')); // select "bar\n" (head on \n)
    ed.handle_key(key('c'));

    assert_eq!(
        ed.state.kill_ring.head(),
        Some(["bar".to_string()].as_slice()),
        "kill ring must hold content only, no trailing newline"
    );
}

/// `d` after `x` (select-line) still removes the whole line including its `\n`
/// — regression guard ensuring `d` was not affected by the `c`-only change.
#[test]
fn d_after_select_line_removes_entire_line() {
    let mut ed = editor_from("foo\n-[b]>ar\nbaz\n");
    ed.handle_key(key('x')); // selects "bar\n" (head on \n)
    ed.handle_key(key('d'));

    assert_eq!(
        ed.doc().text().to_string(),
        "foo\nbaz\n",
        "whole line including \\n must be deleted"
    );
    assert_eq!(
        ed.state.kill_ring.head(),
        Some(["bar\n".to_string()].as_slice()),
        "kill ring holds full line including \\n"
    );
}

// ── `d` / `xd` on blank last line ────────────────────────────────────────────

/// `d` on a blank last line must delete it, not silently no-op.
///
/// A blank last line is a collapsed cursor on the structural trailing `\n`.
/// Before the fix, `delete_one_grapheme` would no-op because the cursor is
/// already on the structural `\n`. After the fix it routes through
/// `delete_sel_region`'s merge path, consuming the preceding `\n`.
#[test]
fn d_on_blank_last_line_removes_it() {
    let mut ed = editor_from("foo\n-[\n]>");
    ed.handle_key(key('d'));

    assert_eq!(
        state(&ed),
        "-[f]>oo\n",
        "blank last line must be removed, cursor on first line"
    );
    assert_eq!(
        ed.state.kill_ring.head(),
        Some(["\n".to_string()].as_slice()),
        "kill ring holds the blank line"
    );
}

/// `x` on a blank last line leaves a collapsed selection (existing behaviour);
/// the subsequent `d` must still remove the blank line.
#[test]
fn xd_on_blank_last_line_removes_it() {
    let mut ed = editor_from("foo\n-[\n]>");
    ed.handle_key(key('x')); // collapsed selection stays on structural '\n'
    ed.handle_key(key('d'));

    assert_eq!(
        state(&ed),
        "-[f]>oo\n",
        "xd on blank last line must remove it"
    );
}

/// Reported regression: file ends in two blank lines; `d` on the last one
/// must remove exactly one blank line, leaving the other intact.
#[test]
fn d_on_last_of_two_blank_lines_removes_one() {
    let mut ed = editor_from("foo\n\n-[\n]>");
    ed.handle_key(key('d'));

    assert_eq!(
        state(&ed),
        "foo\n-[\n]>",
        "one blank line removed, second (now last) blank line intact"
    );
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
    let mut ed = editor_from_kitty("-[h]>ello -[w]>orld\n");

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

// ── `o` in extend mode ─────────────────────────────────────────────────────────

/// The default Extend override trie is empty, so `o` in Extend mode falls
/// through to the Normal trie (with extend=true) like any other unbound-in-Extend
/// key — same as `o` in Normal mode, `open-line-below`. The vim-style flip
/// alias lives only in `core:vim-keybind` (see `tests/vim_keybind.rs`).
#[test]
fn o_in_extend_mode_falls_through_to_open_line_below() {
    let mut ed = editor_from("-[hell]>o\n");
    ed.state.mode = Mode::Extend;

    ed.handle_key(key('o'));

    assert_eq!(ed.state.mode, Mode::Insert);
    assert_eq!(ed.doc().text().to_string(), "hello\n\n");
}

#[test]
fn o_in_normal_mode_still_opens_line_below() {
    let mut ed = editor_from("-[h]>ello\n");
    // extend is off (default).

    ed.handle_key(key('o'));

    assert_eq!(ed.state.mode, Mode::Insert);
    assert_eq!(ed.doc().text().to_string(), "hello\n\n");
}

// ── `Ctrl+e` flips the selection in Normal AND Extend mode ───────────────────

/// `Ctrl+e` in Normal mode must swap anchor and head. This works on legacy
/// terminals because `Ctrl+e` emits 0x05.
#[test]
fn ctrl_e_in_normal_mode_flips_selection() {
    let mut ed = editor_from("-[hell]>o\n");
    // Normal mode (the default) — no Extend active.

    ed.handle_key(key_ctrl('e'));

    // anchor and head are swapped; selection is now backward.
    assert_eq!(state(&ed), "<[hell]-o\n");
    // Normal mode stays; flip does not enter or exit Extend.
    assert_eq!(ed.state.mode, Mode::Normal);
}

/// `Ctrl+e` in Extend mode also flips (it falls through to the Normal trie with
/// extend=true; `cmd_flip_selections` ignores MotionMode). Extend mode must
/// remain active after the flip.
#[test]
fn ctrl_e_in_extend_mode_flips_selection() {
    let mut ed = editor_from("-[hell]>o\n");
    ed.state.mode = Mode::Extend;

    ed.handle_key(key_ctrl('e'));

    assert_eq!(state(&ed), "<[hell]-o\n");
    assert_eq!(ed.state.mode, Mode::Extend);
}

// ── `;` collapses selection AND clears extend mode ─────────────────────────

/// `;` must (a) collapse every selection to its head and (b) clear the
/// `extend` flag. The extend side-effect only exists in the mapping — a pure
/// `cmd_collapse_selection_to_head` test cannot see it.
#[test]
fn semicolon_collapses_selection_and_resets_extend() {
    let mut ed = editor_from("-[hell]>o\n");
    ed.state.mode = Mode::Extend;

    ed.handle_key(key(';'));

    assert_eq!(ed.state.mode, Mode::Normal, "extend cleared by ';'");
    // head of the original selection was 'l' (last char of "hell").
    assert_eq!(state(&ed), "hel-[l]>o\n");
}

// ── `Ctrl+;` collapses selection to anchor AND clears extend mode ─────────────

/// `Ctrl+;` must (a) collapse every selection to its anchor and (b) clear the
/// `extend` flag — the exact mirror of `;` with `head` replaced by `anchor`.
#[test]
fn ctrl_semicolon_collapses_to_anchor_and_resets_extend() {
    let mut ed = editor_from_kitty("-[hell]>o\n");
    ed.state.mode = Mode::Extend;

    ed.handle_key(key_ctrl(';'));

    assert_eq!(ed.state.mode, Mode::Normal, "extend cleared by 'Ctrl+;'");
    // anchor of the original selection was 'h' (offset 0).
    assert_eq!(state(&ed), "-[h]>ello\n");
}
