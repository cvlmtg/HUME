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

    let mut init_host = EditorHostImpl { state: &mut ed.state, view: &mut ed.view };
    host
        .eval_source_returning_defs(
            r#"(define-command! "move-right-5" "" (lambda () (call! "move-right" 5)))"#.to_owned(),
            Default::default(),
            &mut init_host,
        )
        .expect("define-command! must succeed");

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

    let mut init_host = EditorHostImpl { state: &mut ed.state, view: &mut ed.view };
    host
        .eval_source_returning_defs(
            r#"(define-command! "move-right-bad" "" (lambda () (call! "move-right" "garbage")))"#.to_owned(),
            Default::default(),
            &mut init_host,
        )
        .expect("define-command! must succeed");

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
/// - Cursor is immediately 1, so the `(when (= (cursor-char-index) 1) ...)` arm
///   fires and calls `(move-right)` a second time → final position 2.
///
/// Fail oracle: if dispatch defers commands → cursor lands at 1 instead of 2.
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
    let mut init_host = EditorHostImpl { state: &mut ed.state, view: &mut ed.view };
    host
        .eval_source_returning_defs(
            r#"(define-command! "test-case-b" "Case B probe"
                 (lambda ()
                   (move-right)
                   (when (= (cursor-char-index) 1)
                     (move-right))))"#
                .to_owned(),
            Default::default(),
            &mut init_host,
        )
        .expect("define-command! must succeed");

    ed.scripting = Some(host);

    ed.execute_keymap_command("test-case-b".into(), 1, false, vec![]);

    let final_state = state(&ed);
    // Both moves ran inside the lambda → cursor at 2, "ab-[c]>\n".
    // Fail oracle: if dispatch defers → cursor at 1, "a-[b]>c\n".
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

    let mut init_host = EditorHostImpl { state: &mut ed.state, view: &mut ed.view };
    host
        .eval_source_returning_defs(
            r#"(define-command! "steel-dot-repeat" "Repeat last action via Steel"
                 (lambda () (call! "repeat-last-action")))"#
                .to_owned(),
            Default::default(),
            &mut init_host,
        )
        .expect("define-command! must succeed");

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

// ── Steel insert-mode dot-repeat ─────────────────────────────────────────────

/// Helper: register all native command names into a fresh ScriptingHost, eval a
/// Steel snippet, attach the host to the editor, and bind the named Steel command
/// to F2 in Normal mode.
fn setup_steel_f2(ed: &mut Editor, snippet: &str, cmd_name: &str) {
    use crate::editor::keymap::BindMode;
    use crossterm::event::KeyCode;

    let names: Vec<String> = ed.state.registry.native_mappable_names().map(str::to_owned).collect();
    let name_refs: Vec<&str> = names.iter().map(String::as_str).collect();

    let mut host = ScriptingHost::new();
    host.register_command_names(&name_refs);

    let mut init_host = EditorHostImpl { state: &mut ed.state, view: &mut ed.view };
    host
        .eval_source_returning_defs(snippet.to_owned(), Default::default(), &mut init_host)
        .expect("Steel snippet must compile and evaluate without error");

    ed.scripting = Some(host);

    let f2 = crossterm::event::KeyEvent::new(KeyCode::F(2), crossterm::event::KeyModifiers::NONE);
    ed.state.keymap.bind_user_with_extend(
        BindMode::Normal, &[f2], std::borrow::Cow::Owned(cmd_name.to_owned()), false,
    );
}

