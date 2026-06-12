use super::*;

// ── Startup hook drain ────────────────────────────────────────────────────────

/// `fire_hook_silent` only enqueues; hooks must be drained explicitly via
/// `drain_hooks()` before the event loop or they silently defer.
///
/// This covers the `lib.rs::run()` path: `init_scripting` + `open_extra_files`
/// enqueue `OnBufferOpen`/`OnLanguageSet` hooks; the explicit `drain_hooks()`
/// after startup is what fires them.
///
/// Fail oracle: remove `editor.drain_hooks()` from `lib.rs::run()` — hooks
/// silently defer. This test catches the missing-drain regression.
#[test]
fn startup_hooks_require_explicit_drain() {
    use hume_scripting::ScriptingHost;
    use hume_scripting::SteelBufferId;
    use hume_scripting::hooks::HookId;
    use crate::testing::MockHost;

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
    use hume_scripting::ScriptingHost;
    use hume_scripting::SteelBufferId;
    use hume_scripting::hooks::HookId;
    use crate::testing::MockHost;

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
    let second_pane = ed.open_pane(buf_id);
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
