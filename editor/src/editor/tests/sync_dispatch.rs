use super::*;

use scripting::ScriptingHost;
use crate::editor::host_impl::EditorHostImpl;
use crate::testing::MockHost;
use scripting::host::EditorHost;

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

/// `run_command_sync` for a `Motion` command must return `Ok(true)` and
/// immediately update the cursor position — no queue involved.
#[test]
fn run_command_sync_motion_returns_true_and_moves_cursor() {
    // "-[a]>bc\n" — cursor at position 0.
    let mut ed = editor_from("-[a]>bc\n");
    let before = live_host!(ed).cursor_char_index().expect("cursor_char_index before");

    {
        let mut host = live_host!(ed);
        // move-right is a Motion — must dispatch synchronously.
        let ran = host.run_command_sync("move-right", 1, false)
            .expect("run_command_sync must not error for move-right");
        assert!(ran, "Motion command must return true (ran sync)");
    }

    let after = live_host!(ed).cursor_char_index().expect("cursor_char_index after");
    assert_eq!(before, 0, "cursor must start at 0");
    assert_eq!(after, 1, "cursor must be at 1 after sync move-right");
}

/// All `EditorCmd` handlers share the State shape and run synchronously via
/// `run_command_sync`, returning `Ok(true)`. `repeat-last-action` enqueues a
/// `PendingRepeat` marker as a pure State handler; the actual replay runs in
/// `drain_pending_repeat` at the tail of `handle_key`.
#[test]
fn run_command_sync_editor_cmd_returns_true() {
    let mut ed = editor_from("-[a]>bc\n");
    let mut host = live_host!(ed);
    // `undo` is a State EditorCmd — runs synchronously; must return true.
    let ran = host.run_command_sync("undo", 1, false)
        .expect("run_command_sync must not error for undo");
    assert!(ran, "State EditorCmd must return true (ran sync)");

    // `repeat-last-action` is also a State EditorCmd now — sets pending_repeat and
    // returns true (no action to repeat, so pending_repeat stays None, but it ran sync).
    let ran_repeat = host.run_command_sync("repeat-last-action", 1, false)
        .expect("run_command_sync must not error for repeat-last-action");
    assert!(ran_repeat, "repeat-last-action must return true (State EditorCmd, runs sync)");
}

/// `run_command_sync` for an unknown name must return `Err`.
#[test]
fn run_command_sync_unknown_name_errors() {
    let mut ed = editor_from("-[a]>bc\n");
    let mut host = live_host!(ed);
    let result = host.run_command_sync("no-such-command-xyzzy", 1, false);
    assert!(result.is_err(), "unknown command must return Err");
}

/// `cursor_char_index` must reflect the live cursor position (not a frozen snapshot).
#[test]
fn cursor_char_index_reads_live_position() {
    // "-[a]>bc\n" — cursor at position 0.
    let mut ed = editor_from("-[a]>bc\n");
    let idx = live_host!(ed).cursor_char_index().expect("cursor_char_index");
    assert_eq!(idx, 0);
}

/// `current_line_number` must return 1 for a single-line buffer with cursor at start.
#[test]
fn current_line_number_reads_live_position() {
    // "-[a]>bc\n" — cursor on line 1 (1-indexed).
    let mut ed = editor_from("-[a]>bc\n");
    let line = live_host!(ed).current_line_number().expect("current_line_number");
    assert_eq!(line, 1);
}

/// `run_command_sync` for a `Selection` command (e.g. `extend-line-end`) must
/// return `Ok(true)` and immediately update the selection.
#[test]
fn run_command_sync_selection_returns_true_and_updates_sel() {
    // "-[a]>bc\n" — cursor at 0, single-char selection covering 'a'.
    let mut ed = editor_from("-[a]>bc\n");
    {
        let mut host = live_host!(ed);
        // select-line is a Selection command.
        let ran = host.run_command_sync("select-line", 1, false)
            .expect("run_command_sync must not error for select-line");
        assert!(ran, "Selection command must return true (ran sync)");
    }
    // select-line covers the full line "abc\n"; head ends on 'c' at position 2.
    let head = live_host!(ed).cursor_char_index().expect("cursor_char_index after sel");
    assert!(head > 0, "selection must have extended past start");
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

    let idx = {
        let state = &ed.state;
        let buf_id = state.panes.state
            .get(state.focused_pane_id).unwrap()
            .values().next().unwrap()
            .selections.primary().head();
        buf_id
    };
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
