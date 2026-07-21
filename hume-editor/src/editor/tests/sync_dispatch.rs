use super::*;

use crate::editor::dispatch::ArgSource;
use crate::editor::host_impl::EditorHostImpl;
use crate::testing::MockHost;
use hume_scripting::ScriptingHost;
use hume_scripting::host::{CommandHost, CursorHost};

// ── Unit tests: run_command_sync ──────────────────────────────────────────────

/// `run_command_sync` for a `Motion` command must immediately update the cursor
/// position — no queue involved.
#[test]
fn run_command_sync_motion_moves_cursor() {
    // "-[a]>bc\n" — cursor at position 0.
    let mut ed = editor_from("-[a]>bc\n");
    assert_eq!(state(&ed), "-[a]>bc\n", "cursor must start at 0");

    {
        let mut host = live_host!(ed);
        // move-right is a Motion — must dispatch synchronously.
        host.run_command_sync("move-right", Some(1), false, None)
            .expect("run_command_sync must not error for move-right");
    }

    assert_eq!(
        state(&ed),
        "a-[b]>c\n",
        "cursor must be at 1 after sync move-right"
    );
}

/// `run_command_sync` for an `EditorCmd` (the fourth native variant) must apply
/// its effect immediately — not queue it.
///
/// Fail oracle: return `Ok(())` from `run_command_sync` without calling
/// `run_dispatch_pipeline` → `undo` never reverts the deletion → assertion fails.
#[test]
fn run_command_sync_editor_cmd_runs_sync() {
    // Buffer "abc\n", selection on 'a'. Delete it via the normal keymap path to
    // create an undoable revision, then undo via run_command_sync.
    let mut ed = editor_from("-[a]>bc\n");
    ed.execute_keymap_command("delete".into(), Some(1), false, ArgSource::Keymap);
    // Buffer is now "bc\n"; cursor should be at 0.
    assert_eq!(
        state(&ed),
        "-[b]>c\n",
        "pre-condition: cursor at 0 after delete"
    );

    {
        let mut host = live_host!(ed);
        host.run_command_sync("undo", Some(1), false, None)
            .expect("run_command_sync must not error for undo");
    }

    // After undo the deleted 'a' must be restored and cursor back at 0 on "abc\n".
    assert_eq!(
        state(&ed),
        "-[a]>bc\n",
        "undo via run_command_sync must restore the deleted character with cursor at 0"
    );
}

/// `run_command_sync` for an unknown name must return `Err`.
#[test]
fn run_command_sync_unknown_name_errors() {
    let mut ed = editor_from("-[a]>bc\n");
    let mut host = live_host!(ed);
    let result = host.run_command_sync("no-such-command-xyzzy", Some(1), false, None);
    assert!(result.is_err(), "unknown command must return Err");
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
    {
        live_host!(ed)
            .run_command_sync("move-down", Some(1), false, None)
            .unwrap();
    }

    let after = live_host!(ed).current_line_number().expect("line after");
    assert_eq!(
        after, 2,
        "current_line_number must reflect the sync move to line 2"
    );
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
        host.run_command_sync("select-line", Some(1), false, None)
            .expect("run_command_sync must not error for select-line");
    }
    // select-line covers the full line "abc\n" (inclusive); head lands on '\n' at position 3.
    assert_eq!(
        state(&ed),
        "-[abc\n]>",
        "select-line must cover the full line with head on '\\n' (inclusive selection)"
    );
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

    let names: Vec<String> = ed
        .state
        .registry
        .native_mappable_names()
        .map(str::to_owned)
        .collect();
    let name_refs: Vec<&str> = names.iter().map(String::as_str).collect();

    let mut host = ScriptingHost::new();
    host.register_command_names(&name_refs);

    let mut init_host = EditorHostImpl::new(&mut ed.state, &mut ed.view);
    host.eval_source_returning_defs(
        r#"(define-command! "move-right-5" "" (lambda () (call! "move-right" 5)))"#.to_owned(),
        Default::default(),
        &mut init_host,
    )
    .expect("define-command! must succeed");

    ed.scripting = Some(host);
    ed.execute_keymap_command("move-right-5".into(), Some(1), false, ArgSource::Keymap);

    let idx = ed
        .state
        .panes
        .state
        .get(ed.state.focused_pane_id)
        .unwrap()
        .values()
        .next()
        .unwrap()
        .selections
        .primary()
        .head();
    assert_eq!(
        idx, 5,
        "cursor must be at position 5 after (call! \"move-right\" 5)"
    );
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

    let names: Vec<String> = ed
        .state
        .registry
        .native_mappable_names()
        .map(str::to_owned)
        .collect();
    let name_refs: Vec<&str> = names.iter().map(String::as_str).collect();

    let mut host = ScriptingHost::new();
    host.register_command_names(&name_refs);

    let mut init_host = EditorHostImpl::new(&mut ed.state, &mut ed.view);
    host.eval_source_returning_defs(
        r#"(define-command! "move-right-bad" "" (lambda () (call! "move-right" "garbage")))"#
            .to_owned(),
        Default::default(),
        &mut init_host,
    )
    .expect("define-command! must succeed");

    ed.scripting = Some(host);

    // execute_keymap_command reports errors to the status bar rather than panicking;
    // check the cursor did not move (the Steel error aborted the eval).
    let before = state(&ed);
    ed.execute_keymap_command("move-right-bad".into(), Some(1), false, ArgSource::Keymap);
    let after = state(&ed);
    assert_eq!(
        before, after,
        "cursor must not move when native arg is malformed"
    );
}

// ── Case B integration test ───────────────────────────────────────────────────

/// **Case B** — a Steel function can observe the effect of `(move-down)` in
/// the same eval via `(current-line-number)`.
///
/// The discriminating logic:
/// - Start on line 1.  Call `(move-down)`.
/// - Cursor is immediately on line 2, so the `(when (= (current-line-number) 2) ...)`
///   arm fires and calls `(move-down)` a second time → final line 3.
///
/// Fail oracle: if dispatch defers commands → cursor lands on line 2 instead of 3.
#[test]
fn case_b_sync_cursor_read_reflects_motion() {
    // "-[a]>\nb\nc\n" — cursor on line 1.
    let mut ed = editor_from("-[a]>\nb\nc\n");

    // Pre-register native command names as Steel bindings so `(move-down)` etc.
    // resolve at compile time.
    let names: Vec<String> = ed
        .state
        .registry
        .native_mappable_names()
        .map(str::to_owned)
        .collect();
    let name_refs: Vec<&str> = names.iter().map(String::as_str).collect();

    let mut host = ScriptingHost::new();
    host.register_command_names(&name_refs);

    // Define a command that exercises the sync-read property.
    let mut init_host = EditorHostImpl::new(&mut ed.state, &mut ed.view);
    host.eval_source_returning_defs(
        r#"(define-command! "test-case-b" "Case B probe"
                 (lambda ()
                   (move-down)
                   (when (= (current-line-number) 2)
                     (move-down))))"#
            .to_owned(),
        Default::default(),
        &mut init_host,
    )
    .expect("define-command! must succeed");

    ed.scripting = Some(host);

    ed.execute_keymap_command("test-case-b".into(), Some(1), false, ArgSource::Keymap);

    let final_state = state(&ed);
    // Both moves ran inside the lambda → cursor on line 3, "a\nb\n-[c]>\n".
    // Fail oracle: if dispatch defers → cursor on line 2, "a\n-[b]>\nc\n".
    assert_eq!(
        final_state, "a\nb\n-[c]>\n",
        "sync dispatch: (current-line-number) must reflect (move-down) effect within same eval"
    );
}

// ── Steel deferred dot-repeat ────────────────────────────────────────────────

