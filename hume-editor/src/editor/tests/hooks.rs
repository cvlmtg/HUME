use super::*;
use crate::editor::commands::open_pane;

// ── OnModeChange: Insert → Normal ─────────────────────────────────────────────

/// `cmd_exit_insert` (Esc) must fire `OnModeChange` for the Insert→Normal
/// transition.  Before the fix, `end_insert_session` wrote `state.mode`
/// directly, bypassing the funnel, so the hook never reached script handlers.
///
/// Verification: install an `on-mode-change` handler that calls `move-right`;
/// the cursor advances only if the hook fired.
#[test]
fn exit_insert_via_esc_fires_on_mode_change() {
    use crate::testing::MockHost;
    use crossterm::event::Event;
    use hume_scripting::ScriptingHost;

    // Two-char buffer; cursor starts at col 0.
    let mut ed = editor_from("-[a]>b\n");

    let mut host = ScriptingHost::new();
    let mut mock = MockHost::new();
    host.eval_source(
        r#"(register-hook! 'on-mode-change (lambda (old new) (call! "move-right")))"#,
        &mut mock,
    )
    .unwrap();
    ed.scripting = Some(host);

    // Enter Insert via `i` (no step-back on exit). Use handle_event so the
    // Normal→Insert hook is drained before we capture the before state.
    ed.handle_event(Event::Key(key('i')));
    assert_eq!(ed.state.mode, Mode::Insert, "must be in Insert after `i`");

    let before = state(&ed);

    // Exit via Esc. handle_event drains hooks after dispatch, so the
    // on-mode-change handler fires within this call.
    ed.handle_event(Event::Key(key_esc()));

    assert_eq!(ed.state.mode, Mode::Normal, "must be Normal after Esc");
    assert_ne!(
        state(&ed),
        before,
        "on-mode-change handler (move-right) must have fired on Insert→Normal via Esc"
    );
}

/// A left mouse click while in Insert mode must fire `OnModeChange` exactly
/// once for the Insert→Normal transition. The click path calls
/// `end_insert_session`, which goes through the funnel on its own — a
/// separate `set_mode(Normal)` after it would double-fire the hook.
#[test]
fn mouse_click_in_insert_fires_on_mode_change() {
    use crate::testing::MockHost;
    use crossterm::event::Event;
    use hume_scripting::ScriptingHost;

    let mut ed = editor_from("-[a]>b\n");
    ed.view.panes[ed.state.focused_pane_id].viewport =
        hume_engine::pane::ViewportState::new(80, 24);

    let mut host = ScriptingHost::new();
    let mut mock = MockHost::new();
    host.eval_source(
        r#"(register-hook! 'on-mode-change (lambda (old new) (call! "move-right")))"#,
        &mut mock,
    )
    .unwrap();
    ed.scripting = Some(host);

    // Enter Insert mode.
    ed.handle_key(key('i'));
    assert_eq!(ed.state.mode, Mode::Insert, "must be in Insert after `i`");

    let before = state(&ed);

    // Left-click at (col=1, row=0) — lands in content, triggers exit-insert.
    let click = crossterm::event::MouseEvent {
        kind: crossterm::event::MouseEventKind::Down(crossterm::event::MouseButton::Left),
        column: 1,
        row: 0,
        modifiers: crossterm::event::KeyModifiers::NONE,
    };
    ed.handle_event(Event::Mouse(click));

    assert_eq!(ed.state.mode, Mode::Normal, "must be Normal after click");
    assert_ne!(
        state(&ed),
        before,
        "on-mode-change handler (move-right) must have fired on Insert→Normal via click"
    );
}

// ── Hook cascade cap ──────────────────────────────────────────────────────────

/// A handler feedback loop (an `on-language-set` handler that always flips the
/// language between two values) must be cut off by the drain cap instead of
/// livelocking the editor.  The watchdog only bounds each individual eval,
/// not the re-drain loop.
///
/// Fail oracle: remove the `MAX_HOOK_DRAIN_HOOKS` cap from `drain_hooks` →
/// this test never returns.
#[test]
fn hook_feedback_loop_is_cut_off_by_drain_cap() {
    use crate::testing::MockHost;
    use hume_scripting::ScriptingHost;

    let mut ed = editor_from("-[a]>b\n");

    let mut host = ScriptingHost::new();
    let mut mock = MockHost::new();
    host.eval_source(
        r#"(register-hook! 'on-language-set
             (lambda (bid lang)
               (set-buffer-language! bid (if (equal? lang "aaa") "bbb" "aaa"))))"#,
        &mut mock,
    )
    .unwrap();
    ed.scripting = Some(host);

    // Kick off the ping-pong: aaa → handler sets bbb → handler sets aaa → …
    let bid = ed.focused_buffer_id();
    let lang = ed.state.languages.intern("aaa");
    ed.set_buffer_language(bid, Some(lang));
    ed.drain_hooks(); // must return, not hang

    assert!(
        ed.state
            .message_log
            .entries()
            .any(|e| e.severity == Severity::Error && e.text.contains("hook cascade exceeded")),
        "drain cap must log an Error naming the hook cascade"
    );
    assert!(
        ed.state.pending_hooks.is_empty(),
        "pending hooks must be dropped when the cap fires"
    );
}

