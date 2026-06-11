use super::*;

use hume_scripting::ScriptingHost;
use crate::editor::host_impl::EditorHostImpl;
use crate::testing::MockHost;
use hume_scripting::host::EditorHost;

// ── Unit tests: run_command_sync ──────────────────────────────────────────────

/// Helper: build a full-live `EditorHostImpl` from a real editor.
///
/// Mirrors the construction in `execute.rs` so the host has the same shape as
/// in production command dispatch.
macro_rules! live_host {
    ($ed:ident) => {{
        EditorHostImpl {
            state: &mut $ed.state,
            view:  &mut $ed.view,
        }
    }};
}

/// `run_command_sync` for a `Motion` command must immediately update the cursor
/// position — no queue involved.
#[test]
fn run_command_sync_motion_moves_cursor() {
    // "-[a]>bc\n" — cursor at position 0.
    let mut ed = editor_from("-[a]>bc\n");
    let before = live_host!(ed).cursor_char_index().expect("cursor_char_index before");

    {
        let mut host = live_host!(ed);
        // move-right is a Motion — must dispatch synchronously.
        host.run_command_sync("move-right", 1, false, None)
            .expect("run_command_sync must not error for move-right");
    }

    let after = live_host!(ed).cursor_char_index().expect("cursor_char_index after");
    assert_eq!(before, 0, "cursor must start at 0");
    assert_eq!(after, 1, "cursor must be at 1 after sync move-right");
}

/// `run_command_sync` for an `EditorCmd` (the fourth native variant) must apply
/// its effect immediately — not queue it.
///
/// Fail oracle: return `Ok(())` from `run_command_sync` without calling
/// `dispatch_native` → `undo` never reverts the deletion → assertion fails.
#[test]
fn run_command_sync_editor_cmd_runs_sync() {
    // Buffer "abc\n", selection on 'a'. Delete it via the normal keymap path to
    // create an undoable revision, then undo via run_command_sync.
    let mut ed = editor_from("-[a]>bc\n");
    ed.execute_keymap_command("delete".into(), 1, false, vec![]);
    // Buffer is now "bc\n"; cursor should be at 0.
    assert_eq!(
        live_host!(ed).cursor_char_index(),
        Some(0),
        "pre-condition: cursor at 0 after delete"
    );

    {
        let mut host = live_host!(ed);
        host.run_command_sync("undo", 1, false, None)
            .expect("run_command_sync must not error for undo");
    }

    // After undo the deleted 'a' must be restored and cursor back at 0 on "abc\n".
    let buf_text: String = ed.doc().text().rope().to_string();
    assert_eq!(
        buf_text, "abc\n",
        "undo via run_command_sync must restore the deleted character"
    );
    assert_eq!(
        live_host!(ed).cursor_char_index(),
        Some(0),
        "cursor must be at 0 after undo"
    );
}

/// `run_command_sync` for an unknown name must return `Err`.
#[test]
fn run_command_sync_unknown_name_errors() {
    let mut ed = editor_from("-[a]>bc\n");
    let mut host = live_host!(ed);
    let result = host.run_command_sync("no-such-command-xyzzy", 1, false, None);
    assert!(result.is_err(), "unknown command must return Err");
}

/// `cursor_char_index` must reflect the live cursor position — after a sync move
/// the index updates, not a frozen pre-move snapshot.
///
/// A stub that always returned 0 would pass the pre-move check but fail after
/// the move, so liveness is genuinely tested.
#[test]
fn cursor_char_index_reads_live_position() {
    let mut ed = editor_from("-[a]>bc\n");
    let before = live_host!(ed).cursor_char_index().expect("cursor_char_index before");
    assert_eq!(before, 0, "cursor starts at 0");

    { live_host!(ed).run_command_sync("move-right", 1, false, None).unwrap(); }

    let after = live_host!(ed).cursor_char_index().expect("cursor_char_index after");
    assert_eq!(after, 1, "cursor_char_index must reflect the sync move");
}

/// `current_line_number` must reflect a live position change across lines.
///
/// A stub always returning 1 would pass a single-line check; the move to a
/// second line proves liveness.
#[test]
fn current_line_number_reads_live_position() {
    // Two-line buffer: "ab\ncd\n"; cursor on line 1.
    let mut ed = editor_from("-[a]>b\ncd\n");
    let before = live_host!(ed).current_line_number().expect("line before");
    assert_eq!(before, 1, "cursor starts on line 1");

    // move-down crosses to line 2.
    { live_host!(ed).run_command_sync("move-down", 1, false, None).unwrap(); }

    let after = live_host!(ed).current_line_number().expect("line after");
    assert_eq!(after, 2, "current_line_number must reflect the sync move to line 2");
}

