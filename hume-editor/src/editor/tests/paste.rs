use super::*;
use hume_scripting::host::NativeCall;
use pretty_assertions::assert_eq;

// ── Explicit-register paste reads (`"N`/`"k`p) ──────────────────────────────

/// `"3y` then `"3p` must round-trip through in-memory register '3'.
/// Digit registers are symmetric: yank writes RegisterSet['3'], paste reads it.
#[test]
fn paste_from_named_register() {
    let mut ed = editor_from("-[hello]>world\n");

    // "3y: yank "hello" into in-memory register '3'.
    ed.handle_key(key('"'));
    ed.handle_key(key('3'));
    ed.handle_key(key('y'));

    assert_eq!(reg(&ed, '3'), &["hello"], "register '3' populated by yank");

    // Move to a fresh position to make the paste visible.
    ed.handle_key(key('w')); // move word → selection on 'w' of "world"

    // Seed clipboard with "wrong" to verify "3p doesn't read it.
    seed_clipboard(&mut ed, "wrong");

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
        "register '3' is empty: d never writes named registers"
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
    let mut ed = editor_from("-[hello]>world\n");
    ed.feed_key(key('d')); // delete "hello" → ring head = ["hello"]

    // Seed clipboard with "wrong" to confirm "kp doesn't read it.
    seed_clipboard(&mut ed, "wrong");

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
    seed_ring(&mut ed, &["first"]);
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

/// A cycle step that deletes the same span the previous paste deleted leaves
/// the text as it is, and still has to hand back selections for that text.
#[test]
fn paste_cycle_step_repeating_the_previous_deletion_keeps_selections_valid() {
    let mut ed = editor_from("-[hello]>world\n");
    ed.state.registers.write_text('5', vec![String::new()]);
    seed_ring(&mut ed, &[""]);

    ed.handle_key(key('"'));
    ed.handle_key(key('5'));
    ed.feed_key(key('p'));
    ed.feed_key(key('['));

    assert_eq!(ed.doc().text().to_string(), "world\n");
    ed.feed_key(key('u'));
    assert_eq!(ed.doc().text().to_string(), "helloworld\n");
}

// ── Explicit register prefix ────────────────────────────────────────────────

/// An explicit `"Xp` right after a ring paste still pastes from register X:
/// an explicit register prefix bypasses smart paste (see `resolve_smart`).
#[test]
fn register_prefix_overrides_the_ring() {
    let mut ed = editor_from("-[x]>\n");
    ed.state.registers.write_text('5', vec!["REG5".to_string()]);
    seed_ring(&mut ed, &["RING"]);

    // Delete 'x' so the ring has "x" at head; RING is at slot 1.
    ed.feed_key(key('d'));
    // Paste via the kill register, so the ring head is what `p` would read.
    ed.handle_key(key('"'));
    ed.handle_key(key('k'));
    ed.feed_key(key('p'));

    // Now paste from named register '5'.
    ed.handle_key(key('"'));
    ed.handle_key(key('5'));
    ed.feed_key(key('p'));

    let buf = ed.doc().text().to_string();
    assert!(
        buf.contains("REG5"),
        "explicit \"5p must paste from register 5, not the ring; buf={buf:?}"
    );
}

/// After an explicit `"Xp` the register prefix must be consumed, so the next
/// command does not pick it up.
#[test]
fn register_prefix_consumed_by_paste() {
    let mut ed = editor_from("-[x]>\n");
    ed.state.registers.write_text('5', vec!["REG5".to_string()]);
    seed_ring(&mut ed, &["RING"]);

    // Prime the ring with a paste.
    ed.feed_key(key('d')); // delete x; ring head = "x"
    ed.handle_key(key('"'));
    ed.handle_key(key('k')); // select kill register
    ed.feed_key(key('p')); // paste ring head

    // Now type "5p, an explicit register paste.
    ed.handle_key(key('"'));
    ed.handle_key(key('5'));
    ed.feed_key(key('p')); // should consume the '5' prefix

    // The prefix must be gone: the next 'd' must NOT route to register 5.
    ed.feed_key(key('d')); // delete; should push to kill ring, not register 5
    let reg5 = ed
        .state
        .registers
        .read('5')
        .and_then(|r| r.as_pieces())
        .map(|v| v.iter().map(|p| p.text().to_owned()).collect::<Vec<_>>());
    assert_eq!(
        reg5,
        Some(vec!["REG5".to_string()]),
        "register 5 must be unchanged after d, prefix leaked if it differs"
    );
}

// ── Smart paste: kills and the ring head ─────────────────────────────────────

/// `d` then `p` reads from the kill ring (char-swap / dp pattern).
#[test]
fn smart_p_dp_reads_ring() {
    let mut ed = editor_from("-[a]>b\n");
    ed.feed_key(key('d')); // delete 'a' → ring = ["a"]
    ed.feed_key(key('p')); // smart-p → ring head 'a'
    assert!(
        ed.doc().text().to_string().contains('a'),
        "ring content pasted after delete"
    );
}

/// `c` <text> Esc then `p` reads the kill ring, not the clipboard: the swap
/// idiom. The text typed in between does not switch `p` to the clipboard.
#[test]
fn smart_p_after_change_reads_ring() {
    let mut ed = editor_from("-[a]>b\n");
    seed_clipboard(&mut ed, "CLIP");
    ed.feed_key(key('c')); // change 'a' → ring = ["a", "CLIP"]
    ed.feed_key(key('x')); // type replacement
    ed.feed_key(key_esc());
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

/// `d` then a motion then `p` reads the ring head.
#[test]
fn smart_p_after_motion_still_reads_ring() {
    let mut ed = editor_from("-[ab]> cd\n");
    seed_clipboard(&mut ed, "CLIP");
    ed.feed_key(key('d')); // delete "ab" → ring = ["ab", "CLIP"]
    ed.feed_key(key('w'));
    ed.feed_key(key('p')); // bare smart-p → ring head
    let buf = ed.doc().text().to_string();
    assert!(
        buf.contains("ab"),
        "a motion between capture and paste keeps p on the ring head; buf={buf:?}"
    );
    assert!(
        !buf.contains("CLIP"),
        "must not fall back to the clipboard; buf={buf:?}"
    );
}

/// `"ky` (kill-ring-only yank) puts its text at the ring head for `p`, as
/// `d`/`c` do.
#[test]
fn ky_yank_is_what_p_pastes() {
    let mut ed = editor_from("-[hello]> world\n");
    seed_clipboard(&mut ed, "CLIP");

    ed.handle_key(key('"'));
    ed.handle_key(key('k'));
    ed.feed_key(key('y')); // "ky → ring only

    ed.feed_key(key('l'));
    ed.feed_key(key('p')); // bare smart-p → ring head

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

/// Bare `y` writes to both the clipboard and the kill ring, so a following bare
/// `p` pastes the yanked text (the two sources hold identical content here,
/// so this only confirms the basic round-trip; see `ky_yank_is_what_p_pastes`
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
    let mut ed = editor_from("-[A]>\nB\n");
    seed_clipboard(&mut ed, "CLIP");

    ed.feed_key(key('x')); // select "A\n"
    ed.feed_key(key('d')); // delete → ring = ["A\n", "CLIP"]
    ed.feed_key(key('p')); // bare smart-p → ring head

    assert_eq!(
        state(&ed),
        "B\n-[A\n]>",
        "xdp must paste the deleted line (ring head), not the clipboard sentinel"
    );
}

/// `d p p p`: each press resolves the ring head again; since it keeps
/// matching what's already selected, each press appends another copy instead
/// of replacing.
#[test]
fn dppp_appends_three_copies_from_ring() {
    let mut ed = editor_from("-[ab]>cd\n");
    ed.feed_key(key('d')); // delete "ab" → ring = ["ab"]
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

/// `y p p`: a bare yank is the ring head; two presses append two copies
/// alongside the original selected text.
#[test]
fn yank_then_two_pastes_appends_two_copies() {
    let mut ed = editor_from("-[ab]>cd\n");
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

// ── Smart paste: new clipboard text joins the ring ─────────────────────────────
//
// A bare `p` pastes the ring head. Clipboard text HUME has not seen (it did
// not write it and has not taken it in) goes onto the ring first, at the
// next `p`, `d`, `c`, `y` or clipboard write.

/// An edit does not switch `p` to the clipboard when the clipboard holds
/// nothing new.
#[test]
fn an_edit_alone_keeps_pasting_the_ring_head() {
    let mut ed = editor_from("-[a]>b\n");
    seed_clipboard(&mut ed, "CLIP");
    ed.feed_key(key('d'));
    ed.feed_keys([key('i'), key('z'), key_esc()]);
    ed.feed_key(key('p'));
    assert_eq!(state(&ed), "z-[a]>b\n");
}

/// Clipboard text already taken in does not come back to the head: the
/// entry a cycle ended on stays what `p` pastes, edit or not.
#[test]
fn a_clipboard_already_taken_in_does_not_return_to_the_head() {
    let mut ed = editor_from("-[z]>\n");
    seed_clipboard(&mut ed, "c");
    seed_ring(&mut ed, &["c", "b", "a"]);

    ed.feed_keys([key('p'), key('['), key(';')]);
    assert_eq!(state(&ed), "z-[a]>\n");
    ed.feed_keys([key('i'), key('y'), key_esc()]);
    ed.feed_key(key('p'));
    assert_eq!(state(&ed), "zy-[a]>a\n");
}

/// A kill takes in a new clipboard first, so the kill stays the head and the
/// outside copy sits one `[` below it.
#[test]
fn a_kill_puts_a_new_clipboard_below_itself() {
    let mut ed = editor_from("-[f]>g\n");
    seed_clipboard(&mut ed, "X");
    ed.feed_key(key('d'));
    ed.feed_key(key('p'));
    assert_eq!(state(&ed), "g-[f]>\n");
    ed.feed_key(key('['));
    assert_eq!(state(&ed), "g-[X]>\n");
}

/// Text HUME yanked to the clipboard is not new: a later kill stays what `p`
/// pastes.
#[test]
fn a_yank_then_a_kill_pastes_the_kill() {
    let mut ed = editor_from("-[a]>bc\n");
    ed.feed_key(key('y'));
    ed.feed_keys([key('l'), key('d')]);
    ed.feed_keys([key('i'), key('z'), key_esc()]);
    ed.feed_key(key('p'));
    assert_eq!(state(&ed), "az-[b]>c\n");
}

/// `"cy` writes the clipboard without making it new for `p`.
#[test]
fn a_clipboard_only_yank_is_not_pasted_by_p() {
    use hume_ops::register::CLIPBOARD_REGISTER;

    let mut ed = editor_from("-[a]>b\n");
    seed_ring(&mut ed, &["R"]);
    ed.feed_keys([key('"'), key(CLIPBOARD_REGISTER), key('y')]);
    ed.feed_key(key('p'));
    assert_eq!(state(&ed), "a-[R]>b\n");
}

/// A clipboard write takes in the outside copy it overwrites.
#[test]
fn a_clipboard_write_keeps_the_outside_copy_it_replaces() {
    use hume_ops::register::CLIPBOARD_REGISTER;

    let mut ed = editor_from("-[a]>b\n");
    seed_clipboard(&mut ed, "X");
    ed.feed_keys([key('"'), key(CLIPBOARD_REGISTER), key('y')]);
    ed.feed_key(key('p'));
    assert_eq!(state(&ed), "a-[X]>b\n");
}

/// An OSC 52 clipboard cannot be read, so it never holds anything new.
#[test]
fn an_unreadable_osc52_clipboard_leaves_p_on_the_ring() {
    use hume_ops::register::CLIPBOARD_REGISTER;

    let mut ed = editor_from("-[x]>\n");
    ed.state
        .registers
        .write_text(CLIPBOARD_REGISTER, vec!["MIRROR"]);
    seed_ring(&mut ed, &["R"]);
    ed.feed_key(key('p'));
    assert_eq!(state(&ed), "x-[R]>\n");
}

/// With nothing on the ring and a clipboard that cannot be read, `p` pastes
/// nothing and gives the clipboard's reason.
#[test]
fn p_with_an_empty_ring_and_a_failed_clipboard_reports_the_clipboard() {
    let mut ed = editor_from("-[x]>\n");
    ed.state.clipboard.force_unavailable();
    ed.feed_key(key('p'));
    assert_eq!(state(&ed), "-[x]>\n");
    assert!(
        status(&ed).starts_with("system clipboard unavailable"),
        "status: {}",
        status(&ed)
    );
}

// ── Equal-text collapse (repeat vs swap) ───────────────────────────────────────
//
// Applies to bare smart pastes only: plain paste and any `"<reg>`-prefixed
// paste always replace a non-collapsed selection. When the resolved values
// match the currently selected text one-to-one, the selections collapse
// first so the paste appends alongside the existing text; any mismatch
// (including a value/selection count mismatch) replaces it instead. See
// `collapse_if_repeat` in `commands/paste.rs`.

/// Pasting different text over a non-collapsed selection replaces it: the
/// baseline the equal-text rule is contrasted against.
#[test]
fn paste_over_different_selection_replaces() {
    let mut ed = editor_from("-[old]>\n");
    seed_ring(&mut ed, &["new"]);
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
    seed_ring(&mut ed, &["ab"]);
    ed.feed_key(key('p'));
    let buf = ed.doc().text().to_string();
    assert_eq!(
        buf.matches("ab").count(),
        2,
        "identical paste over a matching multi-char selection appends; buf={buf:?}"
    );
}

/// A backward selection (head < anchor) still collapses to the span's `end`
/// for `smart-paste-after`, not to `head()`, which on a backward selection
/// is the *left* edge and would land the collapse mid-span.
#[test]
fn paste_after_repeat_over_backward_selection_collapses_to_end() {
    let mut ed = editor_from("-[ab]>cd\n");
    seed_ring(&mut ed, &["ab"]);
    call(&mut ed, "flip-selections"); // anchor/head swap; start()/end() unchanged
    ed.feed_key(key('p'));
    assert_eq!(
        state(&ed),
        "ab-[ab]>cd\n",
        "backward selection must still collapse to the span's end (start()/last()), not head()"
    );
}

/// `smart-paste-before` collapses to `start()`, not `anchor()`.
#[test]
fn paste_before_repeat_over_backward_selection_collapses_to_start() {
    let mut ed = editor_from("-[ab]>cd\n");
    seed_ring(&mut ed, &["ab"]);
    call(&mut ed, "flip-selections");
    ed.feed_key(key('P'));
    assert_eq!(
        state(&ed),
        "-[ab]>abcd\n",
        "backward selection must still collapse to the span's start (start()), not anchor()"
    );
}

/// Multi-cursor: every selection's value matches. All-or-nothing, so every
/// cursor appends.
#[test]
fn paste_multi_cursor_all_match_appends_each() {
    let mut ed = editor_from("-{ab}>x-[cd]>\n");
    ed.state.kill_ring.push(vec!["ab", "cd"]);
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

/// Multi-cursor: one selection's value mismatches. All-or-nothing means the
/// whole set replaces, not a mix of append and replace.
#[test]
fn paste_multi_cursor_partial_match_replaces_all() {
    let mut ed = editor_from("-{ab}>x-[cd]>\n");
    ed.state.kill_ring.push(vec!["ab", "ZZ"]); // 2nd doesn't match "cd"
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

/// A value-count/selection-count mismatch always replaces: equality can't
/// even be evaluated per-selection when the op joins values across the board.
#[test]
fn paste_value_count_mismatch_replaces() {
    let mut ed = editor_from("-{ab}>x-[cd]>\n");
    seed_ring(&mut ed, &["Z"]); // 1 value, 2 selections
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
    seed_ring(&mut ed, &["AB\n"]);
    ed.feed_key(key('p'));
    let buf = ed.doc().text().to_string();
    assert_eq!(
        buf.matches("AB\n").count(),
        2,
        "identical linewise paste over a matching linewise selection appends; buf={buf:?}"
    );
}

/// Matching clipboard text over an identical selection duplicates it, rather
/// than a same-text no-op. The rule is "append", not "skip".
#[test]
fn paste_matching_clipboard_over_identical_selection_duplicates() {
    let mut ed = editor_from("-[ab]>cd\n");
    seed_clipboard(&mut ed, "ab");
    ed.feed_key(key('p')); // new clipboard "ab" == selected "ab" → append
    let buf = ed.doc().text().to_string();
    assert_eq!(
        buf.matches("ab").count(),
        2,
        "matching clipboard text duplicates rather than no-op replacing; buf={buf:?}"
    );
}

/// A repeat paste takes in a clipboard that changed externally between the
/// two presses, so the second replaces the first rather than appending: the
/// resolved value differs from the selection.
#[test]
fn repeat_clipboard_paste_replaces_when_clipboard_changed_externally() {
    let mut ed = editor_from("-[xy]>\n");
    seed_clipboard(&mut ed, "xy");

    ed.feed_key(key('p')); // new clipboard "xy" == selected "xy" → append
    let buf1 = ed.doc().text().to_string();
    assert_eq!(
        buf1.matches("xy").count(),
        2,
        "first paste matches the selection and appends; buf={buf1:?}"
    );

    // External clipboard change before the second press.
    seed_clipboard(&mut ed, "ZZ");

    ed.feed_key(key('p')); // new clipboard "ZZ" taken in → ring head
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
/// still replaces: the equal-text collapse is bare-only, and `"Xp` shares
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

/// `paste-ring-older` / `paste-ring-newer` (`[` / `]`) with no paste to
/// cycle leave the text alone and say why.
#[test]
fn paste_ring_older_empty_ring_is_noop() {
    let mut ed = editor_from("-[a]>bc\n");
    let before = state(&ed);
    ed.feed_key(key('['));
    assert_eq!(state(&ed), before, "[ on empty ring is a no-op");
    assert_eq!(status(&ed), "no paste to cycle");
    // Capture fresh snapshot so ] is verified against actual post-[ state,
    // not the original.
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

    // Open paste session: `p` reads ring head (C\n).
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

/// `[` after an explicit `"cp` skips the ring head when it holds the
/// clipboard text: pasting it again would leave the buffer as it was.
#[test]
fn paste_ring_older_after_explicit_clipboard_paste_skips_the_same_text() {
    use hume_ops::register::CLIPBOARD_REGISTER;

    let mut ed = editor_from("-[x]>\n");
    seed_clipboard(&mut ed, "bar");
    seed_ring(&mut ed, &["foo", "bar"]);

    ed.feed_keys([key('"'), key(CLIPBOARD_REGISTER), key('p')]);
    assert_eq!(state(&ed), "x-[bar]>\n", "\"cp pastes the clipboard");
    ed.feed_key(key('['));
    assert_eq!(state(&ed), "x-[foo]>\n", "[ steps past the head to foo");
}

/// `y` writes the same line to the ring head and the clipboard. After an edit,
/// `p` pastes it from the clipboard, and `[` steps past the head to the line
/// yanked before it.
#[test]
fn paste_ring_older_after_yank_edit_paste_skips_the_yanked_line() {
    let mut ed = editor_from("-[a]>\nb\nc\n");
    ed.feed_key(key('x'));
    ed.feed_key(key('y')); // ring = [a\n]
    ed.feed_key(key('j'));
    ed.feed_key(key('x'));
    ed.feed_key(key('y')); // ring = [b\n, a\n], clipboard = b\n
    ed.feed_key(key('j'));
    ed.feed_key(key('i'));
    ed.feed_key(key('z'));
    ed.feed_key(key_esc());

    ed.feed_key(key('p'));
    assert_eq!(state(&ed), "a\nb\nzc\n-[b\n]>", "p pastes the clipboard");
    ed.feed_key(key('['));
    assert_eq!(state(&ed), "a\nb\nzc\n-[a\n]>", "[ steps past the head");
}

/// `[` after a clipboard paste is refused when the ring's only entry is the
/// clipboard text, and the clipboard paste stays.
#[test]
fn paste_ring_older_after_clipboard_paste_refuses_when_only_the_same_text() {
    let mut ed = editor_from("-[x]>\n");
    seed_clipboard(&mut ed, "bar");
    seed_ring(&mut ed, &["bar"]);

    ed.feed_key(key('p'));
    ed.feed_key(key('['));
    assert_eq!(state(&ed), "x-[bar]>\n");
    assert_eq!(status(&ed), "no older entry");
}

/// The entry a paste session ends on becomes the ring head.
#[test]
fn the_entry_a_paste_session_ends_on_becomes_the_ring_head() {
    let mut ed = editor_from("-[x]>\n");
    seed_ring(&mut ed, &["c", "b", "a"]);

    ed.feed_keys([key('"'), key('k'), key('p'), key('[')]);
    assert_eq!(state(&ed), "x-[b]>\n");
    ed.feed_key(key(';'));
    ed.feed_keys([key('"'), key('k'), key('p')]);
    assert_eq!(state(&ed), "xb-[b]>\n", "\"kp reads the cycled-to entry");
}

/// After an edit, `p` pastes the clipboard, and the entry the previous paste
/// session ended on is one `[` away.
#[test]
fn after_an_edit_the_cycled_to_entry_is_one_step_past_the_clipboard() {
    let mut ed = editor_from("-[x]>\n");
    seed_clipboard(&mut ed, "X");
    seed_ring(&mut ed, &["c", "b", "a"]);

    ed.feed_keys([key('"'), key('k'), key('p'), key('[')]);
    ed.feed_keys([key('i'), key('z'), key_esc()]);
    assert_eq!(state(&ed), "x-[z]>b\n");
    ed.feed_key(key('p'));
    assert_eq!(
        state(&ed),
        "xz-[X]>b\n",
        "p after an edit reads the clipboard"
    );
    ed.feed_key(key('['));
    assert_eq!(state(&ed), "xz-[b]>b\n");
}

/// A smart paste from the clipboard puts the clipboard text on the ring, and
/// cycling past it leaves it as the second entry.
#[test]
fn a_clipboard_paste_joins_the_ring_below_the_entry_cycled_to() {
    let mut ed = editor_from("-[z]>\n");
    seed_clipboard(&mut ed, "X");
    seed_ring(&mut ed, &["c", "b", "a"]);

    ed.feed_key(key('p'));
    assert_eq!(state(&ed), "z-[X]>\n");
    ed.feed_key(key('['));
    assert_eq!(state(&ed), "z-[a]>\n", "[ starts at the ring's old head");
    ed.feed_key(key(';'));
    ed.feed_keys([key('"'), key('k'), key('p')]);
    assert_eq!(state(&ed), "za-[a]>\n");
    ed.feed_key(key('['));
    assert_eq!(state(&ed), "za-[X]>\n");
    ed.feed_key(key('['));
    assert_eq!(state(&ed), "za-[b]>\n");
}

/// A clipboard equal to a ring entry moves that entry to the head instead of
/// showing up twice while cycling.
#[test]
fn a_clipboard_equal_to_a_ring_entry_is_not_cycled_to_twice() {
    let mut ed = editor_from("-[z]>\n");
    seed_clipboard(&mut ed, "c");
    seed_ring(&mut ed, &["c", "b", "a"]);

    ed.feed_key(key('p'));
    ed.feed_keys([key('['), key('[')]);
    assert_eq!(state(&ed), "z-[b]>\n");
    ed.feed_key(key('['));
    assert_eq!(state(&ed), "z-[b]>\n");
    assert_eq!(status(&ed), "no older entry");
}

/// `"cp` reads the clipboard without putting it on the ring.
#[test]
fn an_explicit_clipboard_paste_leaves_the_ring_alone() {
    use hume_ops::register::CLIPBOARD_REGISTER;

    let mut ed = editor_from("-[z]>\n");
    seed_clipboard(&mut ed, "X");
    seed_ring(&mut ed, &["b", "a"]);

    ed.feed_keys([key('"'), key(CLIPBOARD_REGISTER), key('p'), key(';')]);
    ed.feed_keys([key('"'), key('k'), key('p'), key('[')]);
    assert_eq!(state(&ed), "zX-[b]>\n");
    ed.feed_key(key('['));
    assert_eq!(status(&ed), "no older entry");
}

/// Select a line with `x`, delete with `d`, move with `j`, then paste via
/// explicit ring head (`"kp`): the deleted line must appear as its own line *below*
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

    // "A\n" must land as its own line below C, not inside C's text.
    assert_eq!(
        state(&ed),
        "B\nC\n-[A\n]>",
        "\"kp on a linewise ring entry must paste as a new line, not inline"
    );
}

/// `[`/`]` cycle within a paste session REPLACES the previous paste, never
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

    // [: cycle older (slot 0 → slot 1 = A\n). Must REPLACE B, not add another.
    ed.feed_key(key('['));
    let after_older = ed.doc().text().to_string();
    assert_eq!(
        after_older.matches("A\n").count(),
        1,
        "[ replaces paste with A"
    );

    // ]: cycle newer (slot 1 → slot 0 = B\n). Must REPLACE A.
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
/// it back with the head: collapsed selection is not an obstacle.
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
    assert!(!buf.contains('X'), "] replaces X, no 'X' remains");
}

/// `P` (paste-before) opens a before-session; `[`/`]` must re-paste BEFORE the
/// cursor, not after it.
#[test]
fn paste_before_cycle_stays_before_charwise() {
    let mut ed = editor_from("-[c]>d\n"); // cursor on 'c' at index 0
    seed_ring(&mut ed, &["X", "Y"]); // head=Y, slot 1=X

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
    seed_ring(&mut ed, &["X", "Y"]); // head=Y

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
    seed_ring(&mut ed, &["X\n", "Y\n"]); // head=Y\n

    // "kP: linewise paste-before ring head ("Y\n"), inserts above line 0.
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

/// `p [ p` duplicates the currently-cycled entry: the cycled-to entry is the
/// ring head, so the following bare `p` resolves it and, matching what's now
/// selected, appends rather than replacing.
#[test]
fn paste_after_cycle_appends_cycled_entry() {
    let mut ed = editor_from("-[x]>\n");
    seed_clipboard(&mut ed, "CLIP");
    seed_ring(&mut ed, &["RING"]); // ring head = "RING"

    // p: new clipboard "CLIP", pushed onto the ring.
    ed.feed_key(key('p'));
    assert!(
        ed.doc().text().to_string().contains("CLIP"),
        "first p must paste the new clipboard"
    );

    // [: cycle_older 0→1="RING"; replaces "CLIP" with "RING".
    ed.feed_key(key('['));
    // p: ring head "RING" again → matches selection → append.
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
    let mut ed = editor_from("-[ab]>\n");
    // Seed clipboard with "CLIP", distinct from the ring entry, so the text
    // shows which source each p read.
    seed_clipboard(&mut ed, "CLIP");
    ed.feed_key(key('d')); // delete "ab" → ring = ["ab", "CLIP"]
    ed.feed_key(key('p')); // smart-p reads ring head "ab"
    ed.feed_key(key('p')); // "ab" again → matches → append
    let buf = ed.doc().text().to_string();
    assert_eq!(
        buf.matches("ab").count(),
        2,
        "two consecutive p presses must stack two copies of 'ab'"
    );
    assert!(
        !buf.contains("CLIP"),
        "clipboard not used: repeat reads the ring, not the clipboard"
    );
}

/// Consecutive `p` presses append when the previous paste came from the CLIPBOARD
/// and the kill ring is empty: the second `p` must not be a no-op.
#[test]
fn consecutive_clipboard_paste_appends() {
    let mut ed = editor_from("-[z]>\n");
    seed_clipboard(&mut ed, "xy");
    // ring is empty: this is the regression case

    ed.feed_key(key('p')); // new clipboard → ring head "xy", inserted after 'z'
    ed.feed_key(key('p')); // ring head "xy" → matches selection → append
    let buf = ed.doc().text().to_string();
    assert_eq!(
        buf.matches("xy").count(),
        2,
        "two consecutive p presses must stack two copies even with an empty kill ring"
    );
}

/// Consecutive `p` after a clipboard paste repeats the clipboard text, which
/// the first press put on the ring above the older head.
#[test]
fn consecutive_paste_repeats_the_clipboard_over_the_older_head() {
    let mut ed = editor_from("-[z]>\n");
    seed_clipboard(&mut ed, "xy");
    seed_ring(&mut ed, &["ZZ"]); // ring has different content

    ed.feed_key(key('p')); // new clipboard → ring head "xy"
    ed.feed_key(key('p')); // repeats "xy", not the older "ZZ"
    let buf = ed.doc().text().to_string();
    assert_eq!(buf.matches("xy").count(), 2, "clipboard value repeated");
    assert!(
        !buf.contains("ZZ"),
        "ring head must not appear: repeat re-reads the clipboard, not the ring"
    );
}

// ── Plain paste (`paste-after`/`paste-before`) ────────────────────────────────
//
// `p`/`P` dispatch the smart variants (see the smart-p block above); these
// commands are reachable by name only (`:`, `call!`, or a plugin keymap).
// Dispatched the same way `classic-paste`'s Steel wrappers and
// `async_job_steel.rs`/`dot_repeat/` dispatch a command by name: through
// `execute_keymap_command`, the same pipeline a keymap or `(call! …)` uses
// (not a direct fn call, and unlike `:`, it doesn't itself dispatch any
// unrelated command in between two calls).

/// Bare plain paste reads the kill-ring head, not the clipboard. Unlike
/// smart-p, it never takes in the clipboard when there's no register
/// prefix.
#[test]
fn plain_paste_reads_ring_not_clipboard() {
    let mut ed = editor_from("-[x]>\n");
    seed_clipboard(&mut ed, "CLIP");
    seed_ring(&mut ed, &["RING"]);

    call(&mut ed, "paste-after");

    let buf = ed.doc().text().to_string();
    assert!(buf.contains("RING"), "plain paste must read the ring head");
    assert!(
        !buf.contains("CLIP"),
        "plain paste must not fall back to the clipboard"
    );
}

/// `paste-before` (plain) reads the ring head and inserts before the
/// selection: the same source rule as `paste-after`, opposite side. Every
/// other plain-paste test in this file dispatches `paste-after`; this one
/// covers the `before` path (`do_paste`'s `before` flag).
#[test]
fn plain_paste_before_reads_ring_head() {
    let mut ed = editor_from("x-[y]>z\n");
    seed_ring(&mut ed, &["RING"]);

    call(&mut ed, "paste-before");

    let buf = ed.doc().text().to_string();
    assert_eq!(
        buf, "xRINGyz\n",
        "plain paste-before must insert before 'y'"
    );
}

/// Two consecutive plain pastes replace, never stack: plain paste is dumb
/// by design: it always replaces a non-collapsed selection, with no
/// equal-text check at all (that's smart-paste-only; see `collapse_if_repeat`'s
/// doc). A script driving plain paste never has to inspect the selection
/// before pasting; to get an append, it collapses the selection itself first.
#[test]
fn plain_paste_does_not_stack() {
    let mut ed = editor_from("-[x]>\n");
    seed_ring(&mut ed, &["RING"]);

    call(&mut ed, "paste-after");
    call(&mut ed, "paste-after");

    let buf = ed.doc().text().to_string();
    assert_eq!(
        buf.matches("RING").count(),
        1,
        "second plain paste must replace the first, not stack a copy; buf={buf:?}"
    );
}

/// `[` after a plain paste still cycles: plain paste opens the same session
/// smart paste does.
#[test]
fn plain_paste_opens_ring_cycle_session() {
    let mut ed = editor_from("-[x]>\n");
    seed_ring(&mut ed, &["OLDER", "RING"]);

    call(&mut ed, "paste-after");
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
    seed_ring(&mut ed, &["RING"]);

    ed.handle_key(key('"'));
    ed.handle_key(key('3'));
    call(&mut ed, "paste-after");

    let buf = ed.doc().text().to_string();
    assert!(
        buf.contains("REG3"),
        "\"3 + plain paste must read register 3"
    );
    assert!(!buf.contains("RING"));
}

/// `"b` (black hole) on a plain paste is refused, same as on smart paste.
#[test]
fn plain_paste_black_hole_is_noop() {
    let mut ed = editor_from("-[x]>\n");
    seed_ring(&mut ed, &["RING"]);

    ed.handle_key(key('"'));
    ed.handle_key(key('b'));
    call(&mut ed, "paste-after");

    assert_eq!(
        ed.doc().text().to_string(),
        "x\n",
        "\"b + plain paste must be a no-op"
    );
    assert_eq!(status(&ed), "register 'b' is empty");
}

/// A paste with nothing to paste is refused, so a `.`, a macro or a `call!`
/// running it sees the step fail.
#[test]
fn plain_paste_with_an_empty_kill_ring_is_refused() {
    use hume_scripting::host::CommandHost;

    let mut ed = editor_from("-[x]>\n");
    let (pid, bid) = (ed.view.focused_pane(), ed.focused_buffer_id());

    let ran = live_host!(ed)
        .run_command_sync(
            "paste-after",
            hume_core::PaneHandle::with_pane(bid, pid),
            NativeCall {
                count: Count::typed(1),
                ..NativeCall::default()
            },
        )
        .expect("a refused command is still a successful dispatch");

    assert!(!ran, "an empty paste must be refused");
    assert_eq!(ed.doc().text().to_string(), "x\n");
    assert_eq!(status(&ed), "kill ring is empty");
}

/// `"b` (black hole) on a *smart* paste is a no-op too: `resolve_smart`
/// routes an explicit register through `resolve_explicit_register`, the same
/// path plain paste uses, so black-hole shortcuts identically for both.
/// `plain_paste_black_hole_is_noop` above covers the plain path only.
#[test]
fn smart_paste_black_hole_is_noop() {
    let mut ed = editor_from("-[x]>\n");
    seed_ring(&mut ed, &["RING"]);

    ed.handle_key(key('"'));
    ed.handle_key(key('b'));
    ed.feed_key(key('p')); // "bp → smart-paste-after, black hole

    assert_eq!(
        ed.doc().text().to_string(),
        "x\n",
        "\"b + smart paste must be a no-op"
    );
    assert_eq!(status(&ed), "register 'b' is empty");
}

/// A smart paste right after a plain paste appends: both read the ring head,
/// and it matches what's now selected.
#[test]
fn smart_paste_appends_after_plain_paste() {
    let mut ed = editor_from("-[x]>\n");
    seed_ring(&mut ed, &["RING"]);

    call(&mut ed, "paste-after"); // plain: reads ring head "RING"
    ed.feed_key(key('p')); // smart: ring head "RING" again → append

    let buf = ed.doc().text().to_string();
    assert_eq!(
        buf.matches("RING").count(),
        2,
        "smart paste after a plain paste must append; buf={buf:?}"
    );
}

/// Plain paste on a read-only buffer reports and makes no edit.
#[test]
fn plain_paste_refuses_read_only_buffer() {
    let mut ed = editor_with_read_only_view("x\n", "view");
    seed_ring(&mut ed, &["RING"]);

    call(&mut ed, "paste-after");

    assert_eq!(
        ed.doc().text().to_string(),
        "x\n",
        "read-only buffer must refuse the paste"
    );
    assert_eq!(status(&ed), read_only_refusal(&ed));
}

/// A paste reaching the focused pane while its Insert session is still open
/// (a hook or timer `call!`ing `paste-after` mid-typing) must refuse, not
/// open a paste session over the Insert one. Opening it would replace the
/// Insert group in the editor's single session slot, losing every keystroke
/// typed so far from undo.
#[test]
fn paste_during_an_open_insert_session_is_refused_and_leaves_the_session_intact() {
    use hume_scripting::host::CommandHost;

    let mut ed = editor_from("-[a]>bc\n");
    let bid = ed.focused_buffer_id();
    ed.feed_key(key('y'));
    ed.feed_key(key('i'));
    type_chars(&mut ed, "Q");
    assert_eq!(ed.state.mode(), Mode::Insert, "sanity: Insert is open");

    let pane = focused_pane(&ed);
    let ran = live_host!(ed)
        .run_command_sync("paste-after", pane, NativeCall::default())
        .expect("a refused paste reports through its return value, not Err");
    assert!(!ran, "paste-after must report that it refused");
    assert_eq!(
        ed.state.mode(),
        Mode::Insert,
        "the Insert session must survive"
    );
    assert_eq!(ed.state.buffers.get(bid).text().to_string(), "Qabc\n");

    type_chars(&mut ed, "R");
    ed.feed_key(key_esc());
    assert_eq!(ed.state.buffers.get(bid).text().to_string(), "QRabc\n");
    ed.feed_key(key('u'));
    assert_eq!(
        ed.state.buffers.get(bid).text().to_string(),
        "abc\n",
        "one undo must revert the whole Insert session, keystrokes before and after the refusal alike"
    );
}

/// Same conflict as the test above, but for the one-keystroke window right
/// after `i`/`a`/`o`, before anything has been typed: the Insert session is
/// real and open, but its `EditGroup` is still empty. Emptiness alone must
/// not be read as "this is replay's own placeholder, safe to retarget".
/// See `EditSessionKind::Replay`'s own doc.
#[test]
fn paste_during_an_empty_open_insert_session_is_refused_and_leaves_the_session_intact() {
    use hume_scripting::host::CommandHost;

    let mut ed = editor_from("-[a]>bc\n");
    let bid = ed.focused_buffer_id();
    ed.feed_key(key('y'));
    ed.feed_key(key('i'));
    assert_eq!(ed.state.mode(), Mode::Insert, "sanity: Insert is open");

    let pane = focused_pane(&ed);
    let ran = live_host!(ed)
        .run_command_sync("paste-after", pane, NativeCall::default())
        .expect("a refused paste reports through its return value, not Err");
    assert!(!ran, "paste-after must report that it refused");
    assert_eq!(
        ed.state.mode(),
        Mode::Insert,
        "the Insert session must survive"
    );
    assert_eq!(ed.state.buffers.get(bid).text().to_string(), "abc\n");

    type_chars(&mut ed, "R");
    ed.feed_key(key_esc());
    assert_eq!(ed.state.buffers.get(bid).text().to_string(), "Rabc\n");
    ed.feed_key(key('u'));
    assert_eq!(
        ed.state.buffers.get(bid).text().to_string(),
        "abc\n",
        "one undo must revert the whole (still-real, still-Insert) session"
    );
}

/// A direct edit landing while a ring-cycle Paste session is still open on
/// the same pane (bracketed-paste's own `apply_normal_mode_paste`, which
/// bypasses the dispatch pipeline and so never runs `step_paste_commit`)
/// must not leave that edit invisible to a following `[`. `doc_ops::
/// apply_doc_edit` commits the open Paste session first, so `[` (which
/// requires a *still-open* Paste session to do anything at all) finds none
/// and no-ops. The open session's `snapshot` holds the pre-bracketed-paste
/// text, and a cycle re-derives the whole buffer from it
/// (`apply_edit_regrouped`'s contract: "every cycle cleanly discards the
/// previous paste output"), so cycling that session would discard the
/// bracketed paste too.
#[test]
fn direct_edit_during_open_paste_session_commits_it_first() {
    let mut ed = editor_from("-[c]>d\n"); // cursor on 'c' at index 0
    seed_ring(&mut ed, &["X", "Y"]); // head=Y

    // "kp: paste-after ring head ("Y") after 'c' → "cYd\n", with a Paste
    // session left open (native paste always leaves one open, for a
    // following `[`/`]`). Nothing has dispatched since, so it's still open.
    ed.feed_key(key('"'));
    ed.feed_key(key('k'));
    ed.feed_key(key('p'));
    assert_eq!(
        ed.doc().text().to_string(),
        "cYd\n",
        "setup: p must paste Y after the cursor"
    );

    // Bracketed paste lands directly, bypassing dispatch entirely.
    ed.feed_paste("Z");
    assert!(
        ed.doc().text().to_string().contains('Z'),
        "setup: the bracketed paste must land"
    );

    // `[`: cycle to slot 1 ("X"). Must not silently discard 'Z' by
    // re-pasting from the pre-bracketed-paste snapshot.
    ed.feed_key(key('['));
    assert!(
        ed.doc().text().to_string().contains('Z'),
        "the direct edit must survive `[`, got {:?}",
        ed.doc().text().to_string()
    );
}

/// A selection that ends on a line break but does not start its line is
/// charwise: pasting it goes inline, not onto its own line.
#[test]
fn yanking_a_selection_ending_on_a_break_pastes_inline() {
    let mut ed = editor_from("a-[bc\n]>def\n");
    ed.handle_key(key('y'));
    ed.handle_key(key('j'));
    ed.handle_key(key('p'));

    assert_eq!(state(&ed), "abc\ndef-[bc\n]>\n");
}

/// With the write-only OSC 52 clipboard, `y` then an edit then `p` pastes what
/// was yanked, from the in-memory mirror and without a warning.
#[test]
fn smart_p_after_edit_with_osc52_clipboard_pastes_yank_without_warning() {
    use hume_core::Severity;

    let mut ed = editor_from("-[ab]>cd\n");
    ed.state.clipboard = crate::editor::clipboard::SystemClipboard::osc52();

    ed.feed_key(key('y'));
    assert_eq!(
        ed.state.clipboard.take_pending_osc52().as_deref(),
        Some("ab"),
        "yank queues the text for the terminal"
    );
    ed.feed_key(key('d')); // an edit since the yank: smart-p now reads the clipboard
    ed.feed_key(key('p'));

    assert_eq!(ed.doc().text().to_string(), "cabd\n");
    assert!(
        !ed.state
            .message_log
            .entries()
            .any(|e| e.severity == Severity::Warning),
        "an OSC 52 clipboard is not a failure"
    );
}

/// Pressing a key that reads an operand commits the open paste session even
/// when the read is then cancelled, as the command's own keypress would.
#[test]
fn a_cancelled_operand_read_commits_the_paste_session() {
    for read_key in ['/', 'f'] {
        let mut ed = editor_from("-[ab]>c\n");
        ed.feed_key(key('d'));
        ed.feed_key(key('p'));
        assert!(ed.state.open_session().is_some(), "p opens a paste session");
        ed.feed_key(key(read_key));
        ed.feed_key(key_esc());
        assert!(
            ed.state.open_session().is_none(),
            "{read_key} then Esc left the paste session open"
        );
    }
}