/// A Steel command that calls `(call! "repeat-last-action")` must actually replay
/// the last editing action when its key is pressed.
///
/// `(call! "repeat-last-action")` is a sync EditorCmd dispatch: it calls
/// `run_command_sync("repeat-last-action")`, which runs `cmd_repeat` and sets
/// `state.pending_repeat`. The replay then fires in `replay_dot` at the
/// tail of the enclosing `handle_key` call — NOT during the Steel eval.
///
/// The test drives the key through `feed_key` so the full `handle_key` tail
/// (including `replay_dot`) executes before we inspect the buffer.
///
/// Fail oracle: if `replay_dot` were not called at `handle_key`'s tail,
/// `pending_repeat` would be set but never consumed, and the buffer would be
/// unchanged after pressing the Steel key.
#[test]
fn steel_call_repeat_last_action_drains_via_handle_key() {
    use crate::editor::keymap::BindMode;
    use termina::event::KeyCode;

    let mut ed = editor_from("-[foo]> bar\n");

    // Register command names so `(call! "repeat-last-action")` resolves.
    let names: Vec<String> = ed
        .state
        .registry
        .native_mappable_names()
        .map(str::to_owned)
        .collect();
    let name_refs: Vec<&str> = names.iter().map(String::as_str).collect();

    let mut host = ScriptingHost::new();
    host.register_command_names(&name_refs);

    let mut init_host = EditorHostImpl::new(&mut ed.state, &mut ed.view);
    host.eval_source_returning_defs(
        r#"(define-command! "steel-dot-repeat" "Repeat last action via Steel"
                 (lambda () (call! "repeat-last-action")))"#
            .to_owned(),
        Default::default(),
        &mut init_host,
    )
    .expect("define-command! must succeed");

    ed.scripting = Some(host);

    // Bind the Steel command to an unoccupied key (F2) in Normal mode.
    let f2 = termina::event::KeyEvent::new(KeyCode::Function(2), termina::event::Modifiers::NONE);
    ed.state.keymap.bind_user_with_extend(
        BindMode::Normal,
        &[f2],
        "steel-dot-repeat".into(),
        false,
    );

    // Establish last_repeatable_action="delete" via a real keypress.
    ed.feed_key(key('d')); // delete "foo"; buffer = " bar\n"
    assert_eq!(
        ed.doc().text().to_string(),
        " bar\n",
        "setup: 'foo' must be deleted"
    );
    assert!(
        ed.state.last_repeatable_action.is_some(),
        "setup: last_repeatable_action must be set"
    );

    // Move to "bar" and press F2 — the Steel command fires `(call! "repeat-last-action")`,
    // setting pending_repeat during eval; handle_key tail drains it, deleting the
    // selection. "bar" is the first (and only) word on its line, so its leading
    // space is indentation and is never absorbed, and there's no trailing space
    // either (EOL follows) — `w` picks up bare "bar" (default around-word).
    ed.feed_key(key('w'));
    ed.feed_key(f2);

    // "foo" was deleted by the initial `d` (leaving " bar\n"); "bar" was deleted
    // by the Steel repeat — leaving the indentation space, " \n".
    assert_eq!(
        ed.doc().text().to_string(),
        " \n",
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
    use crate::editor::registry::MappableCommand;
    use std::collections::HashSet;

    let mut ed = editor_from("-[a]>\n");

    // Independent oracle: exhaustive match, no `_`.
    // A new MappableCommand variant is a compile error here too.
    let oracle = |cmd: &MappableCommand| -> bool {
        match cmd {
            MappableCommand::Motion { .. }
            | MappableCommand::Selection { .. }
            | MappableCommand::Edit { .. }
            | MappableCommand::EditorCmd { .. } => true,
            MappableCommand::SteelBacked { .. } | MappableCommand::Lazy { .. } => false,
        }
    };

    // Phase 1: collect (name, is_native(), oracle()) while holding an immutable
    // registry borrow. Separating phases avoids borrow conflicts with `live_host!`.
    let triples: Vec<(String, bool, bool)> = ed
        .state
        .registry
        .names()
        .filter_map(|name| {
            ed.state
                .registry
                .get_mappable(name)
                .map(|cmd| (name.to_owned(), cmd.is_native(), oracle(cmd)))
        })
        .collect();

    assert!(
        !triples.is_empty(),
        "registry must have at least one mappable command"
    );

    let native_names: HashSet<String> = ed
        .state
        .registry
        .native_mappable_names()
        .map(str::to_owned)
        .collect();

    // Phase 2: is_native() vs oracle, native_mappable_names() vs oracle.
    for (name, is_nat, expected) in &triples {
        assert_eq!(
            *is_nat, *expected,
            "is_native() disagrees with oracle for '{name}'"
        );
        assert_eq!(
            native_names.contains(name.as_str()),
            *expected,
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
// Each test verifies that a command dispatched from Steel via (call!)
// produces the same bookkeeping as a direct keypress on the same command.
// Flip any assertion to confirm it catches a regression.

/// **Finding 1 — register prefix**: `(set-register-prefix! "a") (call! "yank")` must
/// route the yank to named register `a`, not to the kill ring or clipboard.
///
/// Fail oracle: comment out `register: ctx.current_register_prefix` in
/// `run_command_sync` → register 'a' is empty after the call.
#[test]
fn steel_call_native_respects_register_prefix() {
    let mut ed = editor_from("-[hello]>\n");

    let names: Vec<String> = ed
        .state
        .registry
        .native_mappable_names()
        .map(str::to_owned)
        .collect();
    let name_refs: Vec<&str> = names.iter().map(String::as_str).collect();
    let mut host = ScriptingHost::new();
    host.register_command_names(&name_refs);

    let mut init_host = EditorHostImpl::new(&mut ed.state, &mut ed.view);
    host.eval_source_returning_defs(
        // Register '0' is a valid named storage register (digit registers: 0–9).
        r#"(define-command! "yank-to-0" ""
                 (lambda ()
                   (set-register-prefix! "0")
                   (call! "yank")))"#
            .to_owned(),
        Default::default(),
        &mut init_host,
    )
    .expect("define-command! must succeed");

    ed.scripting = Some(host);
    ed.execute_keymap_command("yank-to-0".into(), Some(1), false, ArgSource::Keymap);

    // Register '0' must hold "hello" (the selection content).
    let contents: Vec<String> = ed
        .state
        .registers
        .read('0')
        .and_then(|r| r.as_text())
        .map(|s| s.to_vec())
        .unwrap_or_default();
    assert!(
        !contents.is_empty() && contents[0].contains("hello"),
        "register '0' must hold yanked text 'hello'; got {:?}",
        contents
    );
}

/// **Finding 2 — last_command (smart-p)**: after `(call! "delete")` from Steel,
/// `last_command` must be `"delete"` so a subsequent `p` reads the kill ring.
///
/// Fail oracle: comment out `state.last_command = Some(name)` in `step_stamp_last_command`
/// → last_command stays stale (prior command) instead of "delete".
#[test]
fn steel_call_delete_sets_last_command_for_smart_p() {
    // Buffer: "foo bar\n", cursor on "foo".
    let mut ed = editor_from("-[foo]> bar\n");

    let names: Vec<String> = ed
        .state
        .registry
        .native_mappable_names()
        .map(str::to_owned)
        .collect();
    let name_refs: Vec<&str> = names.iter().map(String::as_str).collect();
    let mut host = ScriptingHost::new();
    host.register_command_names(&name_refs);

    let mut init_host = EditorHostImpl::new(&mut ed.state, &mut ed.view);
    host.eval_source_returning_defs(
        r#"(define-command! "steel-delete" ""
                 (lambda () (call! "delete")))"#
            .to_owned(),
        Default::default(),
        &mut init_host,
    )
    .expect("define-command! must succeed");

    ed.scripting = Some(host);

    // Execute the Steel delete.
    ed.execute_keymap_command("steel-delete".into(), Some(1), false, ArgSource::Keymap);
    // last_command must be "delete" so smart-p reads the kill ring.
    assert_eq!(
        ed.state.last_command.as_deref(),
        Some("delete"),
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

    let names: Vec<String> = ed
        .state
        .registry
        .native_mappable_names()
        .map(str::to_owned)
        .collect();
    let name_refs: Vec<&str> = names.iter().map(String::as_str).collect();
    let mut host = ScriptingHost::new();
    host.register_command_names(&name_refs);

    let mut init_host = EditorHostImpl::new(&mut ed.state, &mut ed.view);
    // "noop-cmd" dispatches no inner native — no (call! …) anywhere.
    host.eval_source_returning_defs(
        r#"(define-command! "noop-cmd" "" (lambda () (+ 1 0)))"#.to_owned(),
        Default::default(),
        &mut init_host,
    )
    .expect("define-command! must succeed");

    ed.scripting = Some(host);

    // First: run a kill command to set last_command = "delete".
    ed.execute_keymap_command("delete".into(), Some(1), false, ArgSource::Keymap);
    assert_eq!(
        ed.state.last_command.as_deref(),
        Some("delete"),
        "pre-condition"
    );

    // Now run the no-dispatch Steel command — must overwrite last_command.
    ed.execute_keymap_command("noop-cmd".into(), Some(1), false, ArgSource::Keymap);
    assert_eq!(
        ed.state.last_command.as_deref(),
        Some("noop-cmd"),
        "last_command must be 'noop-cmd' after a no-dispatch SteelBacked command"
    );
}

/// **Finding 3 — dot-repeat**: a repeatable native command invoked via Steel must
/// set `last_repeatable_action` so `.` can replay it.
///
/// Fail oracle: comment out the `step_stamp_repeatable` call in `run_dispatch_pipeline`
/// → `last_repeatable_action` is None after the call.
#[test]
fn steel_call_repeatable_cmd_sets_dot_repeat() {
    let mut ed = editor_from("-[foo]> bar\n");

    let names: Vec<String> = ed
        .state
        .registry
        .native_mappable_names()
        .map(str::to_owned)
        .collect();
    let name_refs: Vec<&str> = names.iter().map(String::as_str).collect();
    let mut host = ScriptingHost::new();
    host.register_command_names(&name_refs);

    let mut init_host = EditorHostImpl::new(&mut ed.state, &mut ed.view);
    host.eval_source_returning_defs(
        r#"(define-command! "steel-delete" ""
                 (lambda () (call! "delete")))"#
            .to_owned(),
        Default::default(),
        &mut init_host,
    )
    .expect("define-command! must succeed");

    ed.scripting = Some(host);

    ed.execute_keymap_command("steel-delete".into(), Some(1), false, ArgSource::Keymap);
    // `delete` is repeatable — last_repeatable_action must be set.
    assert!(
        ed.state.last_repeatable_action.is_some(),
        "last_repeatable_action must be set after Steel (call! \"delete\")"
    );
    assert_eq!(
        ed.state
            .last_repeatable_action
            .as_ref()
            .map(|a| a.command.as_ref()),
        Some("delete"),
    );

    // Now press `.` — must replay the delete.
    ed.feed_key(key('w')); // move to "bar"
    let buf_before = ed.doc().text().to_string();
    ed.feed_key(key('.')); // dot-repeat
    let buf_after = ed.doc().text().to_string();
    assert_ne!(
        buf_before, buf_after,
        "dot-repeat must apply the delete again"
    );
}

/// **Finding 4 — jump list**: an explicit-jump EditorCmd (`goto-last-line`) invoked
/// via Steel must push a `JumpEntry` so Ctrl+O can return.
///
/// Fail oracle: comment out the `step_capture_pre_jump` call in `run_dispatch_pipeline`
/// → jump list is empty after the call.
#[test]
fn steel_call_jump_cmd_records_jump_entry() {
    // 10-line buffer so goto-last-line causes a large line delta.
    let content = "-[l]>ine1\nline2\nline3\nline4\nline5\nline6\nline7\nline8\nline9\nline10\n";
    let mut ed = editor_from(content);

    let names: Vec<String> = ed
        .state
        .registry
        .native_mappable_names()
        .map(str::to_owned)
        .collect();
    let name_refs: Vec<&str> = names.iter().map(String::as_str).collect();
    let mut host = ScriptingHost::new();
    host.register_command_names(&name_refs);

    let mut init_host = EditorHostImpl::new(&mut ed.state, &mut ed.view);
    host.eval_source_returning_defs(
        r#"(define-command! "steel-goto-end" ""
                 (lambda () (call! "goto-last-line")))"#
            .to_owned(),
        Default::default(),
        &mut init_host,
    )
    .expect("define-command! must succeed");

    ed.scripting = Some(host);

    let pane_id = ed.state.focused_pane_id;
    let bid = ed.focused_buffer_id();
    // Fresh editor: no jump entries yet.
    let had_entries_before = ed.state.panes.jumps[pane_id].entries_for_buffer(bid);
    ed.execute_keymap_command("steel-goto-end".into(), Some(1), false, ArgSource::Keymap);
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
/// Fail oracle: remove the `step_paste_commit` call from `run_dispatch_pipeline`
/// → after undo, the paste text is still present.
#[test]
fn steel_call_paste_then_motion_commits_paste_session() {
    // Seed the kill ring with "hello" so paste-after has something to paste.
    let mut ed = editor_from("-[w]>orld\n");
    ed.state.kill_ring.push(vec!["hello".to_owned()]);

    // Prime last_command = "delete" so smart-p reads from kill ring.
    use std::borrow::Cow;
    ed.state.last_command = Some(Cow::Borrowed("delete"));

    let names: Vec<String> = ed
        .state
        .registry
        .native_mappable_names()
        .map(str::to_owned)
        .collect();
    let name_refs: Vec<&str> = names.iter().map(String::as_str).collect();
    let mut host = ScriptingHost::new();
    host.register_command_names(&name_refs);

    let mut init_host = EditorHostImpl::new(&mut ed.state, &mut ed.view);
    host.eval_source_returning_defs(
        r#"(define-command! "paste-and-move" ""
                 (lambda ()
                   (call! "paste-after")
                   (call! "move-down")))"#
            .to_owned(),
        Default::default(),
        &mut init_host,
    )
    .expect("define-command! must succeed");

    ed.scripting = Some(host);
    ed.execute_keymap_command("paste-and-move".into(), Some(1), false, ArgSource::Keymap);

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
/// Under the in-Steel dispatch model: `steel-move-right` is applied inline as a Steel
/// funcall (no Rust queue); `delete` is a native and runs synchronously via
/// `%call-native!`. Source order is preserved by the call stack.
///
/// Fail oracle: if `%dispatch-command` forwarded plugin commands to `%call-native!`
/// instead of applying them inline, both would queue and drain post-eval — the order
/// dependency would be removed and the test would become order-independent (both
/// deleting 'a' or 'b' depending on residual state), but still "a\n" by accident.
/// More reliable: flip the expected assertion to "b\n" and confirm it fails.
#[test]
fn steel_call_source_order_native_after_steel() {
    // Buffer "ab\n", cursor on 'a'. The Steel command moves right; delete then
    // deletes 'b'. If order were reversed, 'a' (not 'b') would be deleted.
    let mut ed = editor_from("-[a]>b\n");

    let names: Vec<String> = ed
        .state
        .registry
        .native_mappable_names()
        .map(str::to_owned)
        .collect();
    let name_refs: Vec<&str> = names.iter().map(String::as_str).collect();
    let mut host = ScriptingHost::new();
    host.register_command_names(&name_refs);

    let mut init_host = EditorHostImpl::new(&mut ed.state, &mut ed.view);
    host.eval_source_returning_defs(
        r#"(define-command! "steel-move-right" ""
                 (lambda () (call! "move-right")))
               (define-command! "order-test" ""
                 (lambda ()
                   (call! "steel-move-right")
                   (call! "delete")))"#
            .to_owned(),
        Default::default(),
        &mut init_host,
    )
    .expect("define-command! must succeed");

    ed.scripting = Some(host);
    ed.execute_keymap_command("order-test".into(), Some(1), false, ArgSource::Keymap);

    // If correct order (move-right then delete): 'b' is deleted → "a\n".
    // If reversed (delete then move-right): 'a' is deleted → "b\n".
    assert_eq!(
        ed.doc().text().to_string(),
        "a\n",
        "source order: move-right must run before delete, leaving 'a'"
    );
}

