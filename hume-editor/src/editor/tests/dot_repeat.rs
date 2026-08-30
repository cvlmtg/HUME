use super::*;
use crate::editor::dispatch::ArgSource;
use pretty_assertions::assert_eq;

// ── Dot-repeat tests ──────────────────────────────────────────────────────────

/// `d` deletes the selection. Moving then pressing `.` should delete the next selection.
#[test]
fn dot_repeats_delete() {
    // Cursor starts at 'f'. `foo` is already selected; `d` deletes it.
    // Then from the space at pos 0, `w` selects "bar" — "bar" is the first
    // (and only) word on its line, so its leading space is indentation and
    // is never absorbed; there's no trailing space either (EOL follows), so
    // the default around-word span is bare. `.` deletes just "bar".
    let mut ed = editor_from("-[foo]> bar\n");
    ed.feed_key(key('d')); // delete "foo" → " bar\n", cursor at 0 (space)
    assert_eq!(ed.doc().text().to_string(), " bar\n");

    ed.feed_key(key('w')); // from space, select "bar" (bare — indent kept)
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
    // around-word) — the replayed change removes that leading space too.
    ed.feed_key(key('w')); // select " bar"
    ed.feed_key(key('.')); // repeat: delete " bar", insert "hi"

    assert_eq!(ed.doc().text().to_string(), "hihi\n");
}

