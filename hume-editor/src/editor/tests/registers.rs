use super::*;
use pretty_assertions::assert_eq;

// ── Register prefix `"<reg>` ────────────────────────────────────────────────

/// `"5y` must write text into register '5', leaving `'"'` empty.
#[test]
fn register_prefix_routes_yank_to_named_register() {
    let mut ed = editor_from("-[hell]>o\n");
    ed.handle_key(key('"'));
    ed.handle_key(key('5'));
    ed.handle_key(key('y'));

    assert_eq!(state(&ed), "-[hell]>o\n", "buffer unchanged");
    assert_eq!(reg(&ed, '5'), &["hell"], "register '5' populated");
    assert!(reg(&ed, '"').is_empty(), "'\"' register untouched");
}

/// After `"5y`, the prefix is consumed. The next bare `y` writes to clipboard
/// and the kill ring (not to register '5').
#[test]
fn register_prefix_clears_after_one_operation() {
    use hume_ops::register::CLIPBOARD_REGISTER;

    let mut ed = editor_from("-[hell]>o\n");
    ed.handle_key(key('"'));
    ed.handle_key(key('5'));
    ed.handle_key(key('y'));

    // Now the prefix is cleared. Move right to get a different selection,
    // then yank again without a prefix.
    ed.handle_key(key('l')); // move right
    ed.handle_key(key('y')); // bare yank: writes clipboard + kill ring

    // The second yank updated the clipboard, not register '5'.
    assert!(
        !reg(&ed, CLIPBOARD_REGISTER).is_empty(),
        "clipboard written by bare y"
    );
    // Kill ring head holds the latest bare yank.
    assert!(
        ed.state.kill_ring.head().is_some(),
        "kill ring head set by bare y"
    );
    // '5' is unchanged from the first yank.
    assert_eq!(reg(&ed, '5'), &["hell"], "register '5' unchanged");
}

/// `i` (and `a`/`o`) write no register, so they clear a pending `"<reg>`
/// prefix: an operator after Insert exits must not write into a register
/// armed before the insert.
#[test]
fn insert_session_clears_register_prefix_on_a_writable_buffer() {
    let mut ed = editor_from("-[hell]>o\n");
    ed.handle_key(key('"'));
    ed.handle_key(key('3'));
    ed.handle_key(key('i'));
    ed.handle_key(key_esc());
    ed.handle_key(key('d'));

    assert!(
        reg(&ed, '3').is_empty(),
        "\"3i<Esc>d must not route the delete into register '3'"
    );
    assert!(
        ed.state.kill_ring.head().is_some(),
        "the delete must fall back to the kill ring"
    );
}

/// `Esc` after `"` cancels the prefix; the next `y` writes to clipboard + ring.
#[test]
fn esc_cancels_register_prefix() {
    use hume_ops::register::CLIPBOARD_REGISTER;

    let mut ed = editor_from("-[hell]>o\n");
    ed.handle_key(key('"'));
    ed.handle_key(key_esc()); // cancel
    ed.handle_key(key('y'));

    assert_eq!(
        reg(&ed, CLIPBOARD_REGISTER),
        &["hell"],
        "clipboard populated"
    );
    assert_eq!(
        ed.state.kill_ring.head(),
        Some(ps(&["hell"]).as_slice()),
        "kill ring head populated"
    );
    assert!(reg(&ed, '5').is_empty(), "register '5' untouched");
}

/// The state `keys` leave on a line of six words, from its first char.
fn after_keys(keys: &[KeyEvent]) -> String {
    let mut ed = editor_from("-[o]>ne two three four five six\n");
    for &k in keys {
        ed.handle_key(k);
    }
    state(&ed)
}

/// A count typed before `"` survives a key that names no register: the
/// prefix is cancelled, the count goes to the next command.
#[test]
fn an_invalid_register_name_keeps_the_count() {
    let counted = after_keys(&[key('3'), key('w')]);
    assert_eq!(
        after_keys(&[key('3'), key('"'), key('!'), key('w')]),
        counted
    );
    assert_eq!(
        after_keys(&[key('3'), key('"'), key_ctrl('a'), key('w')]),
        counted
    );
}

/// Esc after `"` resets the count with the prefix, as Esc does in Normal.
#[test]
fn esc_after_register_prefix_clears_the_count() {
    assert_eq!(
        after_keys(&[key('3'), key('"'), key_esc(), key('w')]),
        after_keys(&[key('w')])
    );
}