/// A Steel repeatable command that calls `(call! "insert-before")` must record
/// typed text in `insert_keys` and replay it on `.`.
///
/// Fail oracle:
/// - If `end_insert_session` did NOT back-fill `insert_keys` for Steel actions,
///   `insert_keys` would be empty → `.` inserts nothing → final buffer differs.
/// - If the Gap A fix were reverted (empty recipe), `.` would still insert but
///   only at the raw cursor position instead of the recipe-re-established one.
///   This test uses an empty prior recipe so it cannot distinguish that; Test 2
///   proves Gap A independently.
#[test]
fn steel_repeatable_insert_dot_repeat_replays_command_and_typed_text() {
    let f2 = crossterm::event::KeyEvent::new(
        crossterm::event::KeyCode::F(2),
        crossterm::event::KeyModifiers::NONE,
    );
    let mut ed = editor_from("-[x]>\n");
    setup_steel_f2(
        &mut ed,
        r#"(define-command-repeatable! "steel-ins" "enter insert before selection"
             (lambda () (call! "insert-before")))"#,
        "steel-ins",
    );

    // F2 → enters Insert at selection start.
    ed.feed_key(f2);
    // Type 'a', 'b'.
    ed.feed_key(key('a'));
    ed.feed_key(key('b'));
    ed.feed_key(key_esc()); // back to Normal; buffer is "abx"
    assert_eq!(ed.doc().text().to_string(), "abx\n", "setup: 'ab' must be inserted before 'x'");

    // White-box: insert_keys must be back-filled.
    {
        let action = ed.state.last_repeatable_action.as_ref()
            .expect("last_repeatable_action must be set after steel-ins");
        assert_eq!(action.command.as_ref(), "steel-ins", "action must be attributed to steel-ins");
        assert_eq!(
            action.insert_keys.len(), 2,
            "insert_keys must contain both typed chars ('a' and 'b')"
        );
    }

    // Move selection to 'x' and replay with `.`.
    ed.feed_key(key('w')); // select 'x' (collapsed → non-collapsed word)
    ed.feed_key(key('.')); // replay: enter insert before 'x', retype "ab"

    // After replay: "ab" is inserted again before the 'x' that was at pos 0,
    // giving "ababx\n".
    assert_eq!(
        ed.doc().text().to_string(),
        "ababx\n",
        "`.` must re-enter insert and retype 'ab' before the selection"
    );
}

/// A prior native selection command (`w`) builds the `selection_recipe`. A Steel
/// repeatable command that enters insert via `(call! "insert-before")` must NOT
/// clobber that recipe — the pre-body snapshot must survive the inner dispatch.
///
/// Fail oracle (Gap A): without the pre-body snapshot in `execute.rs`,
/// `insert-before`'s inner dispatch calls into `dispatch_native`, which does
/// `mem::take(selection_recipe)` on the repeatable-edit branch, leaving the recipe
/// empty. The white-box assertion `selection_recipe.len() == 1` catches this:
/// it passes with the fix, fails without it.
#[test]
fn steel_repeatable_insert_preserves_prior_selection_recipe() {
    let f2 = crossterm::event::KeyEvent::new(
        crossterm::event::KeyCode::F(2),
        crossterm::event::KeyModifiers::NONE,
    );
    // "w" from 'f' in "foo bar\n" selects "bar" (skips current word, selects next).
    let mut ed = editor_from("-[f]>oo bar\n");
    setup_steel_f2(
        &mut ed,
        r#"(define-command-repeatable! "steel-ins" "enter insert before selection"
             (lambda () (call! "insert-before")))"#,
        "steel-ins",
    );

    // `w` establishes a real (non-collapsed) selection → recipe non-empty.
    ed.feed_key(key('w'));
    assert_eq!(ed.state.selection_recipe.len(), 1, "pre-condition: w must push a recipe step");

    // F2 → steel-ins → insert 'X' before "bar".
    ed.feed_key(f2);
    ed.feed_key(key('X'));
    ed.feed_key(key_esc());
    // `w` selected "bar" (pos 4-6); insert-before put 'X' at pos 4 → "foo Xbar\n".
    assert_eq!(ed.doc().text().to_string(), "foo Xbar\n");

    // White-box (primary proof): recipe must survive the inner `insert-before` dispatch.
    //
    // Without the Gap A fix in execute.rs the inner dispatch does mem::take on
    // selection_recipe (repeatable-edit branch), leaving it empty → len == 0 → FAILS.
    // With the fix, the snapshot was taken before the body ran → len == 1 → PASSES.
    let action = ed.state.last_repeatable_action.as_ref()
        .expect("last_repeatable_action must be set after steel-ins");
    assert_eq!(
        action.selection_recipe.len(), 1,
        "selection_recipe must not be clobbered by the inner (call! \"insert-before\")"
    );
    assert!(!action.selection_recipe[0].extend, "w step must be a Move (establish)");
}

