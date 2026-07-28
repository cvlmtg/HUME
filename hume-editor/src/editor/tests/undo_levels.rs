use super::*;

use crate::editor::buffer::Buffer;

/// Type 'x' then 'y' as two separate insert sessions (Escape between them,
/// so each records its own undo revision), returning the buffer state
/// captured right after the first session.
fn two_edits(ed: &mut Editor) -> String {
    ed.feed_key(key('i'));
    ed.feed_key(key('x'));
    ed.feed_key(key_esc());
    let after_first_edit = state(ed);

    ed.feed_key(key('i'));
    ed.feed_key(key('y'));
    ed.feed_key(key_esc());

    after_first_edit
}

// ── typed `:set global undo-levels` ─────────────────────────────────────────

#[test]
fn typed_set_applies_to_open_buffers() {
    // Fail oracle: remove the "undo-levels" arm from
    // settings_ops::resync_derived_state and the cap is never pushed to the
    // buffer — both edits would remain undoable instead of the first being
    // evicted/promoted away.
    let mut ed = editor_from("-[h]>ello\n");
    crate::editor::commands::typed_set(&mut ed, Some("global undo-levels=1"), false)
        .expect("set undo-levels");

    let after_first_edit = two_edits(&mut ed);
    assert!(ed.doc().can_undo());

    ed.feed_key(key('u'));
    assert_eq!(state(&ed), after_first_edit);
    assert!(!ed.doc().can_undo());
}

#[test]
fn new_buffer_inherits_undo_levels() {
    // A buffer opened after the :set must pick up the already-configured
    // cap, not start at History's unlimited default.
    // Fail oracle: skip threading undo_levels through lifecycle::open_buffer
    // and this second buffer would allow both edits to stay undoable.
    let mut ed = editor_from("-[h]>ello\n");
    crate::editor::commands::typed_set(&mut ed, Some("global undo-levels=1"), false)
        .expect("set undo-levels");

    let bid2 = ed.open_buffer(Buffer::scratch());
    ed.switch_to_buffer_with_jump(bid2);

    let after_first_edit = two_edits(&mut ed);
    assert!(ed.doc().can_undo());

    ed.feed_key(key('u'));
    assert_eq!(state(&ed), after_first_edit);
    assert!(!ed.doc().can_undo());
}

// ── Steel `(set-option! "undo-levels" …)` ────────────────────────────────

#[test]
fn steel_set_option_applies_undo_levels() {
    // set-option! routes through EditorHostImpl::set_global_option ->
    // settings_ops::apply_global, which resyncs every open buffer's cap
    // inline — no separate pickup step needed after eval returns.
    // Fail oracle: reintroduce a raw write_global call in set_global_option
    // (bypassing settings_ops::apply_global) and this cap never reaches the
    // buffer, so the second undo would still succeed.
    let mut ed = editor_from("-[h]>ello\n");

    let names: Vec<String> = ed
        .state
        .registry
        .native_mappable_names()
        .map(str::to_owned)
        .collect();
    let name_refs: Vec<&str> = names.iter().map(String::as_str).collect();
    let mut host = hume_scripting::ScriptingHost::new();
    host.register_command_names(&name_refs);

    let mut init_host = crate::editor::host_impl::EditorHostImpl::new(&mut ed.state, &mut ed.view);
    let result = host.eval_source_returning_defs(
        r#"(set-option! "undo-levels" 1)"#.to_owned(),
        Default::default(),
        &mut init_host,
    );
    assert!(result.is_ok(), "eval must succeed: {result:?}");
    assert_eq!(ed.state.settings.undo_levels, 1);

    let after_first_edit = two_edits(&mut ed);
    assert!(ed.doc().can_undo());

    ed.feed_key(key('u'));
    assert_eq!(state(&ed), after_first_edit);
    assert!(!ed.doc().can_undo());
}