/// **Finding 7 — native count preserved across plugin→native chain**: a native
/// command that follows a plugin command in the same body must use its own count.
///
/// `noop-steel` is applied inline (no effect); `(call! "move-down" 3)` dispatches
/// via `%call-native!` → `parse_count_extend` extracts `count=3` →
/// `run_command_sync("move-down", 3, false)` → lands on line 4.
///
/// Fail oracle: replace `(call! "move-down" 3)` with `(call! "move-down" 1)` →
/// cursor lands on line 2 instead of 4; the count-preservation assertion fails.
#[test]
fn steel_native_via_call_preserves_own_count() {
    // Buffer with at least 5 lines; cursor starts at line 1.
    let content = "-[a]>\nb\nc\nd\ne\n";
    let mut ed = editor_from(content);

    let names: Vec<String> = ed
        .state
        .registry
        .native_mappable_names()
        .map(str::to_owned)
        .collect();
    let name_refs: Vec<&str> = names.iter().map(String::as_str).collect();
    let mut host = ScriptingHost::new();
    host.register_command_names(&name_refs);

    let mut init_host = EditorHostImpl::new(&mut ed.state, &mut ed.view);
    // noop-steel is a no-op plugin command; move-down 3 runs sync via %call-native!.
    host.eval_source_returning_defs(
        r#"(define-command! "noop-steel" "" (lambda () #t))
               (define-command! "count-chain-test" ""
                 (lambda ()
                   (call! "noop-steel")
                   (call! "move-down" 3)))"#
            .to_owned(),
        Default::default(),
        &mut init_host,
    )
    .expect("define-command! must succeed");

    ed.scripting = Some(host);
    ed.execute_keymap_command("count-chain-test".into(), Some(1), false, ArgSource::Keymap);

    let host = live_host!(ed);
    let line = host.current_line_number().expect("current_line_number");
    // Started on line 1, moved down 3 → should be on line 4.
    assert_eq!(
        line, 4,
        "native count=3 must be preserved in plugin→native chain; got line {line}"
    );
}

/// **Finding 8 — unknown errors, no abort**: a body with an unknown command
/// between two valid moves must execute both valid moves, not abort on the
/// typo. `call!` logs an `Error` for the miss but never raises into Steel.
///
/// Fail oracle: reinstate `Err(e) => steel::stop!` in `call_command_primitive`
/// → the second move-right never runs.
#[test]
fn steel_unknown_cmd_errors_and_continues() {
    // "-[a]>bc\n", cursor at 0. Two moves should bring cursor to 2.
    let mut ed = editor_from("-[a]>bc\n");

    let names: Vec<String> = ed
        .state
        .registry
        .native_mappable_names()
        .map(str::to_owned)
        .collect();
    let name_refs: Vec<&str> = names.iter().map(String::as_str).collect();
    let mut host = ScriptingHost::new();
    host.register_command_names(&name_refs);

    let mut init_host = EditorHostImpl::new(&mut ed.state, &mut ed.view);
    host.eval_source_returning_defs(
        r#"(define-command! "warn-test" ""
                 (lambda ()
                   (call! "move-right")
                   (call! "this-command-does-not-exist")
                   (call! "move-right")))"#
            .to_owned(),
        Default::default(),
        &mut init_host,
    )
    .expect("define-command! must succeed");

    ed.scripting = Some(host);
    ed.execute_keymap_command("warn-test".into(), Some(1), false, ArgSource::Keymap);

    // Both move-rights run inline despite the unknown name — cursor ends at 2.
    assert_eq!(
        state(&ed),
        "ab-[c]>\n",
        "both moves must execute despite unknown command in between"
    );
}

/// **Finding 6 — mouse input drains pending hooks via `handle_event`**: any hooks that
/// are sitting in `state.pending_hooks` before a mouse event must be drained by
/// `handle_event`, which is the single interactive drain choke point.
///
/// Setup: a hook is seeded directly into `pending_hooks` via `fire_hook_silent`.
/// No scripting host is needed — `drain_hooks` skips hooks with no registered handlers
/// while still clearing the queue.
///
/// Fail oracle: remove `self.drain_hooks()` from `handle_event`
/// → `pending_hooks` is non-empty after the click (the pending hook was never cleared).
#[test]
fn mouse_click_drains_hooks_immediately() {
    use hume_scripting::hooks::HookId;
    use termina::event::Event;

    let mut ed = Editor::for_testing(crate::editor::buffer::Buffer::new(
        hume_editing::text::Text::from("hello\n"),
        hume_editing::selection::SelectionSet::default(),
    ));

    // Give the pane a viewport big enough that a click at row=0,col=0 lands in content.
    ed.view.panes[ed.state.focused_pane_id].viewport =
        hume_engine::pane::ViewportState::new(80, 24);

    // Seed a pending hook (OnBufferSave with no args — no handler registered, so
    // drain_hooks skips the Steel call but still removes it from the queue).
    ed.fire_hook_silent(HookId::OnBufferSave, &[]);
    assert!(
        !ed.state.pending_hooks.is_empty(),
        "pending_hooks must be non-empty before the event — drain has not run yet"
    );

    // Simulate a left-click at (0, 0) via handle_event so the drain choke point runs.
    let click = termina::event::MouseEvent {
        kind: termina::event::MouseEventKind::Down(termina::event::MouseButton::Left),
        column: 0,
        row: 0,
        modifiers: termina::event::Modifiers::NONE,
    };
    ed.handle_event(Event::Mouse(click));

    // drain_hooks ran at the tail of handle_event — all pending hooks must be gone.
    assert!(
        ed.state.pending_hooks.is_empty(),
        "pending_hooks must be empty after handle_event; got {:?}",
        ed.state.pending_hooks
    );
}

