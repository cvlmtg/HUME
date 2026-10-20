use super::*;
use crate::editor::replay::{Answer, Input, Pick, Picked};
use crate::editor::settings::WordSelectsWhitespace;
use pretty_assertions::assert_eq;

/// `i` is the recorded unbound key `ch`.
fn is_key(i: &Input, ch: char) -> bool {
    matches!(i, Input::Key(k) if k.code == KeyCode::Char(ch))
}

/// `i` is the recorded binding of `name`.
fn is_binding(i: &Input, name: &str) -> bool {
    matches!(i, Input::Binding { name: n, .. } if n == name)
}

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

/// A repeatable command refused by `ensure_writable` must not clobber
/// the recorded change: it changed nothing, so there is nothing new to
/// repeat.
///
/// `cmd_delete` refuses via `ensure_writable`'s `Err` without touching the
/// buffer, and the pipeline discards a refused dispatch's fresh
/// recording (`EditorState::end_change`), so the user's earlier `change` stays in the slot.
/// Without that gate, the final assert would see `command == "delete"`.
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
    ed.feed_key(key('d')); // delete, refused by ensure_writable

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

// ── Selection-recipe dot-repeat tests ────────────────────────────────────────

/// `x d` (select-line, delete) records recipe `[select-line F]`. Pressing `.`
/// on the next line re-selects the whole line and deletes it.
///
/// Three-line buffer: the first `x d` leaves two lines and the second leaves
/// one.
///
/// Without the recipe replay, `.` would delete only the char
/// the cursor happened to be on (collapsed selection), not the whole line.
#[test]
fn dot_repeats_select_line_delete() {
    // Three lines; cursor on 'a'.
    let mut ed = editor_from("-[a]>aa\nbbb\nccc\n");

    ed.feed_key(key('x')); // select-line → "aaa\n" selected
    ed.feed_key(key('d')); // delete "aaa\n" → "bbb\nccc\n", cursor on 'b'
    assert_eq!(ed.doc().text().to_string(), "bbb\nccc\n");

    // The recipe must be [select-line] (not empty).
    assert_eq!(
        ed.state.last_action().unwrap().selection_recipe.len(),
        1,
        "recipe must contain select-line step"
    );

    // `.` replays: re-select current line ("bbb\n"), delete it.
    ed.feed_key(key('.'));
    assert_eq!(
        ed.doc().text().to_string(),
        "ccc\n",
        "`.` must re-select the current line and delete it"
    );
}

/// `x Ctrl-x d` selects two lines (one establish + one extend) and deletes them.
/// `.` replays the full two-step recipe, deleting the next two lines.
///
/// Four-line buffer: the first `x Ctrl-x d` leaves two lines, and the replay
/// deletes both, so one structural line remains.
#[test]
fn dot_repeats_extend_select_delete() {
    // Four lines; cursor on 'a'.
    let mut ed = editor_from("-[a]>aa\nbbb\nccc\nddd\n");

    ed.feed_key(key('x')); // select "aaa\n"
    ed.feed_key(key_ctrl('x')); // extend to "aaa\nbbb\n"
    ed.feed_key(key('d')); // delete → "ccc\nddd\n", cursor on 'c'
    assert_eq!(ed.doc().text().to_string(), "ccc\nddd\n");

    // Recipe must be [select-line F, select-line T] (establish + one extend).
    let recipe = &ed.state.last_action().unwrap().selection_recipe;
    assert_eq!(recipe.len(), 2, "recipe must have 2 steps");
    assert!(!recipe[0].extend, "first step must be Move (establish)");
    assert!(recipe[1].extend, "second step must be Extend");

    // `.` replays: x (select "ccc\n") + Ctrl-x (extend to "ccc\nddd\n") + d.
    ed.feed_key(key('.'));
    assert_eq!(
        ed.doc().text().to_string(),
        "\n",
        "`.` must re-select two lines and delete them"
    );
}

/// Navigation (`j` = move-down) before `x d` must NOT appear in the recipe.
/// Only the `x` (establish) step is recorded; `.` does NOT move down first.
///
/// Four-line buffer; `j x d` deletes line 1 ("bbb\n"),
/// leaving "aaa\nccc\nddd\n". `.` must re-select line 1 ("ccc\n") and delete
/// it, giving "aaa\nddd\n". If `j` were in the recipe, `.` would instead move
/// down from "ccc" to "ddd" first, deleting "ddd\n" and leaving "aaa\nccc\n".
///
/// The `is_collapsed()` guard is what keeps `j` (a Motion in Move mode) out of
/// the recipe.
#[test]
fn dot_repeat_navigation_not_in_recipe() {
    // Four lines so that the last-line structural-'\n' protection does not
    // interfere with any of the delete operations below.
    let mut ed = editor_from("-[a]>aa\nbbb\nccc\nddd\n");

    ed.feed_key(key('j')); // move-down: collapsed cursor on 'b', recipe cleared
    ed.feed_key(key('x')); // select-line "bbb\n": recipe = [select-line F]
    ed.feed_key(key('d')); // delete "bbb\n" → "aaa\nccc\nddd\n", cursor on 'c'
    assert_eq!(ed.doc().text().to_string(), "aaa\nccc\nddd\n");

    // Recipe must have exactly one step (x only; j must have been cleared).
    assert_eq!(
        ed.state.last_action().unwrap().selection_recipe.len(),
        1,
        "navigation (j) must not appear in the selection recipe"
    );

    // `.` replays x (selects current line "ccc\n") + d (deletes it).
    // If j were in the recipe, `.` would move down to "ddd\n" first and delete
    // that, leaving "aaa\nccc\n". The assertion below tells the two cases apart.
    ed.feed_key(key('.'));
    assert_eq!(
        ed.doc().text().to_string(),
        "aaa\nddd\n",
        "`.` must re-select and delete the current line, NOT move down first"
    );
}

/// `j d` (navigate then delete a collapsed single-char selection) records an
/// empty recipe. `.` replays just the delete on whatever the current selection is.
#[test]
fn dot_repeat_collapsed_cursor_empty_recipe() {
    let mut ed = editor_from("-[a]>aa\nbbb\nccc\n");

    ed.feed_key(key('j')); // move-down: collapsed on 'b', recipe cleared
    ed.feed_key(key('d')); // delete 'b' (1-char selection), recipe = []
    assert_eq!(ed.doc().text().to_string(), "aaa\nbb\nccc\n");

    // Recipe must be empty.
    assert_eq!(
        ed.state.last_action().unwrap().selection_recipe.len(),
        0,
        "empty recipe for plain single-char delete"
    );

    // `.` repeats just the delete (no line reselection).
    ed.feed_key(key('.'));
    assert_eq!(
        ed.doc().text().to_string(),
        "aaa\nb\nccc\n",
        "`.` must delete the current 1-char selection, not reselect a whole line"
    );
}

/// `x c <text> Esc` records recipe=[select-line F] + transcript=[...]. `.` on
/// another line re-selects the full line, runs change, and retypes the text.
///
/// Three-line buffer; `x c z Esc` on line 0 replaces "aaa" with 'z' and keeps
/// the line's '\n' → "z\nbbb\nccc\n". Then `.` on line 1 re-selects "bbb\n"
/// via the recipe, runs change, and inserts 'z' → "z\nz\nccc\n". Without the
/// recipe, `.` would change only the 1-char cursor on 'b' → "z\nzbb\nccc\n".
#[test]
fn dot_repeats_change_reselects_line() {
    // Three lines; `c` after select-line removes the content but keeps the `\n`.
    let mut ed = editor_from("-[a]>aa\nbbb\nccc\n");

    ed.feed_key(key('x')); // select "aaa\n", head on '\n'
    ed.feed_key(key('c')); // change: delete "aaa" (not '\n') → "\nbbb\nccc\n", cursor at 0
    ed.feed_key(key('z')); // type 'z' → "z\nbbb\nccc\n"
    ed.feed_key(key_esc()); // back to Normal, cursor on 'z'
    assert_eq!(ed.doc().text().to_string(), "z\nbbb\nccc\n");

    // Recipe must be [select-line]; transcript must be ['z', Esc].
    {
        let action = ed.state.last_action().unwrap();
        assert_eq!(
            action.selection_recipe.len(),
            1,
            "recipe must have select-line step"
        );
        assert_eq!(
            action.transcript.len(),
            2,
            "transcript must capture the typed char and the Esc binding"
        );
    }

    // Move to 'b' and press `.`.
    ed.feed_key(key('j')); // move-down to 'b' (line 1)
    ed.feed_key(key('.')); // re-select "bbb\n" via recipe, change, retype 'z'

    // Recipe re-selects "bbb\n", change deletes "bbb" (keeps '\n'),
    // inserts 'z' → "z\nz\nccc\n". Without recipe, only 'b' would be deleted → "z\nzbb\nccc\n".
    assert_eq!(
        ed.doc().text().to_string(),
        "z\nz\nccc\n",
        "`.` must re-select the full line, change it, and replay transcript"
    );
}

/// `undo` must clear the selection-recipe buffer so a stale `select-line` does
/// not leak into the next edit's recipe.
///
/// `undo` is `Untracked`, and the `SelectionTracking::Untracked` early-return
/// in `step_update_recipe` (`commands/pipeline.rs`) is what clears the recipe.
#[test]
fn undo_clears_selection_recipe() {
    let mut ed = editor_from("-[a]>aa\nbbb\n");

    // Establish a recipe via `x`.
    ed.feed_key(key('x'));
    assert_eq!(
        ed.state.selection_recipe.len(),
        1,
        "setup: x must establish a 1-step recipe"
    );

    // `u` (undo) is a non-repeatable EditorCmd → must clear the recipe.
    ed.feed_key(key('u'));
    assert_eq!(
        ed.state.selection_recipe.len(),
        0,
        "undo must clear the selection recipe buffer"
    );
}

/// `x C` (select-line, then duplicate the selection onto the next line) must
/// APPEND `copy-selection-on-next-line` onto the recipe `x` already built,
/// not reset it to a single `[C]` step.
///
/// `copy-selection-on-next-line` duplicates whatever selection is already
/// there: it establishes nothing on its own, so recording it as a reset
/// (`SelectionTracking::Establishes`) would replay `.` as "duplicate a bare
/// cursor," silently dropping the `x` that built the real extent (see
/// `dot_repeat_of_copy_selection_replays_the_whole_recipe` below for the
/// end-to-end failure this would cause). Registered with
/// `.composes_selection()` (`SelectionTracking::Composes`) precisely because
/// it transforms the staged extent instead of establishing one (see
/// `registry/defaults/selections.rs`).
#[test]
fn copy_selection_on_next_line_appends_to_the_selection_recipe() {
    let mut ed = editor_from("-[a]>aa\nbbb\n");

    ed.feed_key(key('x')); // select-line: "aaa\n" selected
    assert_eq!(
        ed.state.selection_recipe.len(),
        1,
        "setup: select-line must establish a 1-step recipe"
    );

    ed.feed_key(key('C')); // duplicate the selection onto "bbb\n"
    assert_eq!(
        ed.state.selection_recipe.len(),
        2,
        "copy-selection-on-next-line must append onto the prior x step"
    );
    assert_eq!(ed.state.selection_recipe[0].command.as_ref(), "select-line");
    assert_eq!(
        ed.state.selection_recipe[1].command.as_ref(),
        "copy-selection-on-next-line"
    );
}

/// `mm C d` then `.` must replay the WHOLE recipe (`mm`, `C`, `d`), not just
/// `d` against whatever selection happens to remain.
///
/// `mm` (word-select) is used as the establish step rather than `x`
/// (select-line): `select-line` includes the line's trailing structural
/// newline in the selection, and `copy-selection-on-next-line`'s
/// `DisplayColTarget::NearestContent` clamps a display column landing past
/// content back onto the last content char, so a `select-line` head (which
/// sits ON that newline) does not reproduce the newline in the copy. That's
/// existing, correct `C` behavior, orthogonal to this test; a word selection
/// (all content chars, no newline in range) sidesteps it entirely.
///
/// On `"foo\nbar\nfoo\nbar\n"`, `mm` selects "foo" (line
/// 0), `C` duplicates it onto "bar" (line 1) at the same columns, `d`
/// deletes both: each line's own newline survives, leaving
/// `"\n\nfoo\nbar\n"`. Move down two buffer lines to the second "foo\nbar\n"
/// pair; `.` must replay `mm` + `C` + `d` there too, leaving `"\n\n\n\n"`.
///
/// If `copy-selection-on-next-line` reset the recipe instead of
/// composing (`SelectionTracking::Establishes` instead of `Composes`), `.`
/// would replay `[C, d]` from a bare cursor instead of `[mm, C, d]`. `C` on
/// a collapsed cursor duplicates one cursor onto the next line, and `d`
/// deletes two *characters*, leaving `"\n\nf\nar\n"`-shaped text instead of
/// `"\n\n\n\n"`.
#[test]
fn dot_repeat_of_copy_selection_replays_the_whole_recipe() {
    let mut ed = editor_from("-[f]>oo\nbar\nfoo\nbar\n");

    ed.feed_key(key('m'));
    ed.feed_key(key('m')); // select-word: "foo" (line 0)
    ed.feed_key(key('C')); // + "bar" (line 1)
    ed.feed_key(key('d')); // delete both
    assert_eq!(ed.doc().text().to_string(), "\n\nfoo\nbar\n");

    ed.feed_key(key('j'));
    ed.feed_key(key('j')); // down to the second "foo" line, not in the recipe
    ed.feed_key(key('.')); // replay mm, C, d
    assert_eq!(
        ed.doc().text().to_string(),
        "\n\n\n\n",
        "`.` must replay the full mm/C/d recipe, not just d"
    );
}

/// `f`/`t` are NOT repeatable (they have `=`/`-` for that). Pressing `.`
/// after a find/till motion should be a no-op.
#[test]
fn dot_after_find_is_noop() {
    let mut ed = editor_from("-[h]>ello world\n");

    // `f` + `o` moves cursor to the first 'o' in "hello".
    ed.feed_key(key('f'));
    ed.feed_key(key('o'));
    let state_after_find = state(&ed);

    // `.` should have nothing recorded and leave state unchanged.
    assert!(ed.state.last_action().is_none());
    ed.feed_key(key('.'));
    assert_eq!(state(&ed), state_after_find);
}

/// A dot-repeated delete is itself a fresh capture: a bare `p` right after
/// `.` reads what `.` just deleted, not the clipboard.
///
/// This is a deliberate consequence of `PasteStamp` having no dot-repeat
/// special case at all: `replay_dot` runs the replayed edit through the
/// ordinary `commands::run_body` → `route_kill` → `capture_to_ring`
/// path, which writes the stamp at the replay's own
/// `BufferStore::edit_seq()` exactly as a live `d` would: nothing treats a
/// replayed delete differently from a typed one.
#[test]
fn dot_repeat_of_delete_leaves_ring_fresh_for_paste() {
    let mut ed = editor_from("-[foo]> bar\n");
    // Seed the clipboard with a sentinel value distinct from anything the
    // ring could hold, so a wrong clipboard read is unambiguous.
    seed_clipboard(&mut ed, "CLIP");

    ed.feed_key(key('d')); // delete "foo" → ring head = ["foo"]
    ed.feed_key(key('w')); // move to "bar"
    ed.feed_key(key('.')); // repeat; replay_dot deletes "bar" → ring head = ["bar"]

    ed.feed_key(key('p')); // bare p: must read the ring, not the clipboard
    let text = state(&ed);
    // "bar" is independently known from the scenario itself (the `.` replays
    // a delete on the word we navigated to with `w`), not read back from
    // `kill_ring.head()`, so a `.` that fails to push to the ring at all
    // can't make this pass by accident.
    assert!(
        text.contains("bar"),
        "p after dot-repeat delete must paste the ring head \"bar\"; buf={text:?}"
    );
    assert!(
        !text.contains("CLIP"),
        "p after dot-repeat delete must not read the clipboard; buf={text:?}"
    );
}

/// `w d` (word motion select then delete) must record an empty selection
/// recipe. A bare `.` then deletes the *current* selection, NOT the next word.
///
/// If `w` pushed an establish step, `.` would re-run `select-next-word` from
/// the new cursor position and advance past the intended word, deleting the
/// one after it. The `!ctx.extend` exclusion in `step_update_recipe`'s
/// `SelectionTracking::Extends` arm keeps `w` (a `Motion`, always `Extends`)
/// out of the recipe.
///
/// Buffer "a  foo bar baz\n". `w` selects "foo", `d` deletes it →
/// "a   bar baz\n". Then `w` selects "bar". `.` must delete "bar" (current
/// selection), leaving "a    baz\n".
#[test]
fn dot_repeat_word_motion_acts_on_current_selection() {
    let mut ed = editor_from("-[a]>  foo bar baz\n");
    // This test is about the recipe/replay mechanism, not word-span shape.
    // Pin bare-word selection so the buffer arithmetic in the doc comment
    // above holds regardless of word-selects-whitespace's default.
    ed.state.settings.word_selects_whitespace = WordSelectsWhitespace::Off;

    ed.feed_key(key('w')); // select "foo" (Move mode: Extends, not Establishes)
    ed.feed_key(key('d')); // delete "foo" → "a   bar baz\n"

    // Recipe must be empty: Move-mode `w` must not create an establish step.
    assert_eq!(
        ed.state.last_action().unwrap().selection_recipe.len(),
        0,
        "Move-mode select-next-word must not push an establish step"
    );

    ed.feed_key(key('w')); // select "bar" (cursor now on 'b')
    ed.feed_key(key('.')); // replay: empty recipe → delete current selection ("bar")

    let text = ed.doc().text().to_string();
    assert!(
        !text.contains("bar"),
        "`.` must delete 'bar' (current selection), not advance past it; got: {text:?}"
    );
    assert!(
        text.contains("baz"),
        "`.` must not advance past 'bar' and delete 'baz'; got: {text:?}"
    );
}

/// `ms(` (select surrounding `()` delimiters) is a selection-building step and
/// must be recorded in the dot-repeat recipe, so `.` re-runs `ms(` + `d` on the
/// next pair rather than just deleting the current selection.
///
/// Buffer "(foo) (bar) baz": cursor on the first 'o' of "foo", `ms(` selects
/// the two parens, `d` deletes them → "foo (bar) baz". Collapse to one cursor,
/// move to "bar", `.` must replay `ms(` + `d` on "(bar)" → "foo bar baz".
///
/// If `ms(` is not recorded in the selection recipe (e.g.
/// `surround-paren`'s `selection_tracking` were `Untracked`), the recipe is
/// empty and `.` deletes only the current "bar" selection, leaving
/// "foo () baz" instead of "foo bar baz".
#[test]
fn dot_repeat_of_match_surround_deletes_both_parens() {
    let mut ed = editor_from("(-[f]>oo) (bar) baz\n");

    ed.feed_key(key('m'));
    ed.feed_key(key('s'));
    ed.feed_key(key('(')); // two cursors on the delimiters
    ed.feed_key(key('d'));
    assert_eq!(
        ed.doc().text().to_string(),
        "foo (bar) baz\n",
        "ms( then d must remove the surrounding parens"
    );

    ed.feed_key(key(','));
    ed.feed_key(key('w'));
    ed.feed_key(key('w')); // select "bar"
    ed.feed_key(key('.')); // replay ms( + d on "(bar)"
    assert_eq!(
        ed.doc().text().to_string(),
        "foo bar baz\n",
        "`.` must replay ms( + d and strip the (bar) parens",
    );
}

