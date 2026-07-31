use super::*;
use crate::editor::dispatch::ArgSource;
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
    use crate::ops::register::CLIPBOARD_REGISTER;

    let mut ed = editor_from("-[hell]>o\n");
    ed.handle_key(key('"'));
    ed.handle_key(key('5'));
    ed.handle_key(key('y'));

    // Now the prefix is cleared — move right to get a different selection,
    // then yank again without a prefix.
    ed.handle_key(key('l')); // move right
    ed.handle_key(key('y')); // bare yank — writes clipboard + kill ring

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

/// `Esc` after `"` cancels the prefix — the next `y` writes to clipboard + ring.
#[test]
fn esc_cancels_register_prefix() {
    use crate::ops::register::CLIPBOARD_REGISTER;

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
        Some(["hell".to_string()].as_slice()),
        "kill ring head populated"
    );
    assert!(reg(&ed, '5').is_empty(), "register '5' untouched");
}

/// `"3y` then `"3p` must round-trip through in-memory register '3'.
/// Digit registers are symmetric: yank writes RegisterSet['3'], paste reads it.
#[test]
fn paste_from_named_register() {
    use crate::ops::register::CLIPBOARD_REGISTER;

    let mut ed = editor_from("-[hello]>world\n");

    // "3y: yank "hello" into in-memory register '3'.
    ed.handle_key(key('"'));
    ed.handle_key(key('3'));
    ed.handle_key(key('y'));

    assert_eq!(reg(&ed, '3'), &["hello"], "register '3' populated by yank");

    // Move to a fresh position to make the paste visible.
    ed.handle_key(key('w')); // move word → selection on 'w' of "world"

    // Seed clipboard with "wrong" to verify "3p doesn't read it.
    ed.state
        .registers
        .write_text(CLIPBOARD_REGISTER, vec!["wrong".to_string()]);

    ed.handle_key(key('"'));
    ed.handle_key(key('3'));
    ed.handle_key(key('p')); // "3p → in-memory register '3' = "hello"

    assert!(
        ed.doc().text().to_string().contains("hello"),
        "pasted from in-memory register '3'"
    );
    assert!(
        !ed.doc().text().to_string().contains("wrong"),
        "clipboard not used by \"3p"
    );
}

/// `d` pushes to the kill ring; `"3p` reads in-memory register '3' (empty),
/// NOT ring slot 3. Digit registers are decoupled from the ring.
#[test]
fn digit_register_decoupled_from_kill_ring() {
    // Push 4 deletes so ring slot 3 = "P" (oldest).
    let mut ed = editor_from("-[P]>QRS\n");
    for _ in 0..4 {
        ed.handle_key(key('d'));
    }
    // ring: slot 0 = "S", slot 1 = "R", slot 2 = "Q", slot 3 = "P"
    // in-memory register '3' is empty (nothing yanked into it).
    assert!(
        reg(&ed, '3').is_empty(),
        "register '3' is empty — d never writes named registers"
    );

    // "3p reads in-memory register '3' which is empty → paste is a no-op.
    let text_before = ed.doc().text().to_string();
    ed.handle_key(key('"'));
    ed.handle_key(key('3'));
    ed.handle_key(key('p'));

    assert_eq!(
        ed.doc().text().to_string(),
        text_before,
        "\"3p is a no-op when register '3' is empty (not ring slot 3)"
    );
}

/// `"by` discards the yank — `'"'` must remain empty.
#[test]
fn black_hole_register_via_prefix() {
    use crate::ops::register::BLACK_HOLE_REGISTER;

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
    use crate::editor::Severity;
    use crate::ops::register::CLIPBOARD_REGISTER;

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

/// `"kp` must paste the kill-ring head, not the clipboard.
#[test]
fn kill_ring_register_pastes_ring_head() {
    use crate::ops::register::CLIPBOARD_REGISTER;

    let mut ed = editor_from("-[hello]>world\n");
    ed.feed_key(key('d')); // delete "hello" → ring head = ["hello"]

    // Seed clipboard with "wrong" to confirm "kp doesn't read it.
    ed.state
        .registers
        .write_text(CLIPBOARD_REGISTER, vec!["wrong".to_string()]);

    ed.handle_key(key('"'));
    ed.handle_key(key('k'));
    ed.feed_key(key('p'));

    assert!(
        ed.doc().text().to_string().contains("hello"),
        "\"kp pasted ring head"
    );
    assert!(
        !ed.doc().text().to_string().contains("wrong"),
        "clipboard not used by \"kp"
    );
}

/// `"kp` seeds the `[`/`]` cycle so pressing `[` after cycles to the older entry.
#[test]
fn kill_ring_register_paste_seeds_cycle() {
    // Build a ring with 2 entries: push "first" then "second" (head).
    let mut ed = editor_from("-[second]>X\n");
    ed.feed_key(key('d')); // ring head = ["second"]

    // Manually push an older entry.
    ed.state.kill_ring.push(vec!["first".to_string()]);
    // ring: head = ["first"] (newest push), slot 1 = ["second"]

    // "kp: paste ring head ("first"). This should open a paste session
    // seeded at the head so [ can cycle to the older entry.
    ed.handle_key(key('"'));
    ed.handle_key(key('k'));
    ed.feed_key(key('p'));

    assert!(
        ed.doc().text().to_string().contains("first"),
        "\"kp pasted ring head"
    );

    // [ should cycle to the next-older entry ("second").
    ed.feed_key(key('['));
    assert!(
        ed.doc().text().to_string().contains("second"),
        "[ after \"kp cycled to older ring entry"
    );
}

/// `"ky` pushes the yank onto the kill ring and does NOT touch the clipboard.
#[test]
fn kill_ring_register_yank_pushes_ring_only() {
    use crate::ops::register::CLIPBOARD_REGISTER;

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
        Some(["hello".to_string()].as_slice()),
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
        Some(["hello".to_string()].as_slice()),
        "ring head set by \"kd"
    );
}

