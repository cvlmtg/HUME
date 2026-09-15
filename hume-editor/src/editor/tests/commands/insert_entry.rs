//! insert_entry.rs — `o`/`O` open-line variants and every insert-entry point's cursor/step-back behavior.

use super::super::*;
use pretty_assertions::assert_eq;

// ── `o` / `O` open-line variants ──────────────────────────────────────────

/// `o` must insert a blank line *below* the current line, position the cursor
/// on it, and enter Insert mode — all as a single composed operation.
#[test]
fn o_opens_line_below_and_enters_insert() {
    let mut ed = editor_from("-[h]>ello\n");
    ed.handle_key(key('o'));

    assert_eq!(ed.state.mode, Mode::Insert);
    assert_eq!(ed.doc().text().to_string(), "hello\n\n");
    // Cursor should be on the new blank line (the second '\n').
    assert_eq!(state(&ed), "hello\n-[\n]>");
}

/// `o` + typed text + Enter + Esc must select just the typed text, not the
/// newline Enter inserted — a trailing `\n` is a line terminator, not typed
/// content (see `PaneBufferState::run_ends`'s doc). Verified by yanking the
/// auto-selected span and checking the register text doesn't end in `\n`:
/// `is_register_linewise` (`hume-ops/src/register.rs`) reads exactly that,
/// and a register ending in `\n` is what makes a later `p` paste as a new
/// line instead of inline — same root cause that flips `:format-source`
/// between whole-document and single-range formatting (`is_selection_linewise`,
/// `hume-editing/src/selection/single.rs`).
#[test]
fn o_type_enter_esc_selects_charwise_typed_text() {
    use hume_ops::register::CLIPBOARD_REGISTER;

    let mut ed = editor_from("-[h]>ello\n");
    ed.handle_key(key('o'));
    for ch in "abc".chars() {
        ed.handle_key(key(ch));
    }
    ed.handle_key(key_enter());
    ed.handle_key(key_esc());

    ed.handle_key(key('y'));
    assert_eq!(reg(&ed, CLIPBOARD_REGISTER), &["abc"]);
}

/// `o` on a blank line must open a new blank line *below* it, not overshoot
/// into the line after.
/// Regression: `goto_line_end + move_right` advanced past the `\n` on empty
/// lines, inserting the new `\n` one line too low.
#[test]
fn o_on_empty_line_places_cursor_on_new_blank_line() {
    let mut ed = editor_from("AAA\nBBB\n-[\n]>CCC\n");
    ed.handle_key(key('o'));

    assert_eq!(ed.state.mode, Mode::Insert);
    assert_eq!(ed.doc().text().to_string(), "AAA\nBBB\n\n\nCCC\n");
    assert_eq!(state(&ed), "AAA\nBBB\n\n-[\n]>CCC\n");
}

/// `o` on an indented line carries that line's leading whitespace onto the
/// new line — the same auto-indent rule Enter uses (`insert_newline_indent`).
#[test]
fn o_carries_indent_from_current_line() {
    let mut ed = editor_from("\t-[f]>oo\n");
    ed.handle_key(key('o'));

    assert_eq!(ed.state.mode, Mode::Insert);
    assert_eq!(ed.doc().text().to_string(), "\tfoo\n\t\n");
    assert_eq!(state(&ed), "\tfoo\n\t-[\n]>");
}

/// `o` then a bare Esc must leave a truly empty line, not one with trailing
/// whitespace — vim autoindent parity via `autoindent_pending`, same as
/// Enter's own Esc-trim.
#[test]
fn o_then_esc_trims_unused_indent() {
    let mut ed = editor_from("\t-[f]>oo\n");
    ed.handle_key(key('o'));
    ed.handle_key(key_esc());

    assert_eq!(ed.doc().text().to_string(), "\tfoo\n\n");
}

/// `o` then Enter must keep the indent on the freshly typed-on line and trim
/// it from the line `o` itself opened (now vacated), matching Enter's own
/// repeated-Enter behavior.
#[test]
fn o_then_enter_keeps_indent_on_new_line_and_trims_first() {
    let mut ed = editor_from("\t-[f]>oo\n");
    ed.handle_key(key('o'));
    ed.handle_key(key_enter());

    assert_eq!(ed.doc().text().to_string(), "\tfoo\n\n\t\n");
}

/// `O` must insert a blank line *above* the current line, position the cursor
/// on it, and enter Insert mode.
#[test]
fn capital_o_opens_line_above_and_enters_insert() {
    let mut ed = editor_from("foo\n-[b]>ar\n");
    ed.handle_key(key('O'));

    assert_eq!(ed.state.mode, Mode::Insert);
    assert_eq!(ed.doc().text().to_string(), "foo\n\nbar\n");
    // Cursor on the new blank line between "foo" and "bar".
    assert_eq!(state(&ed), "foo\n-[\n]>bar\n");
}