/// After `.` replays a Steel insert action, a single `u` must undo the entire
/// replayed edit as one step — proving `drain_pending_repeat`'s edit-group
/// bracketing still works correctly for the Steel insert path.
///
/// Mirrors `dot_is_single_undo_step` from dot_repeat.rs but uses a Steel command.
#[test]
fn steel_repeatable_insert_dot_repeat_single_undo() {
    let f2 = crossterm::event::KeyEvent::new(
        crossterm::event::KeyCode::F(2),
        crossterm::event::KeyModifiers::NONE,
    );
    let mut ed = editor_from("-[x]>y\n");
    setup_steel_f2(
        &mut ed,
        r#"(define-command-repeatable! "steel-ins" "enter insert before selection"
             (lambda () (call! "insert-before")))"#,
        "steel-ins",
    );

    // F2, type "AB", Esc → "ABxy"
    ed.feed_key(f2);
    ed.feed_key(key('A'));
    ed.feed_key(key('B'));
    ed.feed_key(key_esc());
    assert_eq!(ed.doc().text().to_string(), "ABxy\n");

    // `w` from 'x' (pos 2 in "ABxy\n") selects "xy" (pos 2-3, to end of word).
    // `.` replay: no recipe, insert-before at the selection start (pos 2), retype "AB".
    ed.feed_key(key('w'));
    ed.feed_key(key('.'));
    assert_eq!(ed.doc().text().to_string(), "ABABxy\n");

    // One undo must revert the entire replay.
    ed.feed_key(key('u'));
    assert_eq!(ed.doc().text().to_string(), "ABxy\n", "one undo must revert the full replay");
}