/// `ma(` (select around `()`, including the delimiters) on `(foo)` leaves a
/// non-collapsed selection, unlike `ms(`
/// (`dot_repeat_of_match_surround_deletes_both_parens` above). This pins that
/// it replays correctly under `SelectionTracking::Establishes`, which
/// restarts the dot-repeat recipe in Move mode (see `step_update_recipe`).
#[test]
fn dot_repeat_of_match_around_deletes_content() {
    let mut ed = editor_from("(-[f]>oo) (bar) baz\n");

    ed.feed_key(key('m'));
    ed.feed_key(key('a'));
    ed.feed_key(key('('));
    ed.feed_key(key('d'));
    assert_eq!(
        ed.doc().text().to_string(),
        " (bar) baz\n",
        "ma( then d must remove (foo)"
    );

    ed.feed_key(key('w'));
    ed.feed_key(key('w')); // select "bar"
    ed.feed_key(key('.')); // replay ma( + d on "(bar)"
    assert_eq!(
        ed.doc().text().to_string(),
        "  baz\n",
        "`.` must replay ma( + d and remove (bar)",
    );
}

/// `mi(` (select inner `()`, excluding the delimiters). Same characterization
/// note as `dot_repeat_of_match_around_deletes_content` above: on `(foo)` it
/// already leaves a non-collapsed selection, so this is coverage for
/// `SelectionTracking::Establishes` rather than a regression test for it.
#[test]
fn dot_repeat_of_match_inner_deletes_content() {
    let mut ed = editor_from("(-[f]>oo) (bar) baz\n");

    ed.feed_key(key('m'));
    ed.feed_key(key('i'));
    ed.feed_key(key('('));
    ed.feed_key(key('d'));
    assert_eq!(
        ed.doc().text().to_string(),
        "() (bar) baz\n",
        "mi( then d must remove 'foo'"
    );

    ed.feed_key(key('w'));
    ed.feed_key(key('w')); // select "bar"
    ed.feed_key(key('.')); // replay mi( + d on "(bar)"
    assert_eq!(
        ed.doc().text().to_string(),
        "() () baz\n",
        "`.` must replay mi( + d and remove 'bar'",
    );
}

#[test]
fn dot_repeat_of_select_all_matches_deletes_content() {
    let mut ed = editor_from("-[f]>oo bar baz foo bar baz\n").with_search_regex("foo");

    ed.feed_key(key('m'));
    ed.feed_key(key('/'));

    assert_eq!(state(&ed), "-{foo}> bar baz -[foo]> bar baz\n");
    assert_eq!(
        ed.state.selection_recipe.len(),
        1,
        "select-all-matches must establish a 1-step recipe"
    );

    ed.feed_key(key('d'));
    assert_eq!(
        ed.doc().text().to_string(),
        " bar baz  bar baz\n",
        "m/ after a search then d must remove all occurrences of 'foo'"
    );

    ed = ed.with_search_regex("bar");
    ed.feed_key(key('.')); // replay m/ + d
    assert_eq!(
        ed.doc().text().to_string(),
        "  baz   baz\n",
        "`.` must replay m/ + d and remove all occurrences of 'bar'"
    );
}

/// `,` (`keep-primary-selection`) after `m/` must APPEND onto the recipe `m/`
/// already built, not reset it: it reduces whatever is staged to the
/// primary selection rather than establishing a fresh extent of its own.
///
/// Buffer "foo bar foo bar foo bar": `m/` selects all three "foo" matches,
/// `,` keeps only the first (primary), `d` deletes it. `.` must replay
/// `m/` + `,` + `d`, re-selecting all remaining "foo" matches, keeping the
/// (new) primary, and deleting it too.
///
/// If `keep-primary-selection` reset the recipe instead of
/// composing, `.` would replay `[,, d]` from whatever selection happens to
/// remain after the first delete. `,` on a single selection is a no-op, so
/// `d` would just delete that leftover selection instead of re-running the
/// search-driven `m/`.
#[test]
fn dot_repeat_of_keep_primary_selection_composes_onto_the_prior_recipe() {
    let mut ed = editor_from("-[f]>oo bar foo bar foo bar\n").with_search_regex("foo");

    ed.feed_key(key('m'));
    ed.feed_key(key('/')); // select all three "foo" matches
    assert_eq!(
        ed.state.selection_recipe.len(),
        1,
        "m/ must establish a 1-step recipe"
    );

    ed.feed_key(key(',')); // keep-primary-selection: reduce to the first match
    assert_eq!(
        ed.state.selection_recipe.len(),
        2,
        ", must compose onto the m/ step, not reset it"
    );
    assert_eq!(
        ed.state.selection_recipe[0].command.as_ref(),
        "select-all-matches"
    );
    assert_eq!(
        ed.state.selection_recipe[1].command.as_ref(),
        "keep-primary-selection"
    );

    ed.feed_key(key('d')); // delete the first "foo"
    assert_eq!(ed.doc().text().to_string(), " bar foo bar foo bar\n");

    ed.feed_key(key('.')); // replay m/ + , + d
    assert_eq!(
        ed.doc().text().to_string(),
        " bar  bar foo bar\n",
        "`.` must replay m/ + , + d and remove the next 'foo'"
    );
}

/// `S` (`split-selection-on-newlines`) after `x` + `Ctrl-x` must APPEND onto
/// the recipe those steps already built, not reset it: it splits whatever
/// multi-line extent is staged into per-line pieces rather than establishing
/// a fresh extent of its own.
///
/// Buffer "aaa\nbbb\nccc\nddd\n": `x` selects "aaa\n", `Ctrl-x` extends to
/// "aaa\nbbb\n", `S` splits into two per-line selections ("aaa" and "bbb\n").
/// `d` deletes both: the first piece excludes its line's newline (so "aaa"
/// → "", newline kept), the second piece runs through its own trailing
/// newline (so "bbb\n" is removed whole). Net effect: one line disappears:
/// "\nccc\nddd\n". `.` must replay `x` + `Ctrl-x` + `S` + `d` on the next two
/// lines too, removing another: "\n\n".
///
/// If `split-selection-on-newlines` reset the recipe instead of
/// composing, `.` would replay `[S, d]` from whatever selection happens to
/// remain. `S` on a single-line (already-collapsed) selection is a no-op,
/// so `d` would delete only that leftover selection.
#[test]
fn dot_repeat_of_split_selection_on_newlines_composes_onto_the_prior_recipe() {
    let mut ed = editor_from("-[a]>aa\nbbb\nccc\nddd\n");

    ed.feed_key(key('x')); // select-line: "aaa\n"
    ed.feed_key(key_ctrl('x')); // extend: "aaa\nbbb\n"
    assert_eq!(
        ed.state.selection_recipe.len(),
        2,
        "setup: x + Ctrl-x must push two recipe steps"
    );

    ed.feed_key(key('S')); // split into "aaa" + "bbb\n"
    assert_eq!(
        ed.state.selection_recipe.len(),
        3,
        "S must compose onto the x/Ctrl-x steps, not reset them"
    );
    assert_eq!(
        ed.state.selection_recipe[2].command.as_ref(),
        "split-selection-on-newlines"
    );

    ed.feed_key(key('d'));
    assert_eq!(ed.doc().text().to_string(), "\nccc\nddd\n");

    ed.feed_key(key('j')); // move onto "ccc"
    ed.feed_key(key('.')); // replay x + Ctrl-x + S + d
    assert_eq!(
        ed.doc().text().to_string(),
        "\n\n",
        "`.` must replay x + Ctrl-x + S + d and clear 'ccc'/'ddd'"
    );
}

/// `mii` (`select-last-insertion`) is an `EditorCmd` that does not opt into
/// the dot-repeat recipe via `.establishes_selection()` (unlike
/// `select-all-matches`; see `editor_cmds.rs`), so unlike `ms(`/`mm` it can never push itself onto
/// `state.selection_recipe`, and (being non-repeatable) it can never overwrite
/// the recorded change either. Running it between an insert and `.` must
/// therefore be inert: `.` still replays the original insert verbatim, not
/// some "reselect + act" recipe.
///
/// `i "ab" Esc` on "x" gives "abx"; `mii` re-selects "ab"
/// (the just-typed span, unrelated to where `.` will act); replaying the
/// insert places "ab" again at the selection's start → "ab" + "ab" + "x" =
/// "ababx". The buffer only grows (no delete ever happens), which is the
/// signal that `.` ran the insert and nothing resembling "mii + d".
///
/// If `mii` corrupted the recorded change (e.g. by being
/// misclassified as repeatable) or leaked into its frozen recipe, `.` would
/// replay some other action (or reselect "ab" again as an operator target)
/// instead of inserting, and the buffer would not end up "ababx".
#[test]
fn dot_repeat_after_select_last_insertion_still_repeats_the_insert() {
    let mut ed = editor_from("-[x]>\n");

    type_text(&mut ed, "ab"); // insert-before, cursor collapses to start; buffer is "abx"
    assert_eq!(ed.doc().text().to_string(), "abx\n");

    // Move off "ab" first: `insert-select-on-exit` (default on) already left
    // it selected, so pressing `mii` right away would be a no-op and the
    // assertion below would hold whether or not `mii` actually did anything.
    set_cursor(&mut ed, 2); // onto 'x'
    ed.feed_key(key('m'));
    ed.feed_key(key('i'));
    ed.feed_key(key('i')); // mii: re-select "ab", the last insertion
    assert_eq!(state(&ed), "-[ab]>x\n");

    ed.feed_key(key('d'));
    assert_eq!(state(&ed), "-[x]>\n");

    type_text(&mut ed, "ab");
    assert_eq!(ed.doc().text().to_string(), "abx\n");

    ed.feed_key(key('.')); // must replay the insert, not act on the mii selection
    assert_eq!(ed.doc().text().to_string(), "ababx\n");
}

/// `select-word-nearest-on-line` is the wrap-aware `EditorCmd` twin of
/// `select-word` (`mm`, a `Selection` → `Establishes`), with the same in-place
/// establishing semantics, `EditorCmd` only because it needs a `DisplayLineMap`.
/// Unbound by default, so this drives it directly via `execute_keymap_command`
/// rather than a keypress. It must be replayable: `.` re-runs the selection
/// step before repeating the edit, not just re-delete whatever selection
/// happens to remain at the new cursor.
///
/// Buffer "foo bar\n": select-word-nearest-on-line selects "foo " (word plus
/// trailing whitespace, the `word-selects-whitespace` default), `d` deletes
/// it, leaving "bar\n" with the cursor collapsed to a 1-char selection on
/// 'b'. `.` must replay select-word-nearest-on-line + d from there too,
/// re-selecting the whole word "bar" rather than deleting just that 1-char
/// cursor.
///
/// If the command were `Untracked`, the recipe would stay empty and `.` would
/// delete only the 1-char cursor left behind, leaving "ar\n".
#[test]
fn dot_repeat_of_select_word_nearest_on_line_deletes_the_word() {
    let mut ed = editor_from("-[f]>oo bar\n");

    ed.execute_keymap_command(
        std::borrow::Cow::Borrowed("select-word-nearest-on-line"),
        Some(1),
        false,
    );
    ed.feed_key(key('d'));
    assert_eq!(ed.doc().text().to_string(), "bar\n");

    ed.feed_key(key('.')); // replay select-word-nearest-on-line + d
    assert_eq!(
        ed.doc().text().to_string(),
        "\n",
        "`.` must replay select-word-nearest-on-line + d and remove the whole word 'bar'"
    );
}

/// `x m/ d` where `m/`'s pattern matches nothing must record `[select-line,
/// delete]`, not `[select-all-matches, delete]`: `select-all-matches`
/// errors on no match (`cmd_select_all_matches` returns `Err("no matches")`)
/// and leaves the selection untouched, so it established no replayable
/// extent of its own and must not overwrite the `x` step that did.
///
/// Buffer "aaa\nbbb\nccc\n": `x` selects "aaa\n"; search pattern "zzz"
/// matches nothing; `m/` no-ops (still "aaa\n" selected); `d` deletes it,
/// leaving "bbb\nccc\n". Move to "ccc"; `.` must replay `select-line` + `d`
/// there too, leaving "bbb\n".
///
/// Without the "selection unchanged" gate, `m/`'s failed run would still
/// reset the recipe to `[select-all-matches]`. `.` replays that
/// (another no-op) then `delete`, which acts on the current 1-char cursor
/// instead of the whole line, leaving "bb\n" instead of "bbb\n".
#[test]
fn dot_repeat_of_failed_select_all_matches_preserves_prior_recipe() {
    let mut ed = editor_from("-[a]>aa\nbbb\nccc\n").with_search_regex("zzz");

    ed.feed_key(key('x')); // select-line: "aaa\n"
    ed.feed_key(key('m'));
    ed.feed_key(key('/')); // no match: selection untouched, command errors
    assert_eq!(
        ed.state.selection_recipe.len(),
        1,
        "a failed select-all-matches must not overwrite the prior x step"
    );
    assert_eq!(ed.state.selection_recipe[0].command.as_ref(), "select-line");

    ed.feed_key(key('d')); // delete "aaa\n"
    assert_eq!(ed.doc().text().to_string(), "bbb\nccc\n");

    ed.feed_key(key('j')); // move onto "ccc"
    ed.feed_key(key('.')); // replay select-line + d
    assert_eq!(
        ed.doc().text().to_string(),
        "bbb\n",
        "`.` must replay select-line + d and remove the whole 'ccc' line"
    );
}

/// `m/ d` recorded under one pattern, replayed under a pattern with no match:
/// the recipe's `select-all-matches` fails on replay, so `.` must not run `d`
/// on the cursor it left behind, and leaves no undo step of its own.
#[test]
fn dot_repeat_whose_recipe_step_fails_leaves_text_and_selections_unchanged() {
    let mut ed = editor_from("-[f]>oo bar baz\n").with_search_regex("foo");

    ed.feed_key(key('m'));
    ed.feed_key(key('/'));
    ed.feed_key(key('d'));
    assert_eq!(ed.doc().text().to_string(), " bar baz\n");

    ed = ed.with_search_regex("qux");
    let before = state(&ed);
    ed.feed_key(key('.'));

    assert_eq!(ed.doc().text().to_string(), " bar baz\n");
    assert_eq!(state(&ed), before);
    assert!(
        status(&ed).contains("no matches"),
        "status: {:?}",
        status(&ed)
    );

    ed.feed_key(key('u'));
    assert_eq!(ed.doc().text().to_string(), "foo bar baz\n");
}

/// `m/ d` replayed with no search pattern at all: `select-all-matches`
/// refuses, so `.` must not run `d` on the cursor.
#[test]
fn dot_repeat_whose_recipe_step_has_no_search_pattern_leaves_text_unchanged() {
    let mut ed = editor_from("-[f]>oo bar baz\n").with_search_regex("foo");

    ed.feed_key(key('m'));
    ed.feed_key(key('/'));
    ed.feed_key(key('d'));
    assert_eq!(ed.doc().text().to_string(), " bar baz\n");

    let bid = ed.focused_buffer_id();
    ed.state.buffers.get_mut(bid).search_pattern = None;
    assert_eq!(ed.state.registers.search_register(), None);
    let before = state(&ed);
    ed.feed_key(key('.'));

    assert_eq!(ed.doc().text().to_string(), " bar baz\n");
    assert_eq!(state(&ed), before);
    assert!(
        status(&ed).contains("no search pattern"),
        "status: {:?}",
        status(&ed)
    );
}

/// `x ms( d` where `ms(` finds no surrounding pair must record
/// `[select-line, delete]`, not `[surround-paren, delete]`: `surround-paren`
/// is infallible and leaves the selection unchanged on no-match (see
/// `select_surround` in `hume-ops/src/surround.rs`), so like the
/// `select-all-matches` case above it established no replayable extent and
/// must not overwrite the prior `x` step.
///
/// Buffer "aaa\nbbb\nccc\n": `x` selects "aaa\n" (no parens on that line, so
/// `ms(` on it would no-op); `d` deletes it, leaving "bbb\nccc\n". Move to
/// "ccc"; `.` must replay `select-line` + `d` there too, leaving "bbb\n".
///
/// Without the "selection unchanged" gate, the no-op `ms(` would still
/// reset the recipe to `[surround-paren]`. `.` replays that (another
/// no-op, cursor unchanged) then `delete`, which acts on the current 1-char
/// cursor instead of the whole line, leaving "bb\n" instead of "bbb\n".
#[test]
fn dot_repeat_of_noop_surround_preserves_prior_recipe() {
    let mut ed = editor_from("-[a]>aa\nbbb\nccc\n");

    ed.feed_key(key('x')); // select-line: "aaa\n"
    ed.feed_key(key('m'));
    ed.feed_key(key('s'));
    ed.feed_key(key('(')); // no parens on this line: selection untouched
    assert_eq!(
        ed.state.selection_recipe.len(),
        1,
        "a no-op surround-paren must not overwrite the prior x step"
    );
    assert_eq!(ed.state.selection_recipe[0].command.as_ref(), "select-line");

    ed.feed_key(key('d')); // delete "aaa\n"
    assert_eq!(ed.doc().text().to_string(), "bbb\nccc\n");

    ed.feed_key(key('j')); // move onto "ccc"
    ed.feed_key(key('.')); // replay select-line + d
    assert_eq!(
        ed.doc().text().to_string(),
        "bbb\n",
        "`.` must replay select-line + d and remove the whole 'ccc' line"
    );
}

/// `.` replaying a paste-ring cycle (`[`/`]`) must actually cycle the ring
/// again, not no-op.
///
/// `repeat-last-action`'s own dispatch must not commit (and thereby close)
/// the still-open paste session before `replay_dot` looks at what it is
/// replaying. See `.defers_paste_commit()` on
/// `repeat-last-action`'s registration (`registry/defaults/editor_cmds.rs`)
/// and `Editor::replay_dot`'s own paste-commit step.
///
/// `KillRing::cycle_position()` is the ring's own cursor,
/// read directly rather than inferred from which text ended up pasted.
///
/// Without `repeat-last-action`'s `.defers_paste_commit()` (and
/// the matching commit/defer decision in `replay_dot`), the paste session is
/// closed before the replay runs; `do_paste_cycle` sees no open Paste-kind
/// `active_session` and returns before ever calling `cycle_older()`, so
/// `cycle_position()` stays at `Some(1)` instead of advancing to `Some(2)`.
#[test]
fn dot_repeat_of_paste_ring_cycle_advances_the_ring() {
    let mut ed = editor_from("-[aaa]> bbb ccc\n");

    ed.feed_key(key('y')); // yank "aaa" → ring: [aaa]
    ed.feed_key(key('w')); // select "bbb"
    ed.feed_key(key('y')); // yank "bbb" → ring: [bbb, aaa]
    ed.feed_key(key('w')); // select "ccc"
    ed.feed_key(key('y')); // yank "ccc" → ring: [ccc, bbb, aaa]

    ed.feed_key(key('p')); // bare paste: reads ring slot 0 ("ccc"), seeds cycle Some(0)
    assert_eq!(
        ed.state.kill_ring.cycle_position(),
        Some(0),
        "setup: p must seed the cycle at slot 0"
    );

    ed.feed_key(key('[')); // paste-ring-older: Some(0) -> Some(1) ("bbb")
    assert_eq!(
        ed.state.kill_ring.cycle_position(),
        Some(1),
        "setup: [ must cycle to slot 1"
    );

    ed.feed_key(key('.')); // replay paste-ring-older: must cycle Some(1) -> Some(2) ("aaa")
    assert_eq!(
        ed.state.kill_ring.cycle_position(),
        Some(2),
        "`.` must replay paste-ring-older as a second cycle step, not no-op"
    );
}

