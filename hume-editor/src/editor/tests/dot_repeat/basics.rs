//! Dot-repeat of plain edits, pastes and Insert sessions.

use super::*;
use pretty_assertions::assert_eq;

// ── Dot-repeat tests ──────────────────────────────────────────────────────────

/// `d` deletes the selection. Moving then pressing `.` should delete the next selection.
#[test]
fn dot_repeats_delete() {
    // Cursor starts at 'f'. `foo` is already selected; `d` deletes it.
    // Then from the space at pos 0, `w` selects "bar": "bar" is the first
    // (and only) word on its line, so its leading space is indentation and
    // is never absorbed; there's no trailing space either (EOL follows), so
    // the default around-word span is bare. `.` deletes just "bar".
    let mut ed = editor_from("-[foo]> bar\n");
    ed.feed_key(key('d')); // delete "foo" → " bar\n", cursor at 0 (space)
    assert_eq!(ed.doc().text().to_string(), " bar\n");

    ed.feed_key(key('w')); // from space, select "bar" (bare, indent kept)
    ed.feed_key(key('.')); // repeat delete
    assert_eq!(ed.doc().text().to_string(), " \n");
}

/// `c` + typed text + Esc should be replayable: the replacement text is reused.
#[test]
fn dot_repeats_change_with_insert() {
    let mut ed = editor_from("-[foo]> bar\n");

    ed.feed_key(key('c')); // change: delete "foo", enter Insert
    ed.feed_key(key('h'));
    ed.feed_key(key('i'));
    ed.feed_key(key_esc()); // back to Normal; buffer is "hi bar"

    assert_eq!(ed.doc().text().to_string(), "hi bar\n");

    // Move to "bar" and repeat. "bar" has no trailing space (EOL follows)
    // but does have a leading one, so `w` picks up " bar" (default
    // around-word), so the replayed change removes that leading space too.
    ed.feed_key(key('w')); // select " bar"
    ed.feed_key(key('.')); // repeat: delete " bar", insert "hi"

    assert_eq!(ed.doc().text().to_string(), "hihi\n");
}

/// A repeatable command refused on a read-only buffer must not clobber
/// the recorded change: it changed nothing, so there is nothing new to
/// repeat.
///
/// `cmd_delete`'s edit is refused with an `Err` before it touches the
/// buffer, and the pipeline discards a refused dispatch's fresh
/// recording (`EditorState::end_change`), so the user's earlier `change` stays in the slot.
#[test]
fn read_only_refusal_does_not_clobber_dot_repeat() {
    let mut ed = editor_from("-[foo]> bar\n");

    // Stage a real repeatable action on the writable buffer.
    ed.feed_key(key('c'));
    ed.feed_key(key('h'));
    ed.feed_key(key('i'));
    ed.feed_key(key_esc());
    assert_eq!(
        ed.state.last_action().unwrap().command.as_ref(),
        "change",
        "setup: change must be the stored action"
    );

    // Switch focus to a read-only view buffer.
    ed.report(Severity::Warning, "msg".to_string());
    ed.execute_typed("messages", None).unwrap();
    assert!(
        ed.doc().is_read_only(),
        "setup: focused buffer must be read-only"
    );

    // Stage a selection, then attempt a repeatable edit. Refused, no text
    // changes, but the pipeline still runs the AFTER stage unconditionally.
    ed.feed_key(key('x')); // select-line (motion-only; not blocked on read-only)
    ed.feed_key(key('d')); // delete, refused: the buffer is read-only

    assert_eq!(
        ed.state.last_action().unwrap().command.as_ref(),
        "change",
        "a refused delete on a read-only buffer must not overwrite the prior change action"
    );
}

/// A repeatable command refused before it changes anything runs no
/// recording scope once it returns.
#[test]
fn a_refused_repeatable_command_leaves_no_recording_scope() {
    let mut ed = editor_from("-[foo]> bar\n");
    ed.report(Severity::Warning, "msg".to_string());
    ed.execute_typed("messages", None).unwrap();
    assert!(
        ed.doc().is_read_only(),
        "setup: focused buffer must be read-only"
    );

    ed.feed_key(key('x'));
    ed.feed_key(key('d'));

    assert!(ed.state.recording_scope.running().is_none());
}

