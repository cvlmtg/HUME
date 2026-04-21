use super::*;

// ── Hook cmd_queue routing ────────────────────────────────────────────────────

/// `fire_hook_silent` must dispatch commands queued by `(call! …)` inside hook
/// bodies — previously they were silently discarded.
#[test]
fn hook_cmd_queue_is_dispatched() {
    use crate::scripting::ScriptingHost;
    use crate::editor::keymap::Keymap;
    use crate::settings::EditorSettings;
    use crate::scripting::hooks::HookId;
    use crate::scripting::builtins::ids::SteelBufferId;

    // Build a two-character buffer so move-right has room; cursor at col 0.
    let mut ed = editor_from("-[a]>b\n");
    // Wire up a scripting host with an on-buffer-open handler that calls move-right.
    let mut host = ScriptingHost::new();
    let mut s = EditorSettings::default();
    let mut km = Keymap::default();
    host.eval_source(
        r#"(register-hook! 'on-buffer-open (lambda (bid) (call! "move-right")))"#,
        &mut s, &mut km,
    ).unwrap();
    ed.scripting = Some(host);

    let before = state(&ed);
    let bid = ed.focused_buffer_id();
    let val = SteelBufferId(bid).into_steel_val();
    ed.fire_hook_silent(HookId::OnBufferOpen, &[val]);

    assert_ne!(state(&ed), before, "hook-queued move-right must move the cursor");
}

/// Propagate an edit through two panes that view the same buffer and verify
/// the non-focused pane's engine selections are updated immediately.
#[test]
fn propagate_cs_syncs_engine_pane_for_non_focused_pane() {
    let mut ed = editor_from("-[a]>b\n");
    let buf_id = ed.focused_buffer_id();

    // Create a second pane (not the focused one) viewing the same buffer.
    let second_pane = ed.open_pane(buf_id);
    assert!(ed.engine_view.panes.contains_key(second_pane));

    // Edit in the focused pane (insert 'x' → "xab\n").
    ed.handle_key(key('i'));
    ed.handle_key(key('x'));
    ed.handle_key(key_esc());

    // The non-focused pane's engine selections must have been synced by
    // propagate_cs_to_panes — not left empty or stale.
    let engine_pane = &ed.engine_view.panes[second_pane];
    assert!(!engine_pane.selections.is_empty(),
        "non-focused pane engine selections must be synced after edit");
}