/// A repeatable command refused by `refuse_if_read_only` must not clobber
/// `last_repeatable_action` — it changed nothing, so there is nothing new to
/// repeat.
///
/// `step_stamp_repeatable` (`commands/pipeline.rs`) fires unconditionally
/// whenever `meta.repeatable`, with no "did the body actually do anything"
/// gate — unlike `step_update_recipe`, which the `018a27c8` commit gated on
/// `selection_changed` for exactly this reason. `cmd_delete` refuses via
/// `refuse_if_read_only` and returns `Ok(())` without touching the buffer,
/// but the pipeline stamps `last_repeatable_action = "delete"` anyway,
/// silently discarding the user's earlier `change`.
///
/// Fail oracle: without a refusal gate on the stamp, the final assert sees
/// `command == "delete"` instead of `"change"`.
#[test]
fn read_only_refusal_does_not_clobber_dot_repeat() {
    let mut ed = editor_from("-[foo]> bar\n");

    // Stage a real repeatable action on the writable buffer.
    ed.feed_key(key('c'));
    ed.feed_key(key('h'));
    ed.feed_key(key('i'));
    ed.feed_key(key_esc());
    assert_eq!(
        ed.state
            .last_repeatable_action
            .as_ref()
            .unwrap()
            .command
            .as_ref(),
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

    // Stage a selection, then attempt a repeatable edit — refused, no text
    // changes, but the pipeline still runs the AFTER stage unconditionally.
    ed.feed_key(key('x')); // select-line (motion-only; not blocked on read-only)
    ed.feed_key(key('d')); // delete — refused by refuse_if_read_only

    assert_eq!(
        ed.state
            .last_repeatable_action
            .as_ref()
            .unwrap()
            .command
            .as_ref(),
        "change",
        "a refused delete on a read-only buffer must not overwrite the prior change action"
    );
}

/// A replayed `c` also ends with the replacement selected — the anchor
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

/// Pinning for `mii` is unconditional (not gated on `select-changed-text`),
/// so a replayed `i` (not just `c`) must re-pin correctly too — `mii` after
/// `.` must select the *replayed* insertion, not the original one.
#[test]
fn mii_after_dot_repeat_selects_replayed_insertion() {
    let mut ed = editor_from("-[x]>\n");

    ed.feed_key(key('i'));
    ed.feed_key(key('a'));
    ed.feed_key(key('b'));
    ed.feed_key(key_esc());
    assert_eq!(ed.doc().text().to_string(), "abx\n");

    ed.feed_key(key('w')); // select 'x'
    ed.feed_key(key('.')); // repeat insert "ab" before 'x'
    assert_eq!(ed.doc().text().to_string(), "ababx\n");

    ed.feed_key(key('m'));
    ed.feed_key(key('i'));
    ed.feed_key(key('i'));
    assert_eq!(state(&ed), "ab-[ab]>x\n");
}

/// `i` + typed text + Esc inserts at the selection start. `.` should replay that insert.
#[test]
fn dot_repeats_insert_before() {
    let mut ed = editor_from("-[x]>\n");

    ed.feed_key(key('i')); // insert-at-selection-start, cursor collapses to start
    ed.feed_key(key('a'));
    ed.feed_key(key('b'));
    ed.feed_key(key_esc()); // back to Normal; buffer is "abx"

    assert_eq!(ed.doc().text().to_string(), "abx\n");

    // Move to 'x' and repeat.
    ed.feed_key(key('w')); // select 'x'
    ed.feed_key(key('.')); // repeat insert "ab" before 'x'

    assert_eq!(ed.doc().text().to_string(), "ababx\n");
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
    // onto the content first — `.` then enters insert there and dedents.
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

    ed.feed_key(key('r')); // wait-char
    ed.feed_key(key('x')); // replace "ab" → "xx cd\n"

    assert_eq!(ed.doc().text().to_string(), "xx cd\n");

    // `w` from the "xx" selection (head at pos 1) selects "cd" — no trailing
    // space (EOL follows), but a leading one, so the default around-word
    // span picks up " cd" instead. `r` replaces every grapheme in the
    // selection, including that space.
    ed.feed_key(key('w'));
    ed.feed_key(key('.')); // repeat replace with 'x' → "xxxxx\n"

    assert_eq!(ed.doc().text().to_string(), "xxxxx\n");
}

/// When `.` is given an explicit count, the stored `last_repeatable_action.count`
/// must not be corrupted — the explicit count is used for the replay but is NOT
/// written back into the stored action.
///
/// The original count must survive so that a subsequent plain `.` can still
/// reproduce the original repetition count.
///
/// Fail oracle: if `replay_dot` wrote `PendingRepeat.count` back into
/// `last_repeatable_action.count`, the final `assert_eq!` would fail.
#[test]
fn explicit_count_on_dot_does_not_corrupt_stored_count() {
    let mut ed = editor_from("-[foo]> bar\n");

    ed.feed_key(key('d')); // delete "foo"; stored count=1
    assert_eq!(
        ed.state.last_repeatable_action.as_ref().unwrap().count,
        1,
        "setup: stored count must be 1"
    );

    ed.feed_key(key('w')); // move to "bar"
    // Press `3.` — explicit count 3 is used for the replay but must NOT be
    // written back into last_repeatable_action (drain restores the original action).
    ed.feed_key(key('3'));
    ed.feed_key(key('.'));

    // Replay must have happened (bar deleted).
    assert!(
        !ed.doc().text().to_string().contains("bar"),
        "bar must be deleted by the repeated command"
    );
    // Stored count must still be 1 — the explicit-count replay must not corrupt it.
    assert_eq!(
        ed.state.last_repeatable_action.as_ref().unwrap().count,
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
    // around-word) — the replayed change removes that leading space too.
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
/// repeatable too — `.repeatable()` on its registry entry is a separate
/// declaration from `smart-paste-after`'s, so a repeat must be verified
/// independently rather than assumed from the smart variant.
#[test]
fn dot_repeats_plain_paste_after() {
    let mut ed = editor_from("-[ab]>cd\n");

    ed.state.kill_ring.push(vec!["ab".to_string()]);
    ed.execute_keymap_command("paste-after".into(), Some(1), false, ArgSource::Keymap);
    // Plain paste never collapses on repeat — move off the pasted text so
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
/// Independent oracle: three-line buffer, first `x d` leaves two lines, second
/// `x d` leaves one line — derived by hand, not from the implementation.
///
/// Regression: if the recipe replay is absent, `.` would delete only the char
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
        ed.state
            .last_repeatable_action
            .as_ref()
            .unwrap()
            .selection_recipe
            .len(),
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

/// `x Ctrl+x d` selects two lines (one establish + one extend) and deletes them.
/// `.` replays the full two-step recipe, deleting the next two lines.
///
/// Independent oracle: four-line buffer: first `x Ctrl+x d` leaves two lines;
/// second replay deletes both → one structural line remains.
#[test]
fn dot_repeats_extend_select_delete() {
    // Four lines; cursor on 'a'.
    let mut ed = editor_from("-[a]>aa\nbbb\nccc\nddd\n");

    ed.feed_key(key('x')); // select "aaa\n"
    ed.feed_key(key_ctrl('x')); // extend to "aaa\nbbb\n"
    ed.feed_key(key('d')); // delete → "ccc\nddd\n", cursor on 'c'
    assert_eq!(ed.doc().text().to_string(), "ccc\nddd\n");

    // Recipe must be [select-line F, select-line T] (establish + one extend).
    let recipe = &ed
        .state
        .last_repeatable_action
        .as_ref()
        .unwrap()
        .selection_recipe;
    assert_eq!(recipe.len(), 2, "recipe must have 2 steps");
    assert!(!recipe[0].extend, "first step must be Move (establish)");
    assert!(recipe[1].extend, "second step must be Extend");

    // `.` replays: x (select "ccc\n") + Ctrl+x (extend to "ccc\nddd\n") + d.
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
/// Independent oracle: four-line buffer; `j x d` deletes line 1 ("bbb\n"),
/// leaving "aaa\nccc\nddd\n". `.` must re-select line 1 ("ccc\n") and delete
/// it, giving "aaa\nddd\n". If `j` were in the recipe, `.` would instead move
/// down from "ccc" to "ddd" first, deleting "ddd\n" and leaving "aaa\nccc\n".
///
/// Fail oracle: if the `is_collapsed()` guard were removed, `j` (a Motion in
/// Move mode) would always enter the recipe as a first step, making `.` move
/// down before deleting, producing "aaa\nccc\n" instead of "aaa\nddd\n".
#[test]
fn dot_repeat_navigation_not_in_recipe() {
    // Four lines so that the last-line structural-'\n' protection does not
    // interfere with any of the delete operations below.
    let mut ed = editor_from("-[a]>aa\nbbb\nccc\nddd\n");

    ed.feed_key(key('j')); // move-down: collapsed cursor on 'b', recipe cleared
    ed.feed_key(key('x')); // select-line "bbb\n": recipe = [select-line F]
    ed.feed_key(key('d')); // delete "bbb\n" → "aaa\nccc\nddd\n", cursor on 'c'
    assert_eq!(ed.doc().text().to_string(), "aaa\nccc\nddd\n");

    // Recipe must have exactly one step (x only — j must have been cleared).
    assert_eq!(
        ed.state
            .last_repeatable_action
            .as_ref()
            .unwrap()
            .selection_recipe
            .len(),
        1,
        "navigation (j) must not appear in the selection recipe"
    );

    // `.` replays x (selects current line "ccc\n") + d (deletes it).
    // If j were in the recipe, `.` would move down to "ddd\n" first and delete
    // that, leaving "aaa\nccc\n" — the oracle below distinguishes the two cases.
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
        ed.state
            .last_repeatable_action
            .as_ref()
            .unwrap()
            .selection_recipe
            .len(),
        0,
        "empty recipe for plain single-char delete"
    );

    // `.` repeats just the delete (no line reselection).
    ed.feed_key(key('.'));
    // Oracle: second 'b' deleted → "aaa\nb\nccc\n"
    assert_eq!(
        ed.doc().text().to_string(),
        "aaa\nb\nccc\n",
        "`.` must delete the current 1-char selection, not reselect a whole line"
    );
}

/// `x c <text> Esc` records recipe=[select-line F] + insert_keys=[...]. `.` on
/// another line re-selects the full line, runs change, and retypes the text.
///
/// Independent oracle: three-line buffer; `x c z Esc` on line 0 deletes
/// "aaa\n" and inserts 'z' before the remaining content → "zbbb\nccc\n".
/// Then `.` on line 1 re-selects "ccc\n" via the recipe, runs change (deletes
/// "ccc", leaving the structural '\n'), inserts 'z' → "zbbb\nz\n".
///
/// Fail oracle: without the recipe, `.` would run change on the collapsed
/// 1-char cursor at 'b', deleting only 'b' and inserting 'z' → "z\nzbb\nccc\n".
#[test]
fn dot_repeats_change_reselects_line() {
    // Three lines; `c` after select-line removes the content but keeps the `\n`.
    let mut ed = editor_from("-[a]>aa\nbbb\nccc\n");

    ed.feed_key(key('x')); // select "aaa\n" — head on '\n'
    ed.feed_key(key('c')); // change: delete "aaa" (not '\n') → "\nbbb\nccc\n", cursor at 0
    ed.feed_key(key('z')); // type 'z' → "z\nbbb\nccc\n"
    ed.feed_key(key_esc()); // back to Normal, cursor on 'z'
    assert_eq!(ed.doc().text().to_string(), "z\nbbb\nccc\n");

    // Recipe must be [select-line]; insert_keys must be ['z'].
    {
        let action = ed.state.last_repeatable_action.as_ref().unwrap();
        assert_eq!(
            action.selection_recipe.len(),
            1,
            "recipe must have select-line step"
        );
        assert_eq!(
            action.insert_keys.len(),
            1,
            "insert_keys must capture typed chars"
        );
    }

    // Move to 'b' and press `.`.
    ed.feed_key(key('j')); // move-down to 'b' (line 1)
    ed.feed_key(key('.')); // re-select "bbb\n" via recipe, change, retype 'z'

    // Oracle: recipe re-selects "bbb\n", change deletes "bbb" (keeps '\n'),
    // inserts 'z' → "z\nz\nccc\n". Without recipe, only 'b' would be deleted → "z\nzbb\nccc\n".
    assert_eq!(
        ed.doc().text().to_string(),
        "z\nz\nccc\n",
        "`.` must re-select the full line, change it, and replay insert_keys"
    );
}

/// `undo` must clear the selection-recipe buffer so a stale `select-line` does
/// not leak into the next edit's recipe.
///
/// Fail oracle: drop the `SelectionTracking::Untracked` early-return clear in
/// `step_update_recipe` (`commands/pipeline.rs`) — `undo` is `Untracked`, so
/// `selection_recipe` would not be cleared and the assert would see len=1
/// instead of 0.
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
/// there — it establishes nothing on its own, so recording it as a reset
/// (`SelectionTracking::Establishes`) would replay `.` as "duplicate a bare
/// cursor," silently dropping the `x` that built the real extent (see
/// `dot_repeat_of_copy_selection_replays_the_whole_recipe` below for the
/// end-to-end failure this would cause). Registered with
/// `.composes_selection()` (`SelectionTracking::Composes`) precisely because
/// it transforms the staged extent instead of establishing one — see
/// `registry/defaults/selections.rs`.
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
/// content back onto the last content char — so a `select-line` head (which
/// sits ON that newline) does not reproduce the newline in the copy. That's
/// existing, correct `C` behavior, orthogonal to this test; a word selection
/// (all content chars, no newline in range) sidesteps it entirely.
///
/// Independent oracle: on `"foo\nbar\nfoo\nbar\n"`, `mm` selects "foo" (line
/// 0), `C` duplicates it onto "bar" (line 1) at the same columns, `d`
/// deletes both — each line's own newline survives, leaving
/// `"\n\nfoo\nbar\n"`. Move down two buffer lines to the second "foo\nbar\n"
/// pair; `.` must replay `mm` + `C` + `d` there too, leaving `"\n\n\n\n"`.
///
/// Fail oracle: if `copy-selection-on-next-line` reset the recipe instead of
/// composing (`SelectionTracking::Establishes` instead of `Composes`), `.`
/// would replay `[C, d]` from a bare cursor instead of `[mm, C, d]` — `C` on
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
    ed.feed_key(key('j')); // down to the second "foo" line — not in the recipe
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
    assert!(ed.state.last_repeatable_action.is_none());
    ed.feed_key(key('.'));
    assert_eq!(state(&ed), state_after_find);
}

/// A dot-repeated delete is itself a fresh capture: a bare `p` right after
/// `.` reads what `.` just deleted, not the clipboard.
///
/// This is a deliberate consequence of `PasteStamp` having no dot-repeat
/// special case at all — `replay_dot` runs the replayed edit through the
/// ordinary `commands::run_native_body` → `route_kill` → `capture_to_ring`
/// path, which writes the stamp at the replay's own
/// `BufferStore::edit_seq()` exactly as a live `d` would: nothing treats a
/// replayed delete differently from a typed one.
///
/// Fail oracle: make `route_kill`/`capture_to_ring` skip stamping when
/// called from `run_native_body` outside the dispatch pipeline (i.e. during
/// replay) — the ring head would then be stale by the time `p` reads it and
/// `p` would fall through to "CLIP" instead.
#[test]
fn dot_repeat_of_delete_leaves_ring_fresh_for_paste() {
    use hume_ops::register::CLIPBOARD_REGISTER;

    let mut ed = editor_from("-[foo]> bar\n");
    // Seed the clipboard with a sentinel value distinct from anything the
    // ring could hold, so a wrong clipboard read is unambiguous.
    ed.state
        .registers
        .write_text(CLIPBOARD_REGISTER, vec!["CLIP".to_string()]);

    ed.feed_key(key('d')); // delete "foo" → ring head = ["foo"]
    ed.feed_key(key('w')); // move to "bar"
    ed.feed_key(key('.')); // repeat; replay_dot deletes "bar" → ring head = ["bar"]

    ed.feed_key(key('p')); // bare p — must read the ring, not the clipboard
    let text = state(&ed);
    // "bar" is independently known from the scenario itself (the `.` replays
    // a delete on the word we navigated to with `w`) — not read back from
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
/// This is the dot-repeat drift bug: before the fix, `w` pushed an establish
/// step, so `.` re-ran `select-next-word` from the new cursor position and
/// advanced past the intended word, deleting the one after it instead.
///
/// Independent oracle: buffer "a foo bar baz\n". `w` selects "foo", `d` deletes
/// it → "a  bar baz\n". Then `w` selects "bar". `.` must delete "bar" (current
/// selection), leaving "a   baz\n". The buggy version would re-run
/// `select-next-word` from "bar", select "baz", and delete that instead.
///
/// Fail oracle: remove the `!ctx.extend` exclusion from `step_update_recipe`'s
/// `SelectionTracking::Extends` arm → `w` (a `Motion`, always `Extends`)
/// pushes an establish step → recipe is non-empty → `.` selects the NEXT word
/// ("baz") → buffer would contain "bar" but not "baz".
#[test]
fn dot_repeat_word_motion_acts_on_current_selection() {
    let mut ed = editor_from("-[a]>  foo bar baz\n");
    // This test is about the recipe/replay mechanism, not word-span shape —
    // pin bare-word selection so the buffer arithmetic in the doc comment
    // above holds regardless of word-selects-whitespace's default.
    ed.state.settings.word_selects_whitespace = false;

    ed.feed_key(key('w')); // select "foo" (Move mode — Extends, not Establishes)
    ed.feed_key(key('d')); // delete "foo" → "a   bar baz\n"

    // Recipe must be empty — Move-mode `w` must not create an establish step.
    assert_eq!(
        ed.state
            .last_repeatable_action
            .as_ref()
            .unwrap()
            .selection_recipe
            .len(),
        0,
        "Move-mode select-next-word must not push an establish step"
    );

    ed.feed_key(key('w')); // select "bar" (cursor now on 'b')
    ed.feed_key(key('.')); // replay: empty recipe → delete current selection ("bar")

    let text = ed.doc().text().to_string();
    assert!(
        !text.contains("bar"),
        "`.` must delete 'bar' (current selection), not advance past it — got: {text:?}"
    );
    assert!(
        text.contains("baz"),
        "`.` must not advance past 'bar' and delete 'baz' — got: {text:?}"
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
/// Fail oracle: if `ms(` is not recorded in the selection recipe (e.g.
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

/// `ma(` (select around `()`, including the delimiters) is characterization
/// coverage, not regression coverage: unlike `ms(` (`dot_repeat_of_match_surround_deletes_both_parens`
/// above), `ma(` on `(foo)` already leaves a non-collapsed selection, so the
/// pre-fix `!is_collapsed()` gate already recorded it — this pins that it
/// still replays correctly under the `!meta.is_motion` gate.
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

/// `mi(` (select inner `()`, excluding the delimiters) — same characterization
/// note as `dot_repeat_of_match_around_deletes_content` above: on `(foo)` it
/// already leaves a non-collapsed selection, so this is coverage for the new
/// `!meta.is_motion` gate rather than a regression test for it.
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

    assert_eq!(state(&ed), "-[foo]> bar baz -[foo]> bar baz\n");
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
/// already built, not reset it — it reduces whatever is staged to the
/// primary selection rather than establishing a fresh extent of its own.
///
/// Buffer "foo bar foo bar foo bar": `m/` selects all three "foo" matches,
/// `,` keeps only the first (primary), `d` deletes it. `.` must replay
/// `m/` + `,` + `d` — re-selecting all remaining "foo" matches, keeping the
/// (new) primary, and deleting it too.
///
/// Fail oracle: if `keep-primary-selection` reset the recipe instead of
/// composing, `.` would replay `[,, d]` from whatever selection happens to
/// remain after the first delete — `,` on a single selection is a no-op, so
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

/// `S` (`split-selection-on-newlines`) after `x` + `Ctrl+x` must APPEND onto
/// the recipe those steps already built, not reset it — it splits whatever
/// multi-line extent is staged into per-line pieces rather than establishing
/// a fresh extent of its own.
///
/// Buffer "aaa\nbbb\nccc\nddd\n": `x` selects "aaa\n", `Ctrl+x` extends to
/// "aaa\nbbb\n", `S` splits into two per-line selections ("aaa" and "bbb\n").
/// `d` deletes both: the first piece excludes its line's newline (so "aaa"
/// → "", newline kept), the second piece runs through its own trailing
/// newline (so "bbb\n" is removed whole) — net effect, one line disappears:
/// "\nccc\nddd\n". `.` must replay `x` + `Ctrl+x` + `S` + `d` on the next two
/// lines too, removing another: "\n\n".
///
/// Fail oracle: if `split-selection-on-newlines` reset the recipe instead of
/// composing, `.` would replay `[S, d]` from whatever selection happens to
/// remain — `S` on a single-line (already-collapsed) selection is a no-op,
/// so `d` would delete only that leftover selection.
#[test]
fn dot_repeat_of_split_selection_on_newlines_composes_onto_the_prior_recipe() {
    let mut ed = editor_from("-[a]>aa\nbbb\nccc\nddd\n");

    ed.feed_key(key('x')); // select-line: "aaa\n"
    ed.feed_key(key_ctrl('x')); // extend: "aaa\nbbb\n"
    assert_eq!(
        ed.state.selection_recipe.len(),
        2,
        "setup: x + Ctrl+x must push two recipe steps"
    );

    ed.feed_key(key('S')); // split into "aaa" + "bbb\n"
    assert_eq!(
        ed.state.selection_recipe.len(),
        3,
        "S must compose onto the x/Ctrl+x steps, not reset them"
    );
    assert_eq!(
        ed.state.selection_recipe[2].command.as_ref(),
        "split-selection-on-newlines"
    );

    ed.feed_key(key('d'));
    assert_eq!(ed.doc().text().to_string(), "\nccc\nddd\n");

    ed.feed_key(key('j')); // move onto "ccc"
    ed.feed_key(key('.')); // replay x + Ctrl+x + S + d
    assert_eq!(
        ed.doc().text().to_string(),
        "\n\n",
        "`.` must replay x + Ctrl+x + S + d and clear 'ccc'/'ddd'"
    );
}

/// `mii` (`select-last-insertion`) is an `EditorCmd` that does not opt into
/// the dot-repeat recipe via `.establishes_selection()` (unlike
/// `select-all-matches` — see `editor_cmds.rs`), so unlike `ms(`/`mm` it can never push itself onto
/// `state.selection_recipe`, and (being non-repeatable) it can never overwrite
/// `last_repeatable_action` either. Running it between an insert and `.` must
/// therefore be inert: `.` still replays the original insert verbatim, not
/// some "reselect + act" recipe.
///
/// Independent oracle: `i "ab" Esc` on "x" gives "abx"; `mii` re-selects "ab"
/// (the just-typed span, unrelated to where `.` will act); replaying the
/// insert places "ab" again at the selection's start → "ab" + "ab" + "x" =
/// "ababx". The buffer only grows — no delete ever happens — which is the
/// signal that `.` ran the insert and nothing resembling "mii + d".
///
/// Fail oracle: if `mii` corrupted `last_repeatable_action` (e.g. by being
/// misclassified as repeatable) or leaked into its frozen recipe, `.` would
/// replay some other action (or reselect "ab" again as an operator target)
/// instead of inserting — the buffer would not end up "ababx".
#[test]
fn dot_repeat_after_select_last_insertion_still_repeats_the_insert() {
    let mut ed = editor_from("-[x]>\n");

    type_text(&mut ed, "ab"); // insert-before, cursor collapses to start; buffer is "abx"
    assert_eq!(ed.doc().text().to_string(), "abx\n");

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
/// `select-word` (`mm`, a `Selection` → `Establishes`) — same in-place
/// establishing semantics, `EditorCmd` only because it needs a `RowMap`.
/// Unbound by default, so this drives it directly via `execute_keymap_command`
/// rather than a keypress. It must be replayable: `.` re-runs the selection
/// step before repeating the edit, not just re-delete whatever selection
/// happens to remain at the new cursor.
///
/// Buffer "foo bar\n": select-word-nearest-on-line selects "foo " (word plus
/// trailing whitespace — the `word-selects-whitespace` default), `d` deletes
/// it, leaving "bar\n" with the cursor collapsed to a 1-char selection on
/// 'b'. `.` must replay select-word-nearest-on-line + d from there too,
/// re-selecting the whole word "bar" rather than deleting just that 1-char
/// cursor.
///
/// Fail oracle: `Untracked` (its state before this fix) means the recipe
/// stays empty, so `.` would delete only the 1-char cursor left behind,
/// leaving "ar\n" instead of "\n".
#[test]
fn dot_repeat_of_select_word_nearest_on_line_deletes_the_word() {
    let mut ed = editor_from("-[f]>oo bar\n");

    ed.execute_keymap_command(
        std::borrow::Cow::Borrowed("select-word-nearest-on-line"),
        Some(1),
        false,
        ArgSource::Keymap,
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
/// delete]`, not `[select-all-matches, delete]` — `select-all-matches`
/// errors on no match (`cmd_select_all_matches` returns `Err("no matches")`)
/// and leaves the selection untouched, so it established no replayable
/// extent of its own and must not overwrite the `x` step that did.
///
/// Buffer "aaa\nbbb\nccc\n": `x` selects "aaa\n"; search pattern "zzz"
/// matches nothing; `m/` no-ops (still "aaa\n" selected); `d` deletes it,
/// leaving "bbb\nccc\n". Move to "ccc"; `.` must replay `select-line` + `d`
/// there too, leaving "bbb\n".
///
/// Fail oracle: without the "selection unchanged" gate, `m/`'s failed run
/// still resets the recipe to `[select-all-matches]` — `.` replays that
/// (another no-op) then `delete`, which acts on the current 1-char cursor
/// instead of the whole line, leaving "bb\n" instead of "bbb\n".
#[test]
fn dot_repeat_of_failed_select_all_matches_preserves_prior_recipe() {
    let mut ed = editor_from("-[a]>aa\nbbb\nccc\n").with_search_regex("zzz");

    ed.feed_key(key('x')); // select-line: "aaa\n"
    ed.feed_key(key('m'));
    ed.feed_key(key('/')); // no match — selection untouched, command errors
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

/// `x ms( d` where `ms(` finds no surrounding pair must record
/// `[select-line, delete]`, not `[surround-paren, delete]` — `surround-paren`
/// is infallible and leaves the selection unchanged on no-match (see
/// `select_surround` in `hume-ops/src/surround.rs`), so like the
/// `select-all-matches` case above it established no replayable extent and
/// must not overwrite the prior `x` step.
///
/// Buffer "aaa\nbbb\nccc\n": `x` selects "aaa\n" (no parens on that line, so
/// `ms(` on it would no-op); `d` deletes it, leaving "bbb\nccc\n". Move to
/// "ccc"; `.` must replay `select-line` + `d` there too, leaving "bbb\n".
///
/// Fail oracle: without the "selection unchanged" gate, the no-op `ms(`
/// still resets the recipe to `[surround-paren]` — `.` replays that (another
/// no-op, cursor unchanged) then `delete`, which acts on the current 1-char
/// cursor instead of the whole line, leaving "bb\n" instead of "bbb\n".
#[test]
fn dot_repeat_of_noop_surround_preserves_prior_recipe() {
    let mut ed = editor_from("-[a]>aa\nbbb\nccc\n");

    ed.feed_key(key('x')); // select-line: "aaa\n"
    ed.feed_key(key('m'));
    ed.feed_key(key('s'));
    ed.feed_key(key('(')); // no parens on this line — selection untouched
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
/// `repeat-last-action`'s own dispatch used to commit — and thereby close —
/// the still-open paste session before `replay_dot` ever got a chance to look
/// at what it was replaying. See `.defers_paste_commit()` on
/// `repeat-last-action`'s registration (`registry/defaults/editor_cmds.rs`)
/// and `Editor::replay_dot`'s own paste-commit step.
///
/// Independent oracle: `KillRing::cycle_position()` is the ring's own cursor,
/// read directly rather than inferred from which text ended up pasted.
///
/// Fail oracle: without `repeat-last-action`'s `.defers_paste_commit()` (and
/// the matching commit/defer decision in `replay_dot`), the paste session is
/// closed before the replay runs; `do_paste_cycle` sees `paste_group == None`
/// and returns before ever calling `cycle_older()`, so `cycle_position()`
/// stays at `Some(1)` instead of advancing to `Some(2)`.
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

/// Build an editor with Steel scripting and a command defined by `source`.
///
/// Evaluates `source` as an init-context eval so `define-command!` and its
/// siblings are in scope.  All native command names are pre-registered so
/// `(call! …)` can invoke them from inside the Steel lambda.
fn editor_with_steel(initial_state: &str, source: &str) -> Editor {
    use crate::editor::host_impl::EditorHostImpl;
    use hume_scripting::ScriptingHost;

    let mut ed = editor_from(initial_state);
    let names: Vec<String> = ed
        .state
        .config
        .registry
        .native_mappable_names()
        .map(str::to_owned)
        .collect();
    let name_refs: Vec<&str> = names.iter().map(String::as_str).collect();

    let mut host = ScriptingHost::new();
    host.register_command_names(&name_refs);

    let mut init_host = EditorHostImpl::new(&mut ed.state, &mut ed.view);
    host.eval_source(source, &mut init_host)
        .expect("Steel eval must succeed in editor_with_steel");

    ed.scripting = Some(host);
    ed
}

/// A Steel command registered with `#:repeatable #t` must overwrite
/// `last_repeatable_action` with its own name so `.` replays the outer
/// Steel body, not the inner native command.
///
/// Independent oracle: buffer is "foo bar\n", initial selection is "foo".
/// Run `del-sel` (repeatable Steel command that calls delete internally) →
/// "foo" is deleted, buffer is " bar\n". Press `w` to select "bar" — the
/// first (and only) word on its line, so its leading space is indentation
/// and is never absorbed, and there's no trailing space either (EOL
/// follows) — the default around-word span is bare "bar" — then `.` replays
/// `del-sel` on that selection, leaving " \n".
///
/// Fail oracle 1: if `meta().repeatable` returned `false` for `SteelBacked`,
/// `last_repeatable_action` would be `None` (no prior recording) — `.` would
/// be a no-op and "bar" would survive.
///
/// Fail oracle 2: if the outer name didn't win the slot, `last_repeatable_action`
/// would be `"delete"` (the inner native) — the result would be the same but
/// the name assertion below would catch the missing outer-name record.
#[test]
fn steel_dot_repeatable_round_trip() {
    let mut ed = editor_with_steel(
        "-[foo]> bar\n",
        r#"(define-command! "del-sel" ""
             (lambda () (call! "delete"))
             #:repeatable #t)"#,
    );

    // Run the repeatable Steel command.
    ed.execute_keymap_command("del-sel".into(), Some(1), false, ArgSource::Keymap);
    // "foo" deleted; buffer is " bar\n".
    assert_eq!(ed.doc().text().to_string(), " bar\n", "first run");

    // last_repeatable_action must name the outer Steel command, not inner "delete".
    assert_eq!(
        ed.state
            .last_repeatable_action
            .as_ref()
            .map(|a| a.command.as_ref()),
        Some("del-sel"),
        "outer Steel command must win the repeat slot over the inner 'delete'"
    );

    // Select "bar" then press `.` — replay must delete the current selection.
    ed.feed_key(key('w')); // select "bar" (bare — indent kept)
    ed.feed_key(key('.')); // replay "del-sel" → delete "bar"
    // Oracle: " bar\n" → " \n" after "bar" is deleted.
    assert_eq!(
        ed.doc().text().to_string(),
        " \n",
        "`.` must replay the Steel command and delete the current selection"
    );
}

/// A plain `define-command!` (non-repeatable) must not overwrite
/// `last_repeatable_action` set by a prior native edit.
///
/// Fail oracle: if `meta().repeatable` returned `true` for `SteelBacked`,
/// running the Steel command would stamp `last_repeatable_action` with its
/// name; the subsequent `.` would replay the Steel command instead of the
/// native delete.
#[test]
fn steel_command_is_not_repeatable() {
    let mut ed = editor_with_steel(
        "-[foo]> bar\n",
        r#"(define-command! "del-sel" "" (lambda () (call! "delete")))"#,
    );

    // Establish a known repeatable native action first.
    ed.feed_key(key('d')); // delete "foo" → " bar\n"
    assert_eq!(ed.doc().text().to_string(), " bar\n");
    assert_eq!(
        ed.state
            .last_repeatable_action
            .as_ref()
            .map(|a| a.command.as_ref()),
        Some("delete"),
        "setup: last_repeatable_action must be 'delete' after d"
    );

    // Run the Steel command — it calls (call! "delete") internally.
    ed.execute_keymap_command("del-sel".into(), Some(1), false, ArgSource::Keymap);

    // last_repeatable_action must still be "delete", not "del-sel".
    assert_eq!(
        ed.state
            .last_repeatable_action
            .as_ref()
            .map(|a| a.command.as_ref()),
        Some("delete"),
        "Steel command must not overwrite last_repeatable_action"
    );
}

/// A plain `define-command!` (non-repeatable) must NOT overwrite
/// `last_repeatable_action` set by a prior native edit.
///
/// Fail oracle: change `meta().repeatable` to `true` for all `SteelBacked`
/// commands in `editor/mod.rs` `dispatch()` and the non-repeatable Steel command
/// would overwrite `last_repeatable_action` — the subsequent `.` would replay
/// the Steel command, the name assertion would differ.
#[test]
fn non_repeatable_steel_does_not_hijack_dot() {
    let mut ed = editor_with_steel(
        "-[foo]> bar\n",
        r#"(define-command! "noop-move" "" (lambda () (call! "move-right")))"#,
    );

    // Establish a known repeatable native action first.
    ed.feed_key(key('d')); // delete "foo" → " bar\n"
    assert_eq!(ed.doc().text().to_string(), " bar\n");
    assert_eq!(
        ed.state
            .last_repeatable_action
            .as_ref()
            .map(|a| a.command.as_ref()),
        Some("delete"),
        "setup: last_repeatable_action must be 'delete' after d"
    );

    // Run the non-repeatable Steel command.
    ed.execute_keymap_command("noop-move".into(), Some(1), false, ArgSource::Keymap);

    // last_repeatable_action must still be "delete".
    assert_eq!(
        ed.state
            .last_repeatable_action
            .as_ref()
            .map(|a| a.command.as_ref()),
        Some("delete"),
        "non-repeatable Steel command must not overwrite last_repeatable_action"
    );

    // `.` must still delete "bar".
    ed.feed_key(key('w'));
    ed.feed_key(key('.'));
    assert!(
        !ed.doc().text().to_string().contains("bar"),
        "`.` must repeat the native delete, not the non-repeatable Steel command"
    );
}