/// `"by` discards the yank: `'"'` must remain empty.
#[test]
fn black_hole_register_via_prefix() {
    use hume_ops::register::BLACK_HOLE_REGISTER;

    let mut ed = editor_from("-[hell]>o\n");
    ed.handle_key(key('"'));
    ed.handle_key(key('b'));
    ed.handle_key(key('y'));

    assert_eq!(state(&ed), "-[hell]>o\n", "buffer unchanged");
    assert!(reg(&ed, '"').is_empty(), "'\"' register untouched");
    assert!(
        ed.state.registers.read(BLACK_HOLE_REGISTER).is_none(),
        "black hole register returns None"
    );
}

// ── Clipboard register fallback (in-memory mirror) ─────────────────────────

/// When the system clipboard is unavailable, `"cy` falls back to the in-memory
/// mirror and logs a Warning. The mirror is then used by `"cp`.
#[test]
fn clipboard_register_falls_back_to_memory_when_unavailable() {
    use hume_core::Severity;
    use hume_ops::register::CLIPBOARD_REGISTER;

    let mut ed = editor_from("-[hello]>\n");
    // Simulate a headless environment with no clipboard server.
    ed.state.clipboard.force_unavailable();

    ed.handle_key(key('"'));
    ed.handle_key(key('c'));
    ed.handle_key(key('y'));

    // A Warning must have been logged.
    assert!(
        ed.state
            .message_log
            .entries()
            .any(|e| e.severity == Severity::Warning),
        "expected a Warning for clipboard unavailable"
    );

    // In-memory mirror must hold the yanked text.
    assert_eq!(
        reg(&ed, CLIPBOARD_REGISTER),
        &["hello"],
        "in-memory mirror populated"
    );

    // Move right so cursor is now on 'o', giving a distinct selection.
    ed.handle_key(key('l'));

    // `"cp` should read from the in-memory mirror and paste "hello".
    ed.handle_key(key('"'));
    ed.handle_key(key('c'));
    ed.handle_key(key('p'));

    assert!(
        ed.doc().text().to_string().contains("hello"),
        "pasted from in-memory mirror"
    );
}

// ── Kill-ring register (`"k`) ─────────────────────────────────────────────────

/// `"ky` pushes the yank onto the kill ring and does NOT touch the clipboard.
#[test]
fn kill_ring_register_yank_pushes_ring_only() {
    use hume_ops::register::CLIPBOARD_REGISTER;

    let mut ed = editor_from("-[hello]>world\n");
    // Ensure clipboard starts empty.
    assert!(
        reg(&ed, CLIPBOARD_REGISTER).is_empty(),
        "clipboard starts empty"
    );

    ed.handle_key(key('"'));
    ed.handle_key(key('k'));
    ed.feed_key(key('y')); // "ky → ring push, no clipboard

    assert_eq!(
        ed.state.kill_ring.head(),
        Some(ps(&["hello"]).as_slice()),
        "ring head set by \"ky"
    );
    assert!(
        reg(&ed, CLIPBOARD_REGISTER).is_empty(),
        "\"ky must not write the clipboard"
    );
}

/// `"kd` deletes and pushes to the ring, identical to bare `d`.
#[test]
fn kill_ring_register_delete_pushes_ring() {
    let mut ed = editor_from("-[hello]>world\n");

    ed.handle_key(key('"'));
    ed.handle_key(key('k'));
    ed.feed_key(key('d')); // "kd → delete + push ring

    assert_eq!(
        ed.doc().text().to_string(),
        "world\n",
        "buffer after delete"
    );
    assert_eq!(
        ed.state.kill_ring.head(),
        Some(ps(&["hello"]).as_slice()),
        "ring head set by \"kd"
    );
}

// ── Explicit register capture routing ───────────────────────────────────────

/// Kill ring depth: after >10 pushes (via `d`), `len() == 10` and the oldest entry
/// is evicted.  The 11th push displaces the 1st.
#[test]
fn kill_ring_depth_capped_at_ten() {
    // 11 one-char lines: A through K.
    let mut ed = editor_from("-[A]>\nB\nC\nD\nE\nF\nG\nH\nI\nJ\nK\n");
    // Delete each line by repeatedly pressing x then d.
    for _ in 0..11 {
        ed.feed_key(key('x')); // select-line
        ed.feed_key(key('d')); // delete line → push ring
        // After delete, cursor lands on next line automatically.
    }
    assert_eq!(ed.state.kill_ring.len(), 10, "kill ring capped at depth 10");
}