/// A repeatable Steel command whose body fails runs no recording scope once
/// it returns, so the next repeatable command records its own change.
#[test]
fn a_failed_steel_body_leaves_the_next_change_recordable() {
    let mut ed = editor_with_steel(
        "-[f]>oo bar\n",
        r#"(define-command! "boom" "" (lambda (pane) (error "boom")) #:repeatable #t)"#,
    );
    ed.execute_keymap_command("boom".into(), Some(1), false);
    assert!(ed.state.recording_scope.running().is_none());

    ed.feed_key(key('d'));
    assert_eq!(
        ed.state.last_action().map(|a| a.command.as_ref()),
        Some("delete")
    );
}

/// A replayed `c` also ends with the replacement selected: the anchor
/// capture in `cmd_change` re-fires on replay (gated on the group being
/// open, which `replay_dot` pre-opens), same as the interactive path.
#[test]
fn dot_repeat_c_selects_replayed_replacement() {
    let mut ed = editor_from("-[foo]> bar\n");

    ed.feed_key(key('c'));
    ed.feed_key(key('h'));
    ed.feed_key(key('i'));
    ed.feed_key(key_esc());

    // "bar" has no trailing space (EOL follows) but does have a leading
    // one, so `w` picks up " bar" (default around-word); the replayed
    // change removes that leading space too, leaving no separator.
    ed.feed_key(key('w'));
    ed.feed_key(key('.'));

    assert_eq!(ed.doc().text().to_string(), "hihi\n");
    assert_eq!(state(&ed), "hi-[hi]>\n");
}

/// Pinning for `mii` is unconditional (not gated on `insert-select-on-exit`),
/// so a replayed `i` (not just `c`) must re-pin correctly too: `mii` after
/// `.` must select the *replayed* insertion, not the original one.
#[test]
fn mii_after_dot_repeat_selects_replayed_insertion() {
    let mut ed = editor_from("-[x]>\n");

    ed.feed_key(key('i'));
    ed.feed_key(key('a'));
    ed.feed_key(key('b'));
    ed.feed_key(key_esc());
    assert_eq!(ed.doc().text().to_string(), "abx\n");

    // Reposition onto 'x' directly rather than via `w`. Esc now selects the
    // typed "ab" itself (`insert-select-on-exit`), and "ab"/"x" are one word
    // ("abx", no separator), so a word motion from inside "ab" has nowhere
    // to advance to.
    set_cursor(&mut ed, 2);
    ed.feed_key(key('.')); // repeat insert "ab" before 'x'
    assert_eq!(ed.doc().text().to_string(), "ababx\n");

    ed.feed_key(key('m'));
    ed.feed_key(key('i'));
    ed.feed_key(key('i'));
    assert_eq!(state(&ed), "ab-[ab]>x\n");
}

/// `i` + typed text + Esc inserts at the selection start. `.` should replay that insert.
///
/// Second word (`y`), not just a following char (`x`): with `select-inserted-
/// text` on (default), Esc leaves "ab" selected rather than a bare cursor,
/// and "ab"/"x" glued together with no separator are one word token. `w`
/// from inside that token has nowhere to advance to and would be a no-op,
/// making `.` replay at the original position and land on the same string
/// by coincidence. A space-separated second word gives `w` a real target to
/// move onto, so the assertion actually exercises the replay position.
#[test]
fn dot_repeats_insert_before() {
    let mut ed = editor_from("-[x]> y\n");

    ed.feed_key(key('i')); // insert-at-selection-start, cursor collapses to start
    ed.feed_key(key('a'));
    ed.feed_key(key('b'));
    ed.feed_key(key_esc()); // back to Normal; buffer is "abx y"

    assert_eq!(ed.doc().text().to_string(), "abx y\n");

    // Move to "y" and repeat. "y" has no trailing space (EOL follows) but
    // does have a leading one, so `w` picks up " y" (default around-word,
    // same convention as `dot_repeats_change_with_insert` above).
    ed.feed_key(key('w')); // select " y"
    ed.feed_key(key('.')); // repeat insert "ab" before " y"

    assert_eq!(ed.doc().text().to_string(), "abxab y\n");
}

