use super::*;
use pretty_assertions::assert_eq;

// ── Dot-repeat tests ──────────────────────────────────────────────────────────

/// `d` deletes the selection. Moving then pressing `.` should delete the next selection.
#[test]
fn dot_repeats_delete() {
    // Cursor starts at 'f'. `w` selects "foo", `d` deletes it.
    // Then from the space at pos 0, `w` selects "bar" (the next word). `.` deletes it.
    let mut ed = editor_from("-[foo]> bar\n");
    ed.feed_key(key('d')); // delete "foo" → " bar\n", cursor at 0 (space)
    assert_eq!(ed.doc().text().to_string(), " bar\n");

    ed.feed_key(key('w')); // from space, select "bar"
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

    // Move to "bar" and repeat.
    ed.feed_key(key('w')); // select "bar"
    ed.feed_key(key('.')); // repeat: delete "bar", insert "hi"

    assert_eq!(ed.doc().text().to_string(), "hi hi\n");
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
fn dot_repeats_replace() {
    // Use a space between words so `w` can navigate to the second word.
    let mut ed = editor_from("-[ab]> cd\n");

    ed.feed_key(key('r')); // wait-char
    ed.feed_key(key('x')); // replace "ab" → "xx cd\n"

    assert_eq!(ed.doc().text().to_string(), "xx cd\n");

    // `w` from the "xx" selection (head at pos 1) selects the next word "cd".
    ed.feed_key(key('w'));
    ed.feed_key(key('.')); // repeat replace with 'x' → "xx xx\n"

    assert_eq!(ed.doc().text().to_string(), "xx xx\n");
}

/// When `.` is given an explicit count, the stored `last_repeatable_action.count`
/// must not be corrupted — the explicit count is used for the replay but is NOT
/// written back into the stored action.
///
/// The original count must survive so that a subsequent plain `.` can still
/// reproduce the original repetition count.
///
/// Fail oracle: if `drain_pending_repeat` wrote `PendingRepeat.count` back into
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

/// When `.` is pressed without a count, the original action's count is reused.
#[test]
fn dot_without_count_uses_original() {
    // Use `select-line` (x) which is repeatable... wait, 'x' is select-line which
    // is a Selection command (not repeatable). Use 'p' (paste) instead.
    // Actually let's test with `d` — record with count, replay without.
    // `d` ignores count anyway, so let's use a simpler repeatable: paste.
    // Use `i` + text + Esc with count, then `.` without count.
    // Actually the simplest: just verify last_repeatable_action.count is preserved.
    let mut ed = editor_from("-[hi]> world\n");

    // `d` (count ignored by the command, but stored as 1 in last_repeatable_action).
    ed.feed_key(key('d'));
    assert_eq!(ed.state.last_repeatable_action.as_ref().unwrap().count, 1);

    // Move to "world", hit `.` without a count.
    ed.feed_key(key('w'));
    ed.feed_key(key('.'));
    // last_repeatable_action.count should still be 1 after replay.
    assert_eq!(ed.state.last_repeatable_action.as_ref().unwrap().count, 1);
    // The delete should have happened.
    assert!(!ed.doc().text().to_string().contains("world"));
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

    // Move to "bar" and repeat.
    ed.feed_key(key('w'));
    ed.feed_key(key('.'));
    assert_eq!(ed.doc().text().to_string(), "hi hi\n");

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

/// `p` (paste-after) is repeatable: the register contents are pasted again.
#[test]
fn dot_repeats_paste_after() {
    let mut ed = editor_from("-[ab]>cd\n");

    // Yank "ab" then delete so we have something to paste.
    ed.feed_key(key('y')); // yank "ab" into default register
    ed.feed_key(key('d')); // delete "ab" → cursor on "cd"

    // Paste after.
    ed.feed_key(key('p')); // pastes "ab" after 'c' → "cabd", "ab" selected
    // Move one char right (off the pasted selection) then repeat.
    ed.feed_key(key('l')); // move right → collapsed cursor after the pasted text
    ed.feed_key(key('.')); // paste again
    // Just verify no panic and paste happened twice (content contains "ab" twice).
    let buf = ed.doc().text().to_string();
    let count = buf.matches("ab").count();
    assert!(
        count >= 2,
        "expected at least 2 occurrences of 'ab', got: {buf:?}"
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
        ed.state.last_repeatable_action.as_ref().unwrap().selection_recipe.len(),
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

    ed.feed_key(key('x'));          // select "aaa\n"
    ed.feed_key(key_ctrl('x'));     // extend to "aaa\nbbb\n"
    ed.feed_key(key('d'));          // delete → "ccc\nddd\n", cursor on 'c'
    assert_eq!(ed.doc().text().to_string(), "ccc\nddd\n");

    // Recipe must be [select-line F, select-line T] (establish + one extend).
    let recipe = &ed.state.last_repeatable_action.as_ref().unwrap().selection_recipe;
    assert_eq!(recipe.len(), 2, "recipe must have 2 steps");
    assert!(!recipe[0].extend, "first step must be Move (establish)");
    assert!(recipe[1].extend,  "second step must be Extend");

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
        ed.state.last_repeatable_action.as_ref().unwrap().selection_recipe.len(),
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
/// empty recipe. `.` replays just the delete on whatever the current selection is
/// — backward-compatible behavior, identical to before this change.
#[test]
fn dot_repeat_collapsed_cursor_empty_recipe() {
    let mut ed = editor_from("-[a]>aa\nbbb\nccc\n");

    ed.feed_key(key('j')); // move-down: collapsed on 'b', recipe cleared
    ed.feed_key(key('d')); // delete 'b' (1-char selection), recipe = []
    assert_eq!(ed.doc().text().to_string(), "aaa\nbb\nccc\n");

    // Recipe must be empty.
    assert_eq!(
        ed.state.last_repeatable_action.as_ref().unwrap().selection_recipe.len(),
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
/// 1-char cursor at 'c', deleting only 'c' and inserting 'z' → "zbbb\nzcc\n".
#[test]
fn dot_repeats_change_reselects_line() {
    // Three lines; the structural-'\n' protection only affects the last content
    // line (here "ccc\n"), which is fine — the test still proves full-line reselect.
    let mut ed = editor_from("-[a]>aa\nbbb\nccc\n");

    ed.feed_key(key('x')); // select "aaa\n"
    ed.feed_key(key('c')); // change: delete "aaa\n" → "bbb\nccc\n", cursor at 'b'; enter Insert
    ed.feed_key(key('z')); // type 'z' → "zbbb\nccc\n"
    ed.feed_key(key_esc()); // back to Normal
    assert_eq!(ed.doc().text().to_string(), "zbbb\nccc\n");

    // Recipe must be [select-line F]; insert_keys must be ['z'].
    {
        let action = ed.state.last_repeatable_action.as_ref().unwrap();
        assert_eq!(action.selection_recipe.len(), 1, "recipe must have select-line step");
        assert_eq!(action.insert_keys.len(), 1, "insert_keys must capture typed chars");
    }

    // Move to 'c' and press `.`.
    ed.feed_key(key('j')); // move-down to 'c' (line 1)
    ed.feed_key(key('.'));  // re-select "ccc\n" via recipe, change, retype 'z'

    // Oracle: recipe re-selects "ccc\n", change deletes "ccc" (structural '\n' stays),
    // inserts 'z' → "zbbb\nz\n". Without recipe, only 'c' would be deleted → "zbbb\nzcc\n".
    assert_eq!(
        ed.doc().text().to_string(),
        "zbbb\nz\n",
        "`.` must re-select the full line, change it, and replay insert_keys"
    );
}

/// `undo` must clear the selection-recipe buffer so a stale `select-line` does
/// not leak into the next edit's recipe.
///
/// Fail oracle: drop the `else { state.selection_recipe.clear() }` branch for
/// non-repeatable EditorCmds → `selection_recipe` is not cleared by `undo` →
/// the assert will see len=1 instead of 0.
#[test]
fn undo_clears_selection_recipe() {
    let mut ed = editor_from("-[a]>aa\nbbb\n");

    // Establish a recipe via `x`.
    ed.feed_key(key('x'));
    assert_eq!(
        ed.state.selection_recipe.len(), 1,
        "setup: x must establish a 1-step recipe"
    );

    // `u` (undo) is a non-repeatable EditorCmd → must clear the recipe.
    ed.feed_key(key('u'));
    assert_eq!(
        ed.state.selection_recipe.len(), 0,
        "undo must clear the selection recipe buffer"
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

/// After `.`, `last_command` must be re-stamped to `"repeat-last-action"`, NOT
/// to the name of the replayed command (`"delete"`, `"change"`, …).
///
/// `drain_pending_repeat` (`mod.rs:482`) does this re-stamp so that smart-p
/// (`SMART_P_LAST_CMDS = ["change","delete"]`) correctly routes a bare `p`/`P`
/// to the clipboard rather than the kill-ring after a dot-repeat.
///
/// Fail oracle: commenting the re-stamp at `drain_pending_repeat`'s last line
/// makes this test fail (last_command would be "delete" after the inner replay).
#[test]
fn dot_restamps_last_command() {
    let mut ed = editor_from("-[foo]> bar\n");

    ed.feed_key(key('d')); // delete "foo"; last_command = "delete"
    assert_eq!(
        ed.state.last_command.as_deref(),
        Some("delete"),
        "setup: last_command must be 'delete' after d"
    );

    ed.feed_key(key('w')); // move to "bar"
    ed.feed_key(key('.')); // repeat; drain_pending_repeat fires and re-stamps

    assert_eq!(
        ed.state.last_command.as_deref(),
        Some("repeat-last-action"),
        "drain_pending_repeat must re-stamp last_command to 'repeat-last-action'"
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
    let names: Vec<String> =
        ed.state.registry.native_mappable_names().map(str::to_owned).collect();
    let name_refs: Vec<&str> = names.iter().map(String::as_str).collect();

    let mut host = ScriptingHost::new();
    host.register_command_names(&name_refs);

    let mut init_host = EditorHostImpl { state: &mut ed.state, view: &mut ed.view };
    host.eval_source_returning_defs(source.to_owned(), Default::default(), &mut init_host)
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
/// "foo" is deleted, buffer is " bar\n". Press `w` to select "bar", then `.`
/// — `.` must replay `del-sel` on the current selection ("bar"), leaving " \n".
///
/// Fail oracle 1: if `is_repeatable()` returned `false` for `SteelBacked`,
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
    ed.execute_keymap_command("del-sel".into(), 1, false, vec![]);
    // "foo" deleted; buffer is " bar\n".
    assert_eq!(ed.doc().text().to_string(), " bar\n", "first run");

    // last_repeatable_action must name the outer Steel command, not inner "delete".
    assert_eq!(
        ed.state.last_repeatable_action.as_ref().map(|a| a.command.as_ref()),
        Some("del-sel"),
        "outer Steel command must win the repeat slot over the inner 'delete'"
    );

    // Select "bar" then press `.` — replay must delete the current selection.
    ed.feed_key(key('w')); // select "bar"
    ed.feed_key(key('.'));  // replay "del-sel" → delete "bar"
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
/// Fail oracle: if `is_repeatable()` returned `true` for `SteelBacked`,
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
        ed.state.last_repeatable_action.as_ref().map(|a| a.command.as_ref()),
        Some("delete"),
        "setup: last_repeatable_action must be 'delete' after d"
    );

    // Run the Steel command — it calls (call! "delete") internally.
    ed.execute_keymap_command("del-sel".into(), 1, false, vec![]);

    // last_repeatable_action must still be "delete", not "del-sel".
    assert_eq!(
        ed.state.last_repeatable_action.as_ref().map(|a| a.command.as_ref()),
        Some("delete"),
        "Steel command must not overwrite last_repeatable_action"
    );
}

/// A plain `define-command!` (non-repeatable) must NOT overwrite
/// `last_repeatable_action` set by a prior native edit.
///
/// Fail oracle: remove the `is_repeatable()` guard in execute.rs and the
/// non-repeatable Steel command would overwrite `last_repeatable_action` with
/// `None` (no recording) — the subsequent `.` would repeat nothing, the name
/// assertion would differ.
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
        ed.state.last_repeatable_action.as_ref().map(|a| a.command.as_ref()),
        Some("delete"),
        "setup: last_repeatable_action must be 'delete' after d"
    );

    // Run the non-repeatable Steel command.
    ed.execute_keymap_command("noop-move".into(), 1, false, vec![]);

    // last_repeatable_action must still be "delete".
    assert_eq!(
        ed.state.last_repeatable_action.as_ref().map(|a| a.command.as_ref()),
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