/// `run_command_sync` for a `Selection` command must immediately update the
/// selection.
#[test]
fn run_command_sync_selection_updates_sel() {
    // "-[a]>bc\n" — cursor at 0, single-char selection covering 'a'.
    let mut ed = editor_from("-[a]>bc\n");
    {
        let mut host = live_host!(ed);
        // select-line is a Selection command.
        host.run_command_sync("select-line", 1, false, None)
            .expect("run_command_sync must not error for select-line");
    }
    // select-line covers the full line "abc\n" (inclusive); head lands on '\n' at position 3.
    let head = live_host!(ed).cursor_char_index().expect("cursor_char_index after sel");
    assert_eq!(head, 3, "select-line head must be at position 3 ('\\n' — inclusive selection)");
}

// ── Native arg validation (classify-then-parse) ───────────────────────────────

/// `(call! "move-right" 5)` from Steel moves the cursor 5 positions synchronously.
///
/// Verifies the native count path: classify → `Ok(true)` → `parse_count_extend`
/// extracts `count=5` → `run_command_sync("move-right", 5, false)` runs immediately.
/// Fail oracle: change expected cursor to 1 — test fails.
#[test]
fn call_bang_count_arg_dispatches_synchronously() {
    let mut ed = editor_from("-[a]>bcdef\n");

    let names: Vec<String> = ed.state.registry.native_mappable_names().map(str::to_owned).collect();
    let name_refs: Vec<&str> = names.iter().map(String::as_str).collect();

    let mut host = ScriptingHost::new();
    host.register_command_names(&name_refs);

    let mut mock = MockHost::new();
    let defs = host
        .eval_source_returning_defs(
            r#"(define-command! "move-right-5" "" (lambda () (call! "move-right" 5)))"#.to_owned(),
            Default::default(),
            &mut mock,
        )
        .expect("define-command! must succeed");

    ed.register_steel_cmds(defs);
    ed.scripting = Some(host);
    ed.execute_keymap_command("move-right-5".into(), 1, false, vec![]);

    let idx = ed.state.panes.state
        .get(ed.state.focused_pane_id).unwrap()
        .values().next().unwrap()
        .selections.primary().head();
    assert_eq!(idx, 5, "cursor must be at position 5 after (call! \"move-right\" 5)");
}

/// `(call! "move-right" "garbage")` must raise a Steel error and NOT move the cursor.
///
/// Verifies fail-fast: classify happens before `run_command_sync`, so the command
/// never executes. The error message must mention the malformed args.
/// Fail oracle: if `execute_keymap_command` returns without error, `state(ed)`
/// would differ from the initial state — the assert catches that.
#[test]
fn call_bang_malformed_arg_to_native_cmd_errors_without_side_effect() {
    let mut ed = editor_from("-[a]>bc\n");

    let names: Vec<String> = ed.state.registry.native_mappable_names().map(str::to_owned).collect();
    let name_refs: Vec<&str> = names.iter().map(String::as_str).collect();

    let mut host = ScriptingHost::new();
    host.register_command_names(&name_refs);

    let mut mock = MockHost::new();
    let defs = host
        .eval_source_returning_defs(
            r#"(define-command! "move-right-bad" "" (lambda () (call! "move-right" "garbage")))"#.to_owned(),
            Default::default(),
            &mut mock,
        )
        .expect("define-command! must succeed");

    ed.register_steel_cmds(defs);
    ed.scripting = Some(host);

    // execute_keymap_command reports errors to the status bar rather than panicking;
    // check the cursor did not move (the Steel error aborted the eval).
    let before = state(&ed);
    ed.execute_keymap_command("move-right-bad".into(), 1, false, vec![]);
    let after = state(&ed);
    assert_eq!(before, after, "cursor must not move when native arg is malformed");
}

// ── Case B integration test ───────────────────────────────────────────────────

/// **Case B** — a Steel function can observe the effect of `(move-right)` in
/// the same eval via `(cursor-char-index)`.
///
/// The discriminating logic:
/// - Start at position 0.  Call `(move-right)`.
/// - If sync: cursor is now 1 — the `(when (= (cursor-char-index) 1) ...)` arm
///   fires and calls `(move-right)` a second time → final position 2.
/// - If deferred: cursor stays at 0 during the lambda — the `(when ...)` does
///   not fire → one deferred `move-right` runs post-eval → final position 1.
///
/// Sync path → cursor 2.  Deferred path → cursor 1.
#[test]
fn case_b_sync_cursor_read_reflects_motion() {
    // "-[a]>bc\n" — cursor at position 0.
    let mut ed = editor_from("-[a]>bc\n");

    // Pre-register native command names as Steel bindings so `(move-right)` etc.
    // resolve at compile time.
    let names: Vec<String> = ed.state.registry.native_mappable_names().map(str::to_owned).collect();
    let name_refs: Vec<&str> = names.iter().map(String::as_str).collect();

    let mut host = ScriptingHost::new();
    host.register_command_names(&name_refs);

    // Define a command that exercises the sync-read property.
    let mut mock = MockHost::new();
    let defs = host
        .eval_source_returning_defs(
            r#"(define-command! "test-case-b" "Case B probe"
                 (lambda ()
                   (move-right)
                   (when (= (cursor-char-index) 1)
                     (move-right))))"#
                .to_owned(),
            Default::default(),
            &mut mock,
        )
        .expect("define-command! must succeed");

    ed.register_steel_cmds(defs);
    ed.scripting = Some(host);

    ed.execute_keymap_command("test-case-b".into(), 1, false, vec![]);

    let final_state = state(&ed);
    // Sync: both moves ran inside the lambda → cursor at 2, "ab-[c]>\n".
    // Deferred: only one move queued → cursor at 1, "a-[b]>c\n".
    assert_eq!(
        final_state, "ab-[c]>\n",
        "sync dispatch: (cursor-char-index) must reflect (move-right) effect within same eval"
    );
}