// ── Count / extend injection into Steel command lambdas ──────────────────────
//
// When a Steel command is triggered from a key binding (not via `:command`),
// `execute_keymap_command` injects `count` and `extend` as leading lambda args
// based on the lambda's declared arity:
//   arity 0 → []
//   arity 1 → [count]
//   arity ≥ 2 or variadic → [count, extend]
// The body then decides what to repeat / extend.

/// A `(lambda (count extend))` command receives the keymap count and extend flag.
///
/// Fail oracle: without injection, `(call! "move-right" count)` always passes 1
/// and the cursor lands at column 1 instead of `count`.
#[test]
fn steel_lambda_receives_count_and_extend() {
    // 10-char buffer; cursor starts at 0.
    let mut ed = editor_from("-[a]>bcdefghij\n");

    let names: Vec<String> = ed
        .state
        .registry
        .native_mappable_names()
        .map(str::to_owned)
        .collect();
    let name_refs: Vec<&str> = names.iter().map(String::as_str).collect();
    let mut host = ScriptingHost::new();
    host.register_command_names(&name_refs);

    let mut init_host = EditorHostImpl::new(&mut ed.state, &mut ed.view);
    host.eval_source_returning_defs(
        r#"(define-command! "step-right" ""
             (lambda (count extend) (call! "move-right" count extend)))"#
            .to_owned(),
        Default::default(),
        &mut init_host,
    )
    .expect("define-command! must succeed");
    ed.scripting = Some(host);

    // Dispatch with count=4: cursor must land at position 4.
    ed.execute_keymap_command("step-right".into(), Some(4), false, ArgSource::Keymap);
    let idx = ed
        .state
        .panes
        .state
        .get(ed.state.focused_pane_id)
        .unwrap()
        .values()
        .next()
        .unwrap()
        .selections
        .primary()
        .head();
    assert_eq!(idx, 4, "count=4 must move cursor 4 positions; got {idx}");

    // Fail oracle: if injection were disabled, cursor would be at 1 (count defaults to 1).
    // Restate with count=1 to prove the assert is live.
    ed.execute_keymap_command("step-right".into(), Some(1), false, ArgSource::Keymap);
    let idx2 = ed
        .state
        .panes
        .state
        .get(ed.state.focused_pane_id)
        .unwrap()
        .values()
        .next()
        .unwrap()
        .selections
        .primary()
        .head();
    assert_eq!(
        idx2, 5,
        "count=1 must advance one more position; got {idx2}"
    );
}

/// A `(lambda ())` command ignores injection — no arity-mismatch error.
///
/// Fail oracle: if injection always passed 2 args regardless of arity, Steel
/// would raise an arity error and execute_keymap_command would report it; the
/// cursor would not move.
#[test]
fn steel_zero_arity_lambda_ignores_injection() {
    let mut ed = editor_from("-[a]>bc\n");

    let names: Vec<String> = ed
        .state
        .registry
        .native_mappable_names()
        .map(str::to_owned)
        .collect();
    let name_refs: Vec<&str> = names.iter().map(String::as_str).collect();
    let mut host = ScriptingHost::new();
    host.register_command_names(&name_refs);

    let mut init_host = EditorHostImpl::new(&mut ed.state, &mut ed.view);
    host.eval_source_returning_defs(
        // 0-arg lambda: always moves right 1.
        r#"(define-command! "fixed-right" "" (lambda () (call! "move-right")))"#.to_owned(),
        Default::default(),
        &mut init_host,
    )
    .expect("define-command! must succeed");
    ed.scripting = Some(host);

    // Dispatch with count=5: the 0-arg lambda ignores count, moves exactly 1.
    let before = ed
        .state
        .panes
        .state
        .get(ed.state.focused_pane_id)
        .unwrap()
        .values()
        .next()
        .unwrap()
        .selections
        .primary()
        .head();
    ed.execute_keymap_command("fixed-right".into(), Some(5), false, ArgSource::Keymap);
    let after = ed
        .state
        .panes
        .state
        .get(ed.state.focused_pane_id)
        .unwrap()
        .values()
        .next()
        .unwrap()
        .selections
        .primary()
        .head();
    assert_eq!(
        after,
        before + 1,
        "0-arg lambda must move 1 regardless of count; got {after}"
    );
    // No error was reported.
    assert!(
        ed.state
            .message_log
            .entries()
            .all(|e| !matches!(e.severity, crate::editor::Severity::Error)),
        "0-arg Steel command must not produce an error on injection"
    );
}

/// `ArgSource::Minibuf`-path args are not replaced by keymap injection.
///
/// When `execute_keymap_command` is called with `ArgSource::Minibuf(Some(..))`
/// (the `:command` code path — see `command_mode.rs`'s `execute_command`),
/// that arg must reach the lambda unchanged — count/extend injection only
/// fires for `ArgSource::Keymap`, a structurally separate match arm.
///
/// Fail oracle: if `Minibuf`'s marshalling fell through to the `Keymap`
/// injection rules, the StringV arg would be silently capped/replaced,
/// breaking the existing `minibuffer_arity_rule_forwards_string_arg_to_arity_1`
/// test.  Here we directly assert the Steel arg we pass is what the lambda sees.
#[test]
fn explicit_minibuf_arg_not_overwritten_by_injection() {
    let mut ed = editor_from("-[a]>bc\n");

    let names: Vec<String> = ed
        .state
        .registry
        .native_mappable_names()
        .map(str::to_owned)
        .collect();
    let name_refs: Vec<&str> = names.iter().map(String::as_str).collect();
    let mut host = ScriptingHost::new();
    host.register_command_names(&name_refs);

    let mut init_host = EditorHostImpl::new(&mut ed.state, &mut ed.view);
    // lambda(x): if x is the string "move-right", run move-right; otherwise no-op.
    host.eval_source_returning_defs(
        r#"(define-command! "echo-cmd" "" (lambda (x) (when (string? x) (call! x))))"#.to_owned(),
        Default::default(),
        &mut init_host,
    )
    .expect("define-command! must succeed");
    ed.scripting = Some(host);

    let before = ed
        .state
        .panes
        .state
        .get(ed.state.focused_pane_id)
        .unwrap()
        .values()
        .next()
        .unwrap()
        .selections
        .primary()
        .head();

    // Pass an explicit Minibuf arg (the `:command` path does this).
    ed.execute_keymap_command(
        "echo-cmd".into(),
        Some(1),
        false,
        ArgSource::Minibuf(Some("move-right".to_string())),
    );

    let after = ed
        .state
        .panes
        .state
        .get(ed.state.focused_pane_id)
        .unwrap()
        .values()
        .next()
        .unwrap()
        .selections
        .primary()
        .head();
    // The lambda received the explicit string "move-right" and ran it → cursor moved.
    assert_eq!(
        after,
        before + 1,
        "explicit StringV arg must reach the lambda unchanged"
    );
}

/// A `(lambda (count))` command receives only the repeat count — no extend arg.
///
/// Fail oracle: if the arity-1 branch injected `[count, extend]` instead of
/// `[count]`, Steel would call `(apply proc (list 4 #f))` on a 1-param lambda
/// and raise an arity error; the cursor would not move.
#[test]
fn steel_arity_1_lambda_receives_count_only() {
    let mut ed = editor_from("-[a]>bcdefghij\n");

    let names: Vec<String> = ed
        .state
        .registry
        .native_mappable_names()
        .map(str::to_owned)
        .collect();
    let name_refs: Vec<&str> = names.iter().map(String::as_str).collect();
    let mut host = ScriptingHost::new();
    host.register_command_names(&name_refs);

    let mut init_host = EditorHostImpl::new(&mut ed.state, &mut ed.view);
    host.eval_source_returning_defs(
        r#"(define-command! "step-count-only" ""
             (lambda (count) (call! "move-right" count)))"#
            .to_owned(),
        Default::default(),
        &mut init_host,
    )
    .expect("define-command! must succeed");
    ed.scripting = Some(host);

    // Dispatch with count=3: cursor must land 3 positions to the right.
    ed.execute_keymap_command("step-count-only".into(), Some(3), false, ArgSource::Keymap);
    let idx = ed
        .state
        .panes
        .state
        .get(ed.state.focused_pane_id)
        .unwrap()
        .values()
        .next()
        .unwrap()
        .selections
        .primary()
        .head();
    assert_eq!(idx, 3, "count=3 must move cursor 3 positions; got {idx}");

    // No arity error was produced.
    assert!(
        ed.state
            .message_log
            .entries()
            .all(|e| !matches!(e.severity, crate::editor::Severity::Error)),
        "arity-1 Steel command must not produce an error on injection"
    );
}