// ── Steel command dot-repeat tests ────────────────────────────────────────────

/// An editor over `initial_state` with the Steel commands `source` defines.
fn editor_with_steel(initial_state: &str, source: &str) -> Editor {
    let mut ed = editor_from(initial_state);
    attach_steel(&mut ed, source);
    ed
}

/// A Steel command registered with `#:repeatable #t` must overwrite
/// the recorded change with its own name so `.` replays the outer
/// Steel body, not the inner native command.
///
/// Buffer is "foo bar\n", initial selection is "foo".
/// Run `del-sel` (repeatable Steel command that calls delete internally) →
/// "foo" is deleted, buffer is " bar\n". Press `w` to select "bar", the
/// first (and only) word on its line, so its leading space is indentation
/// and is never absorbed, and there's no trailing space either (EOL
/// follows), so the default around-word span is bare "bar". Then `.` replays
/// `del-sel` on that selection, leaving " \n".
///
/// If `meta().repeatable` returned `false` for `SteelBacked`,
/// the recorded change would be `None` (no prior recording), so `.` would
/// be a no-op and "bar" would survive. The name assertion covers the other
/// case: if the outer name didn't win the slot, the buffer result would be
/// the same but the recorded change would name the inner `"delete"`.
#[test]
fn steel_dot_repeatable_round_trip() {
    let mut ed = editor_with_steel(
        "-[foo]> bar\n",
        r#"(define-command! "del-sel" ""
             (lambda () (call! "delete" (focused-pane)))
             #:repeatable #t)"#,
    );

    // Run the repeatable Steel command.
    ed.execute_keymap_command("del-sel".into(), Some(1), false);
    // "foo" deleted; buffer is " bar\n".
    assert_eq!(ed.doc().text().to_string(), " bar\n", "first run");

    // the recorded change must name the outer Steel command, not inner "delete".
    assert_eq!(
        ed.state.last_action().map(|a| a.command.as_ref()),
        Some("del-sel"),
        "outer Steel command must win the repeat slot over the inner 'delete'"
    );

    // Select "bar" then press `.`: replay must delete the current selection.
    ed.feed_key(key('w')); // select "bar" (bare, indent kept)
    ed.feed_key(key('.')); // replay "del-sel" → delete "bar"
    assert_eq!(
        ed.doc().text().to_string(),
        " \n",
        "`.` must replay the Steel command and delete the current selection"
    );
}

/// A plain `define-command!` (non-repeatable) must not overwrite
/// the recorded change set by a prior native edit.
///
/// If `meta().repeatable` returned `true` for `SteelBacked`, running the
/// Steel command would stamp the recorded change with its name; the
/// subsequent `.` would replay the Steel command instead of the native delete.
#[test]
fn steel_command_is_not_repeatable() {
    let mut ed = editor_with_steel(
        "-[foo]> bar\n",
        r#"(define-command! "del-sel" "" (lambda () (call! "delete" (focused-pane))))"#,
    );

    // Establish a known repeatable native action first.
    ed.feed_key(key('d')); // delete "foo" → " bar\n"
    assert_eq!(ed.doc().text().to_string(), " bar\n");
    assert_eq!(
        ed.state.last_action().map(|a| a.command.as_ref()),
        Some("delete"),
        "setup: the recorded change must be 'delete' after d"
    );

    // Run the Steel command: it calls (call! "delete" (focused-pane)) internally.
    ed.execute_keymap_command("del-sel".into(), Some(1), false);

    // the recorded change must still be "delete", not "del-sel".
    assert_eq!(
        ed.state.last_action().map(|a| a.command.as_ref()),
        Some("delete"),
        "Steel command must not overwrite the recorded change"
    );
}

/// A plain `define-command!` (non-repeatable) must NOT overwrite
/// the recorded change set by a prior native edit.
///
/// If `meta().repeatable` were `true` for every `SteelBacked` command, the
/// non-repeatable Steel command would overwrite the recorded change and
/// the subsequent `.` would replay it.
#[test]
fn non_repeatable_steel_does_not_hijack_dot() {
    let mut ed = editor_with_steel(
        "-[foo]> bar\n",
        r#"(define-command! "noop-move" "" (lambda () (call! "move-right" (focused-pane))))"#,
    );

    // Establish a known repeatable native action first.
    ed.feed_key(key('d')); // delete "foo" → " bar\n"
    assert_eq!(ed.doc().text().to_string(), " bar\n");
    assert_eq!(
        ed.state.last_action().map(|a| a.command.as_ref()),
        Some("delete"),
        "setup: the recorded change must be 'delete' after d"
    );

    // Run the non-repeatable Steel command.
    ed.execute_keymap_command("noop-move".into(), Some(1), false);

    // the recorded change must still be "delete".
    assert_eq!(
        ed.state.last_action().map(|a| a.command.as_ref()),
        Some("delete"),
        "non-repeatable Steel command must not overwrite the recorded change"
    );

    // `.` must still delete "bar".
    ed.feed_key(key('w'));
    ed.feed_key(key('.'));
    assert!(
        !ed.doc().text().to_string().contains("bar"),
        "`.` must repeat the native delete, not the non-repeatable Steel command"
    );
}

// ── Insert-key dot-repeat (keymap-bound native commands, `insert-key!`) ──────

/// `Ctrl-w` deletes a word mid-Insert-session, dispatched as an Insert-key
/// binding: `.` on a different selection must repeat that delete too, not
/// just the raw keys typed around it.
///
/// Both edit sites sit right after a `(`, a non-word char, so
/// `delete-word-backward`'s word-boundary scan stops there regardless of
/// what came before, and the two sites are directly comparable. (Landing
/// mid-word instead, as `c` on a bare selection with existing text right
/// before the cursor would, makes the two sites' word-boundary context
/// differ, since the replayed delete would then span back into that
/// pre-existing text too, which is `delete-word-backward` working as
/// designed, not a replay bug; `set_current_selections` below sidesteps
/// that entirely by selecting the second target directly, rather than via
/// a word motion whose landing spot relative to the punctuation would add
/// another such context difference.)
///
/// "hello" typed then deleted word-backward nets to
/// nothing, so a correct replay gives the exact same result as typing just
/// "hi" directly into either pair of parens. If `Ctrl-w` weren't replayed,
/// the typed "hello" would survive in the second pair: "(hi) (hellohi)\n".
#[test]
fn dot_repeats_ctrl_w_inside_insert() {
    let mut ed = editor_from("(-[foo]>) (bar)\n");

    ed.feed_key(key('c')); // change: delete "foo", enter Insert; cursor is now right after "("
    ed.feed_key(key('h'));
    ed.feed_key(key('e'));
    ed.feed_key(key('l'));
    ed.feed_key(key('l'));
    ed.feed_key(key('o')); // "(hello) (bar)\n"
    ed.feed_key(key_ctrl('w')); // delete "hello" word-backward, back to "("
    ed.feed_key(key('h'));
    ed.feed_key(key('i'));
    ed.feed_key(key_esc()); // back to Normal

    assert_eq!(ed.doc().text().to_string(), "(hi) (bar)\n");

    select_marked(&mut ed, "(hi) (-[bar]>)\n");
    ed.feed_key(key('.')); // repeat: delete "bar", type "hello", Ctrl-w, type "hi"

    assert_eq!(ed.doc().text().to_string(), "(hi) (hi)\n");
}