/// `O` on an indented line carries that line's leading whitespace onto the
/// new line above it — the line above (unindented) stays untouched.
#[test]
fn capital_o_carries_indent_from_current_line() {
    let mut ed = editor_from("    foo\n    -[b]>ar\n");
    ed.handle_key(key('O'));

    assert_eq!(ed.state.mode, Mode::Insert);
    assert_eq!(ed.doc().text().to_string(), "    foo\n    \n    bar\n");
    assert_eq!(state(&ed), "    foo\n    -[\n]>    bar\n");
}

// ── Insert-entry variants position the cursor correctly ────────────────────

/// `a` collapses to one past the end of the selection and enters Insert mode.
/// On a collapsed cursor this is identical to a plain "append after cursor".
#[test]
fn a_enters_insert_after_selection_end() {
    let mut ed = editor_from("-[h]>ello\n");
    ed.handle_key(key('a'));

    assert_eq!(ed.state.mode, Mode::Insert);
    assert_eq!(state(&ed), "h-[e]>llo\n");
}

/// `A` must jump to the end of the line and then step one right (onto the
/// newline), then enter Insert mode — "append at end of line".
#[test]
fn capital_a_enters_insert_after_end_of_line() {
    let mut ed = editor_from("-[h]>ello\n");
    ed.handle_key(key('A'));

    assert_eq!(ed.state.mode, Mode::Insert);
    assert_eq!(state(&ed), "hello-[\n]>");
}

/// `I` jumps to the first non-blank character on the line and enters Insert mode.
#[test]
fn capital_i_enters_insert_at_line_start() {
    let mut ed = editor_from("  -[hello]>\n");
    ed.handle_key(key('I'));

    assert_eq!(ed.state.mode, Mode::Insert);
    assert_eq!(state(&ed), "  -[h]>ello\n");
}

/// `i` on a multi-char selection collapses to the selection start (not just the
/// cursor head) and enters Insert mode.
#[test]
fn i_on_wide_selection_collapses_to_start() {
    // Backward selection: head=0 (h), anchor=3 (last l) → start=0.
    let mut ed = editor_from("<[hell]-o\n");
    ed.handle_key(key('i'));

    assert_eq!(ed.state.mode, Mode::Insert);
    assert_eq!(state(&ed), "-[h]>ello\n");
}

/// `a` on a multi-char selection collapses to one past the selection end and
/// enters Insert mode — the cursor lands after the last selected character.
#[test]
fn a_on_wide_selection_collapses_after_end() {
    // Forward selection: anchor=0 (h), head=3 (l) → end=3, one past = 4.
    let mut ed = editor_from("-[hel]>lo\n");
    ed.handle_key(key('a'));

    assert_eq!(ed.state.mode, Mode::Insert);
    assert_eq!(state(&ed), "hel-[l]>o\n");
}

// ── `a` / `A` step-back on Esc ────────────────────────────────────────────────

/// After `a` + typing + Esc the cursor must land on the last typed character,
/// not one position past it. A second `a` should re-enter Insert at the same
/// spot rather than advancing further.
///
/// `select-inserted-text` off: with the default on, a single typed char's
/// auto-selected span (`-[X]>`) renders identically to a stepped-back
/// collapsed cursor on that same char, so the assertion would hold even if
/// `step_back_on_exit` were broken — this test is specifically about the
/// step-back mechanism, so it isolates that path.
#[test]
fn a_esc_steps_cursor_back_to_last_typed_char() {
    let mut ed = editor_from("-[h]>ello\n");
    ed.state.settings.select_inserted_text = false;

    ed.handle_key(key('a')); // cursor → 'e', Insert
    ed.handle_key(key('X'));
    ed.handle_key(key_esc());

    // Cursor must be on 'X', not on 'e' (one past where X was inserted).
    assert_eq!(ed.state.mode, Mode::Normal);
    assert_eq!(state(&ed), "h-[X]>ello\n");
}

/// Regression: `$ a <text> Esc a` must not jump to the next line.
/// After Esc the cursor must sit on the last appended character (on the same
/// line), so that a second `a` re-enters Insert at the end of that line.
///
/// `select-inserted-text` off — see `a_esc_steps_cursor_back_to_last_typed_char`'s
/// doc for why isolating the step-back path (not just typing one char)
/// matters here.
#[test]
fn a_esc_at_end_of_line_does_not_advance_to_next_line() {
    let mut ed = editor_from("-[h]>ello\nworld\n");
    ed.state.settings.select_inserted_text = false;

    ed.handle_key(key('A')); // jump to end of line → '\n', Insert
    ed.handle_key(key('X'));
    ed.handle_key(key_esc());

    // Cursor on 'X' (last appended char), still on line 1.
    assert_eq!(state(&ed), "hello-[X]>\nworld\n");

    // A second `a` must re-enter Insert on the same line, not on 'w'.
    ed.handle_key(key('a'));
    assert_eq!(ed.state.mode, Mode::Insert);
    assert_eq!(state(&ed), "helloX-[\n]>world\n");
}