/// A replayed `a` with an empty typed run (nothing typed before Esc) must
/// step the cursor back exactly like the interactive path, not silently
/// stay put: the step-back flag lives on `PaneBufferState`, which a
/// replayed session sets and reads exactly as a live one does.
#[test]
fn dot_repeat_replays_a_empty_run_step_back() {
    let mut ed = editor_from("-[h]>ello\n");
    ed.feed_key(key('a'));
    ed.feed_key(key_esc());
    assert_eq!(
        state(&ed),
        "-[h]>ello\n",
        "sanity: interactive a<Esc> round-trips"
    );

    // Reposition onto 'e' and replay: `.` re-enters `a` there.
    set_cursor(&mut ed, 1);
    ed.feed_key(key('.'));

    // The replayed session must honor its step-back flag. If it were ignored,
    // the cursor would end one grapheme past 'e'.
    assert_eq!(state(&ed), "h-[e]>llo\n");
}

/// `r` + char replaces every character in the selection. `.` should replay with
/// the same replacement character.
#[test]
fn dot_repeat_replays_dedent() {
    // Insert session containing a dedent-on-Backspace must be replayable as a
    // unit: `.` re-enters insert at the new cursor and replays the Backspace,
    // which dedents the second indented line just like the first.
    //
    // 2-space indent + tw=4: Backspace at col 2 snaps to col 0 (deletes both
    // spaces). After the first dedent the cursor sits at col 0 of line 0; `j`
    // would land on col 0 of line 1 (where dedent doesn't apply), so step right
    // onto the content first. `.` then enters insert there and dedents.
    let mut ed = editor_from("  -[x]>\n  y\n");

    ed.feed_key(key('i')); // insert at 'x'
    ed.feed_key(key_backspace()); // dedent line 0 → "x\n  y\n", cursor on 'x'
    ed.feed_key(key_esc()); // back to Normal

    assert_eq!(ed.doc().text().to_string(), "x\n  y\n");

    ed.feed_key(key('j')); // down to col 0 of line 1 (first space)
    ed.feed_key(key('l')); // onto the second space
    ed.feed_key(key('l')); // onto 'y' (col 2, inside leading-ws end)
    ed.feed_key(key('.')); // replay insert-session: insert + Backspace dedents

    assert_eq!(ed.doc().text().to_string(), "x\ny\n");
}

#[test]
fn dot_repeats_replace() {
    // Use a space between words so `w` can navigate to the second word.
    let mut ed = editor_from("-[ab]> cd\n");

    ed.feed_key(key('r')); // reads the next key
    ed.feed_key(key('x')); // replace "ab" → "xx cd\n"

    assert_eq!(ed.doc().text().to_string(), "xx cd\n");

    // `w` from the "xx" selection (head at pos 1) selects "cd": no trailing
    // space (EOL follows), but a leading one, so the default around-word
    // span picks up " cd" instead. `r` replaces every grapheme in the
    // selection, including that space.
    ed.feed_key(key('w'));
    ed.feed_key(key('.')); // repeat replace with 'x' → "xxxxx\n"

    assert_eq!(ed.doc().text().to_string(), "xxxxx\n");
}

/// When `.` is given an explicit count, the recorded change's stored count
/// must not be corrupted: the explicit count is used for the replay but is NOT
/// written back into the stored action.
///
/// The original count must survive so that a subsequent plain `.` can still
/// reproduce the original repetition count.
#[test]
fn explicit_count_on_dot_does_not_corrupt_stored_count() {
    let mut ed = editor_from("-[foo]> bar\n");

    ed.feed_key(key('d')); // delete "foo"; stored count=1
    assert_eq!(
        ed.state.last_action().unwrap().count,
        1,
        "setup: stored count must be 1"
    );

    ed.feed_key(key('w')); // move to "bar"
    // Press `3.`: explicit count 3 is used for the replay but must NOT be
    // written back into the recorded change (drain restores the original action).
    ed.feed_key(key('3'));
    ed.feed_key(key('.'));

    // Replay must have happened (bar deleted).
    assert!(
        !ed.doc().text().to_string().contains("bar"),
        "bar must be deleted by the repeated command"
    );
    // Stored count must still be 1: the explicit-count replay must not corrupt it.
    assert_eq!(
        ed.state.last_action().unwrap().count,
        1,
        "stored count must survive explicit-count replay unchanged"
    );
}

