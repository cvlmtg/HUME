use super::*;
use pretty_assertions::assert_eq;

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