// ── Steel deferred dot-repeat ────────────────────────────────────────────────

/// A Steel command that calls `(call! "repeat-last-action")` must actually replay
/// the last editing action when its key is pressed.
///
/// `(call! "repeat-last-action")` is a sync EditorCmd dispatch: it calls
/// `run_command_sync("repeat-last-action")`, which runs `cmd_repeat` and sets
/// `state.pending_repeat`. The replay then fires in `drain_pending_repeat` at the
/// tail of the enclosing `handle_key` call — NOT during the Steel eval.
///
/// The test drives the key through `feed_key` so the full `handle_key` tail
/// (including `drain_pending_repeat`) executes before we inspect the buffer.
///
/// Fail oracle: if `drain_pending_repeat` were not called at `handle_key`'s tail,
/// `pending_repeat` would be set but never consumed, and the buffer would be
/// unchanged after pressing the Steel key.
#[test]
fn steel_call_repeat_last_action_drains_via_handle_key() {
    use crate::editor::keymap::{BindMode};
    use crossterm::event::KeyCode;

    let mut ed = editor_from("-[foo]> bar\n");

    // Register command names so `(call! "repeat-last-action")` resolves.
    let names: Vec<String> = ed.state.registry.native_mappable_names().map(str::to_owned).collect();
    let name_refs: Vec<&str> = names.iter().map(String::as_str).collect();

    let mut host = ScriptingHost::new();
    host.register_command_names(&name_refs);

    let mut mock = MockHost::new();
    let defs = host
        .eval_source_returning_defs(
            r#"(define-command! "steel-dot-repeat" "Repeat last action via Steel"
                 (lambda () (call! "repeat-last-action")))"#
                .to_owned(),
            Default::default(),
            &mut mock,
        )
        .expect("define-command! must succeed");

    ed.register_steel_cmds(defs);
    ed.scripting = Some(host);

    // Bind the Steel command to an unoccupied key (F2) in Normal mode.
    let f2 = crossterm::event::KeyEvent::new(KeyCode::F(2), crossterm::event::KeyModifiers::NONE);
    ed.state.keymap.bind_user_with_extend(
        BindMode::Normal,
        &[f2],
        "steel-dot-repeat".into(),
        false,
    );

    // Establish last_repeatable_action="delete" via a real keypress.
    ed.feed_key(key('d')); // delete "foo"; buffer = " bar\n"
    assert_eq!(
        ed.doc().text().to_string(), " bar\n",
        "setup: 'foo' must be deleted"
    );
    assert!(
        ed.state.last_repeatable_action.is_some(),
        "setup: last_repeatable_action must be set"
    );

    // Move to "bar" and press F2 — the Steel command fires `(call! "repeat-last-action")`,
    // setting pending_repeat during eval; handle_key tail drains it, deleting "bar".
    ed.feed_key(key('w'));
    ed.feed_key(f2);

    // "foo" was deleted by the initial `d` (leaving " bar\n"); "bar" was deleted by
    // the Steel repeat — leaving only the original space before "bar".
    assert_eq!(
        ed.doc().text().to_string(), " \n",
        "Steel (call! \"repeat-last-action\") must replay the delete via handle_key drain"
    );
}

// ── Agreement test ────────────────────────────────────────────────────────────

