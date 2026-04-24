use super::*;
use pretty_assertions::assert_eq;

// ── Auto-pairs integration tests ──────────────────────────────────────────────

/// Typing `(` before a word character inserts only `(` (context-aware gating).
/// Typing `(` before whitespace or a close char inserts `()`.
#[test]
fn auto_pairs_auto_close() {
    // Before a word char: no auto-close.
    let mut ed = editor_from("-[h]>ello\n");
    ed.handle_key(key('i')); // enter insert at 'h'
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
    ed.handle_key(key('(')); // inserts `()`, cursor on `)`
    ed.handle_key(key(')')); // skip-close: moves cursor past `)`
    assert_eq!(state(&ed), "()-[h]>ello\n");
}

/// Backspace between an empty pair `()` deletes both brackets.
#[test]
fn auto_pairs_auto_delete() {
    let mut ed = editor_from("-[h]>ello\n");
    ed.handle_key(key('i'));
    ed.handle_key(key('(')); // buffer: `(|)hello` — cursor on `)`
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
    ed.handle_key(key('"')); // inserts `""`, cursor on second `"`
    ed.handle_key(key('"')); // skip-close: cursor moves past `"`
    assert_eq!(state(&ed), "\"\"-[x]>\n");
}

/// Typing `)` when the next character is NOT `)` inserts a literal `)`.
#[test]
fn auto_pairs_no_false_skip() {
    let mut ed = editor_from("-[h]>ello\n");
    ed.handle_key(key('i'));
    ed.handle_key(key(')')); // `)` is not already there — insert normally
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
    ed.handle_key(key(')')); // fallback: inserts `)` at both positions
    assert_eq!(state(&ed), "()-[)]>a)-[b]>c\n");
}

/// Multi-cursor delete-pair is all-or-nothing: if one cursor is between a pair
/// and another is not, backspace falls back to plain delete-char-backward for all.
#[test]
fn auto_pairs_auto_delete_mixed_cursors() {
    // cursor 1 between `()`, cursor 2 between `a`+`b` (not a pair).
    let mut ed = editor_from("(-[)]>a-[b]>c\n");
    ed.handle_key(key('i'));
    ed.handle_key(key_backspace()); // fallback: each cursor deletes one char backward
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