// ── surround-add (`mw`) ───────────────────────────────────────────────────────

#[test]
fn mw_wraps_with_bracket() {
    let mut ed = editor_from("-[bar]>\n");
    ed.handle_key(key('m'));
    ed.handle_key(key('w'));
    ed.handle_key(key('['));
    assert_eq!(state(&ed), "[bar-[]]>\n");
}

#[test]
fn mw_wraps_with_brace_via_close_char() {
    // `mw}` should normalize to the pair `{` `}`.
    let mut ed = editor_from("-[bar]>\n");
    ed.handle_key(key('m'));
    ed.handle_key(key('w'));
    ed.handle_key(key('}'));
    assert_eq!(state(&ed), "{bar-[}]>\n");
}

#[test]
fn mw_wraps_symmetric_quote() {
    let mut ed = editor_from("-[bar]>\n");
    ed.handle_key(key('m'));
    ed.handle_key(key('w'));
    ed.handle_key(key('"'));
    assert_eq!(state(&ed), "\"bar-[\"]>\n");
}

#[test]
fn mw_wraps_unknown_char_symmetric() {
    // `*` is not a configured pair — wraps symmetrically open == close == `*`.
    let mut ed = editor_from("-[bar]>\n");
    ed.handle_key(key('m'));
    ed.handle_key(key('w'));
    ed.handle_key(key('*'));
    assert_eq!(state(&ed), "*bar-[*]>\n");
}

#[test]
fn mw_wraps_multi_cursor() {
    let mut ed = editor_from("-[ab]>c-[de]>f\n");
    ed.handle_key(key('m'));
    ed.handle_key(key('w'));
    ed.handle_key(key('('));
    assert_eq!(state(&ed), "(ab-[)]>c(de-[)]>f\n");
}

#[test]
fn mw_wraps_cursor_one_char() {
    let mut ed = editor_from("-[h]>ello\n");
    ed.handle_key(key('m'));
    ed.handle_key(key('w'));
    ed.handle_key(key('['));
    assert_eq!(state(&ed), "[h-[]]>ello\n");
}

#[test]
fn mw_esc_cancels() {
    let mut ed = editor_from("-[bar]>\n");
    ed.handle_key(key('m'));
    ed.handle_key(key('w'));
    ed.handle_key(key_esc()); // cancel before typing the delimiter
    assert_eq!(state(&ed), "-[bar]>\n");
}

#[test]
fn mw_wraps_when_auto_pairs_disabled() {
    // surround-add uses the pairs table only as a lookup; it ignores the
    // auto-pairs-enabled flag. `mw[` must still wrap even when auto-pairs are off.
    let mut ed = editor_from("-[bar]>\n");
    ed.state.settings.auto_pairs_enabled = false;
    ed.handle_key(key('m'));
    ed.handle_key(key('w'));
    ed.handle_key(key('['));
    assert_eq!(state(&ed), "[bar-[]]>\n");
}

// ── Smart-p heuristic and kill ring ──────────────────────────────────────────

/// `d` then `p` reads from the kill ring (char-swap / dp pattern).
/// `last_command` after `d` is "delete" ∈ `SMART_P_LAST_CMDS`, so `p` reads ring.
#[test]
fn smart_p_dp_reads_ring() {
    // Buffer: "ab\n", cursor on 'a'.
    let mut ed = editor_from("-[a]>b\n");
    ed.feed_key(key('d')); // delete 'a' → ring = ["a"]
    // After delete: buffer = "b\n", cursor at 'b'.
    ed.feed_key(key('p')); // paste-after from ring → "ba\n"? No: paste-after on cursor 'b' inserts after 'b'.
    // Actually: after 'd', cursor is on 'b'. paste-after inserts "a" after 'b'. Buffer = "ba\n".
    assert!(
        ed.doc().text().to_string().contains('a'),
        "ring content pasted after delete"
    );
    // Clipboard is not written by bare 'd', so the pasted value came from ring.
    assert!(
        ed.state.kill_ring.head().is_some(),
        "kill ring still has an entry after paste"
    );
}

