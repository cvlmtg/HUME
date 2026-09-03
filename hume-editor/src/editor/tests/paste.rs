use super::*;
use pretty_assertions::assert_eq;

// ── Explicit-register paste reads (`"N`/`"k`p) ──────────────────────────────

/// `"3y` then `"3p` must round-trip through in-memory register '3'.
/// Digit registers are symmetric: yank writes RegisterSet['3'], paste reads it.
#[test]
fn paste_from_named_register() {
    use hume_ops::register::CLIPBOARD_REGISTER;

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

/// `"kp` must paste the kill-ring head, not the clipboard.
#[test]
fn kill_ring_register_pastes_ring_head() {
    use hume_ops::register::CLIPBOARD_REGISTER;

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

// ── Explicit register prefix vs the paste stamp ─────────────────────────────

/// An explicit `"Xp` while a fresh paste stamp exists must still paste from
/// register X — an explicit register prefix never consults the stamp (see
/// `resolve_smart`), regardless of what a preceding bare/`"k` paste
/// left behind.
#[test]
fn register_prefix_ignores_paste_stamp() {
    let mut ed = editor_from("-[x]>\n");
    ed.state.registers.write_text('5', vec!["REG5".to_string()]);
    ed.state.kill_ring.push(vec!["RING".to_string()]);

    // Delete 'x' so the ring has "x" at head; RING is at slot 1.
    ed.feed_key(key('d'));
    // Paste via the kill register to arm a fresh paste stamp.
    ed.handle_key(key('"'));
    ed.handle_key(key('k'));
    ed.feed_key(key('p'));

    // Now try to paste from named register '5' — must ignore the stamp.
    ed.handle_key(key('"'));
    ed.handle_key(key('5'));
    ed.feed_key(key('p'));

    let buf = ed.doc().text().to_string();
    assert!(
        buf.contains("REG5"),
        "explicit \"5p must paste from register 5, ignoring the paste stamp; buf={buf:?}"
    );
}

/// After an explicit `"Xp` the register prefix must be consumed (not leaked).
/// Before the fix the prefix persisted and the NEXT command accidentally used it.
#[test]
fn register_prefix_consumed_by_paste() {
    let mut ed = editor_from("-[x]>\n");
    ed.state.registers.write_text('5', vec!["REG5".to_string()]);
    ed.state.kill_ring.push(vec!["RING".to_string()]);

    // Prime the ring with a paste.
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

// ── Smart-p: the PasteStamp mechanism ────────────────────────────────────────
//
// A bare paste reads the kill ring while `PasteStamp::seq` still matches
// `BufferStore::edit_seq()` — set by every capture that pushes to the ring
// (`d`/`c`/`y`, bare or `"k`-prefixed) and re-stamped by every completed bare
// paste and ring cycle. Any edit (or undo/redo), anywhere, moves `edit_seq`
// past the stamp; a plain motion does not, since it never touches the buffer.
// See `PasteStamp`'s doc in `commands/paste.rs`.

/// `d` then `p` reads from the kill ring (char-swap / dp pattern).
#[test]
fn smart_p_dp_reads_ring() {
    let mut ed = editor_from("-[a]>b\n");
    ed.feed_key(key('d')); // delete 'a' → ring = ["a"], stamp fresh
    ed.feed_key(key('p')); // smart-p → stamp fresh → ring head 'a'
    assert!(
        ed.doc().text().to_string().contains('a'),
        "ring content pasted after delete"
    );
}

/// `c` <text> Esc then `p` reads the kill ring, not the clipboard — the swap
/// idiom. Every keystroke typed during the session bumps `edit_seq`, which
/// would otherwise strand the stamp `c`'s own `route_kill` wrote; the
/// insert session is marked kill-opened (`PaneBufferState::kill_opened_session`)
/// so `end_insert_session` refreshes the stamp's `seq` once typing stops.
///
/// Fail oracle: drop the `kill_opened_session` refresh in `end_insert_session`
/// → the stamp stays stamped at the pre-typing `edit_seq`, `p` sees it as
/// stale, and falls through to the clipboard ("CLIP" would appear instead of 'a').
#[test]
fn smart_p_after_change_reads_ring() {
    use hume_ops::register::CLIPBOARD_REGISTER;

    let mut ed = editor_from("-[a]>b\n");
    ed.state
        .registers
        .write_text(CLIPBOARD_REGISTER, vec!["CLIP".to_string()]);
    ed.feed_key(key('c')); // change 'a' → ring=["a"], stamp fresh
    ed.feed_key(key('x')); // type replacement — bumps edit_seq
    ed.feed_key(key_esc()); // exit-insert → refreshes the stamp's seq
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

/// Negative counterpart to `smart_p_after_change_reads_ring`: an
/// explicit-register `"5c` <text> `Esc` must NOT resurrect an existing
/// stamp. `cmd_change` only sets `kill_opened_session` when `route_kill`
/// reports it actually captured to the ring (`route_kill` returns `false`
/// for an explicit register) — without that gate, `end_insert_session`
/// would refresh whatever stale stamp happens to exist, making a later bare
/// `p` wrongly read `"aaa"` again instead of falling to the clipboard.
///
/// Fail oracle: set `kill_opened_session` unconditionally in `cmd_change`
/// (drop the `if state.route_kill(yanked)` gate) → the stale "aaa" stamp
/// gets refreshed by the `"5c` session anyway, and `p` pastes "aaa" instead
/// of "CLIP".
#[test]
fn explicit_register_change_does_not_resurrect_stale_stamp() {
    use hume_ops::register::CLIPBOARD_REGISTER;

    let mut ed = editor_from("-[aaa]> bbb\n");
    ed.state
        .registers
        .write_text(CLIPBOARD_REGISTER, vec!["CLIP".to_string()]);
    ed.feed_key(key('d')); // delete "aaa" → ring = ["aaa"], stamp fresh
    ed.feed_key(key('w')); // motion onto "bbb" — does not touch edit_seq

    ed.feed_key(key('"'));
    ed.feed_key(key('5'));
    ed.feed_key(key('c')); // "5c → explicit register, no stamp write
    ed.feed_key(key('y')); // type replacement — bumps edit_seq, staling the "aaa" stamp
    ed.feed_key(key_esc()); // exit-insert — must NOT refresh the stale stamp

    ed.feed_key(key('p')); // bare smart-p → stamp stale → clipboard
    let text = ed.doc().text().to_string();
    assert!(
        text.contains("CLIP"),
        "p after \"5c must read the clipboard, not the stale ring stamp; buf={text:?}"
    );
    assert!(
        !text.contains("aaa"),
        "p after \"5c must not resurrect the stale \"aaa\" ring stamp; buf={text:?}"
    );
}

/// A cursor motion (e.g. an arrow key) *inside* an open kill-opened change
/// session doesn't stop `end_insert_session` from refreshing the stamp on
/// exit — `kill_opened_session` is a per-session flag set once by
/// `cmd_change`, not something an in-session motion can reset, so the
/// session-final `p` still reads the ring, not the clipboard.
///
/// Contrast with `smart_p_after_edit_falls_back_to_clipboard`: a motion
/// *before* entering Insert (via `i`, no kill involved) never marks the
/// session kill-opened, so that stamp goes stale as normal.
#[test]
fn smart_p_after_change_with_insert_motion_still_reads_ring() {
    use hume_ops::register::CLIPBOARD_REGISTER;

    let mut ed = editor_from("-[a]>b\n");
    ed.state
        .registers
        .write_text(CLIPBOARD_REGISTER, vec!["CLIP".to_string()]);
    ed.feed_key(key('c')); // change 'a' → ring=["a"], kill-opened session
    ed.feed_key(key('x')); // type replacement
    ed.feed_key(key_left()); // arrow-key motion mid-session — bumps edit_seq, does not clear kill_opened_session
    ed.feed_key(key_esc()); // exit-insert → refreshes the stamp's seq regardless
    ed.feed_key(key('p')); // smart-p → must still read ring head ("a"), not "CLIP"
    let text = ed.doc().text().to_string();
    assert!(
        text.contains('a'),
        "p after change-with-in-session-motion must paste ring content ('a')"
    );
    assert!(
        !text.contains("CLIP"),
        "p after change-with-in-session-motion must not paste clipboard"
    );
}

/// `d` then a motion then `p` still reads the ring — motions never touch
/// `edit_seq`, so they cannot invalidate the stamp; routing is decided by
/// buffer state alone, never by which commands ran in between.
#[test]
fn smart_p_after_motion_still_reads_ring() {
    use hume_ops::register::CLIPBOARD_REGISTER;

    let mut ed = editor_from("-[ab]> cd\n");
    ed.state
        .registers
        .write_text(CLIPBOARD_REGISTER, vec!["CLIP".to_string()]);
    ed.feed_key(key('d')); // delete "ab" → ring = ["ab"], stamp fresh
    ed.feed_key(key('w')); // motion — does not touch edit_seq
    ed.feed_key(key('p')); // bare smart-p → stamp still fresh → ring
    let buf = ed.doc().text().to_string();
    assert!(
        buf.contains("ab"),
        "a motion between capture and paste must not invalidate the ring stamp; buf={buf:?}"
    );
    assert!(
        !buf.contains("CLIP"),
        "must not fall back to the clipboard; buf={buf:?}"
    );
}

/// `d` then a real edit (typing) then `p` falls back to the clipboard — an
/// edit that isn't itself a capture invalidates the stamp.
#[test]
fn smart_p_after_edit_falls_back_to_clipboard() {
    use hume_ops::register::CLIPBOARD_REGISTER;

    let mut ed = editor_from("-[a]>b\n");
    ed.state
        .registers
        .write_text(CLIPBOARD_REGISTER, vec!["CLIP".to_string()]);
    ed.feed_key(key('d')); // delete 'a' → ring = ["a"], stamp fresh
    ed.feed_key(key('i')); // enter insert (not kill-opened)
    ed.feed_key(key('x')); // type 'x' — an edit, bumps edit_seq
    ed.feed_key(key_esc());
    ed.feed_key(key('p')); // bare smart-p → stamp stale → clipboard
    let text = ed.doc().text().to_string();
    assert!(
        text.contains("CLIP"),
        "p after an intervening edit must read the clipboard; buf={text:?}"
    );
}

/// `"ky` (kill-ring-only yank) stamps the ring, same as `d`/`c` — every push
/// onto the ring stamps uniformly (`capture_to_ring`), with no special case
/// for which command did the pushing.
#[test]
fn ky_yank_stamps_the_ring_for_next_paste() {
    use hume_ops::register::CLIPBOARD_REGISTER;

    let mut ed = editor_from("-[hello]> world\n");
    ed.state
        .registers
        .write_text(CLIPBOARD_REGISTER, vec!["CLIP".to_string()]);

    ed.handle_key(key('"'));
    ed.handle_key(key('k'));
    ed.feed_key(key('y')); // "ky → ring only, stamps Ring(0)

    ed.feed_key(key('l')); // motion — does not touch edit_seq
    ed.feed_key(key('p')); // bare smart-p → stamp still fresh → ring

    let buf = ed.doc().text().to_string();
    assert_eq!(
        buf.matches("hello").count(),
        2,
        "p after \"ky must paste a second 'hello' from the ring; buf={buf:?}"
    );
    assert!(
        !buf.contains("CLIP"),
        "p after \"ky must not read the clipboard; buf={buf:?}"
    );
}

/// Bare `y` writes to both the clipboard and the kill ring — a following bare
/// `p` pastes the yanked text (the two sources hold identical content here,
/// so this only confirms the basic round-trip; see `ky_yank_stamps_the_ring_for_next_paste`
/// for the case that actually distinguishes the two sources).
#[test]
fn smart_p_after_bare_yank_pastes_yanked_text() {
    let mut ed = editor_from("-[hello]> world\n");
    ed.feed_key(key('y'));
    ed.feed_key(key('l'));
    ed.feed_key(key('p'));
    let buf = ed.doc().text().to_string();
    assert_eq!(
        buf.matches("hello").count(),
        2,
        "p after bare y pastes a second 'hello' alongside the original; buf={buf:?}"
    );
}

/// `x d p` pastes the kill-ring head, not the clipboard.
#[test]
fn xdp_pastes_ring_head_not_clipboard() {
    use hume_ops::register::CLIPBOARD_REGISTER;

    let mut ed = editor_from("-[A]>\nB\n");
    ed.state.clipboard.force_unavailable();
    ed.state
        .registers
        .write_text(CLIPBOARD_REGISTER, vec!["CLIP".to_string()]);

    ed.feed_key(key('x')); // select "A\n"
    ed.feed_key(key('d')); // delete → ring = ["A\n"], stamp fresh
    ed.feed_key(key('p')); // bare smart-p → ring head

    assert_eq!(
        state(&ed),
        "B\n-[A\n]>",
        "xdp must paste the deleted line (ring head), not the clipboard sentinel"
    );
}

/// The idle replay-queue drain that runs after every `feed_key` must not
/// disturb the paste stamp — a bare `p` after `x d` still reads the ring
/// head. (`feed_key`/`feed_keys` exercise the same per-key ordering as the
/// real event loop, including this drain, so every test above already
/// checks this incidentally; this test pins it explicitly.)
#[test]
fn smart_p_survives_idle_replay_drain() {
    use hume_ops::register::CLIPBOARD_REGISTER;

    let mut ed = editor_from("-[A]>\nB\n");
    ed.state.clipboard.force_unavailable();
    ed.state
        .registers
        .write_text(CLIPBOARD_REGISTER, vec!["CLIP".to_string()]);

    ed.feed_keys([key('x'), key('d'), key('p')]);

    assert_eq!(
        state(&ed),
        "B\n-[A\n]>",
        "idle replay-queue drain must not disturb the paste stamp; p reads the ring head"
    );
}

/// `d p p p`: each press re-resolves and re-stamps the stamp fresh; since
/// the resolved value keeps matching what's already selected, each press
/// appends another copy instead of replacing.
#[test]
fn dppp_appends_three_copies_from_ring() {
    let mut ed = editor_from("-[ab]>cd\n");
    ed.feed_key(key('d')); // delete "ab" → ring = ["ab"], stamp fresh
    ed.feed_key(key('p'));
    ed.feed_key(key('p'));
    ed.feed_key(key('p'));
    let buf = ed.doc().text().to_string();
    assert_eq!(
        buf.matches("ab").count(),
        3,
        "three consecutive p presses stack three copies; buf={buf:?}"
    );
}

/// `y p p`: bare yank stamps the ring; two presses append two copies
/// alongside the original selected text.
#[test]
fn yank_then_two_pastes_appends_two_copies() {
    let mut ed = editor_from("-[ab]>cd\n");
    ed.state.clipboard.force_unavailable(); // bare y still writes the in-memory mirror
    ed.feed_key(key('y')); // yank "ab" → clipboard(mirror) = ring = ["ab"]
    ed.feed_key(key('p'));
    ed.feed_key(key('p'));
    let buf = ed.doc().text().to_string();
    assert_eq!(
        buf.matches("ab").count(),
        3,
        "original selection plus two pasted copies; buf={buf:?}"
    );
}

/// `d u p`: undo bumps `edit_seq`, invalidating the stamp — a bare `p`
/// after undoing the capturing delete falls back to the clipboard.
#[test]
fn undo_after_delete_invalidates_ring_stamp() {
    use hume_ops::register::CLIPBOARD_REGISTER;

    let mut ed = editor_from("-[ab]>cd\n");
    ed.state
        .registers
        .write_text(CLIPBOARD_REGISTER, vec!["CLIP".to_string()]);
    ed.feed_key(key('d')); // delete "ab" → ring = ["ab"], stamp fresh
    ed.feed_key(key('u')); // undo the delete → edit_seq bumps → stamp stale
    ed.feed_key(key('p')); // bare smart-p → clipboard
    let buf = ed.doc().text().to_string();
    assert!(
        buf.contains("CLIP"),
        "p after undo must read the clipboard, not the ring; buf={buf:?}"
    );
}

/// Same as `undo_after_delete_invalidates_ring_stamp`, but through redo:
/// `apply_doc_redo` bumps `edit_seq` too (see `doc_ops.rs`), and nothing
/// re-stamps after a redo, so the stamp stays stale.
#[test]
fn redo_after_undo_keeps_ring_stamp_stale() {
    use hume_ops::register::CLIPBOARD_REGISTER;

    let mut ed = editor_from("-[ab]>cd\n");
    ed.state
        .registers
        .write_text(CLIPBOARD_REGISTER, vec!["CLIP".to_string()]);
    ed.feed_key(key('d')); // delete "ab" → ring = ["ab"], stamp fresh
    ed.feed_key(key('u')); // undo → edit_seq bumps → stamp stale
    ed.feed_key(key('U')); // redo → edit_seq bumps again → stamp still stale
    ed.feed_key(key('p')); // bare smart-p → clipboard
    let buf = ed.doc().text().to_string();
    assert!(
        buf.contains("CLIP"),
        "p after redo must read the clipboard, not the ring; buf={buf:?}"
    );
}

/// An edit in a *different* buffer invalidates the stamp too — `edit_seq`
/// (`BufferStore`) is a single global counter, not per-buffer, so a capture
/// in buffer A followed by any edit in buffer B (before switching back to A)
/// stales A's stamp exactly as an edit in A itself would.
#[test]
fn edit_in_other_buffer_invalidates_ring_stamp() {
    use hume_editing::selection::SelectionSet;
    use hume_editing::text::BufferText;
    use hume_ops::register::CLIPBOARD_REGISTER;

    let mut ed = editor_from("-[ab]>cd\n");
    ed.state
        .registers
        .write_text(CLIPBOARD_REGISTER, vec!["CLIP".to_string()]);
    ed.feed_key(key('d')); // delete "ab" in buffer A → ring = ["ab"], stamp fresh

    let bid_a = ed.focused_buffer_id();
    let bid_b = ed.open_buffer(Buffer::new(
        BufferText::from("xy\n"),
        SelectionSet::default(),
    ));
    ed.switch_to_buffer_without_jump(bid_b);
    // `i`/type/`Esc`, not `d`/`c`/`y` — a capturing edit in B would legitimately
    // write a *fresh* stamp pointing at B's own capture, which isn't what this
    // test is isolating: it must be an edit that bumps `edit_seq` without
    // itself re-stamping, so A's now-stale stamp is the only thing in play.
    ed.feed_key(key('i'));
    ed.feed_key(key('z'));
    ed.feed_key(key_esc()); // bumps the global edit_seq, writes no stamp

    ed.switch_to_buffer_without_jump(bid_a);
    ed.feed_key(key('p')); // bare smart-p in A → stamp stale → clipboard
    let buf = ed.doc().text().to_string();
    assert!(
        buf.contains("CLIP"),
        "p in buffer A after an edit in buffer B must read the clipboard, not the ring; buf={buf:?}"
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

/// An explicit register capture (`"5y`, `"bd`, …) never writes the paste
/// stamp — only a bare or `"k`-prefixed capture does (see `route_kill` /
/// `EditorState::capture_to_ring`). White-box: checks the field directly.
#[test]
fn explicit_register_capture_does_not_write_paste_stamp() {
    let mut ed = editor_from("-[a]>bc\n");
    assert!(ed.state.paste_stamp.is_none(), "setup: no stamp yet");

    ed.handle_key(key('"'));
    ed.handle_key(key('5'));
    ed.feed_key(key('y')); // "5y → digit register only
    assert!(
        ed.state.paste_stamp.is_none(),
        "\"5y must not write the paste stamp"
    );

    ed.feed_key(key('l'));
    ed.handle_key(key('"'));
    ed.handle_key(key('b'));
    ed.feed_key(key('d')); // "bd → black hole
    assert!(
        ed.state.paste_stamp.is_none(),
        "\"bd must not write the paste stamp"
    );
}

/// An explicit `"kp` still seeds the `[`/`]` cycle (unchanged behaviour), but
/// — unlike a bare paste — does not write the paste stamp: the user asked
/// for a specific register, not to arm the heuristic for the next bare paste.
#[test]
fn explicit_k_prefix_paste_does_not_write_stamp() {
    let mut ed = editor_from("-[x]>\n");
    ed.state.kill_ring.push(vec!["RING".to_string()]);

    ed.handle_key(key('"'));
    ed.handle_key(key('k'));
    ed.feed_key(key('p')); // "kp: explicit — must not write the stamp

    assert!(
        ed.state.paste_stamp.is_none(),
        "\"kp must not write the paste stamp"
    );
}

// ── Equal-text collapse (repeat vs swap) ───────────────────────────────────────
//
// Applies to bare smart pastes only — plain paste and any `"<reg>`-prefixed
// paste always replace a non-collapsed selection. When the resolved values
// match the currently selected text one-to-one, the selections collapse
// first so the paste appends alongside the existing text; any mismatch
// (including a value/selection count mismatch) replaces it instead. See
// `collapse_if_repeat` in `commands/paste.rs`.

/// Pasting different text over a non-collapsed selection replaces it — the
/// baseline the equal-text rule is contrasted against.
#[test]
fn paste_over_different_selection_replaces() {
    let mut ed = editor_from("-[old]>\n");
    ed.state.kill_ring.push(vec!["new".to_string()]);
    ed.feed_key(key('p'));
    assert_eq!(
        state(&ed),
        "-[new]>\n",
        "different text replaces the selection"
    );
}

/// Pasting identical text over a matching (forward) non-collapsed selection
/// appends alongside it instead.
#[test]
fn paste_repeat_over_identical_selection_appends() {
    let mut ed = editor_from("-[ab]>\n");
    ed.state.kill_ring.push(vec!["ab".to_string()]);
    ed.feed_key(key('p'));
    let buf = ed.doc().text().to_string();
    assert_eq!(
        buf.matches("ab").count(),
        2,
        "identical paste over a matching multi-char selection appends; buf={buf:?}"
    );
}

/// A backward selection (head < anchor) still collapses to the span's `end`
/// for `smart-paste-after` — not to `head()`, which on a backward selection
/// is the *left* edge and would land the collapse mid-span.
#[test]
fn paste_after_repeat_over_backward_selection_collapses_to_end() {
    let mut ed = editor_from("-[ab]>cd\n");
    ed.state.kill_ring.push(vec!["ab".to_string()]);
    dispatch_command(&mut ed, "flip-selections"); // anchor/head swap; start()/end() unchanged
    ed.feed_key(key('p'));
    assert_eq!(
        state(&ed),
        "ab-[ab]>cd\n",
        "backward selection must still collapse to the span's end (start()/end_inclusive()), not head()"
    );
}

/// Mirrors `paste_after_repeat_over_backward_selection_collapses_to_end` for
/// `smart-paste-before`, which collapses to `start()`, not `anchor()`.
#[test]
fn paste_before_repeat_over_backward_selection_collapses_to_start() {
    let mut ed = editor_from("-[ab]>cd\n");
    ed.state.kill_ring.push(vec!["ab".to_string()]);
    dispatch_command(&mut ed, "flip-selections");
    ed.feed_key(key('P'));
    assert_eq!(
        state(&ed),
        "-[ab]>abcd\n",
        "backward selection must still collapse to the span's start (start()), not anchor()"
    );
}

/// Multi-cursor: every selection's value matches — all-or-nothing, so every
/// cursor appends.
#[test]
fn paste_multi_cursor_all_match_appends_each() {
    let mut ed = editor_from("-[ab]>x-[cd]>\n");
    ed.state
        .kill_ring
        .push(vec!["ab".to_string(), "cd".to_string()]);
    ed.feed_key(key('p'));
    let buf = ed.doc().text().to_string();
    assert_eq!(
        buf.matches("ab").count(),
        2,
        "first cursor's match appends; buf={buf:?}"
    );
    assert_eq!(
        buf.matches("cd").count(),
        2,
        "second cursor's match appends; buf={buf:?}"
    );
}

/// Multi-cursor: one selection's value mismatches — all-or-nothing means the
/// whole set replaces, not a mix of append and replace.
#[test]
fn paste_multi_cursor_partial_match_replaces_all() {
    let mut ed = editor_from("-[ab]>x-[cd]>\n");
    ed.state
        .kill_ring
        .push(vec!["ab".to_string(), "ZZ".to_string()]); // 2nd doesn't match "cd"
    ed.feed_key(key('p'));
    let buf = ed.doc().text().to_string();
    assert_eq!(
        buf.matches("ab").count(),
        1,
        "no cursor appends when any value mismatches; buf={buf:?}"
    );
    assert!(
        buf.contains("ZZ"),
        "mismatched value still replaces; buf={buf:?}"
    );
    assert!(
        !buf.contains("cd"),
        "original selected text is replaced, not kept; buf={buf:?}"
    );
}

/// A value-count/selection-count mismatch always replaces — equality can't
/// even be evaluated per-selection when the op joins values across the board.
#[test]
fn paste_value_count_mismatch_replaces() {
    let mut ed = editor_from("-[ab]>x-[cd]>\n");
    ed.state.kill_ring.push(vec!["Z".to_string()]); // 1 value, 2 selections
    ed.feed_key(key('p'));
    let buf = ed.doc().text().to_string();
    assert!(
        !buf.contains("ab") && !buf.contains("cd"),
        "mismatched counts always replace; buf={buf:?}"
    );
    assert_eq!(
        buf.matches('Z').count(),
        2,
        "joined value applied to both selections; buf={buf:?}"
    );
}

/// The equal-text rule applies to a linewise entry over a matching linewise
/// selection too, not just charwise.
#[test]
fn paste_repeat_linewise_entry_over_linewise_selection_appends() {
    let mut ed = editor_from("-[AB]>\nCD\n");
    ed.feed_key(key('x')); // select-line → "AB\n" (linewise)
    ed.state.kill_ring.push(vec!["AB\n".to_string()]);
    ed.feed_key(key('p'));
    let buf = ed.doc().text().to_string();
    assert_eq!(
        buf.matches("AB\n").count(),
        2,
        "identical linewise paste over a matching linewise selection appends; buf={buf:?}"
    );
}

/// Matching clipboard text over an identical selection duplicates it, rather
/// than a same-text no-op — the rule is "append", not "skip".
#[test]
fn paste_matching_clipboard_over_identical_selection_duplicates() {
    use hume_ops::register::CLIPBOARD_REGISTER;

    let mut ed = editor_from("-[ab]>cd\n");
    ed.state.clipboard.force_unavailable();
    ed.state
        .registers
        .write_text(CLIPBOARD_REGISTER, vec!["ab".to_string()]);
    ed.feed_key(key('p')); // no stamp yet → clipboard "ab" == selected "ab" → append
    let buf = ed.doc().text().to_string();
    assert_eq!(
        buf.matches("ab").count(),
        2,
        "matching clipboard text duplicates rather than no-op replacing; buf={buf:?}"
    );
}

/// A repeat clipboard paste re-reads the clipboard fresh each time — if it
/// changed externally between the two presses, the second replaces the first
/// rather than appending, since the resolved value no longer matches.
#[test]
fn repeat_clipboard_paste_replaces_when_clipboard_changed_externally() {
    use hume_ops::register::CLIPBOARD_REGISTER;

    let mut ed = editor_from("-[xy]>\n");
    ed.state.clipboard.force_unavailable();
    ed.state
        .registers
        .write_text(CLIPBOARD_REGISTER, vec!["xy".to_string()]);

    ed.feed_key(key('p')); // no stamp yet → clipboard "xy" == selected "xy" → append
    let buf1 = ed.doc().text().to_string();
    assert_eq!(
        buf1.matches("xy").count(),
        2,
        "first paste matches the selection and appends; buf={buf1:?}"
    );

    // External clipboard change before the second press.
    ed.state
        .registers
        .write_text(CLIPBOARD_REGISTER, vec!["ZZ".to_string()]);

    ed.feed_key(key('p')); // stamp fresh(Clipboard) → re-reads clipboard fresh → "ZZ"
    let buf2 = ed.doc().text().to_string();
    assert!(
        buf2.contains("ZZ"),
        "second paste reads the changed clipboard; buf={buf2:?}"
    );
    assert_eq!(
        buf2.matches("xy").count(),
        1,
        "the second paste replaces its selection rather than appending; buf={buf2:?}"
    );
}

/// An explicit-register smart paste whose content equals the selected text
/// still replaces — the equal-text collapse is bare-only, and `"Xp` shares
/// plain paste's replace contract (see `collapse_if_repeat`'s doc).
#[test]
fn explicit_register_paste_with_equal_text_replaces() {
    let mut ed = editor_from("-[ab]>cd\n");
    ed.state.registers.write_text('5', vec!["ab".to_string()]);
    ed.handle_key(key('"'));
    ed.handle_key(key('5'));
    ed.feed_key(key('p'));
    let buf = ed.doc().text().to_string();
    assert_eq!(
        buf.matches("ab").count(),
        1,
        "\"5p over an identical selection must replace, never stack; buf={buf:?}"
    );
}

/// A repeat press after a clipboard paste must repeat the clipboard value or
/// do nothing: when the re-read fails entirely (no OS clipboard, empty
/// in-memory mirror), the press warns and no-ops rather than substituting
/// the ring head — which would not match the selected just-pasted text and
/// so would replace it.
#[test]
fn repeat_clipboard_paste_with_dead_clipboard_is_noop() {
    use crate::editor::Severity;
    use crate::editor::commands::{PasteSource, PasteStamp};

    let mut ed = editor_from("-[xy]>\n");
    ed.state.clipboard.force_unavailable();
    ed.state.kill_ring.push(vec!["ZZ".to_string()]);
    // A fresh Clipboard stamp with nothing readable behind it: as if the
    // previous press pasted external clipboard text that has since become
    // unreadable (transient failure, never mirrored in-memory).
    ed.state.paste_stamp = Some(PasteStamp {
        seq: ed.state.buffers.edit_seq(),
        source: PasteSource::Clipboard,
    });
    ed.feed_key(key('p'));
    let buf = ed.doc().text().to_string();
    assert_eq!(
        buf, "xy\n",
        "repeat with a dead clipboard must no-op, not paste the ring head; buf={buf:?}"
    );
    assert!(
        ed.state
            .message_log
            .entries()
            .any(|e| e.severity == Severity::Warning),
        "the refused repeat must warn, not fail silently"
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

// ── Ring cycling (`[`/`]`) ──────────────────────────────────────────────────

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

    // Open paste session: `p` reads ring head (C\n) — stamp is fresh, nothing
    // has been edited since the last delete.
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

    // p: reads ring head B\n, opens session.
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

    // p: reads ring head Y, opens session, seeds cycle at 0.
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

/// `p [ p` duplicates the currently-cycled entry: `[` re-stamps the stamp to
/// the cycled slot, so the following bare `p` resolves that same slot fresh
/// and — matching what's now selected — appends rather than replacing.
#[test]
fn paste_after_cycle_appends_cycled_entry() {
    use hume_ops::register::CLIPBOARD_REGISTER;

    let mut ed = editor_from("-[x]>\n");
    ed.state.clipboard.force_unavailable();
    ed.state
        .registers
        .write_text(CLIPBOARD_REGISTER, vec!["CLIP".to_string()]);
    ed.state.kill_ring.push(vec!["RING".to_string()]); // ring head = "RING"

    // p: no stamp yet → clipboard "CLIP".
    ed.feed_key(key('p'));
    assert!(
        ed.doc().text().to_string().contains("CLIP"),
        "first p must paste clipboard (no stamp yet)"
    );

    // [: cycle_older None→0="RING"; replaces "CLIP" with "RING"; stamp → Ring(0).
    ed.feed_key(key('['));
    // p: stamp fresh(Ring(0)) → "RING" again → matches selection → append.
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
    use hume_ops::register::CLIPBOARD_REGISTER;

    let mut ed = editor_from("-[ab]>\n");
    ed.state.clipboard.force_unavailable();
    // Seed clipboard with "CLIP" (distinct from ring) to falsify the assertion:
    // if the second p reads clipboard instead of the ring, "CLIP" would appear.
    ed.state
        .registers
        .write_text(CLIPBOARD_REGISTER, vec!["CLIP".to_string()]);
    ed.feed_key(key('d')); // delete "ab" → ring = ["ab"], stamp fresh
    ed.feed_key(key('p')); // smart-p reads ring head "ab"
    ed.feed_key(key('p')); // stamp still fresh → "ab" again → matches → append
    let buf = ed.doc().text().to_string();
    assert_eq!(
        buf.matches("ab").count(),
        2,
        "two consecutive p presses must stack two copies of 'ab'"
    );
    assert!(
        !buf.contains("CLIP"),
        "clipboard not used — repeat reads the ring, not the clipboard"
    );
}

/// Consecutive `p` presses append when the previous paste came from the CLIPBOARD
/// and the kill ring is empty — the second `p` must not be a no-op.
#[test]
fn consecutive_clipboard_paste_appends() {
    use hume_ops::register::CLIPBOARD_REGISTER;

    let mut ed = editor_from("-[z]>\n");
    ed.state.clipboard.force_unavailable(); // headless: reads fall back to in-memory mirror
    ed.state
        .registers
        .write_text(CLIPBOARD_REGISTER, vec!["xy".to_string()]);
    // ring is empty — this is the regression case

    ed.feed_key(key('p')); // no stamp → clipboard → inserts "xy" after 'z'; stamp → Clipboard
    ed.feed_key(key('p')); // stamp fresh(Clipboard) → re-reads "xy" → matches selection → append
    let buf = ed.doc().text().to_string();
    assert_eq!(
        buf.matches("xy").count(),
        2,
        "two consecutive p presses must stack two copies even with an empty kill ring"
    );
}

/// Consecutive `p` after a clipboard paste must repeat the clipboard value, not
/// whatever happens to be at the ring head.
#[test]
fn consecutive_paste_repeats_last_not_ring_head() {
    use hume_ops::register::CLIPBOARD_REGISTER;

    let mut ed = editor_from("-[z]>\n");
    ed.state.clipboard.force_unavailable();
    ed.state
        .registers
        .write_text(CLIPBOARD_REGISTER, vec!["xy".to_string()]);
    ed.state.kill_ring.push(vec!["ZZ".to_string()]); // ring has different content

    ed.feed_key(key('p')); // clipboard → "xy"; stamp → Clipboard
    ed.feed_key(key('p')); // stamp fresh(Clipboard) → repeats "xy", not ring head "ZZ"
    let buf = ed.doc().text().to_string();
    assert_eq!(buf.matches("xy").count(), 2, "clipboard value repeated");
    assert!(
        !buf.contains("ZZ"),
        "ring head must not appear — repeat re-reads the clipboard, not the ring"
    );
}

// ── Plain paste (`paste-after`/`paste-before`) ────────────────────────────────
//
// `p`/`P` dispatch the smart variants (see the smart-p block above); these
// commands are reachable by name only (`:`, `call!`, or a plugin keymap).
// Dispatched the same way `classic-paste`'s Steel wrappers and
// `async_job_steel.rs`/`dot_repeat.rs` dispatch a command by name — through
// `execute_keymap_command`, the same pipeline a keymap or `(call! …)` uses
// (not a direct fn call, and unlike `:`, it doesn't itself dispatch any
// unrelated command in between two calls).

fn dispatch_command(ed: &mut Editor, name: &str) {
    ed.execute_keymap_command(name.to_string().into(), None, false);
}

/// Bare plain paste reads the kill-ring head, not the clipboard — unlike
/// smart-p, it never falls through to the clipboard when there's no register
/// prefix, and it never consults the paste stamp to decide.
#[test]
fn plain_paste_reads_ring_not_clipboard() {
    use hume_ops::register::CLIPBOARD_REGISTER;

    let mut ed = editor_from("-[x]>\n");
    ed.state
        .registers
        .write_text(CLIPBOARD_REGISTER, vec!["CLIP".to_string()]);
    ed.state.kill_ring.push(vec!["RING".to_string()]);

    dispatch_command(&mut ed, "paste-after");

    let buf = ed.doc().text().to_string();
    assert!(buf.contains("RING"), "plain paste must read the ring head");
    assert!(
        !buf.contains("CLIP"),
        "plain paste must not fall back to the clipboard"
    );
}

/// `paste-before` (plain) reads the ring head and inserts before the
/// selection — same source rule as `paste-after`, opposite side. Every
/// other plain-paste test in this file dispatches `paste-after`; this one
/// exercises `cmd_paste_before` directly so a bug isolated to the `before`
/// path (e.g. `do_paste`'s `before` flag) isn't masked by the `after` tests.
#[test]
fn plain_paste_before_reads_ring_head() {
    let mut ed = editor_from("x-[y]>z\n");
    ed.state.kill_ring.push(vec!["RING".to_string()]);

    dispatch_command(&mut ed, "paste-before");

    let buf = ed.doc().text().to_string();
    assert_eq!(
        buf, "xRINGyz\n",
        "plain paste-before must insert before 'y'"
    );
}

/// Two consecutive plain pastes replace, never stack — plain paste is dumb
/// by design: it always replaces a non-collapsed selection, with no
/// equal-text check at all (that's smart-paste-only; see `collapse_if_repeat`'s
/// doc). A script driving plain paste never has to inspect the selection
/// before pasting; to get an append, it collapses the selection itself first.
#[test]
fn plain_paste_does_not_stack() {
    let mut ed = editor_from("-[x]>\n");
    ed.state.kill_ring.push(vec!["RING".to_string()]);

    dispatch_command(&mut ed, "paste-after");
    dispatch_command(&mut ed, "paste-after");

    let buf = ed.doc().text().to_string();
    assert_eq!(
        buf.matches("RING").count(),
        1,
        "second plain paste must replace the first, not stack a copy; buf={buf:?}"
    );
}

/// `[` after a plain paste still cycles — plain paste opens the same session
/// smart paste does.
#[test]
fn plain_paste_opens_ring_cycle_session() {
    let mut ed = editor_from("-[x]>\n");
    ed.state.kill_ring.push(vec!["OLDER".to_string()]);
    ed.state.kill_ring.push(vec!["RING".to_string()]);

    dispatch_command(&mut ed, "paste-after");
    ed.feed_key(key('[')); // cycle to the older ring entry

    let buf = ed.doc().text().to_string();
    assert!(
        buf.contains("OLDER"),
        "[ after a plain paste must cycle the ring; buf={buf:?}"
    );
    assert!(!buf.contains("RING"));
}

/// An explicit register prefix on a plain paste reads that register, same as
/// on smart paste.
#[test]
fn plain_paste_honors_register_prefix() {
    let mut ed = editor_from("-[x]>\n");
    ed.state.registers.write_text('3', vec!["REG3".to_string()]);
    ed.state.kill_ring.push(vec!["RING".to_string()]);

    ed.handle_key(key('"'));
    ed.handle_key(key('3'));
    dispatch_command(&mut ed, "paste-after");

    let buf = ed.doc().text().to_string();
    assert!(
        buf.contains("REG3"),
        "\"3 + plain paste must read register 3"
    );
    assert!(!buf.contains("RING"));
}

/// `"b` (black hole) on a plain paste is a no-op, same as on smart paste.
#[test]
fn plain_paste_black_hole_is_noop() {
    let mut ed = editor_from("-[x]>\n");
    ed.state.kill_ring.push(vec!["RING".to_string()]);

    ed.handle_key(key('"'));
    ed.handle_key(key('b'));
    dispatch_command(&mut ed, "paste-after");

    assert_eq!(
        ed.doc().text().to_string(),
        "x\n",
        "\"b + plain paste must be a no-op"
    );
}

/// `"b` (black hole) on a *smart* paste is a no-op too — `resolve_smart`
/// routes an explicit register through `resolve_explicit_register`, the same
/// path plain paste uses, so black-hole shortcuts identically for both.
/// `plain_paste_black_hole_is_noop` above covers the plain path only.
#[test]
fn smart_paste_black_hole_is_noop() {
    let mut ed = editor_from("-[x]>\n");
    ed.state.kill_ring.push(vec!["RING".to_string()]);

    ed.handle_key(key('"'));
    ed.handle_key(key('b'));
    ed.feed_key(key('p')); // "bp → smart-paste-after, black hole

    assert_eq!(
        ed.doc().text().to_string(),
        "x\n",
        "\"b + smart paste must be a no-op"
    );
}

/// A smart paste right after a plain paste appends: a bare
/// plain paste writes the stamp too (`Ring(0)`), so the immediately
/// following bare smart paste resolves the same slot fresh and — matching
/// what's now selected — appends.
#[test]
fn smart_paste_appends_after_plain_paste() {
    use hume_ops::register::CLIPBOARD_REGISTER;

    let mut ed = editor_from("-[x]>\n");
    ed.state.clipboard.force_unavailable();
    ed.state
        .registers
        .write_text(CLIPBOARD_REGISTER, vec!["CLIP".to_string()]);
    ed.state.kill_ring.push(vec!["RING".to_string()]);

    dispatch_command(&mut ed, "paste-after"); // plain: reads ring head "RING"
    ed.feed_key(key('p')); // smart: stamp fresh(Ring(0)) → "RING" again → append

    let buf = ed.doc().text().to_string();
    assert_eq!(
        buf.matches("RING").count(),
        2,
        "smart paste after a plain paste must append, not read fresh; buf={buf:?}"
    );
}

/// Plain paste on a read-only buffer reports and makes no edit.
#[test]
fn plain_paste_refuses_read_only_buffer() {
    let mut ed = editor_from("-[x]>\n");
    ed.state.kill_ring.push(vec!["RING".to_string()]);
    ed.doc_mut().read_only = true;

    dispatch_command(&mut ed, "paste-after");

    assert_eq!(
        ed.doc().text().to_string(),
        "x\n",
        "read-only buffer must refuse the paste"
    );
    assert_eq!(ed.state.status_msg.as_deref(), Some("Buffer is read-only"));
}