/// The `change` command (`c`) opens its undo group *through* `begin_insert_session`
/// (the only code path that opens a group). A Steel command that wraps `change`
/// via `(call! "change")` must therefore still create an InsertSession and record
/// `insert_keys` correctly — the edit-then-insert group-guard case is NOT
/// triggered here because `change` never leaves a bare edit group open without a
/// session.
///
/// This test guards that the one reachable edit-before-insert path remains safe.
#[test]
fn steel_repeatable_change_via_call_records_insert_keys() {
    let f2 = crossterm::event::KeyEvent::new(
        crossterm::event::KeyCode::F(2),
        crossterm::event::KeyModifiers::NONE,
    );
    let mut ed = editor_from("-[foo]> bar\n");
    setup_steel_f2(
        &mut ed,
        r#"(define-command-repeatable! "steel-chg" "change selection"
             (lambda () (call! "change")))"#,
        "steel-chg",
    );

    // F2 (steel-chg = change), type "hi", Esc.
    ed.feed_key(f2);
    ed.feed_key(key('h'));
    ed.feed_key(key('i'));
    ed.feed_key(key_esc()); // "hi bar\n"
    assert_eq!(ed.doc().text().to_string(), "hi bar\n");

    // White-box: insert_keys must be back-filled.
    {
        let action = ed.state.last_repeatable_action.as_ref()
            .expect("last_repeatable_action must be set after steel-chg");
        assert_eq!(action.command.as_ref(), "steel-chg");
        assert_eq!(action.insert_keys.len(), 2, "insert_keys must have 'h' and 'i'");
    }

    // Move to "bar" and replay: change "bar", retype "hi".
    ed.feed_key(key('w'));
    ed.feed_key(key('.'));
    assert_eq!(
        ed.doc().text().to_string(),
        "hi hi\n",
        "`.` must change the next selection and retype 'hi'"
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

    let names: Vec<String> = ed.state.registry.native_mappable_names().map(str::to_owned).collect();
    let name_refs: Vec<&str> = names.iter().map(String::as_str).collect();
    let mut host = ScriptingHost::new();
    host.register_command_names(&name_refs);

    let mut init_host = EditorHostImpl { state: &mut ed.state, view: &mut ed.view };
    host
        .eval_source_returning_defs(
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

    let mut init_host = EditorHostImpl { state: &mut ed.state, view: &mut ed.view };
    host
        .eval_source_returning_defs(
            r#"(define-command! "steel-delete" ""
                 (lambda () (call! "delete")))"#
                .to_owned(),
            Default::default(),
            &mut init_host,
        )
        .expect("define-command! must succeed");

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

    let mut init_host = EditorHostImpl { state: &mut ed.state, view: &mut ed.view };
    // "noop-cmd" dispatches no inner native — no (call! …) anywhere.
    host
        .eval_source_returning_defs(
            r#"(define-command! "noop-cmd" "" (lambda () (+ 1 0)))"#.to_owned(),
            Default::default(),
            &mut init_host,
        )
        .expect("define-command! must succeed");

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

    let mut init_host = EditorHostImpl { state: &mut ed.state, view: &mut ed.view };
    host
        .eval_source_returning_defs(
            r#"(define-command! "steel-delete" ""
                 (lambda () (call! "delete")))"#
                .to_owned(),
            Default::default(),
            &mut init_host,
        )
        .expect("define-command! must succeed");

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

    let mut init_host = EditorHostImpl { state: &mut ed.state, view: &mut ed.view };
    host
        .eval_source_returning_defs(
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

    let mut init_host = EditorHostImpl { state: &mut ed.state, view: &mut ed.view };
    host
        .eval_source_returning_defs(
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

    let names: Vec<String> = ed.state.registry.native_mappable_names().map(str::to_owned).collect();
    let name_refs: Vec<&str> = names.iter().map(String::as_str).collect();
    let mut host = ScriptingHost::new();
    host.register_command_names(&name_refs);

    let mut init_host = EditorHostImpl { state: &mut ed.state, view: &mut ed.view };
    host
        .eval_source_returning_defs(
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
    ed.execute_keymap_command("order-test".into(), 1, false, vec![]);

    // If correct order (move-right then delete): 'b' is deleted → "a\n".
    // If reversed (delete then move-right): 'a' is deleted → "b\n".
    assert_eq!(
        ed.doc().text().to_string(), "a\n",
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

    let names: Vec<String> = ed.state.registry.native_mappable_names().map(str::to_owned).collect();
    let name_refs: Vec<&str> = names.iter().map(String::as_str).collect();
    let mut host = ScriptingHost::new();
    host.register_command_names(&name_refs);

    let mut init_host = EditorHostImpl { state: &mut ed.state, view: &mut ed.view };
    // noop-steel is a no-op plugin command; move-down 3 runs sync via %call-native!.
    host
        .eval_source_returning_defs(
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
    ed.execute_keymap_command("count-chain-test".into(), 1, false, vec![]);

    let host = live_host!(ed);
    let line = host.current_line_number().expect("current_line_number");
    // Started on line 1, moved down 3 → should be on line 4.
    assert_eq!(line, 4, "native count=3 must be preserved in plugin→native chain; got line {line}");
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

    let mut init_host = EditorHostImpl { state: &mut ed.state, view: &mut ed.view };
    host
        .eval_source_returning_defs(
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
    ed.execute_keymap_command("warn-test".into(), 1, false, vec![]);

    let idx = live_host!(ed).cursor_char_index().expect("cursor_char_index");
    // Both move-rights run inline despite the unknown name — cursor ends at 2.
    assert_eq!(idx, 2, "both moves must execute despite unknown command in between; got {idx}");
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
    use crossterm::event::Event;
    use hume_scripting::hooks::HookId;

    let mut ed = Editor::for_testing(crate::editor::buffer::Buffer::new(
        hume_editing::text::Text::from("hello\n"),
        hume_editing::selection::SelectionSet::default(),
    ));

    // Give the pane a viewport big enough that a click at row=0,col=0 lands in content.
    ed.view.panes[ed.state.focused_pane_id].viewport = hume_engine::pane::ViewportState::new(80, 24);

    // Seed a pending hook (OnBufferSave with no args — no handler registered, so
    // drain_hooks skips the Steel call but still removes it from the queue).
    ed.fire_hook_silent(HookId::OnBufferSave, &[]);
    assert!(
        !ed.state.pending_hooks.is_empty(),
        "pending_hooks must be non-empty before the event — drain has not run yet"
    );

    // Simulate a left-click at (0, 0) via handle_event so the drain choke point runs.
    let click = crossterm::event::MouseEvent {
        kind: crossterm::event::MouseEventKind::Down(crossterm::event::MouseButton::Left),
        column: 0,
        row: 0,
        modifiers: crossterm::event::KeyModifiers::NONE,
    };
    ed.handle_event(Event::Mouse(click));

    // drain_hooks ran at the tail of handle_event — all pending hooks must be gone.
    assert!(
        ed.state.pending_hooks.is_empty(),
        "pending_hooks must be empty after handle_event; got {:?}", ed.state.pending_hooks
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
    let mut init_host = EditorHostImpl { state: &mut ed_steel.state, view: &mut ed_steel.view };
    host
        .eval_source_returning_defs(
            r#"(define-command! "steel-delete" "" (lambda () (call! "delete")))"#.to_owned(),
            Default::default(),
            &mut init_host,
        )
        .expect("define-command! must succeed");
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
    let mut init_host = EditorHostImpl { state: &mut ed_steel.state, view: &mut ed_steel.view };
    host
        .eval_source_returning_defs(
            r#"(define-command! "steel-goto-end" "" (lambda () (call! "goto-last-line")))"#.to_owned(),
            Default::default(),
            &mut init_host,
        )
        .expect("define-command! must succeed");
    ed_steel.scripting = Some(host);
    ed_steel.execute_keymap_command("steel-goto-end".into(), 1, false, vec![]);
    let snap_steel = snapshot_bookkeeping(&ed_steel);

    assert_eq!(
        snap_key, snap_steel,
        "keypress vs Steel dispatch of 'goto-last-line' must leave identical jump bookkeeping"
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
/// `inner-move` is applied inline (plugin funcall in the VM), so cursor=1 by
/// the time `(cursor-char-index)` is evaluated → the `(when …)` branch fires →
/// second move-right → cursor=2.
///
/// Fail oracle: comment out the `if proc { apply proc args }` branch in
/// `%dispatch-command` so all commands fall through to `%call-native!` — the
/// plugin command queues, cursor stays 0 during eval, branch does not fire → 1.
#[test]
fn plugin_calls_plugin_cursor_read_is_live() {
    // "-[a]>bc\n", cursor at position 0.
    let mut ed = editor_from("-[a]>bc\n");

    let names: Vec<String> = ed.state.registry.native_mappable_names().map(str::to_owned).collect();
    let name_refs: Vec<&str> = names.iter().map(String::as_str).collect();
    let mut host = ScriptingHost::new();
    host.register_command_names(&name_refs);

    let mut init_host = EditorHostImpl { state: &mut ed.state, view: &mut ed.view };
    // inner-move: plugin command that wraps a single move-right.
    // outer-cmd: calls inner-move (plugin→plugin), reads cursor, conditionally
    //   moves right again if cursor advanced past 0.
    host
        .eval_source_returning_defs(
            r#"(define-command! "inner-move" ""
                 (lambda () (call! "move-right")))
               (define-command! "outer-cmd" ""
                 (lambda ()
                   (call! "inner-move")
                   (when (> (cursor-char-index) 0)
                     (call! "move-right"))))"#
                .to_owned(),
            Default::default(),
            &mut init_host,
        )
        .expect("define-command! must succeed");

    ed.scripting = Some(host);
    ed.execute_keymap_command("outer-cmd".into(), 1, false, vec![]);

    let idx = live_host!(ed).cursor_char_index().expect("cursor_char_index after outer-cmd");
    // Inline: inner-move ran synchronously (cursor=1 during eval), branch fired → cursor=2.
    // Deferred: inner-move queued (cursor=0 during eval), branch skipped → cursor=1.
    assert_eq!(
        idx, 2,
        "plugin→plugin inline dispatch: cursor must be 2 (branch fired on live read); \
         got {idx} — likely deferral regression"
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
    let _defs = host
        .eval_source_returning_defs(
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
/// `is_init` branch → eval returns `Err`, the `(set-option! …)` is never reached,
/// assertions 1 and 2 fail.
#[test]
fn native_call_bang_at_init_top_level_warns_and_skips() {
    // Content is irrelevant — cursor stays at 0.
    let mut ed = editor_from("-[a]>bc\n");

    let names: Vec<String> = ed.state.registry.native_mappable_names().map(str::to_owned).collect();
    let name_refs: Vec<&str> = names.iter().map(String::as_str).collect();
    let mut host = ScriptingHost::new();
    host.register_command_names(&name_refs);

    let mut init_host = EditorHostImpl { state: &mut ed.state, view: &mut ed.view };
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
    assert!(result.is_ok(), "eval must not abort on native call at init top-level; got: {result:?}");

    // 2. The line after the native call must have been applied.
    ed.state.history.set_capacity(ed.state.settings.history_capacity);
    assert_eq!(
        ed.state.settings.history_capacity, 77,
        "set-option! after native call must be applied (eval continued past it)"
    );

    // 3. The native command itself must have been skipped.
    let idx = live_host!(ed).cursor_char_index().expect("cursor_char_index");
    assert_eq!(idx, 0, "cursor must not move (native command skipped during init); got {idx}");

    // 4. A warning must have been produced for the skipped command.
    let msgs = host.take_pending_messages();
    let has_warn = msgs.iter().any(|(lvl, txt)| {
        matches!(lvl, hume_scripting::LogLevel::Warning) && txt.contains("move-right")
    });
    assert!(has_warn, "a Warning containing 'move-right' must be emitted; got: {msgs:?}");
}