/// All three native-classification sites must agree for every registered command:
/// `MappableCommand::is_native()`, `command_is_native()`, and the
/// `native_mappable_names()` set must all return the same true/false answer.
///
/// The independent oracle is a hand-written exhaustive match (no `_`) inside
/// this test, re-stating the four-variant native set from first principles.
/// If any of the three sites diverges from the oracle, this test fails.
///
/// Fail oracle: flip one variant to `false` in the oracle closure — the test
/// fails for every registered command of that variant type, proving all three
/// sites are actually checked.
#[test]
fn classification_sites_all_agree() {
    use std::collections::HashSet;
    use crate::editor::registry::MappableCommand;

    let mut ed = editor_from("-[a]>\n");

    // Independent oracle: exhaustive match, no `_`.
    // A new MappableCommand variant is a compile error here too.
    let oracle = |cmd: &MappableCommand| -> bool {
        match cmd {
            MappableCommand::Motion { .. }
            | MappableCommand::Selection { .. }
            | MappableCommand::Edit { .. }
            | MappableCommand::EditorCmd { .. } => true,
            MappableCommand::SteelBacked { .. }
            | MappableCommand::Lazy { .. } => false,
        }
    };

    // Phase 1: collect (name, is_native(), oracle()) while holding an immutable
    // registry borrow. Separating phases avoids borrow conflicts with `live_host!`.
    let triples: Vec<(String, bool, bool)> = ed.state.registry.names()
        .filter_map(|name| {
            ed.state.registry
                .get_mappable(name)
                .map(|cmd| (name.to_owned(), cmd.is_native(), oracle(cmd)))
        })
        .collect();

    assert!(!triples.is_empty(), "registry must have at least one mappable command");

    let native_names: HashSet<String> =
        ed.state.registry.native_mappable_names().map(str::to_owned).collect();

    // Phase 2: is_native() vs oracle, native_mappable_names() vs oracle.
    for (name, is_nat, expected) in &triples {
        assert_eq!(
            *is_nat, *expected,
            "is_native() disagrees with oracle for '{name}'"
        );
        assert_eq!(
            native_names.contains(name.as_str()), *expected,
            "native_mappable_names() membership disagrees with oracle for '{name}'"
        );
    }

    // Phase 3: command_is_native() via the live host. Registry borrow is gone.
    for (name, _, expected) in &triples {
        let host = live_host!(ed);
        assert_eq!(
            host.command_is_native(name).expect("must be registered"),
            *expected,
            "command_is_native() disagrees with oracle for '{name}'"
        );
    }
}

// ── Bookkeeping regression tests (findings 1–5) ───────────────────────────────
//
// Each test verifies that a native command dispatched from Steel via (call!)
// produces the same bookkeeping as a direct keypress on the same command.
// All nine tests must FAIL if you revert to the old run_command_sync body that
// omitted the bookkeeping (flip the assertion to confirm).

/// **Finding 1 — register prefix**: `(set-register-prefix! "a") (call! "yank")` must
/// route the yank to named register `a`, not to the kill ring or clipboard.
///
/// Fail oracle: comment out `register: ctx.current_register_prefix` in
/// `run_command_sync` → register 'a' is empty after the call.
#[test]
fn steel_call_native_respects_register_prefix() {
    let mut ed = editor_from("-[hello]>\n");

    let names: Vec<String> = ed.state.registry.native_mappable_names().map(str::to_owned).collect();
    let name_refs: Vec<&str> = names.iter().map(String::as_str).collect();
    let mut host = ScriptingHost::new();
    host.register_command_names(&name_refs);

    let mut mock = MockHost::new();
    let defs = host
        .eval_source_returning_defs(
            // Register '0' is a valid named storage register (digit registers: 0–9).
            r#"(define-command! "yank-to-0" ""
                 (lambda ()
                   (set-register-prefix! "0")
                   (call! "yank")))"#
                .to_owned(),
            Default::default(),
            &mut mock,
        )
        .expect("define-command! must succeed");

    ed.register_steel_cmds(defs);
    ed.scripting = Some(host);
    ed.execute_keymap_command("yank-to-0".into(), 1, false, vec![]);

    // Register '0' must hold "hello" (the selection content).
    let contents: Vec<String> = ed.state.registers
        .read('0')
        .and_then(|r| r.as_text())
        .map(|s| s.to_vec())
        .unwrap_or_default();
    assert!(
        !contents.is_empty() && contents[0].contains("hello"),
        "register '0' must hold yanked text 'hello'; got {:?}", contents
    );
}

/// **Finding 2 — last_command (smart-p)**: after `(call! "delete")` from Steel,
/// `last_command` must be `"delete"` so a subsequent `p` reads the kill ring.
///
/// Fail oracle: comment out `state.last_command = Some(name)` in `dispatch_native`
/// → last_command stays stale (prior command) instead of "delete".
#[test]
fn steel_call_delete_sets_last_command_for_smart_p() {
    // Buffer: "foo bar\n", cursor on "foo".
    let mut ed = editor_from("-[foo]> bar\n");

    let names: Vec<String> = ed.state.registry.native_mappable_names().map(str::to_owned).collect();
    let name_refs: Vec<&str> = names.iter().map(String::as_str).collect();
    let mut host = ScriptingHost::new();
    host.register_command_names(&name_refs);

    let mut mock = MockHost::new();
    let defs = host
        .eval_source_returning_defs(
            r#"(define-command! "steel-delete" ""
                 (lambda () (call! "delete")))"#
                .to_owned(),
            Default::default(),
            &mut mock,
        )
        .expect("define-command! must succeed");

    ed.register_steel_cmds(defs);
    ed.scripting = Some(host);

    // Execute the Steel delete.
    ed.execute_keymap_command("steel-delete".into(), 1, false, vec![]);
    // last_command must be "delete" so smart-p reads the kill ring.
    assert_eq!(
        ed.state.last_command.as_deref(), Some("delete"),
        "last_command must be 'delete' after Steel (call! \"delete\")"
    );
}