/// **Extend-exit via inner native dispatch**: a Steel command that calls native
/// `(delete)` while Extend mode is active must exit Extend, even though
/// `SteelBacked.clears_extend` is always `false`.
///
/// The mechanism: `(call! "delete")` routes through `run_command_sync` →
/// `run_dispatch_pipeline`, which runs `delete`'s own `step_clear_extend` with
/// `clears_extend=true`.  Mode is still `Extend` when the inner pipeline fires,
/// so it flips to Normal.  The outer Steel dispatch branch deliberately omits
/// `step_clear_extend` — the inner command's meta drives the transition.
///
/// Fail oracle: replace `(call! "delete")` with `(+ 1 0)` (no-op body) →
/// mode stays `Extend` → the mode assertion fails, proving the test is not vacuous.
#[test]
fn steel_call_delete_in_extend_exits_extend_mode() {
    let mut ed = editor_from("-[hell]>o\n");

    let names: Vec<String> = ed
        .state
        .registry
        .native_mappable_names()
        .map(str::to_owned)
        .collect();
    let name_refs: Vec<&str> = names.iter().map(String::as_str).collect();
    let mut host = ScriptingHost::new();
    host.register_command_names(&name_refs);

    let mut init_host = EditorHostImpl::new(&mut ed.state, &mut ed.view);
    host.eval_source_returning_defs(
        r#"(define-command! "wrap-delete" "" (lambda () (call! "delete")))"#.to_owned(),
        Default::default(),
        &mut init_host,
    )
    .expect("define-command! must succeed");

    ed.scripting = Some(host);
    ed.state.mode = Mode::Extend;

    ed.execute_keymap_command("wrap-delete".into(), Some(1), false, ArgSource::Keymap);

    assert_eq!(
        ed.state.mode,
        Mode::Normal,
        "Steel wrapping (call! \"delete\") must exit Extend via inner command's clears_extend"
    );
    // Also confirm the delete actually ran — the selection "hell" must be gone.
    assert_eq!(
        ed.doc().text().to_string(),
        "o\n",
        "inner (delete) must have removed the selected text"
    );
}

// ── Dual-path parity tests ────────────────────────────────────────────────────
//
// The original regression: `run_command_sync` executed native commands naked —
// cursor moved correctly but the bookkeeping cluster (jump list, last_command,
// dot-repeat, paste-session commit) was silently dropped.  These tests assert
// that dispatching the same native command via the keypress path AND via a Steel
// `(call! …)` wrapper leaves IDENTICAL `BookkeepingSnapshot` state.
//
// Each test documents a fail oracle: which single line in `run_dispatch_pipeline`
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
    ed_key.execute_keymap_command("delete".into(), Some(1), false, ArgSource::Keymap);
    let snap_key = snapshot_bookkeeping(&ed_key);

    // Path B — Steel (call! "delete").
    let mut ed_steel = editor_from("-[f]>oo\n");
    let names: Vec<String> = ed_steel
        .state
        .registry
        .native_mappable_names()
        .map(str::to_owned)
        .collect();
    let name_refs: Vec<&str> = names.iter().map(String::as_str).collect();
    let mut host = ScriptingHost::new();
    host.register_command_names(&name_refs);
    let mut init_host = EditorHostImpl::new(&mut ed_steel.state, &mut ed_steel.view);
    host.eval_source_returning_defs(
        r#"(define-command! "steel-delete" "" (lambda () (call! "delete")))"#.to_owned(),
        Default::default(),
        &mut init_host,
    )
    .expect("define-command! must succeed");
    ed_steel.scripting = Some(host);
    let before_steel = snapshot_bookkeeping(&ed_steel);
    ed_steel.execute_keymap_command("steel-delete".into(), Some(1), false, ArgSource::Keymap);
    let snap_steel = snapshot_bookkeeping(&ed_steel);

    // Pre-conditions: both editors start from identical bookkeeping state.
    assert_eq!(
        before_key, before_steel,
        "pre-condition: both editors must start identical"
    );

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
    ed_key.execute_keymap_command("goto-last-line".into(), Some(1), false, ArgSource::Keymap);
    let snap_key = snapshot_bookkeeping(&ed_key);

    // Path B — Steel (call! "goto-last-line").
    let mut ed_steel = editor_from(content);
    let names: Vec<String> = ed_steel
        .state
        .registry
        .native_mappable_names()
        .map(str::to_owned)
        .collect();
    let name_refs: Vec<&str> = names.iter().map(String::as_str).collect();
    let mut host = ScriptingHost::new();
    host.register_command_names(&name_refs);
    let mut init_host = EditorHostImpl::new(&mut ed_steel.state, &mut ed_steel.view);
    host.eval_source_returning_defs(
        r#"(define-command! "steel-goto-end" "" (lambda () (call! "goto-last-line")))"#.to_owned(),
        Default::default(),
        &mut init_host,
    )
    .expect("define-command! must succeed");
    ed_steel.scripting = Some(host);
    ed_steel.execute_keymap_command("steel-goto-end".into(), Some(1), false, ArgSource::Keymap);
    let snap_steel = snapshot_bookkeeping(&ed_steel);

    assert_eq!(
        snap_key, snap_steel,
        "keypress vs Steel dispatch of 'goto-last-line' must leave identical jump bookkeeping"
    );
}

/// **Parity: Steel-branch bookkeeping cluster** — a `#:repeatable` SteelBacked
/// command dispatched through `Editor::dispatch`'s Steel branch must run the
/// SAME funnel stages as the native `run_dispatch_pipeline`.
///
/// The existing parity tests above compare *native-via-keypress* vs
/// *native-via-inner-`(call!)`* — both go through `run_dispatch_pipeline`.
/// This test exercises the **Steel branch's own AFTER stages** (the hand-composed
/// sequence in `mod.rs:dispatch`), which the other tests leave untouched.
///
/// The `single_native_dispatch_funnel` lint only guards the body funnel; it
/// cannot detect a stage added to one pipeline and forgotten in the other.
/// Pinning the full cluster here means any such omission causes a divergence.
///
/// Fail oracle (dot-repeat): delete the `step_stamp_repeatable` call in the Steel
///   AFTER block of `Editor::dispatch` → `steel.last_repeatable` is `None` while
///   native is `Some` → assertion fails.
/// Fail oracle (paste-session): delete the `step_paste_commit` call in the Steel
///   BEFORE block of `Editor::dispatch` → a pre-armed session survives on the Steel
///   path → `paste_session_open` diverges.
#[test]
fn parity_steel_branch_cluster_vs_native() {
    // ── Case 1: repeatable edit — pins dot-repeat and last_command stages ─────
    // Path A — native repeatable edit.
    let mut ed_native = editor_from("-[f]>oo\n");
    ed_native.execute_keymap_command("delete".into(), Some(1), false, ArgSource::Keymap);
    let snap_native = snapshot_bookkeeping(&ed_native);

    // Path B — a `#:repeatable` Steel command whose body calls `delete`.
    // Goes through the Steel branch of `Editor::dispatch` (outer), which
    // must run `step_stamp_repeatable` in AFTER just as the native path does.
    let mut ed_steel = editor_from("-[f]>oo\n");
    let names: Vec<String> = ed_steel
        .state
        .registry
        .native_mappable_names()
        .map(str::to_owned)
        .collect();
    let name_refs: Vec<&str> = names.iter().map(String::as_str).collect();
    let mut host = ScriptingHost::new();
    host.register_command_names(&name_refs);
    let mut init_host = EditorHostImpl::new(&mut ed_steel.state, &mut ed_steel.view);
    host.eval_source_returning_defs(
        r#"(define-command! "steel-del" "" (lambda () (call! "delete")) #:repeatable #t)"#
            .to_owned(),
        Default::default(),
        &mut init_host,
    )
    .expect("define-command! must succeed");
    ed_steel.scripting = Some(host);
    ed_steel.execute_keymap_command("steel-del".into(), Some(1), false, ArgSource::Keymap);
    let snap_steel = snapshot_bookkeeping(&ed_steel);

    // jump_len and paste_session_open must match exactly.
    assert_eq!(
        snap_native.jump_len, snap_steel.jump_len,
        "jump-list stage parity"
    );
    assert_eq!(
        snap_native.paste_session_open, snap_steel.paste_session_open,
        "paste-session stage parity"
    );
    // The Steel pre-stamp puts "steel-del" into last_command before the body,
    // then the inner `(call! "delete")` running through `run_dispatch_pipeline`
    // overwrites with "delete" (inner wins — intentional for smart-p routing).
    // Verify the inner name landed (and last_command is non-None on both paths).
    assert_eq!(
        snap_native.last_command.as_deref(),
        snap_steel.last_command.as_deref(),
        "last_command must be identical on both paths (inner 'delete' wins on both)"
    );
    // Steel AFTER records `last_repeatable` for the OUTER command name ("steel-del"),
    // not the inner "delete" from `call!`. Outer-name-wins preserves the correct
    // semantic so `.` replays the outer Steel command, not the primitive it wrapped.
    //
    // Fail oracle: delete the `step_stamp_repeatable` call in the Steel AFTER block
    //   of `Editor::dispatch` → the inner `call! "delete"` dispatch stamps "delete"
    //   as the name instead → `s_name == "delete"` ≠ "steel-del" → assertion fails.
    let (s_name, s_count, s_char) = snap_steel
        .last_repeatable
        .as_ref()
        .expect("Steel branch must record dot-repeat");
    assert_eq!(
        s_name, "steel-del",
        "Steel AFTER must record outer name in last_repeatable"
    );
    let (_, n_count, n_char) = snap_native
        .last_repeatable
        .expect("native must record dot-repeat");
    assert_eq!(
        (n_count, n_char),
        (*s_count, *s_char),
        "dot-repeat payload (count, char_arg) parity"
    );

    // ── Case 2: non-repeatable Steel command with an open paste session ────────
    // A paste session (`p`) must be committed by `step_paste_commit` in the
    // Steel BEFORE stage before any non-ring-cycle command runs. Ring-cycle
    // commands (`[`/`]`) are exempt; a plain Steel command is not.
    //
    // Fail oracle: delete the `step_paste_commit` call in the Steel BEFORE block
    //   of `Editor::dispatch` → the paste session remains open → assertion fails.
    let mut ed2 = editor_from("-[a]>bc\n");
    // Seed the kill ring so `p` has something to paste.
    ed2.state.kill_ring.push(vec!["X".to_string()]);
    // Set last_command to a kill (smart-p routes `p` to the ring when the prior
    // command was change/delete/yank; "change" is in SMART_P_LAST_CMDS).
    ed2.state.last_command = Some("change".into());
    ed2.feed_key(key('p')); // paste-after → resolves ring head, opens paste session
    let pane_id = ed2.state.focused_pane_id;
    let buf_id = ed2.focused_buffer_id();
    assert!(
        ed2.state.panes.state[pane_id][buf_id].paste_group.is_some(),
        "pre-condition: paste-after must have opened a paste session"
    );

    let names2: Vec<String> = ed2
        .state
        .registry
        .native_mappable_names()
        .map(str::to_owned)
        .collect();
    let name_refs2: Vec<&str> = names2.iter().map(String::as_str).collect();
    let mut host2 = ScriptingHost::new();
    host2.register_command_names(&name_refs2);
    let mut init_host2 = EditorHostImpl::new(&mut ed2.state, &mut ed2.view);
    // The Steel command must NOT call any native command internally — any inner
    // `(call! …)` would route through `run_dispatch_pipeline` which also calls
    // `step_paste_commit`, masking a missing outer commit. A pure Steel no-op
    // (body returns a value without dispatching) isolates the outer BEFORE stage.
    host2
        .eval_source_returning_defs(
            r#"(define-command! "pure-noop" "" (lambda () (+ 1 0)))"#.to_owned(),
            Default::default(),
            &mut init_host2,
        )
        .expect("define-command! must succeed");
    ed2.scripting = Some(host2);
    ed2.execute_keymap_command("pure-noop".into(), Some(1), false, ArgSource::Keymap);

    // Fail oracle: delete the `step_paste_commit` call in the Steel BEFORE block
    //   of `Editor::dispatch` → pure-noop runs without committing → paste_group
    //   stays Some → assertion fails.
    assert!(
        ed2.state.panes.state[pane_id][buf_id].paste_group.is_none(),
        "step_paste_commit must close the paste session on the Steel path"
    );
}

