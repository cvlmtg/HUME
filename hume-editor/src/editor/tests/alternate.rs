use super::*;
use hume_scripting::host::CommandHost;
use pretty_assertions::assert_eq;

// ── Helpers ───────────────────────────────────────────────────────────────────

// ── alternate_buffer() ────────────────────────────────────────────────────────

#[test]
fn alternate_buffer_none_with_single_buffer() {
    let ed = editor_from("-[h]>ello\n");
    assert_eq!(ed.state.buffers.second_most_recent(), None);
}

// ── goto-alternate-buffer  ────────────────────────────────────────────────────

/// Worked example from `cmd_goto_alternate_buffer`'s own doc: a remote call
/// on a non-focused pane B must use the *global* history (`mru`'s own last
/// two entries), not B's own buffer — and a second remote call on B must
/// toggle back, since B's own touches are now part of that same history.
#[test]
fn goto_alternate_buffer_on_a_remote_pane_uses_the_global_history_and_toggles_on_repeat() {
    use crate::editor::buffer::Buffer;
    use crate::editor::commands::open_pane_in_layout;
    use hume_editing::selection::SelectionSet;
    use hume_editing::text::BufferText;
    use hume_engine::pipeline::Direction;
    use hume_scripting::PaneHandle;

    let mut ed = editor_from("-[b]>ar\n"); // "bar\n"
    let bar_bid = ed.focused_buffer_id();
    let pid_a = ed.state.focus.id();

    let baz_bid = ed.open_buffer(Buffer::new(
        BufferText::from("baz\n"),
        SelectionSet::default(),
    ));
    let foo_bid = ed.open_buffer(Buffer::new(
        BufferText::from("foo\n"),
        SelectionSet::default(),
    ));
    // `open` seeds `mru` at open time regardless of whether a pane ever
    // shows the buffer; re-touch bar then foo directly (bypassing real
    // focus events, which this test doesn't need) so `mru` ends at
    // `[baz, bar, foo]` — foo most recent, bar the one before it.
    ed.state.buffers.touch_mru(bar_bid);
    ed.state.buffers.touch_mru(foo_bid);
    assert_eq!(
        ed.state.buffers.second_most_recent(),
        Some(bar_bid),
        "setup: bar must be the second-most-recent"
    );

    // A actually shows foo (the focused buffer for this whole test); B is a
    // sibling pane showing baz. Neither call touches `mru`.
    crate::editor::buffer::lifecycle::switch_pane_to_buffer(
        &mut ed.state,
        &mut ed.view,
        pid_a,
        foo_bid,
    );
    let pid_b = open_pane_in_layout(
        &mut ed.state,
        &mut ed.view,
        pid_a,
        baz_bid,
        Direction::Horizontal,
    )
    .expect("split must succeed");
    assert_eq!(
        ed.focused_buffer_id(),
        foo_bid,
        "setup: A must stay focused on foo; opening B must not move focus"
    );

    // First remote call: B (baz) → bar, the buffer excluded by the *focused*
    // pane's history — not baz, B's own buffer, which the pre-fix code used.
    let ran = live_host!(ed)
        .run_command_sync(
            "goto-alternate-buffer",
            PaneHandle::with_pane(baz_bid, pid_b),
            Some(1),
            false,
            None,
        )
        .expect("goto-alternate-buffer on B must not error");
    assert!(ran);
    assert_eq!(
        ed.view.panes[pid_b].buffer_id, bar_bid,
        "B must switch to bar, the buffer before foo in the global history"
    );
    assert_eq!(
        ed.focused_buffer_id(),
        foo_bid,
        "A must be untouched by a remote dispatch on B"
    );

    // Second remote call, B now on bar: must toggle back to baz — B's own
    // outgoing touch from the first call is now part of the same history.
    let ran = live_host!(ed)
        .run_command_sync(
            "goto-alternate-buffer",
            PaneHandle::with_pane(bar_bid, pid_b),
            Some(1),
            false,
            None,
        )
        .expect("goto-alternate-buffer on B must not error");
    assert!(ran);
    assert_eq!(
        ed.view.panes[pid_b].buffer_id, baz_bid,
        "a second remote call on B must toggle back to baz"
    );
    assert_eq!(
        ed.focused_buffer_id(),
        foo_bid,
        "A must still be untouched after the second remote dispatch"
    );
}

#[test]
fn goto_alternate_buffer_warns_when_no_alternate() {
    let mut ed = editor_from("-[h]>ello\n");
    let id_before = ed.focused_buffer_id();
    let pane = focused_pane(&ed);
    let ran = live_host!(ed)
        .run_command_sync("goto-alternate-buffer", pane, Some(1), false, None)
        .expect("goto-alternate-buffer must not error");
    assert!(!ran, "a refusal with no alternate must report Ok(false)");
    assert_eq!(
        ed.focused_buffer_id(),
        id_before,
        "no buffer change with no alternate"
    );
    let msg = ed
        .state
        .status_msg
        .as_deref()
        .expect("warning should be reported");
    assert!(
        msg.contains("No alternate buffer"),
        "unexpected status: {msg:?}"
    );
}

// ── %/# expansion in typed commands ──────────────────────────────────────────

#[test]
fn colon_e_hash_errors_with_no_alternate() {
    let mut ed = editor_from("-[h]>ello\n");
    type_cmd(&mut ed, ":e #");
    let msg = ed
        .state
        .status_msg
        .as_deref()
        .expect("error should be reported");
    assert!(
        msg.contains("No alternate buffer"),
        "unexpected status: {msg:?}"
    );
}

#[test]
fn colon_e_percent_errors_with_no_path() {
    let mut ed = editor_from("-[h]>ello\n");
    type_cmd(&mut ed, ":e %");
    let msg = ed
        .state
        .status_msg
        .as_deref()
        .expect("error should be reported");
    assert!(msg.contains("No file name"), "unexpected status: {msg:?}");
}

// ── goto-alternate-buffer in registry ──────────────────────────────────────────

#[test]
fn goto_alternate_buffer_is_registered_as_jump() {
    let reg = super::super::registry::CommandRegistry::with_defaults();
    let cmd = reg
        .get_mappable("goto-alternate-buffer")
        .expect("goto-alternate-buffer must be registered");
    assert!(
        cmd.meta().is_jump,
        "goto-alternate-buffer must have jump:true"
    );
}