/// **Regression: no-dispatch SteelBacked must stamp its own name, not stay stale.**
///
/// A SteelBacked command that runs no inner native command must overwrite
/// `last_command` with its own name. If it does not, a prior "delete" (in
/// `SMART_P_LAST_CMDS`) stays as `last_command` and a subsequent `p` wrongly
/// pastes from the kill ring instead of the clipboard.
///
/// Fail oracle: remove the pre-stamp `self.state.last_command = Some(name.clone())`
/// from `execute_keymap_command` → last_command stays "delete" → test fails.
#[test]
fn steel_no_dispatch_cmd_stamps_own_name() {
    let mut ed = editor_from("-[f]>oo bar\n");

    let names: Vec<String> = ed.state.registry.native_mappable_names().map(str::to_owned).collect();
    let name_refs: Vec<&str> = names.iter().map(String::as_str).collect();
    let mut host = ScriptingHost::new();
    host.register_command_names(&name_refs);

    let mut mock = MockHost::new();
    // "noop-cmd" dispatches no inner native — no (call! …) anywhere.
    let defs = host
        .eval_source_returning_defs(
            r#"(define-command! "noop-cmd" "" (lambda () (+ 1 0)))"#.to_owned(),
            Default::default(),
            &mut mock,
        )
        .expect("define-command! must succeed");

    ed.register_steel_cmds(defs);
    ed.scripting = Some(host);

    // First: run a kill command to set last_command = "delete".
    ed.execute_keymap_command("delete".into(), 1, false, vec![]);
    assert_eq!(ed.state.last_command.as_deref(), Some("delete"), "pre-condition");

    // Now run the no-dispatch Steel command — must overwrite last_command.
    ed.execute_keymap_command("noop-cmd".into(), 1, false, vec![]);
    assert_eq!(
        ed.state.last_command.as_deref(), Some("noop-cmd"),
        "last_command must be 'noop-cmd' after a no-dispatch SteelBacked command"
    );
}

/// **Finding 3 — dot-repeat**: a repeatable native command invoked via Steel must
/// set `last_repeatable_action` so `.` can replay it.
///
/// Fail oracle: comment out the `is_repeatable` block in `dispatch_native`
/// → `last_repeatable_action` is None after the call.
#[test]
fn steel_call_repeatable_cmd_sets_dot_repeat() {
    let mut ed = editor_from("-[foo]> bar\n");

    let names: Vec<String> = ed.state.registry.native_mappable_names().map(str::to_owned).collect();
    let name_refs: Vec<&str> = names.iter().map(String::as_str).collect();
    let mut host = ScriptingHost::new();
    host.register_command_names(&name_refs);

    let mut mock = MockHost::new();
    let defs = host
        .eval_source_returning_defs(
            r#"(define-command! "steel-delete" ""
                 (lambda () (call! "delete")))"#
                .to_owned(),
            Default::default(),
            &mut mock,
        )
        .expect("define-command! must succeed");

    ed.register_steel_cmds(defs);
    ed.scripting = Some(host);

    ed.execute_keymap_command("steel-delete".into(), 1, false, vec![]);
    // `delete` is repeatable — last_repeatable_action must be set.
    assert!(
        ed.state.last_repeatable_action.is_some(),
        "last_repeatable_action must be set after Steel (call! \"delete\")"
    );
    assert_eq!(
        ed.state.last_repeatable_action.as_ref().map(|a| a.command.as_ref()),
        Some("delete"),
    );

    // Now press `.` — must replay the delete.
    ed.feed_key(key('w')); // move to "bar"
    let buf_before = ed.doc().text().to_string();
    ed.feed_key(key('.')); // dot-repeat
    let buf_after = ed.doc().text().to_string();
    assert_ne!(buf_before, buf_after, "dot-repeat must apply the delete again");
}

/// **Finding 4 — jump list**: an explicit-jump EditorCmd (`goto-last-line`) invoked
/// via Steel must push a `JumpEntry` so Ctrl+O can return.
///
/// Fail oracle: comment out the `pre_jump` capture in `dispatch_native`
/// → jump list is empty after the call.
#[test]
fn steel_call_jump_cmd_records_jump_entry() {
    // 10-line buffer so goto-last-line causes a large line delta.
    let content = "-[l]>ine1\nline2\nline3\nline4\nline5\nline6\nline7\nline8\nline9\nline10\n";
    let mut ed = editor_from(content);

    let names: Vec<String> = ed.state.registry.native_mappable_names().map(str::to_owned).collect();
    let name_refs: Vec<&str> = names.iter().map(String::as_str).collect();
    let mut host = ScriptingHost::new();
    host.register_command_names(&name_refs);

    let mut mock = MockHost::new();
    let defs = host
        .eval_source_returning_defs(
            r#"(define-command! "steel-goto-end" ""
                 (lambda () (call! "goto-last-line")))"#
                .to_owned(),
            Default::default(),
            &mut mock,
        )
        .expect("define-command! must succeed");

    ed.register_steel_cmds(defs);
    ed.scripting = Some(host);

    let pane_id = ed.state.focused_pane_id;
    let bid = ed.focused_buffer_id();
    // Fresh editor: no jump entries yet.
    let had_entries_before = ed.state.panes.jumps[pane_id].entries_for_buffer(bid);
    ed.execute_keymap_command("steel-goto-end".into(), 1, false, vec![]);
    let has_entries_after = ed.state.panes.jumps[pane_id].entries_for_buffer(bid);
    assert!(
        !had_entries_before && has_entries_after,
        "jump list must gain entries after Steel (call! \"goto-last-line\")"
    );
}

