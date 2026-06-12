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

/// `define-command-repeatable!` opts the command into dot-repeat.
///
/// Run the Steel command, verify `last_repeatable_action` carries the Steel
/// command name (not the inner native), then press `.` and confirm the edit
/// is applied a second time.
///
/// Fail oracle: if `is_repeatable()` returns `false` for `SteelBacked`,
/// `last_repeatable_action` would carry `"delete"` (from the inner native
/// dispatch) rather than `"del-sel"`, and the name assertion fails.
#[test]
fn steel_dot_repeatable_round_trip() {
    let mut ed = editor_with_steel(
        "-[foo]> bar\n",
        r#"(define-command-repeatable! "del-sel" "" (lambda () (call! "delete")))"#,
    );

    // Run the Steel repeatable command: deletes "foo".
    ed.execute_keymap_command("del-sel".into(), 1, false, vec![]);
    assert_eq!(ed.doc().text().to_string(), " bar\n");

    // last_repeatable_action must name the outer Steel command, not "delete".
    assert_eq!(
        ed.state.last_repeatable_action.as_ref().map(|a| a.command.as_ref()),
        Some("del-sel"),
        "last_repeatable_action must point to the Steel command, not the inner native"
    );

    // Move to "bar" and press `.`.
    ed.feed_key(key('w'));
    ed.feed_key(key('.'));

    // Independent oracle: "foo" was deleted first; "bar" must be deleted by `.`.
    assert_eq!(
        ed.doc().text().to_string(),
        " \n",
        "dot-repeat must replay the Steel command, deleting 'bar'"
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

/// When a repeatable Steel command internally calls a native repeatable command
/// via `(call! …)`, the OUTER Steel name must win the repeat slot, not the inner
/// native name.
///
/// Without the post-dispatch recording in execute.rs, the inner native
/// dispatch would write `last_repeatable_action.command = "delete"`;
/// the outer Steel recording overwrites it with `"wrap-del"`.
///
/// Fail oracle: remove the post-dispatch recording block — `last_repeatable_action`
/// would be `Some("delete")` instead of `Some("wrap-del")`.
#[test]
fn repeatable_steel_wrapper_wins_over_inner_native() {
    let mut ed = editor_with_steel(
        "-[foo]> bar\n",
        r#"(define-command-repeatable! "wrap-del" "" (lambda () (call! "delete")))"#,
    );

    ed.execute_keymap_command("wrap-del".into(), 1, false, vec![]);
    assert_eq!(ed.doc().text().to_string(), " bar\n");

    // The outer Steel wrapper must own the repeat slot.
    assert_eq!(
        ed.state.last_repeatable_action.as_ref().map(|a| a.command.as_ref()),
        Some("wrap-del"),
        "outer repeatable Steel wrapper must win over inner 'delete' in last_repeatable_action"
    );

    // Verify `.` still applies the correct edit.
    ed.feed_key(key('w'));
    ed.feed_key(key('.'));
    assert!(
        !ed.doc().text().to_string().contains("bar"),
        "dot-repeat via Steel wrapper must delete 'bar'"
    );
}

/// A `define-command-repeatable!` defined inside a lazy plugin is recorded for
/// dot-repeat on its FIRST dispatch (after Lazy → SteelBacked activation).
///
/// The re-query in execute.rs covers the Lazy→SteelBacked swap: `reg_cmd`
/// still holds the pre-dispatch `Lazy` variant (is_repeatable = false), but
/// after `call_steel_cmd` the registry entry is `SteelBacked { repeatable: true }`.
///
/// Not on Windows: Scheme `require` strings embed OS paths with forward slashes.
#[test]
#[cfg(not(windows))]
fn lazy_repeatable_round_trip() {
    use crate::editor::scripting_setup::make_init_host;
    use hume_scripting::ScriptingHost;

    let dir = {
        let _lock = HUME_RUNTIME_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
        tempfile::tempdir().unwrap()
    };
    let plugin_dir = dir.path().join("plugins").join("user").join("tp");
    std::fs::create_dir_all(&plugin_dir).unwrap();
    std::fs::write(
        plugin_dir.join("plugin.scm"),
        r#"(define-command-repeatable! "tp-del" "" (lambda () (call! "delete")))"#,
    ).unwrap();
    let init_path = dir.path().join("init.scm");
    std::fs::write(
        &init_path,
        r#"(declare-plugin "user/tp" #:on-command '("tp-del"))"#,
    ).unwrap();

    let mut ed = editor_from("-[foo]> bar\n");
    let mut host = ScriptingHost::new();
    host.set_data_dir(dir.path().to_path_buf());
    {
        let mut ih = make_init_host(&mut ed.state, &mut ed.view);
        host.eval_init(&init_path, 10_000, &mut ih, Default::default())
    }.expect("eval_init must succeed");
    let triggers = host.command_triggers();
    ed.register_lazy_command_stubs(&triggers);
    ed.scripting = Some(host);

    // First dispatch: Lazy miss → plugin activates → SteelBacked runs.
    ed.execute_keymap_command("tp-del".into(), 1, false, vec![]);
    assert_eq!(ed.doc().text().to_string(), " bar\n");

    // The re-query must see the now-activated SteelBacked repeatable entry.
    assert_eq!(
        ed.state.last_repeatable_action.as_ref().map(|a| a.command.as_ref()),
        Some("tp-del"),
        "lazy-activated repeatable command must be recorded on first dispatch"
    );

    // `.` must replay via the activated SteelBacked entry.
    ed.feed_key(key('w'));
    ed.feed_key(key('.'));
    assert!(
        !ed.doc().text().to_string().contains("bar"),
        "dot-repeat must replay the lazy-activated Steel command"
    );
}