/// Same proof for a Steel command that calls a native command mid-Insert
/// (`dispatch`'s Steel path → `call!` → `run_command_sync`), rather than a
/// native binding like `Ctrl-w`. The expected result is derived the same way
/// as in `dot_repeats_ctrl_w_inside_insert`.
#[test]
fn steel_insert_binding_calling_native_command_is_recorded_for_dot_repeat() {
    let tmp = tempfile::tempdir().unwrap();
    let mut ed = editor_from("(-[foo]>) (bar)\n");
    run(
        &mut ed,
        tmp.path(),
        r#"(define-command! "del-word" "" (lambda (pane) (call! "delete-word-backward" pane)))
           (bind-key! 'insert "ctrl-x" "del-word")"#,
    );

    ed.feed_key(key('c'));
    ed.feed_key(key('h'));
    ed.feed_key(key('e'));
    ed.feed_key(key('l'));
    ed.feed_key(key('l'));
    ed.feed_key(key('o'));
    ed.feed_key(key_ctrl('x')); // Steel wrapper → (call! "delete-word-backward" pane)
    ed.feed_key(key('h'));
    ed.feed_key(key('i'));
    ed.feed_key(key_esc());

    assert_eq!(ed.doc().text().to_string(), "(hi) (bar)\n");

    select_marked(&mut ed, "(hi) (-[bar]>)\n");
    ed.feed_key(key('.'));

    assert_eq!(ed.doc().text().to_string(), "(hi) (hi)\n");
}

/// `.` after a Tab binding took its "insert a tab" branch (via
/// `insert-key!`) repeats exactly one tab, proving the binding itself is
/// recorded as a `Command`, re-run once on replay (not the `insert-key!`
/// fallback's raw key; see `Input`'s own doc for why the binding is
/// what's recorded), and recorded only once.
///
/// Identical setup and expected result to `tabs.rs`'s
/// `dot_repeat_replays_tab` (the plain unbound-Tab case): `tab-or-complete`
/// must reproduce that exact behaviour, not a doubled or dropped tab.
#[test]
fn dot_repeat_replays_tab_via_insert_key_once() {
    let tmp = tempfile::tempdir().unwrap();
    let mut ed = editor_from("-[h]>ello\n");
    run(
        &mut ed,
        tmp.path(),
        r#"(define-command! "tab-or-complete" "" (lambda (pane) (insert-key! pane "tab")))
           (bind-key! 'insert "tab" "tab-or-complete")"#,
    );

    ed.feed_key(key('i'));
    ed.feed_key(key_tab());
    ed.feed_key(key_esc()); // Esc selects the typed run, just the tab
    assert_eq!(state(&ed), "-[\t]>hello\n");

    ed.feed_key(key('l'));
    ed.feed_key(key('l'));
    ed.feed_key(key('.'));
    assert_eq!(state(&ed), "\th-[\t]>ello\n");

    let action = ed
        .state
        .last_action()
        .expect("`i` must have stamped a repeatable action");
    let inputs = &action.transcript;
    assert_eq!(
        inputs.len(),
        2,
        "the tab-or-complete binding once, not doubled, then Esc: {inputs:?}"
    );
    assert!(
        is_binding(&inputs[0], "tab-or-complete") && is_binding(&inputs[1], "exit-insert"),
        "must record the binding, not a raw Key: {inputs:?}"
    );
}

/// A binding that leaves Insert and re-enters it keeps recording into the
/// same repeatable action: `.` must not lose everything typed before the
/// binding ran, nor what was typed after it.
#[test]
fn insert_key_binding_reentering_insert_keeps_earlier_keys() {
    let tmp = tempfile::tempdir().unwrap();
    let mut ed = editor_from("-[h]>ello\nworld\n");
    run(
        &mut ed,
        tmp.path(),
        r#"(define-command! "reenter-tab" "" (lambda (pane)
             (call! "exit-insert" pane)
             (call! "insert-at-line-end" pane)
             (insert-key! pane "tab")))
           (bind-key! 'insert "ctrl-x" "reenter-tab")"#,
    );

    ed.feed_key(key('i'));
    ed.feed_key(key('a'));
    ed.feed_key(key('b'));
    ed.feed_key(key_ctrl('x'));
    ed.feed_key(key('c'));
    ed.feed_key(key('d'));
    ed.feed_key(key_esc());
    assert_eq!(ed.doc().text().to_string(), "abhello\tcd\nworld\n");

    let action = ed
        .state
        .last_action()
        .expect("`i` must have stamped a repeatable action");
    let keys = &action.transcript;
    assert_eq!(keys.len(), 6, "got {keys:?}");
    assert!(
        is_key(&keys[0], 'a') && is_key(&keys[1], 'b'),
        "got {keys:?}"
    );
    assert!(is_binding(&keys[2], "reenter-tab"), "got {keys:?}");
    assert!(
        is_key(&keys[3], 'c') && is_key(&keys[4], 'd'),
        "got {keys:?}"
    );
    assert!(is_binding(&keys[5], "exit-insert"), "got {keys:?}");

    ed.feed_key(key('j'));
    ed.feed_key(key('g'));
    ed.feed_key(key('h'));
    ed.feed_key(key('.'));
    assert_eq!(ed.doc().text().to_string(), "abhello\tcd\nabworld\tcd\n");
}

/// Records `i`, `a`, a `ctrl-x` binding running `body`, Esc on
/// `"hello\nworld\n"`, then moves to the start of line 2, the shared
/// setup for the replay-failure tests below.
fn record_insert_key_binding_then_move(body: &str) -> (Editor, tempfile::TempDir) {
    let tmp = tempfile::tempdir().unwrap();
    let mut ed = editor_from("-[h]>ello\nworld\n");
    run(
        &mut ed,
        tmp.path(),
        &format!(
            r#"{body}
               (bind-key! 'insert "ctrl-x" "tab-key")"#
        ),
    );
    ed.feed_key(key('i'));
    ed.feed_key(key('a'));
    ed.feed_key(key_ctrl('x'));
    ed.feed_key(key_esc());
    assert_eq!(ed.doc().text().to_string(), "a\thello\nworld\n");
    ed.feed_key(key('j'));
    ed.feed_key(key('g'));
    ed.feed_key(key('h'));
    (ed, tmp)
}

/// A recorded `Binding` whose command was unregistered before `.` (a plugin
/// reload) reports and is skipped. Replay continues to the next recorded
/// entry (`exit-insert`), which is what actually closes the session, and
/// the repeatable action itself is left intact.
#[test]
fn dot_repeat_reports_an_unregistered_insert_key_binding_and_skips_it() {
    let (mut ed, _tmp) = record_insert_key_binding_then_move(
        r#"(define-command! "tab-key" "" (lambda (pane) (insert-key! pane "tab")))"#,
    );
    ed.state.config.registry.unregister("tab-key");

    ed.feed_key(key('.'));
    assert_eq!(ed.doc().text().to_string(), "a\thello\naworld\n");
    assert!(
        status(&ed).contains("unknown command"),
        "got: {}",
        status(&ed)
    );
    assert_eq!(ed.state.mode(), Mode::Normal);
    assert_eq!(
        ed.state.last_action().map(|a| a.command.as_ref()),
        Some("insert-at-selection-start")
    );
}

/// A recorded `Binding` whose Steel body fails on replay (it succeeded the
/// first time) is skipped the same way.
#[test]
fn dot_repeat_skips_an_insert_key_binding_that_fails_on_replay() {
    let (mut ed, _tmp) = record_insert_key_binding_then_move(
        r#"(define tab-key-runs 0)
           (define-command! "tab-key" "" (lambda (pane)
             (set! tab-key-runs (+ tab-key-runs 1))
             (if (> tab-key-runs 1) (error "boom") (insert-key! pane "tab"))))"#,
    );

    ed.feed_key(key('.'));
    assert_eq!(ed.doc().text().to_string(), "a\thello\naworld\n");
    assert!(status(&ed).contains("boom"), "got: {}", status(&ed));
    assert_eq!(ed.state.mode(), Mode::Normal);
    assert_eq!(
        ed.state.last_action().map(|a| a.command.as_ref()),
        Some("insert-at-selection-start")
    );
}

/// An unbound Insert key with no default behaviour (an unbound Ctrl-chord)
/// does nothing, so it must not be recorded for dot-repeat. Only the typed
/// char that follows it, and the Esc binding, are.
#[test]
fn unbound_insert_key_with_no_default_behaviour_is_not_recorded() {
    let mut ed = editor_from("-[h]>ello\n");

    ed.feed_key(key('i'));
    ed.feed_key(key_ctrl('x'));
    ed.feed_key(key('z'));
    ed.feed_key(key_esc());
    assert_eq!(state(&ed), "-[z]>hello\n");

    let action = ed
        .state
        .last_action()
        .expect("`i` must have stamped a repeatable action");
    let inputs = &action.transcript;
    assert_eq!(inputs.len(), 2, "got {inputs:?}");
    assert!(
        is_key(&inputs[0], 'z') && is_binding(&inputs[1], "exit-insert"),
        "only the typed `z` and Esc may be recorded, got {inputs:?}"
    );
}

/// A Tab binding that takes its "trigger completion" branch marks itself
/// interactive and writes nothing itself, so its `Binding` placeholder is
/// dropped rather than recorded: `.` never reopens the popup at all.
#[test]
fn tab_or_complete_completion_branch_leaves_no_popup_after_dot_repeat() {
    let tmp = tempfile::tempdir().unwrap();
    let mut ed = editor_from("-[h]>ello\n");
    let source = format!(
        "{}\n{}",
        completion_source("test", &completion_labels(&["foo"]), ""),
        r#"(define-command! "tab-or-complete" "" (lambda (pane) (call! "completion-trigger" pane)))
           (bind-key! 'insert "tab" "tab-or-complete")"#
    );
    run(&mut ed, tmp.path(), &source);

    ed.feed_key(key('i'));
    ed.feed_key(key_tab());
    ed.settle();
    ed.feed_key(key_esc()); // dismisses the popup, staying in Insert
    ed.feed_key(key_esc()); // leaves Insert

    let inputs = &ed
        .state
        .last_action()
        .expect("`i` must have stamped a repeatable action")
        .transcript;
    assert_eq!(inputs.len(), 1, "got {inputs:?}");
    assert!(is_binding(&inputs[0], "exit-insert"), "got {inputs:?}");

    ed.feed_key(key('.'));
    ed.settle();
    assert_eq!(ed.state.mode(), Mode::Normal);
    assert!(ed.state.input.buffer_completion().is_none());
    assert_eq!(ed.doc().text().to_string(), "hello\n");
}

/// Drives `c`, types "he", opens completion, then presses `accept`, which
/// must accept "hello", optionally followed by `then`, and leaves Insert.
/// Whether `accept` is the popup's own Enter or an Insert-key binding
/// calling `completion-accept!`, the result is recorded the same way: one
/// `Input::Result` covering the whole accept, and, for a binding that
/// edits further after accepting, that follow-up edit too. The two can't
/// be split, since neither is safe to re-derive once either has gone
/// interactive (see `DotCapture`'s own doc). The `completion-trigger`
/// dispatch (Ctrl-Space) that opened the popup marks itself interactive too
/// and writes nothing itself, so its own placeholder never appears at all.
/// Asserts that shape, and that `.` on "bar" reproduces the same text in one
/// undo step.
///
/// The text a live `c he<accept>` wrote in the first
/// pair of parens is exactly what `.` must write in the second.
fn assert_accepted_completion_is_dot_repeated(extra_script: &str, accept: KeyEvent, then: &str) {
    let tmp = tempfile::tempdir().unwrap();
    let mut ed = editor_from("(-[foo]>) (bar)\n");
    let source = format!(
        "{}\n{extra_script}",
        completion_source("test", &completion_labels(&["hello"]), "")
    );
    run(&mut ed, tmp.path(), &source);

    ed.feed_key(key('c'));
    ed.feed_key(key('h'));
    ed.feed_key(key('e'));
    ed.feed_key(key_ctrl(' '));
    ed.settle();
    ed.feed_key(accept);
    ed.feed_key(key_esc());
    assert_eq!(
        ed.doc().text().to_string(),
        format!("(hello{then}) (bar)\n")
    );

    let inputs = &ed
        .state
        .last_action()
        .expect("`c` must have stamped a repeatable action")
        .transcript;
    assert_eq!(inputs.len(), 4, "got {inputs:?}");
    let expect_text = format!("hello{then}");
    assert!(
        is_key(&inputs[0], 'h')
            && is_key(&inputs[1], 'e')
            && matches!(&inputs[2], Input::Result(r)
                if r.back == 2 && r.forward == 0 && r.text == expect_text)
            && is_binding(&inputs[3], "exit-insert"),
        "got {inputs:?}"
    );

    select_marked(&mut ed, &format!("(hello{then}) (-[bar]>)\n"));
    ed.feed_key(key('.'));
    assert_eq!(
        ed.doc().text().to_string(),
        format!("(hello{then}) (hello{then})\n")
    );

    ed.feed_key(key('u'));
    assert_eq!(
        ed.doc().text().to_string(),
        format!("(hello{then}) (bar)\n")
    );
}

#[test]
fn dot_repeats_a_completion_accepted_with_enter() {
    assert_accepted_completion_is_dot_repeated("", key_enter(), "");
}

/// An Insert-key binding calling `completion-accept!` is never re-run by
/// `.`: its net edit is recorded as an `Input::Result` and replayed
/// directly, with no completion session to pick from (see that variant's
/// own doc).
#[test]
fn dot_repeats_a_completion_accepted_by_an_insert_key_binding() {
    assert_accepted_completion_is_dot_repeated(
        r#"(define-command! "accept-first" "" (lambda (pane) (completion-accept! 0)))
           (bind-key! 'insert "ctrl-y" "accept-first")"#,
        key_ctrl('y'),
        "",
    );
}

/// A binding that accepts and then edits records the whole dispatch as one
/// net edit (the accepted text and the follow-up edit together, not the
/// accept alone), and `.` replays that one edit, never the binding itself.
#[test]
fn dot_repeats_an_insert_key_binding_that_accepts_then_edits() {
    assert_accepted_completion_is_dot_repeated(
        r#"(define-command! "accept-space" "" (lambda (pane)
             (completion-accept! 0)
             (insert-key! pane "space")))
           (bind-key! 'insert "ctrl-y" "accept-space")"#,
        key_ctrl('y'),
        " ",
    );
}

/// A bound motion inside the session (here the Left arrow) is replayed in
/// order, so text typed after it lands where it did live.
///
/// `i ab <Left> c` writes "acb" (the `c` lands between
/// `a` and `b`), so `.` at the next line's start must write "acb" too.
#[test]
fn dot_repeats_an_arrow_key_inside_insert() {
    let mut ed = editor_from("-[h]>ello\nworld\n");

    ed.feed_key(key('i'));
    ed.feed_key(key('a'));
    ed.feed_key(key('b'));
    ed.feed_key(key_left());
    ed.feed_key(key('c'));
    ed.feed_key(key_esc());
    assert_eq!(ed.doc().text().to_string(), "acbhello\nworld\n");

    ed.feed_key(key('j'));
    ed.feed_key(key('g'));
    ed.feed_key(key('h'));
    ed.feed_key(key('.'));
    assert_eq!(ed.doc().text().to_string(), "acbhello\nacbworld\n");
}

/// An arrow key inside the session ends the typed run, so Esc leaves a
/// cursor instead of selecting what was typed. `.` leaves the same cursor.
#[test]
fn dot_after_an_arrow_key_inside_insert_leaves_the_cursor_it_left_live() {
    let mut ed = editor_from("-[h]>ello\nworld\n");

    ed.feed_key(key('i'));
    ed.feed_key(key('a'));
    ed.feed_key(key('b'));
    ed.feed_key(key_left());
    ed.feed_key(key('c'));
    ed.feed_key(key_esc());
    assert_eq!(state(&ed), "ac-[b]>hello\nworld\n");

    select_marked(&mut ed, "acbhello\n-[w]>orld\n");
    ed.feed_key(key('.'));
    assert_eq!(state(&ed), "acbhello\nac-[b]>world\n");
}

/// A pattern command bound to an Insert key opens a prompt `.` cannot
/// answer, so the change ends before that key: `.` repeats the text typed
/// before it and opens no prompt.
#[test]
fn dot_stops_before_an_insert_key_that_opened_a_pattern_prompt() {
    let tmp = tempfile::tempdir().unwrap();
    let mut ed = editor_from("-[h]>ello\nworld\n");
    run(
        &mut ed,
        tmp.path(),
        r#"(bind-key! 'insert "ctrl-s" "search-forward")"#,
    );

    ed.feed_key(key('i'));
    ed.feed_key(key('x'));
    ed.feed_key(key_ctrl('s'));
    ed.feed_key(key_esc());
    assert_eq!(ed.doc().text().to_string(), "xhello\nworld\n");

    select_marked(&mut ed, "xhello\n-[w]>orld\n");
    ed.feed_key(key('.'));
    assert_eq!(ed.doc().text().to_string(), "xhello\nxworld\n");
    assert_eq!(ed.state.mode(), Mode::Normal);
    assert!(
        ed.state
            .input
            .find::<crate::editor::input_stack::SearchLayer>()
            .is_none()
    );
    assert_eq!(status(&ed), "");
}

/// The `:` line is an input `.` cannot answer, so an Insert key that opens
/// it ends the change there: `.` repeats the text typed before it and opens
/// no `:` line.
#[test]
fn dot_stops_before_an_insert_key_that_opened_the_command_line() {
    let tmp = tempfile::tempdir().unwrap();
    let mut ed = editor_from("-[h]>ello\nworld\n");
    run(
        &mut ed,
        tmp.path(),
        r#"(bind-key! 'insert "ctrl-s" "command-mode")"#,
    );

    ed.feed_key(key('i'));
    ed.feed_key(key('x'));
    ed.feed_key(key_ctrl('s'));
    ed.feed_key(key_esc());
    ed.feed_key(key_esc());
    assert_eq!(ed.doc().text().to_string(), "xhello\nworld\n");
    assert_eq!(ed.state.mode(), Mode::Normal);

    select_marked(&mut ed, "xhello\n-[w]>orld\n");
    ed.feed_key(key('.'));
    assert_eq!(ed.doc().text().to_string(), "xhello\nxworld\n");
    assert_eq!(ed.state.mode(), Mode::Normal);
    assert!(
        ed.state
            .input
            .find::<crate::editor::input_stack::command::CommandLayer>()
            .is_none()
    );
    assert_eq!(status(&ed), "");
}

/// A native command bound directly to an Insert key must not overwrite
/// the recorded change if it's itself repeatable. That slot belongs to
/// whichever command opened the Insert session; the key is recorded into
/// that change's transcript instead (proven by
/// `dot_repeats_ctrl_w_inside_insert`'s same mechanism).
///
/// Without the `repeat_slot_owned` gate on beginning a fresh change,
/// the recorded change's command would read `"delete"` (stamped by
/// `Ctrl-x`'s own dispatch through `commands::run`) instead of the command
/// that opened the session.
#[test]
fn native_command_bound_mid_insert_does_not_hijack_the_recorded_change() {
    let tmp = tempfile::tempdir().unwrap();
    let mut ed = editor_from("-[foo]> bar\n");
    run(
        &mut ed,
        tmp.path(),
        r#"(bind-key! 'insert "ctrl-x" "delete")"#,
    );

    ed.feed_key(key('a')); // insert-at-selection-end: repeatable, opens the session
    ed.feed_key(key_ctrl('x')); // "delete" is also repeatable, dispatched mid-insert
    ed.feed_key(key_esc());

    assert_eq!(
        ed.state.last_action().map(|a| a.command.as_ref()),
        Some("insert-at-selection-end"),
        "the command that opened the Insert session must keep the slot"
    );
}

/// `.` must re-run the Insert-key binding itself, not a frozen native
/// command name it happened to `call!`: a Steel binding armed with
/// `set-register-prefix!` before its `call!` must arm that same register
/// again on replay, at the new site, rather than always writing wherever
/// the very first run wrote.
///
/// Deleting "a" then "d" with register 3 armed each
/// time must leave register 3 holding "d" (the most recent capture), the
/// same as pressing the binding twice by hand, once per site.
#[test]
fn insert_key_binding_register_prefix_reruns_on_replay() {
    let tmp = tempfile::tempdir().unwrap();
    let mut ed = editor_from("-[a]>bc def\n");
    run(
        &mut ed,
        tmp.path(),
        r#"(define-command! "del-to-reg3" ""
             (lambda (pane) (set-register-prefix! "3") (call! "delete" pane)))
           (bind-key! 'insert "ctrl-x" "del-to-reg3")"#,
    );

    ed.feed_key(key('i')); // insert-at-selection-start: cursor lands on "a"
    ed.feed_key(key_ctrl('x')); // del-to-reg3: deletes "a" into register 3
    ed.feed_key(key_esc());

    assert_eq!(ed.doc().text().to_string(), "bc def\n");
    assert_eq!(reg(&ed, '3'), vec!["a".to_string()]);

    select_marked(&mut ed, "bc -[d]>ef\n");
    ed.feed_key(key('.'));

    assert_eq!(ed.doc().text().to_string(), "bc ef\n");
    assert_eq!(
        reg(&ed, '3'),
        vec!["d".to_string()],
        "replay must re-run set-register-prefix!, not drop it and fall back to the kill ring"
    );
}

/// A binding that itself leaves Insert mid-body (`exit-insert`) and then
/// runs a repeatable native command must still leave the recorded change
/// naming the command that *opened* the session, and `.` must reproduce
/// both the typed text and the binding's own edit at the new site.
///
/// "Delete a char, type one char, then immediately delete that same char"
/// nets to exactly the original deletion, so a correct replay leaves the
/// second site exactly as if only the plain delete had run there, with no
/// typed character surviving.
#[test]
fn insert_key_binding_exiting_insert_mid_body_stays_off_the_repeat_slot() {
    let tmp = tempfile::tempdir().unwrap();
    let mut ed = editor_from("-[x]>yz abc\n");
    run(
        &mut ed,
        tmp.path(),
        r#"(define-command! "flush-char" ""
             (lambda (pane) (call! "exit-insert" pane) (call! "delete" pane)))
           (bind-key! 'insert "ctrl-k" "flush-char")"#,
    );

    ed.feed_key(key('c')); // change: delete "x", enter Insert
    ed.feed_key(key('q')); // "qyz abc\n"
    ed.feed_key(key_ctrl('k')); // flush-char: exit-insert, then delete "q" back out

    assert_eq!(ed.doc().text().to_string(), "yz abc\n");
    assert_eq!(
        ed.state.last_action().map(|a| a.command.as_ref()),
        Some("change"),
        "the command that opened the Insert session must keep the slot, \
         not the inner `delete` dispatched after exit-insert"
    );

    select_marked(&mut ed, "yz -[a]>bc\n");
    ed.feed_key(key('.'));

    assert_eq!(
        ed.doc().text().to_string(),
        "yz bc\n",
        "replay must delete \"a\", type \"q\", then delete \"q\" back out, no leftover \"q\""
    );
}

/// A binding that runs a pure motion via `call!` before an `insert-key!`
/// fallback must have that motion replayed too. Otherwise the fallback's
/// effect lands at the pre-motion cursor instead of where the motion left it.
///
/// In Insert mode `goto-line-end` lands on the line break, so the semicolon
/// is appended to each line.
///
/// Without the motion being replayed, the fallback
/// would insert right where `insert-at-selection-start` left the cursor,
/// before "e", giving "d;ef" instead.
#[test]
fn insert_key_binding_motion_via_call_is_replayed_before_the_fallback() {
    let tmp = tempfile::tempdir().unwrap();
    let mut ed = editor_from("-[a]>bc\ndef\n");
    run(
        &mut ed,
        tmp.path(),
        r#"(define-command! "append-semi" ""
             (lambda (pane) (call! "goto-line-end" pane) (insert-key! pane ";")))
           (bind-key! 'insert "ctrl-e" "append-semi")"#,
    );

    ed.feed_key(key('i')); // insert-at-selection-start: cursor before "a"
    ed.feed_key(key_ctrl('e')); // append-semi: jump to end of "abc", insert ";"
    ed.feed_key(key_esc());

    assert_eq!(ed.doc().text().to_string(), "abc;\ndef\n");

    select_marked(&mut ed, "abc;\nd-[e]>f\n");
    ed.feed_key(key('.'));

    assert_eq!(
        ed.doc().text().to_string(),
        "abc;\ndef;\n",
        "replay must redo the goto-line-end motion before the ';' fallback, \
         appending the semicolon to the second line"
    );
}

// ── Interactive Insert-key bindings: picker ─────────────────────────────────

/// An Insert-key binding that opens a picker records its pick: `.` re-runs
/// the binding with the picker answered by that pick, never reopening it.
///
/// A live `i <ctrl-y> <enter>` deletes the word behind
/// the cursor ("abc ") with nothing typed in between, so `.` at the front
/// of "xyz" must delete "uvw " the same way, with no picker ever opening
/// during replay.
#[test]
fn dot_repeats_a_picker_pick_from_an_insert_key_binding() {
    let tmp = tempfile::tempdir().unwrap();
    let mut ed = editor_from("abc -[d]>ef\nuvw xyz\n");
    run(
        &mut ed,
        tmp.path(),
        r#"(define-command! "pick-word" ""
             (lambda (pane)
               (picker! pane (list (cons "alpha" "ALPHA"))
                 (lambda (payload)
                   (when payload (call! "delete-word-backward" pane)))))
             #:repeatable #t)
           (bind-key! 'insert "ctrl-y" "pick-word")"#,
    );

    ed.feed_key(key('i'));
    ed.feed_key(key_ctrl('y'));
    assert!(ed.state.input.picker().is_some(), "sanity: picker open");
    ed.feed_key(key_enter());
    ed.settle();
    assert!(
        ed.state.input.picker().is_none(),
        "the pick's own edit must close the picker"
    );
    ed.feed_key(key_esc());
    assert_eq!(ed.doc().text().to_string(), "def\nuvw xyz\n");

    let inputs = &ed
        .state
        .last_action()
        .expect("`i` must have stamped a repeatable action")
        .transcript;
    assert_eq!(inputs.len(), 2, "got {inputs:?}");
    assert!(
        matches!(&inputs[0], Input::Binding { name, answers, .. }
                if name == "pick-word"
                    && matches!(answers.as_slice(), [Answer::Pick(Pick { picked: Some(Picked::Choice(c)), .. })] if c.label == "alpha" && c.nth == 0))
            && is_binding(&inputs[1], "exit-insert"),
        "got {inputs:?}"
    );

    select_marked(&mut ed, "def\nuvw -[x]>yz\n");
    ed.feed_key(key('.'));
    assert_eq!(
        ed.doc().text().to_string(),
        "def\nxyz\n",
        "replay must delete \"uvw \" directly, never reopen the picker"
    );
    assert!(ed.state.input.picker().is_none());
}

/// An edit landing through the shared funnel from anywhere else (a timer,
/// an LSP response) while a binding's picker sits open is not part of the
/// change: the binding records only its pick. Simulates that "anywhere
/// else" edit directly through `apply_doc_edit_grouped`, the one funnel a
/// real one would also use.
#[test]
fn dot_repeat_ignores_a_foreign_edit_made_while_a_picker_is_open() {
    let tmp = tempfile::tempdir().unwrap();
    let mut ed = editor_from("abc -[d]>ef\nuvw xyz\n");
    run(
        &mut ed,
        tmp.path(),
        r#"(define-command! "pick-word" ""
             (lambda (pane)
               (picker! pane (list (cons "alpha" "ALPHA"))
                 (lambda (payload)
                   (when payload (call! "delete-word-backward" pane)))))
             #:repeatable #t)
           (bind-key! 'insert "ctrl-y" "pick-word")"#,
    );

    ed.feed_key(key('i'));
    ed.feed_key(key_ctrl('y'));
    assert!(ed.state.input.picker().is_some(), "sanity: picker open");

    // A foreign edit, far from the cursor, landing on the same buffer while
    // the picker sits open, standing in for a timer/LSP edit unrelated to
    // the pick.
    let fp = FocusedPane::current(&ed.view);
    let (pid, bid) = (fp.pid(), fp.bid(&ed.view));
    let (buffers, mut stores) = ed.state.edit_parts();
    crate::editor::doc_ops::apply_doc_edit_grouped(buffers, &mut stores, pid, bid, |s| {
        hume_ops::edit::insert_char(s, 'Z')
    })
    .expect("the buffer is not held");
    assert_eq!(ed.doc().text().to_string(), "abc Zdef\nuvw xyz\n");

    ed.feed_key(key_enter());
    ed.settle();
    assert!(
        ed.state.input.picker().is_none(),
        "the pick's own edit must close the picker"
    );
    ed.feed_key(key_esc());

    let inputs = &ed
        .state
        .last_action()
        .expect("`i` must have stamped a repeatable action")
        .transcript;
    assert_eq!(inputs.len(), 2, "got {inputs:?}");
    assert!(
        matches!(&inputs[0], Input::Binding { name, answers, .. }
                if name == "pick-word"
                    && matches!(answers.as_slice(), [Answer::Pick(Pick { picked: Some(Picked::Choice(c)), .. })] if c.label == "alpha" && c.nth == 0))
            && is_binding(&inputs[1], "exit-insert"),
        "the binding records its pick, and nothing of the foreign 'Z' insert: {inputs:?}"
    );
}

/// A binding that edits the buffer *before* opening a picker, with a foreign
/// edit landing on the same buffer while that picker sits open: the binding
/// is re-run on `.`, so what it edits is decided again there and the
/// foreign edit has nothing to break. Its pick is recorded with no warning.
#[test]
fn dot_repeat_keeps_a_pick_when_a_foreign_edit_follows_the_bindings_own_edit() {
    let tmp = tempfile::tempdir().unwrap();
    let mut ed = editor_from("abc -[d]>ef\nuvw xyz\n");
    run(
        &mut ed,
        tmp.path(),
        r#"(define-command! "edit-then-pick" ""
             (lambda (pane)
               (insert-key! pane "x")
               (picker! pane (list (cons "alpha" "ALPHA"))
                 (lambda (payload)
                   (when payload (call! "delete-word-backward" pane)))))
             #:repeatable #t)
           (bind-key! 'insert "ctrl-y" "edit-then-pick")"#,
    );

    ed.feed_key(key('i'));
    ed.feed_key(key_ctrl('y'));
    assert!(ed.state.input.picker().is_some(), "sanity: picker open");
    assert_eq!(
        ed.doc().text().to_string(),
        "abc xdef\nuvw xyz\n",
        "sanity: the binding's own edit landed before the picker opened"
    );

    // A foreign edit, landing on the same buffer while the picker sits open,
    // standing in for a timer/LSP edit unrelated to the pick.
    let fp = FocusedPane::current(&ed.view);
    let (pid, bid) = (fp.pid(), fp.bid(&ed.view));
    let (buffers, mut stores) = ed.state.edit_parts();
    crate::editor::doc_ops::apply_doc_edit_grouped(buffers, &mut stores, pid, bid, |s| {
        hume_ops::edit::insert_char(s, 'Z')
    })
    .expect("the buffer is not held");

    ed.feed_key(key_enter()); // picks "alpha", runs delete-word-backward
    ed.settle();
    ed.feed_key(key_esc());

    assert!(
        !ed.state
            .message_log
            .entries()
            .any(|e| e.severity == Severity::Warning && e.text.contains("won't repeat")),
        "nothing to warn about: {:?}",
        ed.state.message_log.entries().collect::<Vec<_>>()
    );

    let inputs = &ed
        .state
        .last_action()
        .expect("`i` must have stamped a repeatable action")
        .transcript;
    assert!(
        matches!(inputs.first(), Some(Input::Binding { name, answers, .. })
            if name == "edit-then-pick"
                && matches!(answers.as_slice(), [Answer::Pick(Pick { picked: Some(Picked::Choice(c)), .. })] if c.label == "alpha" && c.nth == 0)),
        "the binding records its pick: {inputs:?}"
    );
}

/// A picker dismissed with Esc is recorded as a dismissal: `.` re-runs the
/// binding with its picker answered `#f`, which does nothing here, between
/// the keys typed around it.
#[test]
fn dot_repeat_replays_a_dismissed_pick_as_a_dismissal() {
    let tmp = tempfile::tempdir().unwrap();
    let mut ed = editor_from("-[h]>ello\nworld\n");
    run(
        &mut ed,
        tmp.path(),
        r#"(define-command! "pick-word" ""
             (lambda (pane)
               (picker! pane (list (cons "alpha" "ALPHA"))
                 (lambda (payload)
                   (when payload (call! "delete-word-backward" pane)))))
             #:repeatable #t)
           (bind-key! 'insert "ctrl-y" "pick-word")"#,
    );

    ed.feed_key(key('i'));
    ed.feed_key(key('x'));
    ed.feed_key(key('y'));
    ed.feed_key(key_ctrl('y'));
    ed.feed_key(key_esc()); // dismisses the picker, staying in Insert
    ed.settle();
    assert_eq!(ed.state.mode(), Mode::Insert, "sanity: still in Insert");
    ed.feed_key(key_esc()); // leaves Insert
    assert_eq!(ed.doc().text().to_string(), "xyhello\nworld\n");

    let inputs = &ed
        .state
        .last_action()
        .expect("`i` must have stamped a repeatable action")
        .transcript;
    assert_eq!(inputs.len(), 4, "got {inputs:?}");
    assert!(
        is_key(&inputs[0], 'x')
            && is_key(&inputs[1], 'y')
            && matches!(&inputs[2], Input::Binding { name, answers, .. }
                if name == "pick-word"
                    && matches!(answers.as_slice(), [Answer::Pick(Pick { picked: None, .. })]))
            && is_binding(&inputs[3], "exit-insert"),
        "got {inputs:?}"
    );

    select_marked(&mut ed, "xyhello\n-[w]>orld\n");
    ed.feed_key(key('.'));
    assert_eq!(ed.doc().text().to_string(), "xyhello\nxyworld\n");
}

// ── Interactive Insert-key bindings: completion via a session query ────────

/// A binding that consults `completion-top` to decide its own branch: live,
/// with no completion open, it falls back to a literal tab, a
/// non-interactive outcome, so `.` re-runs the binding itself, which
/// re-checks `completion-top` fresh at the new cursor (also empty there)
/// and reaches the same fallback.
#[test]
fn dot_repeat_smart_accept_binding_fallback_branch() {
    let tmp = tempfile::tempdir().unwrap();
    let mut ed = editor_from("-[h]>ello\nworld\n");
    let source = format!(
        "{}\n{}",
        completion_source("test", &completion_labels(&["hi"]), ""),
        r#"(define-command! "smart-accept" ""
             (lambda (pane)
               (if (null? (completion-top 1))
                   (insert-key! pane "tab")
                   (completion-accept! 0))))
           (bind-key! 'insert "ctrl-y" "smart-accept")"#
    );
    run(&mut ed, tmp.path(), &source);

    ed.feed_key(key('i'));
    ed.feed_key(key_ctrl('y'));
    ed.feed_key(key_esc());
    assert_eq!(ed.doc().text().to_string(), "\thello\nworld\n");

    let inputs = &ed
        .state
        .last_action()
        .expect("`i` must have stamped a repeatable action")
        .transcript;
    assert_eq!(inputs.len(), 2, "got {inputs:?}");
    assert!(
        is_binding(&inputs[0], "smart-accept") && is_binding(&inputs[1], "exit-insert"),
        "the fallback branch is non-interactive, recorded as a re-runnable Binding: {inputs:?}"
    );

    select_marked(&mut ed, "\thello\n-[w]>orld\n");
    ed.feed_key(key('.'));
    assert_eq!(
        ed.doc().text().to_string(),
        "\thello\n\tworld\n",
        "replay re-runs the binding, which reaches the same fallback at the new cursor"
    );
}

/// The same `smart-accept` binding's *accept* branch: live, with a
/// completion open, `completion-top` is non-empty and it accepts instead,
/// recorded as an `Input::Result`, not a `Binding`, so `.` applies
/// the accepted text directly rather than re-checking `completion-top`
/// (which would find no session during replay and wrongly fall back to a
/// literal tab).
#[test]
fn dot_repeat_smart_accept_binding_accept_branch() {
    let tmp = tempfile::tempdir().unwrap();
    let mut ed = editor_from("(-[foo]>) (bar)\n");
    let source = format!(
        "{}\n{}",
        completion_source("test", &completion_labels(&["hello"]), ""),
        r#"(define-command! "smart-accept" ""
             (lambda (pane)
               (if (null? (completion-top 1))
                   (insert-key! pane "tab")
                   (completion-accept! 0))))
           (bind-key! 'insert "ctrl-y" "smart-accept")"#
    );
    run(&mut ed, tmp.path(), &source);

    ed.feed_key(key('c'));
    ed.feed_key(key('h'));
    ed.feed_key(key('e'));
    ed.feed_key(key_ctrl(' '));
    ed.settle();
    ed.feed_key(key_ctrl('y'));
    ed.feed_key(key_esc());
    assert_eq!(ed.doc().text().to_string(), "(hello) (bar)\n");

    let inputs = &ed
        .state
        .last_action()
        .expect("`c` must have stamped a repeatable action")
        .transcript;
    assert_eq!(inputs.len(), 4, "got {inputs:?}");
    assert!(
        matches!(&inputs[2], Input::Result(r)
            if r.back == 2 && r.forward == 0 && r.text == "hello"),
        "got {inputs:?}"
    );

    select_marked(&mut ed, "(hello) (-[bar]>)\n");
    ed.feed_key(key('.'));
    assert_eq!(
        ed.doc().text().to_string(),
        "(hello) (hello)\n",
        "replay must apply the recorded accept, never re-check completion-top"
    );
}

// ── Interactive Insert-key bindings: failure and mode-exit corner cases ────

/// A binding that errors (live and again on replay) is reported and
/// skipped, not fatal to the rest of the session: keys typed after it must
/// still replay, matching what the live session actually did.
#[test]
fn dot_repeat_continues_past_a_binding_that_fails_on_replay() {
    let tmp = tempfile::tempdir().unwrap();
    let mut ed = editor_from("-[h]>ello\nworld\n");
    run(
        &mut ed,
        tmp.path(),
        r#"(define-command! "sometimes-fails" "" (lambda (pane) (error "boom")))
           (bind-key! 'insert "ctrl-x" "sometimes-fails")"#,
    );

    ed.feed_key(key('i'));
    ed.feed_key(key('a'));
    ed.feed_key(key_ctrl('x')); // errors live too, reported
    assert!(status(&ed).contains("boom"), "got: {}", status(&ed));
    ed.feed_key(key('b'));
    ed.feed_key(key_esc());
    assert_eq!(ed.doc().text().to_string(), "abhello\nworld\n");

    select_marked(&mut ed, "abhello\n-[w]>orld\n");
    ed.feed_key(key('.'));
    assert_eq!(
        ed.doc().text().to_string(),
        "abhello\nabworld\n",
        "replay must still type 'b' after the failing binding, not stop there"
    );
}

/// A binding whose branch depends on mutable Steel state takes a
/// non-interactive path live (recorded as a re-runnable `Binding`) but the
/// interactive one on replay (the same global flipped by the live run):
/// `completion-accept!` refuses loudly instead of silently accepting
/// against a session that was never opened this time.
#[test]
fn dot_repeat_binding_taking_the_interactive_branch_only_on_replay_errors_loudly() {
    let tmp = tempfile::tempdir().unwrap();
    let mut ed = editor_from("-[h]>ello\nworld\n");
    run(
        &mut ed,
        tmp.path(),
        r#"(define first-run #t)
           (define-command! "toggle" ""
             (lambda (pane)
               (if first-run
                   (begin (set! first-run #f) (insert-key! pane "tab"))
                   (completion-accept! 0))))
           (bind-key! 'insert "ctrl-x" "toggle")"#,
    );

    ed.feed_key(key('i'));
    ed.feed_key(key_ctrl('x'));
    ed.feed_key(key_esc());
    assert_eq!(ed.doc().text().to_string(), "\thello\nworld\n");

    select_marked(&mut ed, "\thello\n-[w]>orld\n");
    ed.feed_key(key('.'));

    assert!(
        status(&ed).contains("reached during"),
        "got: {}",
        status(&ed)
    );
    assert!(
        ed.state.input.picker().is_none(),
        "the guard must refuse before touching any session"
    );
    assert_eq!(
        ed.doc().text().to_string(),
        "\thello\nworld\n",
        "the failed toggle inserted nothing, but the replayed exit-insert still ran"
    );
}

// ── A replayed macro that leaves Insert open ───────────────────────────────

/// A replayed macro can leave Insert mode open (its own Esc rebound to a
/// no-op before replay, so the macro's queue drains while still in Insert).
/// The session the user finishes by hand belongs to the macro's own entry
/// command, and `.` repeats all of it.
#[test]
fn dot_after_macro_leaves_insert_open_repeats_the_macros_own_session() {
    let tmp = tempfile::tempdir().unwrap();
    let mut ed = editor_from("-[x]>yz\nabc\n");
    run(
        &mut ed,
        tmp.path(),
        r#"(define-command! "stay-in-insert" "" (lambda (pane) (begin)))
           (bind-key! 'insert "escape" "stay-in-insert")"#,
    );

    ed.feed_key(key('d')); // stamps "delete", must not survive the macro below
    assert_eq!(ed.doc().text().to_string(), "yz\nabc\n");

    ed.state
        .registers
        .write_macro('q', vec![key('i'), key('f'), key('o'), key('o'), key_esc()]);
    ed.feed_key(key('q'));
    ed.feed_key(key('q'));
    ed.settle();

    assert_eq!(
        ed.state.mode(),
        Mode::Insert,
        "sanity: the rebound Esc must not have left Insert"
    );
    assert_eq!(ed.doc().text().to_string(), "fooyz\nabc\n");

    ed.feed_key(key('b'));
    ed.feed_key(key('a'));
    ed.feed_key(key('r'));
    ed.feed_key(key_ctrl('c')); // the real exit
    assert_eq!(ed.doc().text().to_string(), "foobaryz\nabc\n");

    select_marked(&mut ed, "foobaryz\n-[a]>bc\n");
    ed.feed_key(key('.'));
    assert_eq!(
        ed.doc().text().to_string(),
        "foobaryz\nfoobarabc\n",
        "`.` must repeat the macro's own `i` session (foo, stay-in-insert, bar, exit), \
         not the pre-macro `delete`"
    );
}

/// `completion-trigger` refuses loudly under `.`, the same discipline
/// `completion-accept!`/`picker!` follow: a binding that reaches it during
/// replay decided differently than it did live, and there is no completion
/// request to make sense of replaying.
#[test]
fn dot_repeat_completion_trigger_reached_only_on_replay_errors_loudly() {
    let tmp = tempfile::tempdir().unwrap();
    let mut ed = editor_from("-[h]>ello\nworld\n");
    run(
        &mut ed,
        tmp.path(),
        r#"(define first-run #t)
           (define-command! "toggle" ""
             (lambda (pane)
               (if first-run
                   (begin (set! first-run #f) (insert-key! pane "tab"))
                   (call! "completion-trigger" pane))))
           (bind-key! 'insert "ctrl-x" "toggle")"#,
    );

    ed.feed_key(key('i'));
    ed.feed_key(key_ctrl('x'));
    ed.feed_key(key_esc());
    assert_eq!(ed.doc().text().to_string(), "\thello\nworld\n");

    select_marked(&mut ed, "\thello\n-[w]>orld\n");
    ed.feed_key(key('.'));

    assert!(
        status(&ed).contains("reached during"),
        "got: {}",
        status(&ed)
    );
    assert!(
        ed.state.input.buffer_completion().is_none(),
        "the guard must refuse before opening anything"
    );
    assert_eq!(ed.doc().text().to_string(), "\thello\nworld\n");
}

/// A `Binding` that leaves Insert and then fails must stop the replay loop
/// there (the same as one that leaves Insert and succeeds), not keep going
/// and type the rest of the recorded session's keys outside any Insert
/// session, in whatever mode is now current.
#[test]
fn dot_repeat_stops_when_a_binding_leaves_insert_and_then_fails_on_replay() {
    let tmp = tempfile::tempdir().unwrap();
    let mut ed = editor_from("-[h]>ello\nworld\n");
    run(
        &mut ed,
        tmp.path(),
        r#"(define first-run #t)
           (define-command! "toggle" ""
             (lambda (pane)
               (if first-run
                   (begin (set! first-run #f) (insert-key! pane "tab"))
                   (begin (call! "exit-insert" pane) (error "boom")))))
           (bind-key! 'insert "ctrl-x" "toggle")"#,
    );

    ed.feed_key(key('i'));
    ed.feed_key(key_ctrl('x')); // first-run: inserts a tab, stays in Insert
    ed.feed_key(key('q')); // typed after it, into the same session
    ed.feed_key(key_esc());
    assert_eq!(ed.doc().text().to_string(), "\tqhello\nworld\n");

    select_marked(&mut ed, "\tqhello\n-[w]>orld\n");
    ed.feed_key(key('.'));

    assert!(status(&ed).contains("boom"), "got: {}", status(&ed));
    assert_eq!(ed.state.mode(), Mode::Normal);
    assert_eq!(
        ed.doc().text().to_string(),
        "\tqhello\nworld\n",
        "toggle's own exit-insert ran, but 'q' and the replayed exit-insert \
         must not run after replay has already left Insert"
    );
}

// ── Interactive edits captured at the funnel, not diffed after the fact ────

/// An accept whose item carries `additionalTextEdits` edits two separate
/// regions of the buffer in one dispatch (the import, then the cursor's own
/// replacement): recording must capture only the cursor's own region, not
/// error out over the extra one.
#[test]
fn dot_repeat_of_an_accept_with_additional_text_edits_replays_only_the_cursor_edit() {
    let tmp = tempfile::tempdir().unwrap();
    let mut ed = editor_from("(-[x]>) (bar)\n");
    let source = completion_source(
        "test",
        r#"(list (hash "label" "hello" "insertText" "hello"
                       "additionalTextEdits"
                         (list (hash "range" (hash "start" (hash "line" 0 "character" 0)
                                                  "end" (hash "line" 0 "character" 0))
                                 "newText" "// "))))"#,
        "",
    );
    run(&mut ed, tmp.path(), &source);

    ed.feed_key(key('c'));
    ed.feed_key(key('h'));
    ed.feed_key(key('e'));
    ed.feed_key(key_ctrl(' '));
    ed.settle();
    ed.feed_key(key_enter());
    assert!(
        !status(&ed).contains("dot-repeat"),
        "accept must not report a multi-region error: {}",
        status(&ed)
    );
    ed.feed_key(key_esc());
    assert_eq!(ed.doc().text().to_string(), "// (hello) (bar)\n");

    select_marked(&mut ed, "// (hello) (-[bar]>)\n");
    ed.feed_key(key('.'));
    assert_eq!(
        ed.doc().text().to_string(),
        "// (hello) (hello)\n",
        "`.` must replay only the cursor's own replacement, not re-insert the import"
    );
}

/// A multi-cursor accept composes one `ChangeSet` with an edited region at
/// every cursor: recording must extract only the primary's own region
/// instead of erroring over the others.
#[test]
fn dot_repeat_of_a_multi_cursor_accept_replays_at_every_cursor() {
    let mut ed = editor_from("-{foo}> -[bar]>\nfoo bar\n");
    ed.feed_key(key('c'));
    type_chars(&mut ed, "st");
    open_completion_session(&mut ed, &["std"]);
    ed.feed_key(key_enter());
    assert!(
        !status(&ed).contains("dot-repeat"),
        "a multi-cursor accept must not report a multi-region error: {}",
        status(&ed)
    );
    ed.feed_key(key('!'));
    ed.feed_key(key_esc());
    assert_eq!(ed.doc().text().to_string(), "std! std!\nfoo bar\n");

    select_marked(&mut ed, "std! std!\n-{foo}> -[bar]>\n");
    ed.feed_key(key('.'));
    assert_eq!(ed.doc().text().to_string(), "std! std!\nstd! std!\n");
}

/// Typing, moving the cursor away, typing again, then accepting: the
/// session's cumulative changeset has two separate insert regions (one per
/// typed run) by the time the accept lands, but only the accept's own
/// dispatch (the window `DotCapture` is armed for) feeds its capture.
#[test]
fn dot_repeat_of_an_accept_after_typing_moving_and_typing_again() {
    let tmp = tempfile::tempdir().unwrap();
    let mut ed = editor_from("( )\n(-[ ]>)\n");
    run(
        &mut ed,
        tmp.path(),
        &completion_source("test", &completion_labels(&["hello"]), ""),
    );

    ed.feed_key(key('i'));
    ed.feed_key(key('z')); // typed run #1, on line 2
    ed.feed_key(key_up()); // moves to line 1, same column, inside "( )"
    ed.feed_key(key('h'));
    ed.feed_key(key('e')); // typed run #2, on line 1
    ed.feed_key(key_ctrl(' '));
    ed.settle();
    ed.feed_key(key_enter()); // accepts "hello" over "he"
    assert!(
        !status(&ed).contains("dot-repeat"),
        "a delta spanning two disjoint typed runs must not error: {}",
        status(&ed)
    );
    ed.feed_key(key_esc());
    assert_eq!(ed.doc().text().to_string(), "( hello)\n(z )\n");
}

/// A binding that calls `completion-accept!` and then `exit-insert` in the
/// same body tears the Insert session down from inside its own dispatch,
/// before `handle_insert`'s post-dispatch checkpoint ever runs:
/// `tear_down_insert`'s own backstop must still finalize the capture, not
/// lose the accept.
#[test]
fn dot_repeat_of_a_binding_that_accepts_then_exits_insert_in_one_body() {
    let tmp = tempfile::tempdir().unwrap();
    let mut ed = editor_from("(-[x]>) (bar)\n");
    run(
        &mut ed,
        tmp.path(),
        &format!(
            "{}\n{}",
            completion_source("test", &completion_labels(&["hello"]), ""),
            r#"(define-command! "accept-and-leave" ""
                 (lambda (pane)
                   (completion-accept! 0)
                   (call! "exit-insert" pane)))
               (bind-key! 'insert "ctrl-y" "accept-and-leave")"#
        ),
    );

    ed.feed_key(key('c'));
    ed.feed_key(key('h'));
    ed.feed_key(key('e'));
    ed.feed_key(key_ctrl(' '));
    ed.settle();
    ed.feed_key(key_ctrl('y'));
    assert_eq!(
        ed.state.mode(),
        Mode::Normal,
        "accept-and-leave must leave Insert"
    );
    assert_eq!(ed.doc().text().to_string(), "(hello) (bar)\n");

    select_marked(&mut ed, "(hello) (-[bar]>)\n");
    ed.feed_key(key('.'));
    assert_eq!(
        ed.doc().text().to_string(),
        "(hello) (hello)\n",
        "`.` must reproduce the accept, not lose it to the same-dispatch exit-insert"
    );
}

/// A `MAX_EVENT_DRAIN` cascade drop that catches the batch holding a
/// picker's own `on_select`: the pick the user made is still the binding's
/// answer, and the session keeps recording what is typed after it.
#[test]
fn dot_repeat_keeps_a_pick_whose_on_select_is_lost_to_the_drain_cap() {
    let tmp = tempfile::tempdir().unwrap();
    let mut ed = editor_from("-[h]>ello\n");
    run(
        &mut ed,
        tmp.path(),
        r#"(define-command! "pick-word" ""
             (lambda (pane)
               (picker! pane (list (cons "alpha" "ALPHA"))
                 (lambda (payload) (void))))
             #:repeatable #t)
           (bind-key! 'insert "ctrl-y" "pick-word")"#,
    );

    ed.feed_key(key('i'));
    ed.feed_key(key_ctrl('y'));
    assert!(ed.state.input.picker().is_some(), "sanity: picker open");
    ed.feed_key(key_enter()); // closes the picker, queues on_select
    assert!(
        !ed.state.config.pending_work.is_empty(),
        "sanity: on_select is queued, not yet run"
    );

    // Pad the same batch past the drain cap (1000, `scripting_setup.rs`'s
    // `MAX_EVENT_DRAIN`), so the whole thing, on_select included, is
    // dropped without ever running.
    for _ in 0..1100 {
        ed.state
            .config
            .pending_work
            .push_back(crate::editor::event::PendingWork::Event(
                crate::editor::event::EditorEvent::OnFocusGained,
            ));
    }
    ed.settle();
    assert!(
        ed.state
            .message_log
            .entries()
            .any(|e| e.severity == Severity::Error
                && e.text.contains("event/callback cascade exceeded")),
        "the drain cap must have tripped"
    );

    assert!(
        matches!(
            ed.state
                .last_action()
                .and_then(|a| a.transcript.last()),
            Some(Input::Binding { name, answers, .. })
                if name == "pick-word"
                    && matches!(answers.as_slice(), [Answer::Pick(Pick { picked: Some(Picked::Choice(c)), .. })] if c.label == "alpha" && c.nth == 0)
        ),
        "the binding records the pick the user made"
    );

    // The session must still be usable: later typing records normally.
    ed.feed_key(key('z'));
    ed.feed_key(key_esc());
    assert_eq!(ed.doc().text().to_string(), "zhello\n");
    let inputs = &ed
        .state
        .last_action()
        .expect("`i` must have stamped a repeatable action")
        .transcript;
    assert!(
        inputs.iter().any(|i| is_key(i, 'z')),
        "typing after the dropped callback must still be recorded: {inputs:?}"
    );
}

/// `PickerLayer::tear_down`'s own hand-off, reached when a picker is
/// removed by something other than `close_picker`/`close_picker_with`
/// (every other picker test in this file closes through Enter or Esc,
/// which route through those). `end_insert_session` (what a reload or a
/// focus/buffer switch runs) truncates the Insert layer top-first, tearing
/// down the Picker on top of it via `tear_down` (not `close_picker`) and
/// then Insert itself. `on_select` must still fire, after the session is
/// gone, and the binding stays recorded with its picker answered as a
/// dismissal.
#[test]
fn picker_torn_down_by_a_reload_fires_on_select_and_records_a_dismissal() {
    let tmp = tempfile::tempdir().unwrap();
    let mut ed = editor_from("abc -[d]>ef\nuvw xyz\n");
    run(
        &mut ed,
        tmp.path(),
        r#"(define-command! "edit-then-pick" ""
             (lambda (pane)
               (insert-key! pane "x")
               (picker! pane (list (cons "alpha" "ALPHA"))
                 (lambda (payload)
                   (when payload (call! "delete-word-backward" pane)))))
             #:repeatable #t)
           (bind-key! 'insert "ctrl-y" "edit-then-pick")"#,
    );

    ed.feed_key(key('i'));
    ed.feed_key(key_ctrl('y'));
    assert!(ed.state.input.picker().is_some(), "sanity: picker open");
    assert_eq!(ed.doc().text().to_string(), "abc xdef\nuvw xyz\n");

    // Simulate the teardown a reload or a focus/buffer switch runs
    // (`end_focus_sessions`'s own chokepoint) while the picker still sits
    // on top of the Insert session.
    crate::editor::commands::end_insert_session(&mut ed.state, &ed.view);
    ed.settle();

    assert!(ed.state.input.picker().is_none(), "the picker must be gone");
    assert_ne!(
        ed.state.mode(),
        Mode::Insert,
        "the Insert session must be gone too"
    );
    assert_eq!(
        ed.doc().text().to_string(),
        "abc xdef\nuvw xyz\n",
        "on_select(#f) must have run and done nothing, not panicked or been skipped"
    );

    let inputs = &ed
        .state
        .last_action()
        .expect("`i` must have stamped a repeatable action")
        .transcript;
    assert!(
        matches!(inputs.as_slice(), [Input::Binding { name, answers, .. }]
            if name == "edit-then-pick"
                && matches!(answers.as_slice(), [Answer::Pick(Pick { picked: None, .. })])),
        "the binding is recorded, its torn-down picker as a dismissal: {inputs:?}"
    );
}

/// An edit before the last insertion moves it; `mii` follows it.
#[test]
fn mii_follows_the_insertion_when_text_before_it_changes() {
    let mut ed = editor_from("abc-[d]>ef\n");
    ed.feed_key(key('i'));
    ed.feed_key(key('X'));
    ed.feed_key(key('Y'));
    ed.feed_key(key_esc());
    assert_eq!(ed.doc().text().to_string(), "abcXYdef\n");

    set_cursor(&mut ed, 0);
    ed.feed_key(key('d'));
    assert_eq!(ed.doc().text().to_string(), "bcXYdef\n");

    ed.feed_key(key('m'));
    ed.feed_key(key('i'));
    ed.feed_key(key('i'));
    assert_eq!(state(&ed), "bc-[XY]>def\n");
}

/// Deleting the last insertion outright leaves nothing for `mii` to select.
#[test]
fn mii_refuses_once_the_insertion_is_deleted() {
    let mut ed = editor_from("-[a]>bc\n");
    ed.feed_key(key('i'));
    ed.feed_key(key('X'));
    ed.feed_key(key_esc());
    assert_eq!(ed.doc().text().to_string(), "Xabc\n");

    set_cursor(&mut ed, 0);
    ed.feed_key(key('d'));
    assert_eq!(ed.doc().text().to_string(), "abc\n");
    let before = state(&ed);

    ed.feed_key(key('m'));
    ed.feed_key(key('i'));
    ed.feed_key(key('i'));
    assert_eq!(state(&ed), before, "no insertion is left to select");
}

/// An Insert binding to a command that reads its character from the next
/// key repeats the edit it made, not the binding: `.` reads no key.
#[test]
fn dot_repeats_an_insert_binding_that_read_a_character() {
    let tmp = tempfile::tempdir().unwrap();
    let mut ed = editor_from("-[h]>ello\nworld\n");
    run(
        &mut ed,
        tmp.path(),
        r#"(bind-key! 'insert "ctrl-x" "replace")"#,
    );
    ed.feed_key(key('i'));
    ed.feed_key(key('a'));
    ed.feed_key(key_ctrl('x'));
    ed.feed_key(key('z'));
    ed.feed_key(key_esc());
    assert_eq!(ed.doc().text().to_string(), "azello\nworld\n");
    ed.feed_key(key('j'));
    ed.feed_key(key('g'));
    ed.feed_key(key('h'));

    ed.feed_key(key('.'));
    assert_eq!(ed.doc().text().to_string(), "azello\nazorld\n");
    assert!(!reading_char(&ed));
    assert_eq!(ed.state.mode(), Mode::Normal);
}

/// Defines the repeatable `rd` from `body`, runs it on line 1 of
/// `hello\nworld`, feeds `keys` one at a time, then moves to line 2.
fn record_rd(body: &str, keys: &[KeyEvent]) -> Editor {
    let mut ed = editor_with_steel(
        "-[h]>ello\nworld\n",
        &format!(r#"(define-command! "rd" "" (lambda (pane) {body}) #:repeatable #t)"#),
    );
    ed.execute_keymap_command("rd".into(), Some(1), false);
    ed.settle();
    for &k in keys {
        ed.feed_key(k);
        ed.settle();
    }
    assert_eq!(
        ed.state.last_action().map(|a| a.command.as_ref()),
        Some("rd")
    );
    ed.feed_key(key('j'));
    ed.feed_key(key('g'));
    ed.feed_key(key('h'));
    ed
}

/// A cancelled read is answered with `#f` on replay too, without waiting
/// for a key.
#[test]
fn dot_answers_a_cancelled_read_with_false() {
    let mut ed = record_rd(
        r#"(on-next-char! (lambda (c) (if c (+ 1 0) (call! "replace" (focused-pane) #\z))))"#,
        &[key_esc()],
    );
    assert_eq!(ed.doc().text().to_string(), "zello\nworld\n");
    ed.feed_key(key('.'));
    assert_eq!(ed.doc().text().to_string(), "zello\nzorld\n");
    assert!(!reading_char(&ed));
    assert!(
        !status(&ed).contains("reached during"),
        "got: {}",
        status(&ed)
    );
}

/// A reload drops a read still pending from a repeatable command. The change
/// that read belonged to stops accepting input with it, so the next
/// repeatable command starts a change of its own.
#[test]
fn reload_drops_a_pending_reads_hold_on_the_change() {
    let mut ed = editor_with_steel(
        "-[h]>ello\nworld\n",
        r#"(define-command! "rd" "" (lambda (pane) (on-next-char! (lambda (c) (+ 1 0)))) #:repeatable #t)"#,
    );
    ed.execute_keymap_command("rd".into(), Some(1), false);
    ed.settle();
    assert!(reading_char(&ed));
    ed.reset_config_state();
    ed.feed_key(key('d'));
    ed.feed_key(key('j'));
    ed.feed_key(key('g'));
    ed.feed_key(key('h'));
    ed.feed_key(key('.'));
    assert_eq!(ed.doc().text().to_string(), "ello\norld\n");
}

/// `.` repeats the command with the character it read, and the edits its
/// callback made are the command's: none of them takes `.`'s slot. No
/// settle after `.`: the callback ran during it.
#[test]
fn dot_repeats_a_repeatable_command_with_the_character_it_read() {
    let mut ed = record_rd(
        r#"(on-next-char!
             (lambda (c)
               (when c (call! "replace" pane c) (call! "surround-add" pane #\())))"#,
        &[key('z')],
    );
    assert_eq!(ed.doc().text().to_string(), "(z)ello\nworld\n");
    ed.feed_key(key('.'));
    assert_eq!(ed.doc().text().to_string(), "(z)ello\n(z)orld\n");
    assert!(!reading_char(&ed));
}

#[test]
fn dot_repeats_characters_read_by_a_chained_callback() {
    let mut ed = record_rd(
        r#"(on-next-char!
             (lambda (a)
               (when a
                 (call! "replace" pane a)
                 (on-next-char! (lambda (b) (when b (call! "surround-add" pane b)))))))"#,
        &[key('z'), key('[')],
    );
    assert_eq!(ed.doc().text().to_string(), "[z]ello\nworld\n");
    ed.feed_key(key('.'));
    assert_eq!(ed.doc().text().to_string(), "[z]ello\n[z]orld\n");
    assert!(!reading_char(&ed));
}

/// A read the body ends itself (here, by leaving Insert mode above it)
/// hears `#f` before the command is recorded, and `.` answers it the same.
#[test]
fn dot_answers_a_read_the_body_itself_ended() {
    let mut ed = record_rd(
        r#"(call! "insert-before" pane)
           (on-next-char! (lambda (c) (unless c (call! "replace" pane #\z))))
           (call! "exit-insert" pane)"#,
        &[],
    );
    assert_eq!(ed.doc().text().to_string(), "zello\nworld\n");
    ed.feed_key(key('.'));
    assert_eq!(ed.doc().text().to_string(), "zello\nzorld\n");
    assert!(
        !status(&ed).contains("reached during"),
        "got: {}",
        status(&ed)
    );
}

/// Replay reading more characters than the original run did refuses the
/// extra read, keeping what the answered ones did.
#[test]
fn dot_refuses_a_read_the_recording_has_no_answer_for() {
    let mut ed = editor_with_steel(
        "-[h]>ello\nworld\n",
        r#"(define runs 0)
           (define-command! "rd" ""
             (lambda (pane)
               (set! runs (+ runs 1))
               (on-next-char!
                 (lambda (c)
                   (call! "replace" pane c)
                   (when (> runs 1) (on-next-char! (lambda (d) (+ 1 0)))))))
             #:repeatable #t)"#,
    );
    ed.execute_keymap_command("rd".into(), Some(1), false);
    ed.feed_key(key('z'));
    ed.settle();
    select_marked(&mut ed, "zello\n-[w]>orld\n");
    ed.feed_key(key('.'));
    assert_eq!(ed.doc().text().to_string(), "zello\nzorld\n");
    assert!(!reading_char(&ed));
    assert!(
        status(&ed).contains("on-next-char!: reached during `.`"),
        "got: {}",
        status(&ed)
    );
}

// ── Several inputs waiting at once from one Insert-key binding ───────────────

#[test]
fn dot_repeats_both_edits_of_two_queued_character_reads() {
    let tmp = tempfile::tempdir().unwrap();
    let mut ed = editor_from("-[h]>ello\nworld\n");
    run(
        &mut ed,
        tmp.path(),
        r#"(define-command! "two-reads" ""
             (lambda (pane)
               (on-next-char! (lambda (a) (when a (call! "replace" pane a))))
               (on-next-char! (lambda (b) (when b (call! "surround-add" pane b)))))
             #:repeatable #t)
           (bind-key! 'insert "ctrl-x" "two-reads")"#,
    );
    ed.feed_key(key('i'));
    ed.feed_key(key_ctrl('x'));
    ed.feed_key(key('z'));
    ed.settle();
    ed.feed_key(key('('));
    ed.settle();
    ed.feed_key(key_esc());
    assert_eq!(ed.doc().text().to_string(), "(z)ello\nworld\n");

    select_marked(&mut ed, "(z)ello\n-[w]>orld\n");
    ed.feed_key(key('.'));
    assert_eq!(ed.doc().text().to_string(), "(z)ello\n(z)orld\n");
    assert!(!reading_char(&ed));
}

/// The picker opens above the pending read, so it resolves first; `.`
/// replays both edits in the order they happened.
#[test]
fn dot_repeats_a_pick_and_a_character_read_from_one_binding() {
    let tmp = tempfile::tempdir().unwrap();
    let mut ed = editor_from("abc -[d]>ef\nuvw xyz\n");
    run(
        &mut ed,
        tmp.path(),
        r#"(define-command! "read-and-pick" ""
             (lambda (pane)
               (on-next-char! (lambda (c) (when c (call! "replace" pane c))))
               (picker! pane (list (cons "alpha" "ALPHA"))
                 (lambda (payload)
                   (when payload (call! "delete-word-backward" pane)))))
             #:repeatable #t)
           (bind-key! 'insert "ctrl-y" "read-and-pick")"#,
    );
    ed.feed_key(key('i'));
    ed.feed_key(key_ctrl('y'));
    ed.feed_key(key_enter());
    ed.settle();
    ed.feed_key(key('z'));
    ed.settle();
    ed.feed_key(key_esc());
    assert_eq!(ed.doc().text().to_string(), "zef\nuvw xyz\n");

    select_marked(&mut ed, "zef\nuvw -[x]>yz\n");
    ed.feed_key(key('.'));
    assert_eq!(ed.doc().text().to_string(), "zef\nzyz\n");
    assert!(ed.state.input.picker().is_none());
    assert!(!reading_char(&ed));
}

#[test]
fn dot_repeats_an_edit_a_binding_makes_after_starting_a_character_read() {
    let tmp = tempfile::tempdir().unwrap();
    let mut ed = editor_from("-[h]>ello\nworld\n");
    run(
        &mut ed,
        tmp.path(),
        r#"(define-command! "read-then-type" ""
             (lambda (pane)
               (on-next-char! (lambda (c) (when c (call! "replace" pane c))))
               (insert-key! pane "x"))
             #:repeatable #t)
           (bind-key! 'insert "ctrl-x" "read-then-type")"#,
    );
    ed.feed_key(key('i'));
    ed.feed_key(key_ctrl('x'));
    ed.feed_key(key('z'));
    ed.settle();
    ed.feed_key(key_esc());
    assert_eq!(ed.doc().text().to_string(), "xzello\nworld\n");

    select_marked(&mut ed, "xzello\n-[w]>orld\n");
    ed.feed_key(key('.'));
    assert_eq!(ed.doc().text().to_string(), "xzello\nxzorld\n");
    assert!(!reading_char(&ed));
}

#[test]
fn dot_repeats_an_edit_a_binding_makes_after_opening_a_picker() {
    let tmp = tempfile::tempdir().unwrap();
    let mut ed = editor_from("-[h]>ello\nworld\n");
    run(
        &mut ed,
        tmp.path(),
        r#"(define-command! "pick-then-type" ""
             (lambda (pane)
               (picker! pane (list (cons "alpha" "ALPHA"))
                 (lambda (payload) (when payload (call! "replace" pane #\z))))
               (insert-key! pane "x"))
             #:repeatable #t)
           (bind-key! 'insert "ctrl-y" "pick-then-type")"#,
    );
    ed.feed_key(key('i'));
    ed.feed_key(key_ctrl('y'));
    ed.feed_key(key_enter());
    ed.settle();
    ed.feed_key(key_esc());
    assert_eq!(ed.doc().text().to_string(), "xzello\nworld\n");

    select_marked(&mut ed, "xzello\n-[w]>orld\n");
    ed.feed_key(key('.'));
    assert_eq!(ed.doc().text().to_string(), "xzello\nxzorld\n");
    assert!(ed.state.input.picker().is_none());
}

/// A read for a command bound directly to the key, cancelled with Esc,
/// leaves nothing for `.` to replay from the binding.
#[test]
fn dot_skips_a_binding_whose_character_read_was_cancelled() {
    let tmp = tempfile::tempdir().unwrap();
    let mut ed = editor_from("-[h]>ello\nworld\n");
    run(
        &mut ed,
        tmp.path(),
        r#"(bind-key! 'insert "ctrl-x" "replace")"#,
    );
    ed.feed_key(key('i'));
    ed.feed_key(key('a'));
    ed.feed_key(key_ctrl('x'));
    ed.feed_key(key_esc());
    ed.feed_key(key_esc());
    assert_eq!(ed.doc().text().to_string(), "ahello\nworld\n");

    select_marked(&mut ed, "ahello\n-[w]>orld\n");
    ed.feed_key(key('.'));
    assert_eq!(ed.doc().text().to_string(), "ahello\naworld\n");
    assert!(!reading_char(&ed));
    assert_eq!(ed.state.mode(), Mode::Normal);
}

/// A prompt a non-repeatable Insert binding opens ends the change: `.`
/// repeats neither the binding nor the edit it made before the prompt, and
/// never opens the prompt.
#[test]
fn a_non_repeatable_insert_bindings_prompt_ends_the_change() {
    let tmp = tempfile::tempdir().unwrap();
    let mut ed = editor_from("-[h]>ello\nworld\n");
    run(
        &mut ed,
        tmp.path(),
        r#"(define-command! "type-then-ask" ""
             (lambda (pane)
               (insert-key! pane "x")
               (prompt! pane "x:" (lambda (t) (+ 1 0)))))
           (bind-key! 'insert "ctrl-x" "type-then-ask")"#,
    );
    ed.feed_key(key('i'));
    ed.feed_key(key_ctrl('x'));
    ed.feed_key(key_esc());
    ed.settle();
    assert_eq!(ed.doc().text().to_string(), "xhello\nworld\n");
    assert!(
        ed.state
            .last_action()
            .is_some_and(|a| a.transcript.is_empty()),
        "got {:?}",
        ed.state.last_action()
    );

    select_marked(&mut ed, "xhello\n-[w]>orld\n");
    ed.feed_key(key('.'));
    assert_eq!(ed.doc().text().to_string(), "xhello\nworld\n");
    assert_eq!(ed.state.mode(), Mode::Normal);
    assert!(
        !status(&ed).contains("reached during"),
        "got: {}",
        status(&ed)
    );
}

/// `.` repeats the menu item a command's menu was answered with, matched by
/// its label, without opening the menu.
#[test]
fn dot_repeats_a_commands_menu_choice() {
    let mut ed = record_rd(
        r#"(show-menu! pane (list "one" "two")
             (lambda (i) (when i (call! "replace" pane (if (= i 1) #\t #\o)))))"#,
        &[key('j'), key_enter()],
    );
    assert_eq!(ed.doc().text().to_string(), "tello\nworld\n");
    ed.feed_key(key('.'));
    ed.settle();
    assert!(ed.state.input.menu().is_none(), "`.` must not open a menu");
    assert_eq!(ed.doc().text().to_string(), "tello\ntorld\n");
}

/// A menu `close-menu!` closed before it was answered: `.` opens nothing
/// for it and runs no callback, and the rest of the command repeats.
#[test]
fn dot_replays_a_menu_closed_unanswered_as_nothing() {
    let mut ed = record_rd(
        r#"(let ((tok (show-menu! pane (list "one")
                        (lambda (i) (call! "replace" pane #\x)))))
             (close-menu! tok)
             (call! "delete" pane))"#,
        &[],
    );
    assert_eq!(ed.doc().text().to_string(), "ello\nworld\n");
    assert!(
        matches!(
            ed.state.last_action().map(|a| a.answers.as_slice()),
            Some([Answer::Unanswered])
        ),
        "got: {:?}",
        ed.state.last_action()
    );
    ed.feed_key(key('.'));
    ed.settle();
    assert!(ed.state.input.menu().is_none(), "`.` must not open a menu");
    assert_eq!(ed.doc().text().to_string(), "ello\norld\n");
}

/// A drawer is not an input of the command that opened it: picking a row
/// records no answer, and a repeatable command its `on-select` runs is a
/// change of its own.
#[test]
fn a_drawer_pick_is_no_part_of_the_commands_change() {
    let mut ed = editor_with_steel(
        "-[h]>ello\nworld\n",
        r#"(define-command! "dr" ""
             (lambda (pane)
               (call! "replace" pane #\x)
               (show-drawer-list! pane (list "a" "b")
                 (lambda (idx tok) (when idx (call! "delete" pane)))))
             #:repeatable #t)"#,
    );
    ed.execute_keymap_command("dr".into(), Some(1), false);
    ed.settle();
    assert!(ed.state.input.drawer().is_some(), "sanity: drawer open");
    assert_eq!(ed.doc().text().to_string(), "xello\nworld\n");
    ed.feed_key(key_enter());
    ed.settle();
    assert_eq!(ed.doc().text().to_string(), "ello\nworld\n");
    let action = ed.state.last_action().expect("a change is recorded");
    assert_eq!(action.command.as_ref(), "delete");
    assert!(action.answers.is_empty(), "got: {:?}", action.answers);
}

/// Of two menu items with the same text, `.` repeats the one picked.
#[test]
fn dot_repeats_the_second_of_two_menu_items_with_the_same_text() {
    let mut ed = record_rd(
        r#"(show-menu! pane (list "dup" "dup")
             (lambda (i) (when i (call! "replace" pane (if (= i 1) #\b #\a)))))"#,
        &[key('j'), key_enter()],
    );
    assert_eq!(ed.doc().text().to_string(), "bello\nworld\n");
    ed.feed_key(key('.'));
    ed.settle();
    assert_eq!(ed.doc().text().to_string(), "bello\nborld\n");
}

/// Of two `picker!` items with the same text, `.` repeats the one picked.
#[test]
fn dot_repeats_the_second_of_two_picker_items_with_the_same_text() {
    let mut ed = record_rd(
        r#"(picker! pane (list (cons "dup" #\a) (cons "dup" #\b))
             (lambda (c) (when c (call! "replace" pane c))))"#,
        &[key_down(), key_enter()],
    );
    assert_eq!(ed.doc().text().to_string(), "bello\nworld\n");
    ed.feed_key(key('.'));
    ed.settle();
    assert_eq!(ed.doc().text().to_string(), "bello\nborld\n");
}

/// `.` repeats the text a command's prompt was answered with, without
/// opening the prompt.
#[test]
fn dot_repeats_a_commands_prompt_answer() {
    let mut ed = record_rd(
        r#"(prompt! pane "x:"
             (lambda (t) (when t (call! "replace" pane (string-ref t 0)))))"#,
        &[key('z'), key('q'), key_enter()],
    );
    assert_eq!(ed.doc().text().to_string(), "zello\nworld\n");
    ed.feed_key(key('.'));
    ed.settle();
    assert_eq!(ed.state.mode(), Mode::Normal);
    assert_eq!(ed.doc().text().to_string(), "zello\nzorld\n");
}

// ── Recording source accounting ──────────────────────────────────────────────

/// A character read opened mid-Insert by code outside every recorded
/// change (here a hook) ends the change: `.` replays the keys typed before
/// it, not its answer nor the keys after it.
#[test]
fn a_read_opened_outside_the_change_mid_insert_ends_the_change() {
    let tmp = tempfile::tempdir().unwrap();
    let mut ed = editor_from("-[h]>ello\nworld\n");
    run(
        &mut ed,
        tmp.path(),
        r#"(define armed #t)
           (register-hook! 'on-text-changed
             (lambda (bid)
               (when armed
                 (set! armed #f)
                 (on-next-char! (lambda (c) (+ 1 0))))))"#,
    );
    ed.feed_key(key('i'));
    ed.feed_key(key('a'));
    ed.settle();
    assert!(reading_char(&ed), "setup: the hook's read must be pending");
    ed.feed_key(key('b'));
    ed.feed_key(key('c'));
    ed.feed_key(key_esc());
    assert_eq!(ed.doc().text().to_string(), "achello\nworld\n");

    let action = ed
        .state
        .last_action()
        .expect("`i` must have recorded a change");
    assert!(
        action.answers.is_empty(),
        "a hook's read answer must not join the change: {:?}",
        action.answers
    );
    assert!(
        matches!(action.transcript.as_slice(), [i] if is_key(i, 'a')),
        "got {:?}",
        action.transcript
    );

    select_marked(&mut ed, "achello\n-[w]>orld\n");
    ed.feed_key(key('.'));
    assert_eq!(ed.doc().text().to_string(), "achello\naworld\n");
}

/// A character read opened by code outside every recorded change (here a
/// hook) after `c`, before the first typed key, ends the change there: `.`
/// repeats `c` with no text.
#[test]
fn a_read_opened_outside_the_change_before_its_first_key_ends_the_change() {
    let tmp = tempfile::tempdir().unwrap();
    let mut ed = editor_from("-[h]>ello\nworld\n");
    run(
        &mut ed,
        tmp.path(),
        r#"(define armed #t)
           (register-hook! 'on-mode-change
             (lambda (old new)
               (when armed
                 (set! armed #f)
                 (on-next-char! (lambda (c) (+ 1 0))))))"#,
    );
    ed.feed_key(key('c'));
    ed.settle();
    assert!(reading_char(&ed), "setup: the hook's read must be pending");
    ed.feed_key(key('q'));
    ed.settle();
    ed.feed_key(key('a'));
    ed.feed_key(key('b'));
    ed.feed_key(key_esc());
    assert_eq!(ed.doc().text().to_string(), "abello\nworld\n");

    let answers = &ed
        .state
        .last_action()
        .expect("`c` must have recorded a change")
        .answers;
    assert!(
        answers.is_empty(),
        "a hook's read answer must not join the change: {answers:?}"
    );

    assert!(
        ed.state
            .last_action()
            .is_some_and(|a| a.transcript.is_empty()),
        "got {:?}",
        ed.state.last_action()
    );

    select_marked(&mut ed, "abello\n-[w]>orld\n");
    ed.feed_key(key('.'));
    assert_eq!(ed.doc().text().to_string(), "abello\norld\n");
}

/// A callback of an Insert-key binding's read that opens a second read
/// records that answer under the binding too.
#[test]
fn dot_repeats_both_reads_a_binding_and_its_callback_make() {
    let tmp = tempfile::tempdir().unwrap();
    let mut ed = editor_from("-[h]>ello\nworld\n");
    run(
        &mut ed,
        tmp.path(),
        r#"(define-command! "read-twice" ""
             (lambda (pane)
               (on-next-char!
                 (lambda (a)
                   (on-next-char! (lambda (b) (when b (call! "replace" pane b)))))))
             #:repeatable #t)
           (bind-key! 'insert "ctrl-x" "read-twice")"#,
    );
    ed.feed_key(key('i'));
    ed.feed_key(key_ctrl('x'));
    ed.settle();
    ed.feed_key(key('a'));
    ed.settle();
    ed.feed_key(key('z'));
    ed.settle();
    ed.feed_key(key_esc());
    assert_eq!(ed.doc().text().to_string(), "zello\nworld\n");

    select_marked(&mut ed, "zello\n-[w]>orld\n");
    ed.feed_key(key('.'));
    ed.settle();
    assert_eq!(ed.doc().text().to_string(), "zello\nzorld\n");
    assert!(
        !status(&ed).contains("reached during"),
        "got: {}",
        status(&ed)
    );
}

/// A repeatable command that opened a menu, then `key`, which dismisses
/// the menu and runs as a key of its own.
fn dismiss_a_recorded_menu_with(key: KeyEvent) -> Editor {
    let mut ed = editor_with_steel(
        "-[h]>ello\nworld\n",
        r#"(define-command! "menu" ""
             (lambda (pane)
               (show-menu! pane (list "one")
                 (lambda (i) (when i (call! "replace" pane #\o)))))
             #:repeatable #t)"#,
    );
    ed.execute_keymap_command("menu".into(), Some(1), false);
    ed.settle();
    assert!(
        ed.state.input.menu().is_some(),
        "setup: the menu must be open"
    );
    ed.feed_key(key);
    ed.settle();
    assert!(
        ed.state.input.menu().is_none(),
        "setup: the key closes the menu"
    );
    ed
}

/// The key that dismissed a recorded command's menu is a change of its
/// own: `.` repeats it, not the menu's command.
#[test]
fn dot_repeats_the_edit_key_that_dismissed_a_recorded_menu() {
    let mut ed = dismiss_a_recorded_menu_with(key('d'));
    assert_eq!(ed.doc().text().to_string(), "ello\nworld\n");

    select_marked(&mut ed, "ello\n-[w]>orld\n");
    ed.feed_key(key('.'));
    ed.settle();
    assert_eq!(ed.doc().text().to_string(), "ello\norld\n");
}

/// An Insert session entered by the key that dismissed a recorded
/// command's menu records its own change.
#[test]
fn dot_repeats_the_insert_entered_by_the_key_that_dismissed_a_recorded_menu() {
    let mut ed = dismiss_a_recorded_menu_with(key('i'));
    ed.feed_key(key('x'));
    ed.feed_key(key_esc());
    assert_eq!(ed.doc().text().to_string(), "xhello\nworld\n");

    select_marked(&mut ed, "xhello\n-[w]>orld\n");
    ed.feed_key(key('.'));
    ed.settle();
    assert_eq!(ed.doc().text().to_string(), "xhello\nxworld\n");
    assert_eq!(ed.state.mode(), Mode::Normal);
    assert_ne!(
        ed.state.last_action().map(|a| a.command.as_ref()),
        Some("menu")
    );
}

/// A native command that dismissed a recorded command's menu and then read
/// its operand records that operand as its own.
#[test]
fn dot_repeats_the_operand_key_that_dismissed_a_recorded_menu() {
    let mut ed = dismiss_a_recorded_menu_with(key('r'));
    assert!(reading_char(&ed), "setup: `r` must be reading its operand");
    ed.feed_key(key('q'));
    ed.settle();
    assert_eq!(ed.doc().text().to_string(), "qello\nworld\n");

    select_marked(&mut ed, "qello\n-[w]>orld\n");
    ed.feed_key(key('.'));
    ed.settle();
    assert_eq!(ed.doc().text().to_string(), "qello\nqorld\n");
}

/// An Insert-key binding whose `picker!` is refused (here: aimed at a pane
/// that isn't focused) leaves nothing open on the recording, so the next
/// change records normally.
#[test]
fn a_refused_picker_in_an_insert_binding_leaves_the_next_change_recordable() {
    use crate::editor::commands::open_pane_in_layout;
    let tmp = tempfile::tempdir().unwrap();
    let mut ed = editor_from("-[a]>bc def\n");
    let bid = ed.focused_buffer_id();
    open_pane_in_layout(
        &mut ed.state,
        &mut ed.view,
        bid,
        hume_engine::pipeline::Direction::Horizontal,
    );
    run(
        &mut ed,
        tmp.path(),
        r#"(define-command! "pick-other" ""
             (lambda (pane)
               (define other
                 (car (filter (lambda (p) (not (equal? p (focused-pane)))) (panes))))
               (picker! other (list (cons "alpha" "ALPHA")) (lambda (payload) (+ 1 0)))))
           (bind-key! 'insert "ctrl-y" "pick-other")"#,
    );

    ed.feed_key(key('i'));
    ed.feed_key(key_ctrl('y'));
    ed.settle();
    assert!(
        ed.state.input.picker().is_none(),
        "setup: the picker must have been refused"
    );
    ed.feed_key(key_esc());

    ed.feed_key(key('d'));
    assert_eq!(
        ed.state.last_action().map(|a| a.command.as_ref()),
        Some("delete"),
        "the delete after the refused picker must be the recorded change"
    );
}

/// A repeatable Steel command that opens an Insert session and then fails
/// still owns that session: its keys record into its own change, never into
/// the change recorded before it.
#[test]
fn a_failed_steel_body_that_opened_insert_keeps_its_session_keys() {
    let mut ed = editor_with_steel(
        "-[f]>oo bar\n",
        r#"(define-command! "ins-boom" ""
             (lambda (pane) (call! "insert-before" pane) (error "boom"))
             #:repeatable #t)"#,
    );
    ed.feed_key(key('d'));
    assert_eq!(
        ed.state.last_action().map(|a| a.command.as_ref()),
        Some("delete")
    );

    ed.execute_keymap_command("ins-boom".into(), Some(1), false);
    assert_eq!(
        ed.state.mode(),
        Mode::Insert,
        "setup: the body entered Insert"
    );
    ed.feed_key(key('x'));
    ed.feed_key(key_esc());

    let action = ed.state.last_action().expect("a change must be recorded");
    assert_eq!(action.command.as_ref(), "ins-boom");
    assert!(
        action.transcript.first().is_some_and(|i| is_key(i, 'x')),
        "got {:?}",
        action.transcript
    );
}

// ── Top-level command: picker ───────────────────────────────────────────────

/// Defines the repeatable `pk` from `body` (with `pane` in scope), runs it
/// on line 1 of `hello\nworld`, feeds `keys` one at a time, then moves to
/// line 2.
fn record_pk(tmp: &std::path::Path, body: &str, keys: &[KeyEvent]) -> Editor {
    let mut ed = editor_from("-[h]>ello\nworld\n");
    run(
        &mut ed,
        tmp,
        &format!(r#"(define-command! "pk" "" (lambda (pane) {body}) #:repeatable #t)"#),
    );
    ed.execute_keymap_command("pk".into(), Some(1), false);
    ed.settle();
    assert!(ed.state.input.picker().is_some(), "sanity: picker open");
    for &k in keys {
        ed.feed_key(k);
        ed.settle();
    }
    assert!(ed.state.input.picker().is_none(), "sanity: picker closed");
    ed.feed_key(key('j'));
    ed.feed_key(key('g'));
    ed.feed_key(key('h'));
    ed
}

/// `on_select` for the picker tests: replaces with the payload's first
/// character and wraps it in parentheses, so a repeat of only its last
/// `call!` is visible.
const PK_SELECT: &str = r#"(lambda (p)
                 (when p (call! "replace" pane (string-ref p 0)) (call! "surround-add" pane #\()))"#;

/// `.` repeats the item the user picked, without opening the picker: the
/// pick and every edit its `on_select` made belong to the command's change.
#[test]
fn dot_repeats_a_commands_picker_selection() {
    let tmp = tempfile::tempdir().unwrap();
    let mut ed = record_pk(
        tmp.path(),
        &format!(r#"(picker! pane (list (cons "a" "A") (cons "b" "B")) {PK_SELECT})"#),
        &[key_down(), key_enter()],
    );
    assert_eq!(ed.doc().text().to_string(), "(B)ello\nworld\n");
    ed.feed_key(key('.'));
    ed.settle();
    assert!(
        ed.state.input.picker().is_none(),
        "`.` must not open a picker"
    );
    assert_eq!(ed.doc().text().to_string(), "(B)ello\n(B)orld\n");
    assert!(
        !status(&ed).contains("reached during"),
        "got: {}",
        status(&ed)
    );
}

/// A pick made through an `#:actions` key is repeated through that action.
#[test]
fn dot_repeats_a_commands_picker_action() {
    let tmp = tempfile::tempdir().unwrap();
    let mut ed = record_pk(
        tmp.path(),
        &format!(
            r#"(picker! pane (list (cons "a" "A")) {PK_SELECT}
                 #:actions (list (cons "ctrl-t"
                   (lambda (p) (call! "replace" pane (string-ref p 0)) (call! "surround-add" pane #\[)))))"#
        ),
        &[key_ctrl('t')],
    );
    assert_eq!(ed.doc().text().to_string(), "[A]ello\nworld\n");
    ed.feed_key(key('.'));
    ed.settle();
    assert_eq!(ed.doc().text().to_string(), "[A]ello\n[A]orld\n");
}

/// A dismissed picker is repeated as a dismissal: `on_select` gets `#f`.
#[test]
fn dot_repeats_a_commands_dismissed_picker() {
    let tmp = tempfile::tempdir().unwrap();
    let mut ed = record_pk(
        tmp.path(),
        r#"(picker! pane (list (cons "a" "A"))
             (lambda (p)
               (unless p (call! "replace" pane #\z) (call! "surround-add" pane #\())))"#,
        &[key_esc()],
    );
    assert_eq!(ed.doc().text().to_string(), "(z)ello\nworld\n");
    ed.feed_key(key('.'));
    ed.settle();
    assert_eq!(ed.doc().text().to_string(), "(z)ello\n(z)orld\n");
}

/// A live picker's pick is repeated without feeding it any item.
#[test]
fn dot_repeats_a_commands_live_picker_selection() {
    let tmp = tempfile::tempdir().unwrap();
    let mut ed = record_pk(
        tmp.path(),
        &format!(
            r#"(let ((tok (live-picker! pane {PK_SELECT} #:source (picker-command-source (lambda (q) #f)))))
                 (picker-push! tok (list (cons "b" "B"))))"#
        ),
        &[key_enter()],
    );
    assert_eq!(ed.doc().text().to_string(), "(B)ello\nworld\n");
    ed.feed_key(key('.'));
    ed.settle();
    assert!(
        ed.state.input.picker().is_none(),
        "`.` must not open a picker"
    );
    assert_eq!(ed.doc().text().to_string(), "(B)ello\n(B)orld\n");
}

/// A re-run command that opens a picker where the change recorded a
/// character read took a different branch: `.` refuses rather than open it.
#[test]
fn dot_refuses_a_picker_where_a_read_was_recorded() {
    let tmp = tempfile::tempdir().unwrap();
    let mut ed = editor_from("-[h]>ello\nworld\n");
    run(
        &mut ed,
        tmp.path(),
        r#"(define runs 0)
           (define-command! "br" ""
             (lambda (pane)
               (set! runs (+ runs 1))
               (if (= runs 1)
                   (on-next-char! (lambda (c) (when c (call! "replace" pane c))))
                   (picker! pane (list (cons "a" "A")) (lambda (p) (+ 1 0)))))
             #:repeatable #t)"#,
    );
    ed.execute_keymap_command("br".into(), Some(1), false);
    ed.settle();
    ed.feed_key(key('z'));
    ed.settle();
    assert_eq!(ed.doc().text().to_string(), "zello\nworld\n");
    ed.feed_key(key('j'));
    ed.feed_key(key('.'));
    ed.settle();
    assert!(
        ed.state.input.picker().is_none(),
        "`.` must not open a picker"
    );
    assert!(
        status(&ed).contains("reached during"),
        "got: {}",
        status(&ed)
    );
}

/// A reload keeps a change holding a `picker!` pick: `.` finds the picked
/// label among the items the reloaded command builds.
#[test]
fn reload_keeps_a_label_pick() {
    let tmp = tempfile::tempdir().unwrap();
    let body = format!(r#"(picker! pane (list (cons "a" "A")) {PK_SELECT})"#);
    let mut ed = record_pk(tmp.path(), &body, &[key_enter()]);
    assert_eq!(ed.doc().text().to_string(), "(A)ello\nworld\n");
    ed.reset_config_state();
    run(
        &mut ed,
        tmp.path(),
        &format!(r#"(define-command! "pk" "" (lambda (pane) {body}) #:repeatable #t)"#),
    );
    ed.feed_key(key('.'));
    ed.settle();
    assert_eq!(ed.doc().text().to_string(), "(A)ello\n(A)orld\n");
}

/// A `live-picker!` pick is replayed from its payload, which a reload
/// forgets: `.` then refuses rather than guess.
#[test]
fn reload_forgets_a_live_picks_payload() {
    let tmp = tempfile::tempdir().unwrap();
    let body = format!(
        r#"(let ((tok (live-picker! pane {PK_SELECT} #:source (picker-command-source (lambda (q) #f)))))
             (picker-push! tok (list (cons "b" "B"))))"#
    );
    let mut ed = record_pk(tmp.path(), &body, &[key_enter()]);
    assert_eq!(ed.doc().text().to_string(), "(B)ello\nworld\n");
    ed.reset_config_state();
    run(
        &mut ed,
        tmp.path(),
        &format!(r#"(define-command! "pk" "" (lambda (pane) {body}) #:repeatable #t)"#),
    );
    ed.feed_key(key('.'));
    ed.settle();
    assert_eq!(ed.doc().text().to_string(), "(B)ello\nworld\n");
    assert!(
        status(&ed).contains("a config reload forgot the picked value"),
        "got: {}",
        status(&ed)
    );
}

/// A `picker!` that gets its items after it opens is replayed from the
/// payload its pick got: the re-run picker holds none of them yet when `.`
/// answers it.
#[test]
fn dot_repeats_a_pick_from_items_pushed_after_the_picker_opened() {
    let tmp = tempfile::tempdir().unwrap();
    let mut ed = record_pk(
        tmp.path(),
        &format!(
            r#"(let ((tok (picker! pane '() {PK_SELECT})))
                 (picker-push! tok (list (cons "b" "B"))))"#
        ),
        &[key_enter()],
    );
    assert_eq!(ed.doc().text().to_string(), "(B)ello\nworld\n");
    ed.feed_key(key('.'));
    ed.settle();
    assert!(
        ed.state.input.picker().is_none(),
        "`.` must not open a picker"
    );
    assert_eq!(ed.doc().text().to_string(), "(B)ello\n(B)orld\n");
}

/// A re-run `picker!` with no item of the picked text: `.` refuses and
/// names the item it looked for.
#[test]
fn dot_refuses_a_pick_whose_text_is_gone() {
    let tmp = tempfile::tempdir().unwrap();
    let mut ed = record_pk(
        tmp.path(),
        &format!(
            r#"(picker! pane
                 (if (= (buffer-cursor-line pane) 0) (list (cons "b" "B")) (list (cons "c" "C")))
                 {PK_SELECT})"#
        ),
        &[key_enter()],
    );
    assert_eq!(ed.doc().text().to_string(), "(B)ello\nworld\n");
    ed.feed_key(key('.'));
    ed.settle();
    assert_eq!(ed.doc().text().to_string(), "(B)ello\nworld\n");
    assert!(
        status(&ed).contains(r#"no item "b""#),
        "got: {}",
        status(&ed)
    );
}

/// A pick made through an `#:actions` key the re-run picker does not bind:
/// `.` refuses and says so.
#[test]
fn dot_refuses_a_pick_whose_action_key_is_unbound() {
    let tmp = tempfile::tempdir().unwrap();
    let mut ed = record_pk(
        tmp.path(),
        &format!(
            r#"(picker! pane (list (cons "a" "A")) {PK_SELECT}
                 #:actions (if (= (buffer-cursor-line pane) 0)
                               (list (cons "ctrl-t" {PK_SELECT}))
                               '()))"#
        ),
        &[key_ctrl('t')],
    );
    assert_eq!(ed.doc().text().to_string(), "(A)ello\nworld\n");
    ed.feed_key(key('.'));
    ed.settle();
    assert_eq!(ed.doc().text().to_string(), "(A)ello\nworld\n");
    assert!(
        status(&ed).contains("not one of this picker's actions"),
        "got: {}",
        status(&ed)
    );
}

/// `.` matches a `picker!` pick by its label among the items the re-run
/// builds, so a payload that depends on where the command runs is the one
/// for the new spot.
#[test]
fn dot_matches_a_pick_by_label_at_the_new_spot() {
    let tmp = tempfile::tempdir().unwrap();
    let mut ed = record_pk(
        tmp.path(),
        &format!(
            r#"(picker! pane
                 (list (cons "same" (if (= (buffer-cursor-line pane) 0) "A" "B")))
                 {PK_SELECT})"#
        ),
        &[key_enter()],
    );
    assert_eq!(ed.doc().text().to_string(), "(A)ello\nworld\n");
    ed.feed_key(key('.'));
    ed.settle();
    assert_eq!(ed.doc().text().to_string(), "(A)ello\n(B)orld\n");
}

// ── Insert-key binding: reads and picks ─────────────────────────────────────

/// An Insert-key binding that opens a picker is re-run by `.`, its picker
/// answered with the recorded pick: what the pick's callback does is decided
/// again at the new cursor. Here it deletes the word behind the cursor,
/// which is longer on line 2 than on line 1.
#[test]
fn dot_reruns_an_insert_binding_answering_its_picker() {
    let tmp = tempfile::tempdir().unwrap();
    let mut ed = editor_from("abc -[d]>ef\nlonger xyz\n");
    run(
        &mut ed,
        tmp.path(),
        r#"(define-command! "pick-word" ""
             (lambda (pane)
               (picker! pane (list (cons "alpha" "ALPHA"))
                 (lambda (payload)
                   (when payload (call! "delete-word-backward" pane)))))
             #:repeatable #t)
           (bind-key! 'insert "ctrl-y" "pick-word")"#,
    );
    ed.feed_key(key('i'));
    ed.feed_key(key_ctrl('y'));
    ed.feed_key(key_enter());
    ed.settle();
    ed.feed_key(key_esc());
    assert_eq!(ed.doc().text().to_string(), "def\nlonger xyz\n");

    select_marked(&mut ed, "def\nlonger -[x]>yz\n");
    ed.feed_key(key('.'));
    ed.settle();
    assert!(
        ed.state.input.picker().is_none(),
        "`.` must not open a picker"
    );
    assert_eq!(ed.doc().text().to_string(), "def\nxyz\n");
}

/// An Insert-key binding that reads a character is re-run by `.` with the
/// recorded character.
#[test]
fn dot_reruns_an_insert_binding_answering_its_read() {
    let tmp = tempfile::tempdir().unwrap();
    let mut ed = editor_from("abc -[d]>ef\nlonger xyz\n");
    run(
        &mut ed,
        tmp.path(),
        r#"(define-command! "read-word" ""
             (lambda (pane)
               (on-next-char!
                 (lambda (c) (when (eqv? c #\w) (call! "delete-word-backward" pane)))))
             #:repeatable #t)
           (bind-key! 'insert "ctrl-y" "read-word")"#,
    );
    ed.feed_key(key('i'));
    ed.feed_key(key_ctrl('y'));
    ed.feed_key(key('w'));
    ed.settle();
    ed.feed_key(key_esc());
    assert_eq!(ed.doc().text().to_string(), "def\nlonger xyz\n");

    select_marked(&mut ed, "def\nlonger -[x]>yz\n");
    ed.feed_key(key('.'));
    ed.settle();
    assert!(!reading_char(&ed), "`.` must not wait for a key");
    assert_eq!(ed.doc().text().to_string(), "def\nxyz\n");
}

/// Keys `i a <ctrl-y> <answer> b <Esc>` on "hello": `ctrl-y` runs a
/// non-repeatable command whose `open` input, once answered, deletes the word
/// behind the cursor.
fn record_insert_with_unrepeatable_input(open: &str, answer: KeyEvent) -> Editor {
    let tmp = tempfile::tempdir().unwrap();
    let mut ed = editor_from("-[h]>ello\nworld\n");
    run(
        &mut ed,
        tmp.path(),
        &format!(
            r#"(define-command! "ask" "" (lambda (pane) {open}))
               (bind-key! 'insert "ctrl-y" "ask")"#
        ),
    );
    ed.feed_key(key('i'));
    ed.feed_key(key('a'));
    ed.feed_key(key_ctrl('y'));
    ed.feed_key(answer);
    ed.settle();
    ed.feed_key(key('b'));
    ed.feed_key(key_esc());
    assert_eq!(ed.doc().text().to_string(), "bhello\nworld\n");
    select_marked(&mut ed, "bhello\n-[w]>orld\n");
    ed
}

/// A picker a non-repeatable Insert binding opens ends the change: `.`
/// repeats the keys typed before that binding, not the binding, its pick,
/// or the keys after it.
#[test]
fn a_non_repeatable_insert_bindings_picker_ends_the_change() {
    let mut ed = record_insert_with_unrepeatable_input(
        r#"(picker! pane (list (cons "w" "W"))
             (lambda (p) (when p (call! "delete-word-backward" pane))))"#,
        key_enter(),
    );
    let transcript = &ed.state.last_action().expect("`i` recorded").transcript;
    assert!(
        matches!(transcript.as_slice(), [i] if is_key(i, 'a')),
        "got {transcript:?}"
    );
    ed.feed_key(key('.'));
    ed.settle();
    assert!(
        ed.state.input.picker().is_none(),
        "`.` must not open a picker"
    );
    assert_eq!(ed.doc().text().to_string(), "bhello\naworld\n");
}

/// A character read a non-repeatable Insert binding opens ends the change,
/// the same as a picker.
#[test]
fn a_non_repeatable_insert_bindings_read_ends_the_change() {
    let mut ed = record_insert_with_unrepeatable_input(
        r#"(on-next-char!
             (lambda (c) (when (eqv? c #\w) (call! "delete-word-backward" pane))))"#,
        key('w'),
    );
    let transcript = &ed.state.last_action().expect("`i` recorded").transcript;
    assert!(
        matches!(transcript.as_slice(), [i] if is_key(i, 'a')),
        "got {transcript:?}"
    );
    ed.feed_key(key('.'));
    ed.settle();
    assert!(!reading_char(&ed), "`.` must not wait for a key");
    assert_eq!(ed.doc().text().to_string(), "bhello\naworld\n");
}

/// An answer whose binding entry a completion accept removed is dropped: it
/// never lands in the entry of a binding recorded after it. No key sequence
/// reaches this, since a waiting input takes every key, so it drives the
/// recording directly.
#[test]
fn a_stale_answer_never_lands_in_a_later_bindings_entry() {
    use crate::editor::dot_chain::DotChain;
    use crate::editor::edit_session::DotCapture;
    use crate::editor::replay::{
        AnswerTicket, InputOpener, RecordingScope, finalize_dot_capture, with_recording_scope,
    };

    fn open_input(ed: &mut Editor, scope: RecordingScope) -> AnswerTicket {
        with_recording_scope(&mut ed.state, Some(scope), |state| {
            state
                .waiting_input("ask", InputOpener::Code)
                .expect("not during `.`")
                .take_answer()
        })
        .expect("a repeatable binding's input records")
    }

    let mut ed = editor_from("-[h]>ello\n");
    ed.feed_key(key('i'));

    let (a_entry, a) = ed
        .state
        .record_binding("a".into(), true)
        .expect("`i` is recording");
    let stale = open_input(&mut ed, a);

    let text = ed.doc().text().clone();
    let state = &mut ed.state;
    let session = state.input.insert_session().expect("`i` opened a session");
    finalize_dot_capture(
        &mut state.recording,
        session,
        DotCapture {
            chain: DotChain::new(test_fixtures::testing::at(&text, 0), &text),
            interactive: true,
            binding: Some(a_entry),
        },
    )
    .expect("nothing edited the buffer under the capture");

    let (_, b) = ed
        .state
        .record_binding("b".into(), true)
        .expect("`i` is still recording");
    let _b_ticket = open_input(&mut ed, b);

    ed.state.record_answer(&stale, Answer::Char(Some('z')));

    let transcript = &ed.state.last_action().expect("`i` recorded").transcript;
    assert!(
        matches!(transcript.as_slice(), [Input::Binding { name, answers, .. }]
            if name == "b" && matches!(answers.as_slice(), [Answer::Unanswered])),
        "got {transcript:?}"
    );
}

/// A reload while an Insert-key binding's picker is open releases that
/// change: a later repeatable command starts its own.
#[test]
fn reload_with_a_bindings_picker_open_releases_the_change() {
    let tmp = tempfile::tempdir().unwrap();
    let mut ed = editor_from("-[h]>ello\nworld\n");
    run(
        &mut ed,
        tmp.path(),
        r#"(define-command! "pick" ""
             (lambda (pane) (picker! pane (list (cons "a" "A")) (lambda (p) (+ 1 0)))))
           (bind-key! 'insert "ctrl-y" "pick")"#,
    );
    ed.feed_key(key('i'));
    ed.feed_key(key_ctrl('y'));
    assert!(ed.state.input.picker().is_some(), "sanity: picker open");
    ed.reset_config_state();
    ed.feed_key(key('d'));
    ed.feed_key(key('j'));
    ed.feed_key(key('g'));
    ed.feed_key(key('h'));
    ed.feed_key(key('.'));
    assert_eq!(ed.doc().text().to_string(), "ello\norld\n");
}