/// Deleting the same word twice moves the existing ring entry to the head
/// instead of taking a second slot.
#[test]
fn repeated_kill_of_same_word_takes_one_ring_slot() {
    let mut ed = editor_from("-[foo]> bar foo baz\n");
    ed.feed_key(key('d')); // delete "foo" on line 1
    ed.feed_key(key('w')); // land on "bar"
    ed.feed_key(key('w')); // land on the second "foo"
    ed.feed_keys([key('m'), key('i'), key('w')]); // narrow to the bare word (whitespace-setting-proof)
    ed.feed_key(key('d')); // delete it: same text, already in the ring
    assert_eq!(
        ed.state.kill_ring.len(),
        1,
        "re-killing identical text must not grow the ring"
    );
}

/// `"cy` writes clipboard only, no kill-ring push.
#[test]
fn explicit_cy_writes_clipboard_only() {
    use hume_ops::register::CLIPBOARD_REGISTER;

    let mut ed = editor_from("-[hello]>\n");
    // Kill the ring beforehand so we can detect any erroneous push.
    ed.feed_key(key('"'));
    ed.feed_key(key('c'));
    ed.feed_key(key('y')); // "cy → clipboard only

    assert_eq!(
        reg(&ed, CLIPBOARD_REGISTER),
        &["hello"],
        "clipboard written"
    );
    assert!(
        ed.state.kill_ring.head().is_none(),
        "kill ring NOT pushed by explicit \"cy"
    );
}

/// `"5y` writes the in-memory named register '5'; kill ring is not touched.
///
/// Digit-register writes route through `write_register` → `registers.write_text`,
/// not through `kill_ring.push`. The in-memory and ring storage are orthogonal.
#[test]
fn explicit_digit_y_writes_in_memory_only() {
    let mut ed = editor_from("-[hello]>\n");
    ed.feed_key(key('"'));
    ed.feed_key(key('5'));
    ed.feed_key(key('y')); // "5y → in-memory register '5' (not kill ring push)

    assert_eq!(reg(&ed, '5'), &["hello"], "register '5' written");
    assert!(
        ed.state.kill_ring.head().is_none(),
        "kill ring head untouched by explicit \"5y"
    );
}

/// `"5y` then `"5p` round-trips via in-memory storage, regardless of kill-ring contents.
#[test]
fn digit_register_roundtrip_inmemory() {
    let mut ed = editor_from("-[INMEM]>\n");
    ed.feed_key(key('"'));
    ed.feed_key(key('5'));
    ed.feed_key(key('y'));
    // Ring: empty (no d/c). In-memory register '5' = "INMEM".
    // "5p must paste from in-memory, not clipboard or ring.
    ed.feed_key(key(';')); // collapse selection
    ed.feed_key(key('"'));
    ed.feed_key(key('5'));
    ed.feed_key(key('p'));

    assert!(
        ed.doc().text().to_string().contains("INMEM"),
        "\"5p must paste what \"5y wrote (in-memory round-trip)"
    );
}

// ── Register prefix persistence across non-register commands ────────────────

/// `"5` arms the prefix; `l` (a motion) does not consume it; the next `y` writes
/// to register 5. This is the intended sticky behaviour: the prefix persists
/// until a register-consuming command runs or Esc cancels it.
#[test]
fn register_prefix_persists_across_motion() {
    let mut ed = editor_from("-[hell]>o\n");
    ed.handle_key(key('"'));
    ed.handle_key(key('5'));
    ed.handle_key(key('l')); // motion, does not consume the prefix
    ed.handle_key(key('y')); // yank targets register 5, not '"'

    assert!(
        !reg(&ed, '5').is_empty(),
        "register '5' written after motion"
    );
    assert!(reg(&ed, '"').is_empty(), "'\"' register untouched");
}

/// A `d` that removes nothing still consumes the `"<reg>` prefix.
#[test]
fn register_prefix_is_consumed_by_a_delete_that_removes_nothing() {
    let mut ed = editor_from("hello-[\n]>");
    ed.handle_key(key('"'));
    ed.handle_key(key('5'));
    ed.handle_key(key('d'));

    assert!(ed.state.register_prefix.is_none(), "prefix consumed by d");
    assert!(reg(&ed, '5').is_empty(), "register '5' untouched");
}

/// A `c` that removes nothing leaves the kill ring as it was.
#[test]
fn change_that_removes_nothing_leaves_the_kill_ring_alone() {
    let mut ed = editor_from("-[hell]>o\n\n");
    ed.handle_key(key('y'));
    ed.handle_key(key('j'));
    ed.handle_key(key('c'));

    assert_eq!(
        ed.state.kill_ring.head(),
        Some(ps(&["hell"]).as_slice()),
        "kill ring head still the earlier yank"
    );
}