/// **Finding 5 — paste session**: `(call! "paste-after")` followed by
/// `(call! "move-down")` in one body must commit the paste session so that
/// one undo step reverts the paste cleanly.
///
/// Fail oracle: remove `state.commit_paste_session()` from `dispatch_native`
/// → after undo, the paste text is still present.
#[test]
fn steel_call_paste_then_motion_commits_paste_session() {
    // Seed the kill ring with "hello" so paste-after has something to paste.
    let mut ed = editor_from("-[w]>orld\n");
    ed.state.kill_ring.push(vec!["hello".to_owned()]);

    // Prime last_command = "delete" so smart-p reads from kill ring.
    use std::borrow::Cow;
    ed.state.last_command = Some(Cow::Borrowed("delete"));

    let names: Vec<String> = ed.state.registry.native_mappable_names().map(str::to_owned).collect();
    let name_refs: Vec<&str> = names.iter().map(String::as_str).collect();
    let mut host = ScriptingHost::new();
    host.register_command_names(&name_refs);

    let mut mock = MockHost::new();
    let defs = host
        .eval_source_returning_defs(
            r#"(define-command! "paste-and-move" ""
                 (lambda ()
                   (call! "paste-after")
                   (call! "move-down")))"#
                .to_owned(),
            Default::default(),
            &mut mock,
        )
        .expect("define-command! must succeed");

    ed.register_steel_cmds(defs);
    ed.scripting = Some(host);
    ed.execute_keymap_command("paste-and-move".into(), 1, false, vec![]);

    let buf_after_paste = ed.doc().text().to_string();
    assert!(
        buf_after_paste.contains("hello"),
        "paste must have inserted 'hello'; got: {buf_after_paste:?}"
    );

    // Undo — the paste session must be committed so this single undo removes the paste.
    ed.feed_key(key('u'));
    let buf_after_undo = ed.doc().text().to_string();
    assert!(
        !buf_after_undo.contains("hello"),
        "undo must revert the paste; 'hello' still present in: {buf_after_undo:?}"
    );
}

/// **Finding 7 — source order**: a Steel body `(call! my-steel-cmd) (call! "delete")`
/// must execute the Steel command first, then the delete — not reversed.
///
/// Fail oracle: remove the `cmd_queue.is_empty()` guard in `call_command_primitive`
/// → `delete` runs before `my-steel-cmd`.
#[test]
fn steel_call_source_order_native_after_steel() {
    // Buffer "ab\n", cursor on 'a'. The Steel command moves right; delete then
    // deletes 'b'. If order were reversed, 'a' (not 'b') would be deleted.
    let mut ed = editor_from("-[a]>b\n");

    let names: Vec<String> = ed.state.registry.native_mappable_names().map(str::to_owned).collect();
    let name_refs: Vec<&str> = names.iter().map(String::as_str).collect();
    let mut host = ScriptingHost::new();
    host.register_command_names(&name_refs);

    let mut mock = MockHost::new();
    let defs = host
        .eval_source_returning_defs(
            r#"(define-command! "steel-move-right" ""
                 (lambda () (call! "move-right")))
               (define-command! "order-test" ""
                 (lambda ()
                   (call! "steel-move-right")
                   (call! "delete")))"#
                .to_owned(),
            Default::default(),
            &mut mock,
        )
        .expect("define-command! must succeed");

    ed.register_steel_cmds(defs);
    ed.scripting = Some(host);
    ed.execute_keymap_command("order-test".into(), 1, false, vec![]);

    // If correct order (move-right then delete): 'b' is deleted → "a\n".
    // If reversed (delete then move-right): 'a' is deleted → "b\n".
    assert_eq!(
        ed.doc().text().to_string(), "a\n",
        "source order: move-right must run before delete, leaving 'a'"
    );
}