/// **Parity: Extend-exit via inner native dispatch** — `delete` from Extend mode
/// dispatched via keypress vs via a Steel `(call! "delete")` wrapper must both
/// land in `Mode::Normal`. This is the mode-exit parity test that gives the
/// `BookkeepingSnapshot.mode` field its teeth.
///
/// Inner mechanism: `(call! "delete")` routes through `run_command_sync` →
/// `run_dispatch_pipeline`, which runs `step_clear_extend` with `delete`'s
/// `clears_extend=true`. Mode is `Extend` when the inner pipeline fires, so both
/// paths exit to Normal.
///
/// Fail oracle: change the Steel body to `(+ 1 0)` (no inner delete) →
/// Steel path's `mode` stays `Extend` ≠ `Normal` → `assert_eq!(snap_key, snap_steel)` fails.
#[test]
fn parity_extend_exit_keypress_vs_steel() {
    // Path A — keypress.
    let mut ed_key = editor_from("-[f]>oo\n");
    ed_key.state.mode = Mode::Extend;
    ed_key.execute_keymap_command("delete".into(), Some(1), false, ArgSource::Keymap);
    let snap_key = snapshot_bookkeeping(&ed_key);

    // Path B — Steel (call! "delete").
    let mut ed_steel = editor_from("-[f]>oo\n");
    ed_steel.state.mode = Mode::Extend;
    let names: Vec<String> = ed_steel
        .state
        .registry
        .native_mappable_names()
        .map(str::to_owned)
        .collect();
    let name_refs: Vec<&str> = names.iter().map(String::as_str).collect();
    let mut host = ScriptingHost::new();
    host.register_command_names(&name_refs);
    let mut init_host = EditorHostImpl::new(&mut ed_steel.state, &mut ed_steel.view);
    host.eval_source_returning_defs(
        r#"(define-command! "steel-delete" "" (lambda () (call! "delete")))"#.to_owned(),
        Default::default(),
        &mut init_host,
    )
    .expect("define-command! must succeed");
    ed_steel.scripting = Some(host);
    ed_steel.execute_keymap_command("steel-delete".into(), Some(1), false, ArgSource::Keymap);
    let snap_steel = snapshot_bookkeeping(&ed_steel);

    assert_eq!(
        snap_key, snap_steel,
        "keypress vs Steel dispatch of 'delete' from Extend must produce identical bookkeeping (including mode)"
    );
}

// ── In-Steel plugin dispatch (core goal) ─────────────────────────────────────
//
// Plugin commands are applied directly on the Steel call stack via (apply proc args).
// A state read after (call! plugin-cmd) within the SAME body reflects the plugin's
// side-effects immediately.

/// **Core goal**: a plugin command that calls another plugin command can observe
/// the inner command's effect via a state read in the same body.
///
/// `inner-move` is applied inline (plugin funcall in the VM), so the cursor is
/// on line 2 by the time `(current-line-number)` is evaluated → the `(when …)`
/// branch fires → second move-down → line 3.
///
/// Fail oracle: comment out the `if proc { apply proc args }` branch in
/// `%dispatch-command` so all commands fall through to `%call-native!` — the
/// plugin command queues, cursor stays on line 1 during eval, branch does not
/// fire → line 2.
#[test]
fn plugin_calls_plugin_cursor_read_is_live() {
    // "-[a]>\nb\nc\n", cursor on line 1.
    let mut ed = editor_from("-[a]>\nb\nc\n");

    let names: Vec<String> = ed
        .state
        .registry
        .native_mappable_names()
        .map(str::to_owned)
        .collect();
    let name_refs: Vec<&str> = names.iter().map(String::as_str).collect();
    let mut host = ScriptingHost::new();
    host.register_command_names(&name_refs);

    let mut init_host = EditorHostImpl::new(&mut ed.state, &mut ed.view);
    // inner-move: plugin command that wraps a single move-down.
    // outer-cmd: calls inner-move (plugin→plugin), reads cursor, conditionally
    //   moves down again if cursor advanced past line 1.
    host.eval_source_returning_defs(
        r#"(define-command! "inner-move" ""
                 (lambda () (call! "move-down")))
               (define-command! "outer-cmd" ""
                 (lambda ()
                   (call! "inner-move")
                   (when (> (current-line-number) 1)
                     (call! "move-down"))))"#
            .to_owned(),
        Default::default(),
        &mut init_host,
    )
    .expect("define-command! must succeed");

    ed.scripting = Some(host);
    ed.execute_keymap_command("outer-cmd".into(), Some(1), false, ArgSource::Keymap);

    // Inline: inner-move ran synchronously (line 2 during eval), branch fired → line 3.
    // Deferred: inner-move queued (line 1 during eval), branch skipped → line 2.
    assert_eq!(
        state(&ed),
        "a\nb\n-[c]>\n",
        "plugin→plugin inline dispatch: cursor must be on line 3 (branch fired on live read); \
         deferral regression leaves it on line 2"
    );
}

