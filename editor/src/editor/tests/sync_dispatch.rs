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
        let fpid = $ed.focused_pane_id;
        EditorHostImpl {
            settings:        &mut $ed.settings,
            keymap:          &mut $ed.keymap,
            focused_pane_id: fpid,
            buffers:         Some(&mut $ed.buffers),
            engine_view:     Some(&mut $ed.engine_view),
            pane_state:      Some(&mut $ed.pane_state),
            pane_jumps:      Some(&mut $ed.pane_jumps),
            languages:       Some(&mut $ed.languages),
            registry:        Some(&$ed.registry),
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

/// `run_command_sync` for an `EditorCmd` must return `Ok(false)` — the caller
/// is responsible for queuing it for post-eval dispatch.
#[test]
fn run_command_sync_editor_cmd_returns_false() {
    let mut ed = editor_from("-[a]>bc\n");
    let mut host = live_host!(ed);
    // `undo` is an EditorCmd — cannot run sync; must return false.
    let ran = host.run_command_sync("undo", 1, false)
        .expect("run_command_sync must not error for undo");
    assert!(!ran, "EditorCmd must return false (defer to queue)");
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
    let names: Vec<String> = ed.registry.native_mappable_names().map(str::to_owned).collect();
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