/// `i` never sets `step_back_on_exit` — with `select-inserted-text` on, its
/// typed run is simply selected on Esc.
#[test]
fn i_esc_selects_the_typed_run() {
    let mut ed = editor_from("-[h]>ello\n");

    ed.handle_key(key('i')); // cursor stays on 'h', Insert
    ed.handle_key(key('X'));
    ed.handle_key(key_esc());

    assert_eq!(ed.state.mode, Mode::Normal);
    assert_eq!(state(&ed), "-[X]>hello\n");
}

/// With `select-inserted-text` off, `i` never steps the cursor back on Esc
/// either (only `a`/`A`/`o`/`O`'s empty-run fallback does) — it leaves the
/// cursor exactly where typing left it.
#[test]
fn i_esc_does_not_step_cursor_back_setting_off() {
    let mut ed = editor_from("-[h]>ello\n");
    ed.state.settings.select_inserted_text = false;

    ed.handle_key(key('i')); // cursor stays on 'h', Insert
    ed.handle_key(key('X'));
    ed.handle_key(key_esc());

    // No step-back: cursor stays one past 'X', on 'h' — not stepped back
    // onto 'X' itself the way `a`/`A`/`o`/`O` would.
    assert_eq!(ed.state.mode, Mode::Normal);
    assert_eq!(state(&ed), "X-[h]>ello\n");
}

/// After `I` + typing + Esc the typed run is selected.
#[test]
fn capital_i_esc_selects_the_typed_run() {
    let mut ed = editor_from("  -[hello]>\n");

    ed.handle_key(key('I'));
    ed.handle_key(key('X'));
    ed.handle_key(key('Y'));
    ed.handle_key(key_esc());

    assert_eq!(state(&ed), "  -[XY]>hello\n");
}

/// After `A` + typing + Esc the typed run is selected, same as `a`.
#[test]
fn capital_a_esc_selects_the_typed_run() {
    let mut ed = editor_from("-[h]>ello\n");

    ed.handle_key(key('A'));
    ed.handle_key(key('X'));
    ed.handle_key(key('Y'));
    ed.handle_key(key_esc());

    assert_eq!(state(&ed), "hello-[XY]>\n");
}

/// `a` + immediate Esc (nothing typed): the empty-run fallback steps the
/// cursor back one grapheme — the only case `step_back_on_exit` still governs.
#[test]
fn a_esc_with_nothing_typed_steps_back() {
    let mut ed = editor_from("-[h]>ello\n");

    ed.handle_key(key('a')); // cursor → 'e'
    ed.handle_key(key_esc());

    assert_eq!(state(&ed), "-[h]>ello\n");
}

/// `A` + immediate Esc (nothing typed): same empty-run fallback as `a`.
#[test]
fn capital_a_esc_with_nothing_typed_steps_back() {
    let mut ed = editor_from("-[h]>ello\n");

    ed.handle_key(key('A')); // cursor → trailing '\n'
    ed.handle_key(key_esc());

    assert_eq!(state(&ed), "hell-[o]>\n");
}

// ── Multi-cursor auto-select on Esc ─────────────────────────────────────────

/// `i` on two cursors: each typed run is selected independently on Esc.
#[test]
fn i_multi_cursor_selects_each_typed_run() {
    let mut ed = editor_from("-[foo]> -[bar]>\n");
    ed.handle_key(key('i'));
    ed.handle_key(key('x'));
    ed.handle_key(key('y'));
    ed.handle_key(key_esc());
    assert_eq!(state(&ed), "-[xy]>foo -[xy]>bar\n");
}

// ── `o` / `O` step-back on Esc ───────────────────────────────────────────────

/// After `o` + typing + Esc the typed run is selected — not just a cursor on
/// the last character, and not on the trailing `\n` of the new line.
///
/// Regression: without `mark_insert_step_back`'s empty-run fallback, `o` +
/// immediate `Esc` (nothing typed) would select the *next* line's `\n`
/// rather than staying on the just-created blank one — see
/// `o_esc_on_empty_line_does_not_step_to_previous_line` for that case.
#[test]
fn o_esc_selects_the_typed_run() {
    let mut ed = editor_from("-[h]>ello\nworld\n");

    ed.handle_key(key('o'));
    ed.handle_key(key('a'));
    ed.handle_key(key('b'));
    ed.handle_key(key('c'));
    ed.handle_key(key_esc());

    assert_eq!(ed.state.mode, Mode::Normal);
    assert_eq!(state(&ed), "hello\n-[abc]>\nworld\n");
}