/// **Finding 7 — deferred native count**: a native command deferred for ordering
/// must use its own count, not the outer `count` from `drain_command_queue`.
///
/// Fail oracle: remove the per-entry `parse_count_extend` in `drain_command_queue`
/// and always use the outer count → cursor moves 1 instead of 3.
#[test]
fn steel_deferred_native_uses_own_count() {
    // Buffer with at least 5 lines; cursor starts at line 1.
    let content = "-[a]>\nb\nc\nd\ne\n";
    let mut ed = editor_from(content);

    let names: Vec<String> = ed.state.registry.native_mappable_names().map(str::to_owned).collect();
    let name_refs: Vec<&str> = names.iter().map(String::as_str).collect();
    let mut host = ScriptingHost::new();
    host.register_command_names(&name_refs);

    let mut mock = MockHost::new();
    // The body queues a Steel-defined cmd first (forces native to defer),
    // then (call! "move-down" 3).
    let defs = host
        .eval_source_returning_defs(
            r#"(define-command! "noop-steel" "" (lambda () #t))
               (define-command! "deferred-count-test" ""
                 (lambda ()
                   (call! "noop-steel")
                   (call! "move-down" 3)))"#
                .to_owned(),
            Default::default(),
            &mut mock,
        )
        .expect("define-command! must succeed");

    ed.register_steel_cmds(defs);
    ed.scripting = Some(host);
    ed.execute_keymap_command("deferred-count-test".into(), 1, false, vec![]);

    let host = live_host!(ed);
    let line = host.current_line_number().expect("current_line_number");
    // Started on line 1, moved down 3 → should be on line 4.
    assert_eq!(line, 4, "deferred native must use its own count=3; got line {line}");
}

/// **Finding 8 — unknown warns, no abort**: a body with an unknown command between
/// two valid moves must execute both valid moves, not abort on the typo.
///
/// Fail oracle: reinstate `Err(e) => steel::stop!` in `call_command_primitive`
/// → the second move-right never runs.
#[test]
fn steel_unknown_cmd_warns_and_continues() {
    // "-[a]>bc\n", cursor at 0. Two moves should bring cursor to 2.
    let mut ed = editor_from("-[a]>bc\n");

    let names: Vec<String> = ed.state.registry.native_mappable_names().map(str::to_owned).collect();
    let name_refs: Vec<&str> = names.iter().map(String::as_str).collect();
    let mut host = ScriptingHost::new();
    host.register_command_names(&name_refs);

    let mut mock = MockHost::new();
    let defs = host
        .eval_source_returning_defs(
            r#"(define-command! "warn-test" ""
                 (lambda ()
                   (call! "move-right")
                   (call! "this-command-does-not-exist")
                   (call! "move-right")))"#
                .to_owned(),
            Default::default(),
            &mut mock,
        )
        .expect("define-command! must succeed");

    ed.register_steel_cmds(defs);
    ed.scripting = Some(host);
    ed.execute_keymap_command("warn-test".into(), 1, false, vec![]);

    let idx = live_host!(ed).cursor_char_index().expect("cursor_char_index");
    // Both move-rights deferred after the unknown name — cursor ends at 2.
    assert_eq!(idx, 2, "both moves must execute despite unknown command in between; got {idx}");
}

/// **Finding 6 — mouse hooks drain immediately**: an `OnModeChange` hook registered
/// before a left-click-in-Insert must fire on the click, not be deferred.
///
/// Fail oracle: remove `self.drain_hooks()` from `handle_mouse`
/// → `pending_hooks` is non-empty after the click (handler never ran).
#[test]
fn mouse_click_drains_hooks_immediately() {
    use crate::editor::scripting_setup::make_init_host;

    let mut ed = Editor::for_testing(crate::editor::buffer::Buffer::new(
        hume_editing::text::Text::from("hello\n"),
        hume_editing::selection::SelectionSet::default(),
    ));

    // Give the pane a viewport big enough that a click at row=0,col=0 lands in content.
    ed.view.panes[ed.state.focused_pane_id].viewport = hume_engine::pane::ViewportState::new(80, 24);

    let mut host_scr = hume_scripting::ScriptingHost::new();
    let names: Vec<String> = ed.state.registry.native_mappable_names().map(str::to_owned).collect();
    let name_refs: Vec<&str> = names.iter().map(String::as_str).collect();
    host_scr.register_command_names(&name_refs);

    // Register an OnModeChange hook that pushes a mode to a shared buffer so we can observe it.
    {
        let mut init_host = make_init_host(&mut ed.state, &mut ed.view);
        host_scr
            .eval_source(
                r#"(register-hook! 'on-mode-change (lambda (old new) #t))"#,
                &mut init_host,
            )
            .expect("register-hook! must succeed");
    }
    ed.scripting = Some(host_scr);

    // Enter Insert mode via a real keypress so begin_insert_session opens an
    // edit group. Directly setting state.mode skips that and causes
    // end_insert_session to panic on commit_edit_group.
    ed.feed_key(key('i'));

    // Simulate a left-click at (0, 0) — triggers mouse_left_down → set_mode → hook queued.
    let click = crossterm::event::MouseEvent {
        kind: crossterm::event::MouseEventKind::Down(crossterm::event::MouseButton::Left),
        column: 0,
        row: 0,
        modifiers: crossterm::event::KeyModifiers::NONE,
    };
    ed.handle_mouse(click);

    // After handle_mouse, pending_hooks must be empty — drain_hooks ran.
    assert!(
        ed.state.pending_hooks.is_empty(),
        "pending_hooks must be empty after handle_mouse; got {:?}", ed.state.pending_hooks
    );
    // Mode must be Normal (the click exited Insert).
    assert_eq!(ed.state.mode, crate::editor::Mode::Normal);
}