/// An *amplifying* handler feedback loop — one that enqueues more hooks than
/// it received — doubles the pending batch every pass (1, 2, 4, 8, …). A cap
/// on pass *count* lets total work explode geometrically (2^100 evals at the
/// old 100-pass limit, never finishing); the cap must instead bound total
/// hooks processed so this terminates quickly regardless of growth shape.
///
/// Each handler invocation sets the buffer's language to `"a"` then `"b"`;
/// since the two calls always differ, both are genuine changes and both
/// re-enqueue `OnLanguageSet` — independent of what the previous invocation
/// left behind.
///
/// Fail oracle: cap `drain_hooks` on pass count instead of total hooks
/// processed → this test times out instead of returning.
#[test]
fn amplifying_hook_cascade_is_cut_off_by_drain_cap() {
    use crate::testing::MockHost;
    use hume_scripting::ScriptingHost;

    let mut ed = editor_from("-[a]>b\n");

    let mut host = ScriptingHost::new();
    let mut mock = MockHost::new();
    host.eval_source(
        r#"(register-hook! 'on-language-set
             (lambda (bid lang)
               (set-buffer-language! bid "a")
               (set-buffer-language! bid "b")))"#,
        &mut mock,
    )
    .unwrap();
    ed.scripting = Some(host);

    let bid = ed.focused_buffer_id();
    let lang = ed.state.languages.intern("start");
    ed.set_buffer_language(bid, Some(lang));
    ed.drain_hooks(); // must return promptly, not after 2^100 evals

    assert!(
        ed.state
            .message_log
            .entries()
            .any(|e| e.severity == Severity::Error && e.text.contains("hook cascade exceeded")),
        "drain cap must log an Error naming the hook cascade"
    );
    assert!(
        ed.state.pending_hooks.is_empty(),
        "pending hooks must be dropped when the cap fires"
    );
}

// ── Startup hook drain ────────────────────────────────────────────────────────

/// `fire_hook_silent` only enqueues; hooks must be drained explicitly via
/// `drain_hooks()` before the event loop or they silently defer.
///
/// This covers the `lib.rs::run()` path: `init_scripting` + `open_extra_files`
/// enqueue `OnBufferOpen`/`OnLanguageSet` hooks (before the terminal is even
/// initialized); the explicit `drain_hooks()` after startup is what fires them.
///
/// Fail oracle: remove `editor.drain_hooks()` from `lib.rs::run()` — hooks
/// silently defer. This test catches the missing-drain regression.
#[test]
fn startup_hooks_require_explicit_drain() {
    use crate::testing::MockHost;
    use hume_scripting::ScriptingHost;
    use hume_scripting::SteelBufferId;
    use hume_scripting::hooks::HookId;

    let mut ed = editor_from("-[a]>b\n");

    // Install a handler: OnBufferOpen → move-right (observable side effect).
    let mut host = ScriptingHost::new();
    let mut mock = MockHost::new();
    host.eval_source(
        r#"(register-hook! 'on-buffer-open (lambda (bid) (call! "move-right")))"#,
        &mut mock,
    )
    .unwrap();
    ed.scripting = Some(host);

    // Simulate what open_extra_files / init_scripting do: enqueue the hook.
    let bid = ed.focused_buffer_id();
    let val = SteelBufferId::new(bid).into_steel_val();
    ed.fire_hook_silent(HookId::OnBufferOpen, &[val]);

    // Hook is enqueued but has not fired yet.
    assert!(
        !ed.state.pending_hooks.is_empty(),
        "pending_hooks must be queued after fire_hook_silent — drain_hooks not called yet"
    );

    let before = state(&ed);

    // drain_hooks fires the enqueued hooks.
    ed.drain_hooks();
    assert!(
        ed.state.pending_hooks.is_empty(),
        "pending_hooks must be empty after drain_hooks"
    );
    assert_ne!(
        state(&ed),
        before,
        "OnBufferOpen handler must have run: move-right should have moved the cursor"
    );
}

// ── Hook (call! …) dispatch ───────────────────────────────────────────────────

/// `fire_hook_silent` must dispatch commands called by `(call! …)` inside hook bodies.
#[test]
fn hook_call_is_dispatched() {
    use crate::testing::MockHost;
    use hume_scripting::ScriptingHost;
    use hume_scripting::SteelBufferId;
    use hume_scripting::hooks::HookId;

    // Build a two-character buffer so move-right has room; cursor at col 0.
    let mut ed = editor_from("-[a]>b\n");
    // Wire up a scripting host with an on-buffer-open handler that calls move-right.
    let mut host = ScriptingHost::new();
    let mut mock = MockHost::new();
    host.eval_source(
        r#"(register-hook! 'on-buffer-open (lambda (bid) (call! "move-right")))"#,
        &mut mock,
    )
    .unwrap();
    ed.scripting = Some(host);

    let before = state(&ed);
    let bid = ed.focused_buffer_id();
    let val = SteelBufferId::new(bid).into_steel_val();
    ed.fire_hook_silent(HookId::OnBufferOpen, &[val]);
    ed.drain_hooks();

    assert_ne!(
        state(&ed),
        before,
        "hook-queued move-right must move the cursor"
    );
}

/// Propagate an edit through two panes that view the same buffer and verify
/// the non-focused pane's engine selections are updated immediately.
#[test]
fn propagate_cs_syncs_engine_pane_for_non_focused_pane() {
    let mut ed = editor_from("-[a]>b\n");
    let buf_id = ed.focused_buffer_id();

    // Create a second pane (not the focused one) viewing the same buffer.
    let second_pane = open_pane(&mut ed.state, &mut ed.view, buf_id);
    assert!(ed.view.panes.contains_key(second_pane));

    // Edit in the focused pane (insert 'x' → "xab\n").
    ed.handle_key(key('i'));
    ed.handle_key(key('x'));
    ed.handle_key(key_esc());

    // The non-focused pane's engine selections must have been synced by
    // propagate_cs_to_panes — not left empty or stale.
    let engine_pane = &ed.view.panes[second_pane];
    assert!(
        !engine_pane.selections.is_empty(),
        "non-focused pane engine selections must be synced after edit"
    );
}