/// `c` <text> Esc then `p` reads the kill ring, not the clipboard.
///
/// Regression: `exit-insert` (Esc) ran through the dispatch pipeline and
/// overwrote `last_command = "exit-insert"` ∉ `SMART_P_LAST_CMDS`, so
/// smart-`p` fell through to the clipboard. Fix: `exit-insert` is registered
/// with `.transparent_to_last_command()`, setting `stamps_last_command = false`
/// on its `CmdMeta`.
///
/// Fail oracle: remove `.transparent_to_last_command()` from `exit-insert`'s
/// registration in `registry/defaults/editor_cmds.rs` → `last_command` becomes "exit-insert"
/// → `p` pastes "CLIP" → `contains('a')` fails.
#[test]
fn smart_p_after_change_reads_ring() {
    use crate::ops::register::CLIPBOARD_REGISTER;

    let mut ed = editor_from("-[a]>b\n");
    ed.state
        .registers
        .write_text(CLIPBOARD_REGISTER, vec!["CLIP".to_string()]);
    ed.feed_key(key('c')); // change 'a' → ring=["a"], enter Insert
    ed.feed_key(key('x')); // type replacement (doesn't touch last_command)
    ed.feed_key(key_esc()); // exit-insert — must NOT clobber last_command
    ed.feed_key(key('p')); // smart-p → must read ring head ("a"), not "CLIP"
    let text = ed.doc().text().to_string();
    assert!(
        text.contains('a'),
        "p after change must paste ring content ('a')"
    );
    assert!(
        !text.contains("CLIP"),
        "p after change must not paste clipboard"
    );
}

/// `exit-insert` must never overwrite `last_command`, regardless of what it held.
///
/// Directly pins the sole exception in the stamp mechanism: `exit-insert` is
/// registered with `stamps_last_command = false` (via `.transparent_to_last_command()`
/// in `registry/defaults/editor_cmds.rs`), so `step_stamp_last_command` skips it.
///
/// Fail oracle: remove `.transparent_to_last_command()` from `exit-insert`'s
/// registration → `stamps_last_command` becomes `true` → marker becomes
/// `Some("exit-insert")`.
#[test]
fn exit_insert_does_not_stamp() {
    let mut ed = editor_from("-[a]>b\n");
    ed.feed_key(key('i')); // enter Insert (stamps "insert-at-selection-start")
    // Override last_command with a known kill marker — simulates a kill having
    // happened inside the insert session (e.g. via call! delete in Steel).
    ed.state.last_command = Some(std::borrow::Cow::Borrowed("delete"));
    ed.feed_key(key_esc()); // exit-insert — must NOT overwrite "delete"
    assert_eq!(
        ed.state.last_command.as_deref(),
        Some("delete"),
        "exit-insert must not stamp last_command",
    );
}

/// A native kill dispatched while in Insert mode stamps `last_command`.
///
/// Only `exit-insert` is exempt from stamping; all other commands — including
/// kills inside Insert — write their name. A future `Ctrl-w`-style command
/// (Steel body doing `call! delete`) therefore correctly informs smart-`p`.
///
/// Fail oracle: add a `stamps_last_command = false` check gated on Insert mode to
/// `step_stamp_last_command` → `last_command` stays `Some("insert-before")` and
/// the assertion fails.
#[test]
fn delete_in_insert_mode_stamps_marker() {
    let mut ed = editor_from("-[a]>b\n");
    ed.feed_key(key('i')); // enter Insert, last_command = Some("enter-insert")
    // Dispatch delete by name — 'd' in Insert self-inserts.
    ed.execute_keymap_command("delete".into(), Some(1), false, ArgSource::Keymap);
    assert_eq!(
        ed.state.last_command.as_deref(),
        Some("delete"),
        "native delete dispatched in Insert must stamp last_command",
    );
}

/// `c` <text> <Left> Esc `p` reads the clipboard, not the ring.
///
/// An arrow key in Insert mode stamps `"move-left"` ∉ `SMART_P_LAST_CMDS`,
/// resetting smart-p to clipboard — consistent with Normal-mode motion
/// behavior (`d j p` → clipboard).
#[test]
fn smart_p_insert_motion_resets_to_clipboard() {
    use crate::ops::register::CLIPBOARD_REGISTER;

    let mut ed = editor_from("-[a]>b\n");
    ed.state
        .registers
        .write_text(CLIPBOARD_REGISTER, vec!["CLIP".to_string()]);
    ed.feed_key(key('c')); // change 'a' → ring=["a"], enter Insert
    ed.feed_key(key('x')); // type replacement
    ed.feed_key(key_left()); // move-left in Insert → stamps "move-left"
    ed.feed_key(key_esc()); // exit-insert — transparent
    ed.feed_key(key('p')); // smart-p → must read clipboard ("CLIP")
    let text = ed.doc().text().to_string();
    assert!(
        text.contains("CLIP"),
        "motion in Insert resets smart-p to clipboard"
    );
    assert!(
        !text.contains('a'),
        "ring head must not be pasted after insert motion"
    );
}