/// Plugin commands registered via `define-command!` are entered into
/// `ScriptingHost.registries.command_table` inline during eval (by `define_command_inner`).
///
/// `command_table` is what `%lookup-plugin-proc` queries to decide whether to apply
/// a command inline in Steel. This test confirms the table is populated after
/// `eval_source_returning_defs` — the precondition for all in-Steel dispatch tests.
///
/// Fail oracle: remove the `command_table.insert(…)` line in `define_command_inner`
/// → `command_table` is empty → `%lookup-plugin-proc` always returns `#f` →
/// `plugin_calls_plugin_cursor_read_is_live` regresses to cursor=1.
#[test]
fn command_table_populated_after_define_command() {
    let mut host = ScriptingHost::new();
    let mut mock = MockHost::new();
    host.eval_source_returning_defs(
        r#"(define-command! "ping" "" (lambda () #t))
               (define-command! "pong" "" (lambda () #t))"#
            .to_owned(),
        Default::default(),
        &mut mock,
    )
    .expect("define-command! must succeed");

    let table = host.command_table_for_test();
    assert!(
        table.contains_key("ping"),
        "'ping' must be in command_table after define-command!"
    );
    assert!(
        table.contains_key("pong"),
        "'pong' must be in command_table after define-command!"
    );
    // Native command names must NOT appear in command_table — only plugin commands.
    assert!(
        !table.contains_key("move-right"),
        "'move-right' (native) must not appear in command_table"
    );
}

/// **Issue 6 — native `(call!)` at init top-level warns and skips, not aborts**.
///
/// A top-level `(call! "move-right")` in init.scm must NOT abort evaluation:
/// the command is skipped with a `Warning`, and lines that follow it are applied.
///
/// Three assertions lock in the full behavior:
/// 1. `eval_source_returning_defs` returns `Ok` (no hard error, no abort).
/// 2. `history-capacity` set after the native call is applied (eval continued past it).
/// 3. The cursor did not move (command was skipped, not run).
///
/// Fail oracle: reinstate `steel::stop!` in `call_command_primitive` for the
/// `EvalSession::Init` branch → eval returns `Err`, the `(set-option! …)` is
/// never reached, assertions 1 and 2 fail.
#[test]
fn native_call_bang_at_init_top_level_warns_and_skips() {
    // Content is irrelevant — cursor stays at 0.
    let mut ed = editor_from("-[a]>bc\n");

    let names: Vec<String> = ed
        .state
        .registry
        .native_mappable_names()
        .map(str::to_owned)
        .collect();
    let name_refs: Vec<&str> = names.iter().map(String::as_str).collect();
    let mut host = ScriptingHost::new();
    host.register_command_names(&name_refs);

    let mut init_host = EditorHostImpl::new(&mut ed.state, &mut ed.view);
    // Eval as init context: native call in the middle, set-option! after it.
    let result = host.eval_source_returning_defs(
        r#"(set-option! "history-capacity" 42)
           (call! "move-right")
           (set-option! "history-capacity" 77)"#
            .to_owned(),
        Default::default(),
        &mut init_host,
    );

    // 1. Eval must succeed — no hard abort.
    assert!(
        result.is_ok(),
        "eval must not abort on native call at init top-level; got: {result:?}"
    );

    // 2. The line after the native call must have been applied.
    assert_eq!(
        ed.state.settings.history_capacity, 77,
        "set-option! after native call must be applied (eval continued past it)"
    );

    // 3. The native command itself must have been skipped.
    assert_eq!(
        state(&ed),
        "-[a]>bc\n",
        "cursor must not move (native command skipped during init)"
    );

    // 4. A warning must have been produced for the skipped command.
    let msgs = host.take_pending_messages();
    let has_warn = msgs.iter().any(|(lvl, txt)| {
        matches!(lvl, hume_scripting::LogLevel::Warning) && txt.contains("move-right")
    });
    assert!(
        has_warn,
        "a Warning containing 'move-right' must be emitted; got: {msgs:?}"
    );
}

// ── Steel insert-mode dot-repeat ─────────────────────────────────────────────

/// Helper: register all native command names, eval a Steel snippet, attach the
/// host, and bind the named command to F2 in Normal mode.
fn setup_steel_f2(ed: &mut Editor, snippet: &str, cmd_name: &str) {
    use crate::editor::keymap::BindMode;
    use termina::event::KeyCode;

    let names: Vec<String> = ed
        .state
        .registry
        .native_mappable_names()
        .map(str::to_owned)
        .collect();
    let name_refs: Vec<&str> = names.iter().map(String::as_str).collect();

    let mut host = ScriptingHost::new();
    host.register_command_names(&name_refs);

    let mut init_host = EditorHostImpl::new(&mut ed.state, &mut ed.view);
    host.eval_source_returning_defs(snippet.to_owned(), Default::default(), &mut init_host)
        .expect("Steel snippet must compile and evaluate without error");

    ed.scripting = Some(host);

    let f2 = termina::event::KeyEvent::new(KeyCode::Function(2), termina::event::Modifiers::NONE);
    ed.state.keymap.bind_user_with_extend(
        BindMode::Normal,
        &[f2],
        std::borrow::Cow::Owned(cmd_name.to_owned()),
        false,
    );
}

/// A Steel `#:repeatable` command that calls `(call! "insert-before")` must
/// record typed text in `insert_keys` and replay it on `.`.
///
/// Fail oracle:
/// - If `end_insert_session` did NOT back-fill `insert_keys` for Steel actions,
///   `insert_keys` would be empty → `.` inserts nothing → final buffer differs.
/// - If the pre-body snapshot fix were reverted, `.` would still insert but only
///   at the raw cursor position instead of the recipe-established one.
#[test]
fn steel_repeatable_insert_dot_repeat_replays_command_and_typed_text() {
    let f2 = termina::event::KeyEvent::new(
        termina::event::KeyCode::Function(2),
        termina::event::Modifiers::NONE,
    );
    let mut ed = editor_from("-[x]>\n");
    setup_steel_f2(
        &mut ed,
        r#"(define-command! "steel-ins" "enter insert before selection"
             (lambda () (call! "insert-before"))
             #:repeatable #t)"#,
        "steel-ins",
    );

    // F2 → enters Insert at selection start.
    ed.feed_key(f2);
    ed.feed_key(key('a'));
    ed.feed_key(key('b'));
    ed.feed_key(key_esc()); // back to Normal; buffer is "abx\n"
    assert_eq!(
        ed.doc().text().to_string(),
        "abx\n",
        "setup: 'ab' must be inserted before 'x'"
    );

    // White-box: insert_keys must be back-filled by end_insert_session.
    {
        let action = ed
            .state
            .last_repeatable_action
            .as_ref()
            .expect("last_repeatable_action must be set after steel-ins");
        assert_eq!(action.command.as_ref(), "steel-ins");
        assert_eq!(
            action.insert_keys.len(),
            2,
            "insert_keys must contain both typed chars"
        );
    }

    // Move selection to 'x' and replay with `.`.
    ed.feed_key(key('w')); // select 'x'
    ed.feed_key(key('.')); // replay: enter insert before 'x', retype "ab"
    assert_eq!(
        ed.doc().text().to_string(),
        "ababx\n",
        "`.` must re-enter insert and retype 'ab' before the selection"
    );
}

/// The `selection_recipe` snapshot taken before the Steel body runs must NOT be
/// clobbered by an inner `(call! "insert-before")` dispatch.
///
/// Fail oracle (Gap A): without the pre-body `mem::take` snapshot in the Steel
/// `dispatch` path, `insert-before`'s inner dispatch takes `selection_recipe` via
/// `run_dispatch_pipeline`, leaving it empty. The white-box assertion
/// `selection_recipe.len() == 1` catches
/// this — it passes with the snapshot, fails without it.
#[test]
fn steel_repeatable_insert_preserves_prior_selection_recipe() {
    let f2 = termina::event::KeyEvent::new(
        termina::event::KeyCode::Function(2),
        termina::event::Modifiers::NONE,
    );
    // `x` (select-line) on "foo bar\n" selects the whole line — an in-place
    // selection that pushes a recipe step. (Reaching motions like `w` don't
    // push establish steps, so `x` is used here as the recipe-building command.)
    let mut ed = editor_from("-[f]>oo bar\n");
    setup_steel_f2(
        &mut ed,
        r#"(define-command! "steel-ins" "enter insert before selection"
             (lambda () (call! "insert-before"))
             #:repeatable #t)"#,
        "steel-ins",
    );

    // `x` establishes a real selection → recipe non-empty.
    ed.feed_key(key('x'));
    assert_eq!(
        ed.state.selection_recipe.len(),
        1,
        "pre-condition: x must push a recipe step"
    );

    // F2 → steel-ins → insert 'X' before the line selection.
    ed.feed_key(f2);
    ed.feed_key(key('X'));
    ed.feed_key(key_esc());
    assert_eq!(ed.doc().text().to_string(), "Xfoo bar\n");

    // White-box: the pre-body snapshot must survive the inner insert-before dispatch.
    let action = ed
        .state
        .last_repeatable_action
        .as_ref()
        .expect("last_repeatable_action must be set after steel-ins");
    assert_eq!(
        action.selection_recipe.len(),
        1,
        "selection_recipe must not be clobbered by inner (call! \"insert-before\")"
    );
    assert!(
        !action.selection_recipe[0].extend,
        "x step must be a Move (establish)"
    );
}

/// After `.` replays a Steel insert action, a single `u` must undo the entire
/// replay as one step — proving `replay_dot`'s edit-group bracketing
/// works correctly for the Steel insert path.
///
/// Mirrors `dot_is_single_undo_step` from dot_repeat.rs but drives insert via Steel.
#[test]
fn steel_repeatable_insert_dot_repeat_single_undo() {
    let f2 = termina::event::KeyEvent::new(
        termina::event::KeyCode::Function(2),
        termina::event::Modifiers::NONE,
    );
    let mut ed = editor_from("-[x]>y\n");
    setup_steel_f2(
        &mut ed,
        r#"(define-command! "steel-ins" "enter insert before selection"
             (lambda () (call! "insert-before"))
             #:repeatable #t)"#,
        "steel-ins",
    );

    // F2, type "AB", Esc → "ABxy\n"
    ed.feed_key(f2);
    ed.feed_key(key('A'));
    ed.feed_key(key('B'));
    ed.feed_key(key_esc());
    assert_eq!(ed.doc().text().to_string(), "ABxy\n");

    // `w` from 'x' selects "xy"; `.` replay inserts "AB" before → "ABABxy\n".
    ed.feed_key(key('w'));
    ed.feed_key(key('.'));
    assert_eq!(ed.doc().text().to_string(), "ABABxy\n");

    // One undo must revert the entire replay as a single step.
    ed.feed_key(key('u'));
    assert_eq!(
        ed.doc().text().to_string(),
        "ABxy\n",
        "one undo must revert the full replay"
    );
}

/// The `change` command opens its undo group through `begin_insert_session` (the
/// only path that opens a group). A Steel `#:repeatable` command wrapping `change`
/// must create an InsertSession and record `insert_keys` correctly.
///
/// This guards that the edit-then-insert undo-group path remains safe.
#[test]
fn steel_repeatable_change_via_call_records_insert_keys() {
    let f2 = termina::event::KeyEvent::new(
        termina::event::KeyCode::Function(2),
        termina::event::Modifiers::NONE,
    );
    let mut ed = editor_from("-[foo]> bar\n");
    setup_steel_f2(
        &mut ed,
        r#"(define-command! "steel-chg" "change selection"
             (lambda () (call! "change"))
             #:repeatable #t)"#,
        "steel-chg",
    );

    // F2 (steel-chg = change), type "hi", Esc → "hi bar\n"
    ed.feed_key(f2);
    ed.feed_key(key('h'));
    ed.feed_key(key('i'));
    ed.feed_key(key_esc());
    assert_eq!(ed.doc().text().to_string(), "hi bar\n");

    // White-box: insert_keys must be back-filled.
    {
        let action = ed
            .state
            .last_repeatable_action
            .as_ref()
            .expect("last_repeatable_action must be set after steel-chg");
        assert_eq!(action.command.as_ref(), "steel-chg");
        assert_eq!(
            action.insert_keys.len(),
            2,
            "insert_keys must have 'h' and 'i'"
        );
    }

    // Move to "bar" and replay: change " bar" ("bar" isn't the first word
    // of its line, so it takes its leading space), retype "hi".
    ed.feed_key(key('w'));
    ed.feed_key(key('.'));
    assert_eq!(
        ed.doc().text().to_string(),
        "hihi\n",
        "`.` must change the next selection and retype 'hi'"
    );
}

// ── Lazy command extend forwarding ───────────────────────────────────────────

/// A 3-param non-variadic lambda can be *registered* (it is valid for `call!`
/// invocations with explicit args) but must produce a graceful error when
/// dispatched via keymap injection — which supplies at most 2 args.
///
/// Fail oracle: remove the `cmd_arity > 2` guard in `execute_keymap_command` —
/// the dispatch falls through to Steel with too few args, producing a raw
/// Steel arity-mismatch error instead of a friendly editor message.
#[test]
fn keymap_dispatch_arity_over_2_reports_error() {
    let mut ed = editor_from("-[a]>b\n");
    let names: Vec<String> = ed
        .state
        .registry
        .native_mappable_names()
        .map(str::to_owned)
        .collect();
    let name_refs: Vec<&str> = names.iter().map(String::as_str).collect();
    let mut host = ScriptingHost::new();
    host.register_command_names(&name_refs);
    let mut init_host = EditorHostImpl::new(&mut ed.state, &mut ed.view);
    host.eval_source_returning_defs(
        r#"(define-command! "three-params" "" (lambda (a b c) (+ a b c)))"#.to_owned(),
        Default::default(),
        &mut init_host,
    )
    .expect("registration must succeed — 3-param lambda is valid for call! use");
    ed.scripting = Some(host);

    ed.execute_keymap_command("three-params".into(), Some(1), false, ArgSource::Keymap);

    assert!(
        ed.state
            .message_log
            .entries()
            .any(|e| e.severity == crate::editor::Severity::Error),
        "dispatch of arity-3 command via keymap must report a user-facing error"
    );
}

/// The Steel dispatch path must *consume* `pending_char`, exactly like the
/// native wait-char consumers do via `.take()`.  A stale `Some(ch)` left
/// behind would make every later `(pending-char)` call — and every later
/// repeatable command's `char_arg` stamp — see a garbage character.
///
/// Fail oracle: revert `.take()` to a plain read in `Editor::dispatch` — the
/// second dispatch still sees `Some('x')`, moves the cursor again, and the
/// final assertion fails.
#[test]
fn steel_dispatch_consumes_pending_char() {
    let mut ed = editor_from("-[a]>bcdef\n");
    let names: Vec<String> = ed
        .state
        .registry
        .native_mappable_names()
        .map(str::to_owned)
        .collect();
    let name_refs: Vec<&str> = names.iter().map(String::as_str).collect();
    let mut host = ScriptingHost::new();
    host.register_command_names(&name_refs);
    let mut init_host = live_host!(ed);
    host.eval_source_returning_defs(
        // Moves the cursor only when a pending char is visible to the body.
        r#"(define-command! "probe-char" ""
             (lambda () (if (pending-char) (call! "move-right" 1) (+ 1 0))))"#
            .to_owned(),
        Default::default(),
        &mut init_host,
    )
    .expect("define-command! must succeed");
    ed.scripting = Some(host);

    // Simulate a WaitChar keymap node having stored the argument char.
    ed.state.pending_char = Some('x');
    ed.execute_keymap_command("probe-char".into(), Some(1), false, ArgSource::Keymap);

    assert_eq!(
        state(&ed),
        "a-[b]>cdef\n",
        "body must see the pending char and move right"
    );
    assert!(
        ed.state.pending_char.is_none(),
        "dispatch must consume pending_char"
    );

    // A later dispatch without a fresh WaitChar must see #f, not the stale 'x'.
    ed.execute_keymap_command("probe-char".into(), Some(1), false, ArgSource::Keymap);
    assert_eq!(
        state(&ed),
        "a-[b]>cdef\n",
        "stale pending_char must not leak into later dispatch"
    );
}

// ── current_selections / char_index_to_line ───────────────────────────────────

/// `current_selections` returns all selections sorted by start, one per cursor.
///
/// Fail oracle: hardcoding a single-element result would pass for one cursor
/// but fail here, where two cursors must both appear in start order.
#[test]
fn current_selections_sorted_multi_cursor() {
    // "-[ab]>c -[de]>f\n" — text "abc def\n": selection 1 anchor=0 head=1,
    // selection 2 anchor=4 head=5 (hand-counted from the annotated buffer).
    let mut ed = editor_from("-[ab]>c -[de]>f\n");
    let host = live_host!(ed);
    let sels = host
        .current_selections()
        .expect("pane state must be seeded");
    assert_eq!(
        sels,
        vec![(0, 1, true), (4, 5, false)],
        "selections must be sorted by start, primary flagged on the first"
    );
}

/// `current_selections` must preserve backward direction (anchor > head),
/// never normalize it.
///
/// Fail oracle: normalizing to `(min, max)` would report `(0, 1, true)`
/// instead of `(1, 0, true)`.
#[test]
fn current_selections_preserves_backward_direction() {
    // "<[ab]-c\n" — backward selection: head=0, anchor=1 (hand-counted).
    let mut ed = editor_from("<[ab]-c\n");
    let host = live_host!(ed);
    let sels = host
        .current_selections()
        .expect("pane state must be seeded");
    assert_eq!(
        sels,
        vec![(1, 0, true)],
        "backward selection must report anchor > head, not normalized"
    );
}

/// The `primary?` flag must follow `SelectionSet`'s `primary_index`, not
/// always the first (start-sorted) selection.
///
/// Fail oracle: always flagging index 0 as primary would report
/// `(0, 0, true)` instead of `(4, 4, true)`.
#[test]
fn current_selections_primary_flag_follows_primary_index() {
    use hume_editing::selection::{Selection, SelectionSet};

    let mut ed = editor_from("-[a]>bcde\n");
    ed.set_current_selections(SelectionSet::from_vec(
        vec![Selection::collapsed(0), Selection::collapsed(4)],
        1,
    ));
    let host = live_host!(ed);
    let sels = host
        .current_selections()
        .expect("pane state must be seeded");
    assert_eq!(
        sels,
        vec![(0, 0, false), (4, 4, true)],
        "primary flag must follow primary_index, not selection order"
    );
}

/// `char_index_to_line` maps a 0-indexed char offset to its 1-indexed line.
///
/// Independent oracle: expected lines are hand-counted from the buffer text,
/// not derived via `char_to_line` or any shared helper.
#[test]
fn char_index_to_line_maps_offsets() {
    // "ab\ncd\n" — a=0 b=1 \n=2 c=3 d=4 \n=5 (6 chars).
    let mut ed = editor_from("-[a]>b\ncd\n");
    let host = live_host!(ed);
    assert_eq!(
        host.char_index_to_line(0),
        Some(1),
        "offset 0 ('a') is on line 1"
    );
    assert_eq!(
        host.char_index_to_line(3),
        Some(2),
        "offset 3 ('c') is on line 2"
    );
}

/// `char_index_to_line` returns `None` for an offset past the buffer's length,
/// but still succeeds at the exact boundary (`idx == len_chars()`).
#[test]
fn char_index_to_line_out_of_range_returns_none() {
    // "ab\ncd\n" — 6 chars, len_chars() == 6. Every buffer ends with a
    // structural '\n' (HUME invariant), so ropey counts a trailing virtual
    // empty line after it: line 1 "ab\n", line 2 "cd\n", line 3 "" — idx 6
    // (== len_chars()) sits on that third, empty line.
    let mut ed = editor_from("-[a]>b\ncd\n");
    let host = live_host!(ed);
    assert_eq!(
        host.char_index_to_line(7),
        None,
        "offset past len_chars() must be None"
    );
    assert_eq!(
        host.char_index_to_line(6),
        Some(3),
        "offset exactly at len_chars() is still a valid boundary (trailing virtual line)"
    );
}

/// End-to-end: a Steel command reads `(current-selections)` and compares it
/// against a literal quoted list — pins the exact ints/bools/list shape that
/// crosses the Steel boundary, not just the Rust-side tuple data.
///
/// Fail oracle: if the Steel-visible shape were wrong (wrong index order,
/// wrong types), `equal?` would fail, the `unless` would fire, and `delete`
/// would mutate the buffer — the assertion on `state(&ed)` catches that.
#[test]
fn current_selections_steel_roundtrip() {
    let mut ed = editor_from("-[a]>bc\n");

    let names: Vec<String> = ed
        .state
        .registry
        .native_mappable_names()
        .map(str::to_owned)
        .collect();
    let name_refs: Vec<&str> = names.iter().map(String::as_str).collect();

    let mut host = ScriptingHost::new();
    host.register_command_names(&name_refs);

    let mut init_host = live_host!(ed);
    host.eval_source_returning_defs(
        r#"(define-command! "probe-selections-roundtrip" ""
             (lambda ()
               (unless (equal? (current-selections) (list (list 0 0 #t)))
                 (call! "delete" 1))))"#
            .to_owned(),
        Default::default(),
        &mut init_host,
    )
    .expect("define-command! must succeed");

    ed.scripting = Some(host);
    ed.execute_keymap_command(
        "probe-selections-roundtrip".into(),
        Some(1),
        false,
        ArgSource::Keymap,
    );

    assert_eq!(
        state(&ed),
        "-[a]>bc\n",
        "buffer must be untouched: (current-selections) must equal '((0 0 #t))"
    );
}