/// After `.`, a single `u` should undo the entire replayed action as one step.
#[test]
fn dot_is_single_undo_step() {
    let mut ed = editor_from("-[foo]> bar\n");

    // `c` + "hi" + Esc = one undo step.
    ed.feed_key(key('c'));
    ed.feed_key(key('h'));
    ed.feed_key(key('i'));
    ed.feed_key(key_esc());
    assert_eq!(ed.doc().text().to_string(), "hi bar\n");

    // Move to "bar" and repeat. "bar" has no trailing space (EOL follows)
    // but does have a leading one, so `w` picks up " bar" (default
    // around-word), so the replayed change removes that leading space too.
    ed.feed_key(key('w'));
    ed.feed_key(key('.'));
    assert_eq!(ed.doc().text().to_string(), "hihi\n");

    // One undo undoes the `.` replay entirely.
    ed.feed_key(key('u'));
    assert_eq!(ed.doc().text().to_string(), "hi bar\n");
}

/// Pressing `.` before any edit has been recorded should be a no-op.
#[test]
fn dot_noop_without_prior_action() {
    let mut ed = editor_from("-[hello]> world\n");
    let before = state(&ed);
    ed.feed_key(key('.'));
    assert_eq!(state(&ed), before);
}

/// `o` (open-line-below) + typed text + Esc should be replayable.
#[test]
fn dot_repeats_open_line_below() {
    let mut ed = editor_from("-[a]>\nb\n");

    ed.feed_key(key('o')); // open line below "a", enter Insert
    ed.feed_key(key('x'));
    ed.feed_key(key_esc()); // back to Normal; buffer is "a\nx\nb"

    assert_eq!(ed.doc().text().to_string(), "a\nx\nb\n");

    // Move cursor to "b" and repeat.
    ed.feed_key(key('j')); // move down to 'x'
    ed.feed_key(key('j')); // move down to 'b'
    ed.feed_key(key('.')); // repeat: open line below "b", insert "x"

    assert_eq!(ed.doc().text().to_string(), "a\nx\nb\nx\n");
}

/// `p` (smart-paste-after) is repeatable: the register contents are pasted
/// again.
#[test]
fn dot_repeats_smart_paste_after() {
    let mut ed = editor_from("-[ab]>cd\n");

    // Yank "ab" then delete so we have something to paste.
    ed.feed_key(key('y')); // yank "ab" into default register
    ed.feed_key(key('d')); // delete "ab" → cursor on "cd"

    // Paste after.
    ed.feed_key(key('p')); // pastes "ab" after 'c' → "cabd", "ab" selected
    // Move one char right (off the pasted selection) then repeat.
    ed.feed_key(key('l')); // move right → collapsed cursor after the pasted text
    ed.feed_key(key('.')); // paste again
    let buf = ed.doc().text().to_string();
    assert_eq!(
        buf.matches("ab").count(),
        2,
        "expected exactly 2 occurrences of 'ab', got: {buf:?}"
    );
}

/// Plain `paste-after` (unbound by default, dispatched by name) is
/// repeatable too: `.repeatable()` on its registry entry is a separate
/// declaration from `smart-paste-after`'s, so a repeat must be verified
/// independently rather than assumed from the smart variant.
#[test]
fn dot_repeats_plain_paste_after() {
    let mut ed = editor_from("-[ab]>cd\n");

    seed_ring(&mut ed, &["ab"]);
    ed.execute_keymap_command("paste-after".into(), Some(1), false);
    // Plain paste never collapses on repeat. Move off the pasted text so
    // the replayed paste lands next to it instead of replacing it.
    ed.feed_key(key('l'));
    ed.feed_key(key('.')); // repeat the plain paste

    let buf = ed.doc().text().to_string();
    assert_eq!(
        buf.matches("ab").count(),
        2,
        "expected exactly 2 occurrences of 'ab', got: {buf:?}"
    );
}