/// `d` then `j` (motion) then `p` reads from clipboard, not ring.
/// Motion is NOT in `SMART_P_LAST_CMDS`, so `p` falls back to clipboard.
#[test]
fn smart_p_motion_resets_to_clipboard() {
    use crate::ops::register::CLIPBOARD_REGISTER;

    // Two-line buffer; cursor on line 0.
    let mut ed = editor_from("-[a]>b\ncd\n");
    // Seed clipboard with something distinct from what 'd' would yank.
    ed.state
        .registers
        .write_text(CLIPBOARD_REGISTER, vec!["CLIP".to_string()]);
    ed.feed_key(key('d')); // delete 'a' → ring = ["a"]
    ed.feed_key(key('j')); // move-down → last_command = "move-down" ∉ SMART_P_LAST_CMDS
    ed.feed_key(key('p')); // paste-after → must read clipboard ("CLIP")
    assert!(
        ed.doc().text().to_string().contains("CLIP"),
        "p after motion reads clipboard"
    );
}

/// Bare `y` writes to both the clipboard AND the kill ring.
/// A subsequent `p` (no preceding `c`/`d`) reads from the clipboard.
#[test]
fn smart_p_after_yank_reads_clipboard() {
    const KILL_CMDS: &[&str] = &["change", "delete"];

    let mut ed = editor_from("-[hello]> world\n");
    ed.feed_key(key('y')); // yank → clipboard="hello" + ring="hello"

    // Verify yank did not set last_command to anything in the kill set.
    assert!(
        !ed.state
            .last_command
            .as_deref()
            .is_some_and(|c| KILL_CMDS.contains(&c)),
        "last_command after bare y is a kill command"
    );

    // Push a distinct value to ring so ring-head ≠ clipboard.
    // Now: clipboard="hello", ring head="RING".
    ed.state.kill_ring.push(vec!["RING".to_string()]);

    // Move right and paste — should read clipboard ("hello"), not ring head ("RING").
    ed.feed_key(key('l'));
    ed.feed_key(key('p'));

    let buf = ed.doc().text().to_string();
    assert!(buf.contains("hello"), "p after y reads clipboard");
    assert!(!buf.contains("RING"), "ring head must not be used after y");
}

/// Consecutive `p p` after `d` keeps reading the ring (last_command stays in set).
#[test]
fn smart_p_consecutive_paste_stays_in_ring() {
    use crate::ops::register::CLIPBOARD_REGISTER;

    let mut ed = editor_from("-[X]>abc\n");
    // Seed clipboard with something distinct.
    ed.state
        .registers
        .write_text(CLIPBOARD_REGISTER, vec!["CLIP".to_string()]);
    ed.feed_key(key('d')); // delete 'X' → ring = ["X"]
    ed.feed_key(key('p')); // first paste → from ring, last_command = "paste-after"
    // last_command = "paste-after", is_paste = true → is_append = true → appends from last_paste.
    ed.feed_key(key('p')); // second paste → still from ring
    // Buffer should contain "X" twice (pasted) and NOT "CLIP".
    let buf = ed.doc().text().to_string();
    assert!(buf.contains("X"), "ring entry appears in buffer");
    assert!(
        !buf.contains("CLIP"),
        "second consecutive p still reads ring"
    );
}

/// `x d p` pastes the kill-ring head, not the clipboard.
///
/// `last_command = "delete"` is in `SMART_P_LAST_CMDS`, so bare `p` reads the
/// ring even when the clipboard holds different content.
#[test]
fn xdp_pastes_ring_head_not_clipboard() {
    use crate::ops::register::CLIPBOARD_REGISTER;

    let mut ed = editor_from("-[A]>\nB\n");
    ed.state.clipboard.force_unavailable();
    ed.state
        .registers
        .write_text(CLIPBOARD_REGISTER, vec!["CLIP".to_string()]);

    ed.feed_key(key('x')); // select "A\n"
    ed.feed_key(key('d')); // delete → ring = ["A\n"], last_command = "delete"
    ed.feed_key(key('p')); // prefer_ring = true → ring head

    assert_eq!(
        state(&ed),
        "B\n-[A\n]>",
        "xdp must paste the deleted line (ring head), not the clipboard sentinel"
    );
}