/// With `select-inserted-text` off, `o` + typing + Esc still steps the
/// cursor back to the last typed character (same as `a`), not the typed run.
#[test]
fn o_esc_steps_cursor_back_to_last_typed_char_setting_off() {
    let mut ed = editor_from("-[h]>ello\nworld\n");
    ed.state.settings.select_inserted_text = false;

    ed.handle_key(key('o'));
    ed.handle_key(key('a'));
    ed.handle_key(key('b'));
    ed.handle_key(key('c'));
    ed.handle_key(key_esc());

    // Cursor on 'c', not on the new line's trailing '\n'.
    assert_eq!(ed.state.mode, Mode::Normal);
    assert_eq!(state(&ed), "hello\nab-[c]>\nworld\n");
}

/// After `O` + typing + Esc the typed run is selected.
#[test]
fn capital_o_esc_selects_the_typed_run() {
    let mut ed = editor_from("hello\n-[w]>orld\n");

    ed.handle_key(key('O'));
    ed.handle_key(key('a'));
    ed.handle_key(key('b'));
    ed.handle_key(key('c'));
    ed.handle_key(key_esc());

    assert_eq!(ed.state.mode, Mode::Normal);
    assert_eq!(state(&ed), "hello\n-[abc]>\nworld\n");
}

/// With `select-inserted-text` off, `O` + typing + Esc still steps the
/// cursor back to the last typed character.
#[test]
fn capital_o_esc_steps_cursor_back_to_last_typed_char_setting_off() {
    let mut ed = editor_from("hello\n-[w]>orld\n");
    ed.state.settings.select_inserted_text = false;

    ed.handle_key(key('O'));
    ed.handle_key(key('a'));
    ed.handle_key(key('b'));
    ed.handle_key(key('c'));
    ed.handle_key(key_esc());

    // Cursor on 'c', not on the new line's trailing '\n'.
    assert_eq!(ed.state.mode, Mode::Normal);
    assert_eq!(state(&ed), "hello\nab-[c]>\nworld\n");
}

/// `o` + immediate Esc (nothing typed): cursor must stay on the new blank
/// line's `\n` and must NOT step back onto the preceding line.
#[test]
fn o_esc_on_empty_line_does_not_step_to_previous_line() {
    let mut ed = editor_from("-[h]>ello\nworld\n");

    ed.handle_key(key('o'));
    ed.handle_key(key_esc());

    // New blank line inserted; cursor on its '\n' (head == line_start so no
    // step-back occurs — the empty-line guard in end_insert_session applies).
    assert_eq!(ed.state.mode, Mode::Normal);
    assert_eq!(state(&ed), "hello\n-[\n]>world\n");
}

// ── multi-cursor `a` collision (merge edge cases) ─────────────────────────────

/// `a` on two cursors where one sits on the last character and the other on the
/// trailing `\n`: both land on the `\n` and must merge to one cursor.
///
/// - cursor on c(2): char_at(2)='c' → `next(2)=3`.
/// - cursor on \n(3): char_at(3)='\n' → stays at 3.
///
/// Both land on 3 → merge → single cursor on \n.
///
/// Regression: without `map` merging after the transform, this leaves two
/// identical collapsed selections — a `SelectionSet` invariant violation.
#[test]
fn a_multi_cursor_clamp_collision_merges_to_one() {
    let mut ed = editor_from("ab-[c]>-[\n]>");
    ed.handle_key(key('a'));

    assert_eq!(ed.state.mode, Mode::Insert);
    assert_eq!(state(&ed), "abc-[\n]>");
}

/// `a Esc` on two cursors where one is on a `\n` and the other on a content char:
/// the `\n`-cursor stays put (on the `\n`) and the content-char cursor advances
/// one grapheme. No collision — they end up on distinct lines after step-back.
#[test]
fn a_esc_newline_cursor_stays_on_its_line() {
    // "ab\ncd\n": a=0 b=1 \n=2 c=3 d=4 \n=5.
    // `a`: \n(2) → stays 2 (it is a \n); c(3) → next(3)=4. Cursors at 2, 4.
    // Esc step-back: head=2, line_start=0, 2>0 → prev(2)=1 (b).
    //                head=4, line_start=3, 4>3 → prev(4)=3 (c).
    let mut ed = editor_from("ab-[\n]>-[c]>d\n");
    ed.handle_key(key('a'));
    ed.handle_key(key_esc());

    assert_eq!(ed.state.mode, Mode::Normal);
    assert_eq!(state(&ed), "a-[b]>\n-[c]>d\n");
}