// ── Dual-path parity tests ────────────────────────────────────────────────────
//
// The original regression: `run_command_sync` executed native commands naked —
// cursor moved correctly but the bookkeeping cluster (jump list, last_command,
// dot-repeat, paste-session commit) was silently dropped.  These tests assert
// that dispatching the same native command via the keypress path AND via a Steel
// `(call! …)` wrapper leaves IDENTICAL `BookkeepingSnapshot` state.
//
// Each test documents a fail oracle: which single line in `dispatch_native`
// (commands/mod.rs) to revert to confirm the assertion breaks on that field.

/// **Parity: repeatable edit** — `delete` dispatched via keypress vs via Steel
/// `(call! "delete")` must produce the same `last_command` and `last_repeatable`.
///
/// Fail oracle (last_command): comment out `state.last_command = Some(name)` at
///   commands/mod.rs:221 → snap_steel.last_command is None; assertion fails.
/// Fail oracle (last_repeatable): comment out the `if is_repeatable { … }` block
///   at commands/mod.rs:210–217 → snap_steel.last_repeatable is None; assertion fails.
#[test]
fn parity_delete_bookkeeping_keypress_vs_steel() {
    // Path A — keypress.
    let mut ed_key = editor_from("-[f]>oo\n");
    let before_key = snapshot_bookkeeping(&ed_key);
    ed_key.execute_keymap_command("delete".into(), 1, false, vec![]);
    let snap_key = snapshot_bookkeeping(&ed_key);

    // Path B — Steel (call! "delete").
    let mut ed_steel = editor_from("-[f]>oo\n");
    let names: Vec<String> = ed_steel.state.registry.native_mappable_names().map(str::to_owned).collect();
    let name_refs: Vec<&str> = names.iter().map(String::as_str).collect();
    let mut host = ScriptingHost::new();
    host.register_command_names(&name_refs);
    let mut mock = MockHost::new();
    let defs = host
        .eval_source_returning_defs(
            r#"(define-command! "steel-delete" "" (lambda () (call! "delete")))"#.to_owned(),
            Default::default(),
            &mut mock,
        )
        .expect("define-command! must succeed");
    ed_steel.register_steel_cmds(defs);
    ed_steel.scripting = Some(host);
    let before_steel = snapshot_bookkeeping(&ed_steel);
    ed_steel.execute_keymap_command("steel-delete".into(), 1, false, vec![]);
    let snap_steel = snapshot_bookkeeping(&ed_steel);

    // Pre-conditions: both editors start from identical bookkeeping state.
    assert_eq!(before_key, before_steel, "pre-condition: both editors must start identical");

    // Both paths must produce the same funnel-owned bookkeeping.
    assert_eq!(
        snap_key, snap_steel,
        "keypress vs Steel dispatch of 'delete' must leave identical bookkeeping"
    );
}

/// **Parity: explicit jump command** — `goto-last-line` dispatched via keypress vs
/// via Steel `(call! "goto-last-line")` must push the same number of jump entries.
///
/// Fail oracle (jump_len): comment out the `pre_jump` / jump-list push block
///   at commands/mod.rs:157–207 → snap_steel.jump_len stays 0; assertion fails.
#[test]
fn parity_jump_bookkeeping_keypress_vs_steel() {
    let content = "-[l]>ine1\nline2\nline3\nline4\nline5\nline6\nline7\nline8\nline9\nline10\n";

    // Path A — keypress.
    let mut ed_key = editor_from(content);
    ed_key.execute_keymap_command("goto-last-line".into(), 1, false, vec![]);
    let snap_key = snapshot_bookkeeping(&ed_key);

    // Path B — Steel (call! "goto-last-line").
    let mut ed_steel = editor_from(content);
    let names: Vec<String> = ed_steel.state.registry.native_mappable_names().map(str::to_owned).collect();
    let name_refs: Vec<&str> = names.iter().map(String::as_str).collect();
    let mut host = ScriptingHost::new();
    host.register_command_names(&name_refs);
    let mut mock = MockHost::new();
    let defs = host
        .eval_source_returning_defs(
            r#"(define-command! "steel-goto-end" "" (lambda () (call! "goto-last-line")))"#.to_owned(),
            Default::default(),
            &mut mock,
        )
        .expect("define-command! must succeed");
    ed_steel.register_steel_cmds(defs);
    ed_steel.scripting = Some(host);
    ed_steel.execute_keymap_command("steel-goto-end".into(), 1, false, vec![]);
    let snap_steel = snapshot_bookkeeping(&ed_steel);

    assert_eq!(
        snap_key, snap_steel,
        "keypress vs Steel dispatch of 'goto-last-line' must leave identical jump bookkeeping"
    );
}