/// Regression: `drain_replay_queue` ran unconditionally after every key, setting
/// `last_command = None` even when the queue was empty. A bare `p` after `x d`
/// must still read the ring head — the idle drain must not neutralize `last_command`
/// (pre-432c24f bug: pasted the clipboard instead). `feed_key` / `feed_keys` include
/// the idle drain so this invariant is checked automatically by all paste tests now.
#[test]
fn smart_p_survives_idle_replay_drain() {
    use crate::ops::register::CLIPBOARD_REGISTER;

    let mut ed = editor_from("-[A]>\nB\n");
    ed.state.clipboard.force_unavailable();
    ed.state
        .registers
        .write_text(CLIPBOARD_REGISTER, vec!["CLIP".to_string()]);

    ed.feed_keys([key('x'), key('d'), key('p')]);

    assert_eq!(
        state(&ed),
        "B\n-[A\n]>",
        "idle replay-queue drain must not reset last_command; p reads the ring head"
    );
}

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

/// `"cy` writes clipboard only — no kill-ring push.
#[test]
fn explicit_cy_writes_clipboard_only() {
    use crate::ops::register::CLIPBOARD_REGISTER;

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

/// `"5p` reads in-memory register '5', not the kill ring.
/// Push 6 entries to the ring via bare `d`; `"5p` must be a no-op (register '5' empty).
#[test]
fn explicit_digit_p_reads_inmemory_not_ring() {
    let mut ed = editor_from("-[a]>bcdefg\n");
    for _ in 0..6 {
        ed.feed_key(key('d'));
    }
    // ring has 6 entries; in-memory register '5' was never written
    let before = state(&ed);
    ed.feed_key(key('"'));
    ed.feed_key(key('5'));
    ed.feed_key(key('p')); // "5p → in-memory register '5' is empty → no-op

    assert_eq!(
        state(&ed),
        before,
        "\"5p must be a no-op when in-memory register '5' is empty"
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

/// `paste-ring-older` / `paste-ring-newer` (`[` / `]`) on an empty ring are no-ops.
#[test]
fn paste_ring_older_empty_ring_is_noop() {
    let mut ed = editor_from("-[a]>bc\n");
    let before = state(&ed);
    ed.feed_key(key('['));
    assert_eq!(state(&ed), before, "[ on empty ring is a no-op");
    // Capture fresh snapshot so ] is verified against actual post-[ state,
    // not the original — if [ accidentally mutated state, this catches both.
    let after_open = state(&ed);
    ed.feed_key(key(']'));
    assert_eq!(state(&ed), after_open, "] on empty ring is a no-op");
}

/// `[ ]` cycle within a paste session: the ring cursor walks older then back newer.
#[test]
fn paste_ring_cycle_older_then_newer() {
    // Push 3 entries: A\n (oldest), B\n, C\n (newest/head at slot 0).
    let mut ed = editor_from("-[A]>\nB\nC\n");
    ed.feed_key(key('x'));
    ed.feed_key(key('d')); // ring = [A\n]
    ed.feed_key(key('x'));
    ed.feed_key(key('d')); // ring = [B\n, A\n]
    ed.feed_key(key('x'));
    ed.feed_key(key('d')); // ring = [C\n, B\n, A\n]

    // Open paste session: `p` reads ring head (C\n) since last_command ∈ SMART_P_LAST_CMDS.
    ed.feed_key(key('p')); // seeds cycle at Some(0) = C\n

    // `[` cycles older: Some(0) → Some(1) = B\n, re-pastes from session snapshot.
    ed.feed_key(key('['));
    let after_first_older = ed.doc().text().to_string();
    assert!(after_first_older.contains('B'), "first [ pastes slot 1 (B)");
    // `[` again → Some(1) → Some(2) = A\n.
    ed.feed_key(key('['));
    let after_second_older = ed.doc().text().to_string();
    assert!(
        after_second_older.contains('A'),
        "second [ pastes slot 2 (A)"
    );
    // `]` retreats → Some(2) → Some(1) = B\n.
    ed.feed_key(key(']'));
    let after_newer = ed.doc().text().to_string();
    assert!(after_newer.contains('B'), "] after two [ pastes slot 1 (B)");
}

/// Select a line with `x`, delete with `d`, move with `j`, then paste via
/// explicit ring head (`"kp`) — the deleted line must appear as its own line *below*
/// the cursor, not embedded inside the current line.
#[test]
fn paste_ring_linewise_pastes_below_not_inline() {
    // Buffer: "A\nB\nC\n". Delete line A (x+d), move to C (j), paste via "kp.
    let mut ed = editor_from("-[A]>\nB\nC\n");
    ed.feed_key(key('x')); // select line "A\n"
    ed.feed_key(key('d')); // push "A\n" to ring head, buffer → "B\nC\n"
    ed.feed_key(key('j')); // cursor → 'C'
    // "kp reads ring head (="A\n") directly — avoids smart-p clipboard routing
    // after the intervening motion cleared last_command.
    ed.feed_key(key('"'));
    ed.feed_key(key('k'));
    ed.feed_key(key('p')); // paste ring head (A\n) linewise below C

    // "A\n" must land as its own line below C — not inside C's text.
    assert_eq!(
        state(&ed),
        "B\nC\n-[A\n]>",
        "\"kp on a linewise ring entry must paste as a new line, not inline"
    );
}

/// `[`/`]` cycle within a paste session REPLACES the previous paste — never
/// accumulates a second copy.
#[test]
fn paste_ring_warm_cycle_replaces_not_accumulates() {
    // Two ring entries: A\n (older, slot 1), B\n (head, slot 0).
    let mut ed = editor_from("-[A]>\nB\nC\n");
    ed.feed_key(key('x'));
    ed.feed_key(key('d')); // ring = [A\n]; buffer = "B\nC\n"
    ed.feed_key(key('x'));
    ed.feed_key(key('d')); // ring = [B\n, A\n]; buffer = "C\n"

    // p: smart-p reads ring head B\n (last_command = "delete"), opens session.
    ed.feed_key(key('p'));
    assert_eq!(
        ed.doc().text().to_string().matches("B\n").count(),
        1,
        "p pastes B once"
    );

    // [: cycle older (slot 0 → slot 1 = A\n) — must REPLACE B, not add another.
    ed.feed_key(key('['));
    let after_older = ed.doc().text().to_string();
    assert_eq!(
        after_older.matches("A\n").count(),
        1,
        "[ replaces paste with A"
    );

    // ]: cycle newer (slot 1 → slot 0 = B\n) — must REPLACE A.
    ed.feed_key(key(']'));
    let after_newer = ed.doc().text().to_string();
    assert_eq!(
        after_newer.matches("B\n").count(),
        1,
        "] replaces back with B"
    );
    assert_eq!(after_newer.matches("A\n").count(), 0, "] removes A");
}

/// Single-char cycle: `[` within a session pastes the older entry, `]` replaces
/// it back with the head — collapsed selection is not an obstacle.
#[test]
fn paste_ring_warm_cycle_replaces_single_char_paste() {
    let mut ed = editor_from("-[X]>Y\n");
    ed.feed_key(key('d')); // kill "X"; ring = [X], buffer = "-[Y]>\n"
    ed.feed_key(key('d')); // kill "Y"; ring = [Y, X], buffer = "-[\n]>"

    // p: reads ring head Y (last_command = "delete"), opens session, seeds cycle at 0.
    ed.feed_key(key('p'));
    assert!(
        ed.doc().text().to_string().contains('Y'),
        "p pastes Y (ring head)"
    );

    // [: cycle older (slot 0 → slot 1 = X), replaces Y.
    ed.feed_key(key('['));
    assert!(
        ed.doc().text().to_string().contains('X'),
        "[ pastes X (slot 1)"
    );

    // ]: cycle newer (slot 1 → slot 0 = Y), replacing X.
    ed.feed_key(key(']'));
    let buf = ed.doc().text().to_string();
    assert!(buf.contains('Y'), "] pastes Y (slot 0)");
    assert!(!buf.contains('X'), "] replaces X — no 'X' remains");
}

/// `P` (paste-before) opens a before-session; `[`/`]` must re-paste BEFORE the
/// cursor, not after it.
#[test]
fn paste_before_cycle_stays_before_charwise() {
    let mut ed = editor_from("-[c]>d\n"); // cursor on 'c' at index 0
    ed.state.kill_ring.push(vec!["X".to_string()]); // slot 1 after next push
    ed.state.kill_ring.push(vec!["Y".to_string()]); // ring=[Y, X]; head=Y, slot 1=X

    // "kP: paste-before ring head ("Y") before cursor 'c'.
    ed.feed_key(key('"'));
    ed.feed_key(key('k'));
    ed.feed_key(key('P'));
    assert_eq!(state(&ed), "-[Y]>cd\n", "P pastes before the cursor");

    // [: cycle to slot 1 ("X"); must re-paste BEFORE the cursor snapshot (at 0).
    ed.feed_key(key('['));
    assert_eq!(
        state(&ed),
        "-[X]>cd\n",
        "[ after P re-pastes before the cursor (would be c-[X]>d if it used paste_after)"
    );
}

/// `p` (paste-after) opens an after-session; cycling stays after (regression).
#[test]
fn paste_after_cycle_stays_after_charwise() {
    let mut ed = editor_from("-[c]>d\n");
    ed.state.kill_ring.push(vec!["X".to_string()]);
    ed.state.kill_ring.push(vec!["Y".to_string()]); // ring=[Y, X]

    ed.feed_key(key('"'));
    ed.feed_key(key('k'));
    ed.feed_key(key('p'));
    assert_eq!(state(&ed), "c-[Y]>d\n", "p pastes after the cursor");

    ed.feed_key(key('['));
    assert_eq!(state(&ed), "c-[X]>d\n", "[ after p stays paste-after");
}

/// `P` on a linewise entry opens a before-session; `[` must re-paste ABOVE the
/// cursor line, not below it.
#[test]
fn paste_before_cycle_stays_above_linewise() {
    let mut ed = editor_from("-[B]>\nC\n"); // cursor on 'B', line 0
    ed.state.kill_ring.push(vec!["X\n".to_string()]); // slot 1
    ed.state.kill_ring.push(vec!["Y\n".to_string()]); // ring=[Y\n, X\n]; head=Y\n

    // "kP: linewise paste-before ring head ("Y\n") — inserts above line 0.
    ed.feed_key(key('"'));
    ed.feed_key(key('k'));
    ed.feed_key(key('P'));
    assert_eq!(
        ed.doc().text().to_string(),
        "Y\nB\nC\n",
        "P pastes above current line"
    );

    // [: cycle to slot 1 ("X\n"); must re-paste ABOVE line 0 (not below).
    ed.feed_key(key('['));
    assert_eq!(
        ed.doc().text().to_string(),
        "X\nB\nC\n",
        "[ after linewise P re-pastes above (would be B\\nX\\nC\\n if it used paste_after)"
    );
}

/// `p [ p` duplicates the currently-cycled entry — never does a fresh clipboard paste.
///
/// After `[` swaps the paste to the ring head, `last_command = "paste-ring-older"`
/// has `is_paste = true`, so the next `p` must append (not replace).
#[test]
fn paste_after_cycle_appends_cycled_entry() {
    use crate::ops::register::CLIPBOARD_REGISTER;

    let mut ed = editor_from("-[x]>\n");
    ed.state.clipboard.force_unavailable();
    ed.state
        .registers
        .write_text(CLIPBOARD_REGISTER, vec!["CLIP".to_string()]);
    ed.state.kill_ring.push(vec!["RING".to_string()]); // ring head = "RING"

    // p: last_command=None → clipboard "CLIP"; seed_cycle(None).
    ed.feed_key(key('p'));
    assert!(
        ed.doc().text().to_string().contains("CLIP"),
        "first p must paste clipboard (last_command=None → not in SMART_P_LAST_CMDS)"
    );

    // [: cycle_older None→0="RING"; replaces "CLIP" with "RING"; last_paste=["RING"].
    ed.feed_key(key('['));
    // p: is_append (last_command="paste-ring-older" ∈ PASTE_FAMILY) → append last_paste.
    ed.feed_key(key('p'));

    let buf = ed.doc().text().to_string();
    assert_eq!(
        buf.matches("RING").count(),
        2,
        "p after [ must duplicate the cycled entry"
    );
    assert!(
        !buf.contains("CLIP"),
        "clipboard must not appear after [ cycle"
    );
}

/// Consecutive `p` presses append copies rather than replacing the selected paste.
#[test]
fn consecutive_paste_appends_copies() {
    use crate::ops::register::CLIPBOARD_REGISTER;

    let mut ed = editor_from("-[ab]>\n");
    ed.state.clipboard.force_unavailable();
    // Seed clipboard with "CLIP" (distinct from ring) to falsify the assertion:
    // if the second p reads clipboard instead of last_paste, "CLIP" would appear.
    ed.state
        .registers
        .write_text(CLIPBOARD_REGISTER, vec!["CLIP".to_string()]);
    ed.feed_key(key('d')); // delete "ab" → ring = ["ab"], last_command = "delete"
    ed.feed_key(key('p')); // smart-p reads ring head "ab" (last_command="delete"); last_paste=["ab"]
    ed.feed_key(key('p')); // is_append → appends from last_paste = ["ab"]
    let buf = ed.doc().text().to_string();
    assert_eq!(
        buf.matches("ab").count(),
        2,
        "two consecutive p presses must stack two copies of 'ab'"
    );
    assert!(
        !buf.contains("CLIP"),
        "clipboard not used — append reads last_paste"
    );
}

/// Consecutive `p` presses append when the previous paste came from the CLIPBOARD
/// and the kill ring is empty — the second `p` must not be a no-op.
#[test]
fn consecutive_clipboard_paste_appends() {
    use crate::ops::register::CLIPBOARD_REGISTER;

    let mut ed = editor_from("-[x]>\n");
    ed.state.clipboard.force_unavailable(); // headless: reads fall back to in-memory mirror
    ed.state
        .registers
        .write_text(CLIPBOARD_REGISTER, vec!["XY".to_string()]);
    // ring is empty — this is the regression case

    ed.feed_key(key('p')); // last_command=None → clipboard → pastes "XY"; last_paste=["XY"]
    ed.feed_key(key('p')); // last_command="paste-after" ∈ PASTE_FAMILY → repeat last_paste
    let buf = ed.doc().text().to_string();
    assert_eq!(
        buf.matches("XY").count(),
        2,
        "two consecutive p presses must stack two copies even with an empty kill ring"
    );
}

/// Consecutive `p` after a clipboard paste must repeat the clipboard value, not
/// whatever happens to be at the ring head.
#[test]
fn consecutive_paste_repeats_last_not_ring_head() {
    use crate::ops::register::CLIPBOARD_REGISTER;

    let mut ed = editor_from("-[x]>\n");
    ed.state.clipboard.force_unavailable();
    ed.state
        .registers
        .write_text(CLIPBOARD_REGISTER, vec!["XY".to_string()]);
    ed.state.kill_ring.push(vec!["ZZ".to_string()]); // ring has different content

    ed.feed_key(key('p')); // clipboard → "XY"; last_paste=["XY"]
    ed.feed_key(key('p')); // append → repeats "XY", not ring head "ZZ"
    let buf = ed.doc().text().to_string();
    assert_eq!(buf.matches("XY").count(), 2, "clipboard value repeated");
    assert!(
        !buf.contains("ZZ"),
        "ring head must not appear — append repeats last paste verbatim"
    );
}

/// After paste-after, the pasted text is selected (covers the full inserted span).
#[test]
fn paste_leaves_output_selected() {
    // Delete "ab" → ring head = "ab". Then paste: selection must cover "ab".
    let mut ed = editor_from("-[ab]>cd\n");
    ed.feed_key(key('d')); // kill "ab"; buffer = "-[c]>d\n"
    ed.feed_key(key('p')); // smart-p reads ring head "ab" (charwise); paste after 'c'
    assert_eq!(
        state(&ed),
        "c-[ab]>d\n",
        "paste must leave the pasted text selected"
    );
}

// ── Register prefix persistence across non-register commands ────────────────

/// `"5` arms the prefix; `l` (a motion) does not consume it; the next `y` writes
/// to register 5. This is the intended sticky behaviour — the prefix persists
/// until a register-consuming command runs or Esc cancels it.
#[test]
fn register_prefix_persists_across_motion() {
    let mut ed = editor_from("-[hell]>o\n");
    ed.handle_key(key('"'));
    ed.handle_key(key('5'));
    ed.handle_key(key('l')); // motion — does not consume the prefix
    ed.handle_key(key('y')); // yank targets register 5, not '"'

    assert!(
        !reg(&ed, '5').is_empty(),
        "register '5' written after motion"
    );
    assert!(reg(&ed, '"').is_empty(), "'\"' register untouched");
}

/// An explicit `"Xp` while in the append state must paste from register X,
/// not silently re-paste the previous value.  Before the fix, the append path
/// returned without calling `take_register_prefix()`, so the named register was
/// ignored AND the prefix leaked into the next command.
#[test]
fn register_prefix_overrides_append_path() {
    let mut ed = editor_from("-[x]>\n");
    ed.state.registers.write_text('5', vec!["REG5".to_string()]);
    ed.state.kill_ring.push(vec!["RING".to_string()]);

    // Delete 'x' so the ring has "x" at head; RING is at slot 1.
    ed.feed_key(key('d'));
    // Paste via kill register to get into the append state with last_paste=[ring head].
    ed.handle_key(key('"'));
    ed.handle_key(key('k'));
    ed.feed_key(key('p'));

    // Now try to paste from named register '5' — must NOT take the append path.
    ed.handle_key(key('"'));
    ed.handle_key(key('5'));
    ed.feed_key(key('p'));

    let buf = ed.doc().text().to_string();
    assert!(
        buf.contains("REG5"),
        "explicit \"5p must paste from register 5, not re-paste last_paste; buf={buf:?}"
    );
}

/// After an explicit `"Xp` the register prefix must be consumed (not leaked).
/// Before the fix the prefix persisted and the NEXT command accidentally used it.
#[test]
fn register_prefix_consumed_by_paste() {
    let mut ed = editor_from("-[x]>\n");
    ed.state.registers.write_text('5', vec!["REG5".to_string()]);
    ed.state.kill_ring.push(vec!["RING".to_string()]);

    // Get into append state via a paste.
    ed.feed_key(key('d')); // delete x; ring head = "x"
    ed.handle_key(key('"'));
    ed.handle_key(key('k')); // select kill register
    ed.feed_key(key('p')); // paste ring head

    // Now type "5p — explicit register paste.
    ed.handle_key(key('"'));
    ed.handle_key(key('5'));
    ed.feed_key(key('p')); // should consume the '5' prefix

    // The prefix must be gone — the next 'd' must NOT route to register 5.
    ed.feed_key(key('d')); // delete; should push to kill ring, not register 5
    // Register 5 must still hold "REG5" — if the prefix leaked into 'd', it
    // would be overwritten with the deleted char.
    let reg5 = ed
        .state
        .registers
        .read('5')
        .and_then(|r| r.as_text())
        .map(|v| v.to_vec());
    assert_eq!(
        reg5,
        Some(vec!["REG5".to_string()]),
        "register 5 must be unchanged after d — prefix leaked if it differs"
    );
}
