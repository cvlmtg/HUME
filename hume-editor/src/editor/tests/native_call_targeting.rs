//! `EditorHostImpl::run_command_sync` resolving `pane` against a native
//! command's `TargetCategory` — the feature `commands::pipeline::
//! resolve_pane`/`run_resolved` implement. `events.rs`'s
//! `on_buffer_save_native_call_*` tests cover the same feature through a
//! Steel hook end to end; these drive `run_command_sync` directly via
//! `live_host!` for tighter per-category coverage.

use super::*;
use crate::editor::commands::open_pane_in_layout;
use hume_engine::pipeline::Direction;
use hume_scripting::PaneHandle;
use hume_scripting::host::CommandHost;

/// Open a second buffer B (`b`'s content) and split A's pane so B is shown
/// in a non-focused pane. Returns `(pid_a, bid_a, pid_b, bid_b)`; focus
/// stays on A.
fn split_two_buffers(ed: &mut Editor, b: &str) -> (PaneId, BufferId, PaneId, BufferId) {
    let bid_a = ed.focused_buffer_id();
    let pid_a = ed.state.focus.id();
    let bid_b = ed.open_buffer(Buffer::new(BufferText::from(b), SelectionSet::default()));
    let pid_b = open_pane_in_layout(
        &mut ed.state,
        &mut ed.view,
        pid_a,
        bid_b,
        Direction::Horizontal,
    )
    .expect("split onto B must succeed");
    // `open_pane_in_layout` itself does not move focus (unlike
    // `split_pane_onto`/`:vsplit` in production) — pin it back to A
    // explicitly so every caller's premise holds regardless.
    ed.state.focus.set_for_test(pid_a);
    (pid_a, bid_a, pid_b, bid_b)
}

// ── Pane category: a remote call edits its target pane, not focus ──────────

#[test]
fn pane_category_remote_call_edits_target_pane_leaves_focus_untouched() {
    let mut ed = editor_from("-[a]>aa\n");
    let (pid_a, bid_a, pid_b, bid_b) = split_two_buffers(&mut ed, "bbb\n");

    let ran = live_host!(ed)
        .run_command_sync(
            "delete",
            PaneHandle::with_pane(bid_b, pid_b),
            Some(1),
            false,
            None,
        )
        .expect("delete on a split-shown pane must not error");
    assert!(ran, "delete must report it ran");

    assert_eq!(ed.state.buffers.get(bid_b).text().to_string(), "bb\n");
    assert_eq!(
        ed.state.buffers.get(bid_a).text().to_string(),
        "aaa\n",
        "A must be untouched by a remote dispatch on B"
    );
    assert_eq!(
        ed.state.focus.id(),
        pid_a,
        "a remote dispatch must not move focus"
    );
}

#[test]
fn pane_category_paneless_handle_errors_no_pane() {
    let mut ed = editor_from("-[a]>bc\n");
    let bid_b = ed.open_buffer(Buffer::new(
        BufferText::from("bbb\n"),
        SelectionSet::default(),
    ));

    let err = live_host!(ed)
        .run_command_sync(
            "delete",
            PaneHandle::buffer_only(bid_b),
            Some(1),
            false,
            None,
        )
        .expect_err("delete on a pane-less handle must error");
    assert!(err.contains("needs a pane"), "unexpected error: {err}");
    assert_eq!(ed.state.buffers.get(bid_b).text().to_string(), "bbb\n");
}

// ── FocusedPane category: only the focused pane's own handle is accepted ───

#[test]
fn focused_pane_category_errors_on_a_non_focused_pane() {
    let mut ed = editor_from("-[a]>bc\n");
    let (pid_a, bid_a, pid_b, bid_b) = split_two_buffers(&mut ed, "bbb\n");
    let mode_before = ed.state.mode();
    let sels_before = ed.state.panes.state[pid_a][bid_a].selections().clone();

    let err = live_host!(ed)
        .run_command_sync(
            "insert-before",
            PaneHandle::with_pane(bid_b, pid_b),
            Some(1),
            false,
            None,
        )
        .expect_err("insert-before on a non-focused pane must error");
    assert!(err.contains("focused pane"), "unexpected error: {err}");

    assert_eq!(ed.state.mode(), mode_before, "mode must be unchanged");
    assert_eq!(
        ed.state.buffers.get(bid_a).text().to_string(),
        "abc\n",
        "A must be untouched"
    );
    assert_eq!(
        &ed.state.panes.state[pid_a][bid_a].selections().clone(),
        &sels_before,
        "A's selections must be unchanged"
    );
    assert_eq!(ed.state.focus.id(), pid_a);
}

// ── Buffer category: needs only the buffer, no pane at all ─────────────────

#[test]
fn buffer_category_clear_search_works_on_a_paneless_handle() {
    let mut ed = editor_from("-[a]>bc\n");
    let bid_a = ed.focused_buffer_id();
    let bid_b = ed.open_buffer(Buffer::new(
        BufferText::from("bbb\n"),
        SelectionSet::default(),
    ));
    ed.state.buffers.get_mut(bid_a).search_pattern =
        crate::editor::search::SearchPattern::compile("a");
    ed.state.buffers.get_mut(bid_b).search_pattern =
        crate::editor::search::SearchPattern::compile("b");

    let ran = live_host!(ed)
        .run_command_sync(
            "clear-search",
            PaneHandle::buffer_only(bid_b),
            None,
            false,
            None,
        )
        .expect("clear-search on a pane-less handle must not error");
    assert!(ran);

    assert!(
        ed.state.buffers.get(bid_b).search_pattern.is_none(),
        "B's pattern must be cleared"
    );
    assert!(
        ed.state.buffers.get(bid_a).search_pattern.is_some(),
        "A's pattern must survive — clear-search only touches its own buffer"
    );
}

#[test]
fn buffer_category_tab_new_opens_a_tab_on_a_paneless_handle() {
    let mut ed = editor_from("-[a]>bc\n");
    let bid_b = ed.open_buffer(Buffer::new(
        BufferText::from("bbb\n"),
        SelectionSet::default(),
    ));
    let tabs_before = ed.state.tabs.len();

    let ran = live_host!(ed)
        .run_command_sync("tab-new", PaneHandle::buffer_only(bid_b), None, false, None)
        .expect("tab-new on a pane-less handle must not error");
    assert!(ran);

    assert_eq!(ed.state.tabs.len(), tabs_before + 1);
    assert_eq!(
        ed.view.panes[ed.state.focus.id()].buffer_id,
        bid_b,
        "the new tab's pane must show B, the buffer tab-new was called with"
    );
}

// ── Global category: no buffer at all, even a closed one ───────────────────

#[test]
fn global_category_ignores_a_closed_buffer() {
    let mut ed = editor_from("-[a]>bc\n");
    let bid_b = ed.open_buffer(Buffer::new(
        BufferText::from("bbb\n"),
        SelectionSet::default(),
    ));
    ed.close_buffer(bid_b);
    assert!(
        ed.state.buffers.try_get(bid_b).is_none(),
        "setup: bid_b must be closed"
    );
    let extend_before = ed.state.mode() == hume_engine::types::EditorMode::Extend;

    let ran = live_host!(ed)
        .run_command_sync(
            "toggle-extend",
            PaneHandle::buffer_only(bid_b),
            None,
            false,
            None,
        )
        .expect("toggle-extend must ignore even a closed buffer — it needs no buffer at all");
    assert!(ran);
    assert_ne!(
        ed.state.mode() == hume_engine::types::EditorMode::Extend,
        extend_before,
        "the global flag must still flip"
    );
}

// ── Closed buffer: every other category errors ──────────────────────────────

#[test]
fn closed_buffer_errors_for_pane_focused_and_buffer_categories() {
    let mut ed = editor_from("-[a]>bc\n");
    let bid_b = ed.open_buffer(Buffer::new(
        BufferText::from("bbb\n"),
        SelectionSet::default(),
    ));
    ed.close_buffer(bid_b);
    assert!(ed.state.buffers.try_get(bid_b).is_none());

    for name in ["delete", "insert-before", "clear-search"] {
        let result = live_host!(ed).run_command_sync(
            name,
            PaneHandle::buffer_only(bid_b),
            Some(1),
            false,
            None,
        );
        let err = match result {
            Err(e) => e,
            Ok(_) => panic!("'{name}' on a closed buffer must error"),
        };
        assert!(
            err.contains("invalid buffer id"),
            "'{name}': unexpected error: {err}"
        );
    }
}

// ── Bookkeeping skip: a remote dispatch leaves the focused pane's own ──────
// ── dot-repeat / paste-session state untouched (locked decision) ──────────

#[test]
fn remote_call_leaves_focused_panes_dot_repeat_recipe_untouched() {
    let mut ed = editor_from("-[a]>aa\n");
    let (pid_a, bid_a, pid_b, bid_b) = split_two_buffers(&mut ed, "bbb\n");
    // Stage A's own dot-repeat action via a real keypress dispatch.
    ed.feed_key(key('d'));
    assert_eq!(ed.state.buffers.get(bid_a).text().to_string(), "aa\n");
    let recipe_before = ed
        .state
        .last_repeatable_action
        .as_ref()
        .map(|a| a.command.to_string());
    assert_eq!(
        recipe_before.as_deref(),
        Some("delete"),
        "setup: A staged delete"
    );
    let jump_len_before = ed.state.panes.jumps[pid_a].len();

    live_host!(ed)
        .run_command_sync(
            "move-down",
            PaneHandle::with_pane(bid_b, pid_b),
            None,
            false,
            None,
        )
        .expect("move-down on B must not error");

    assert_eq!(
        ed.state
            .last_repeatable_action
            .as_ref()
            .map(|a| a.command.to_string()),
        recipe_before,
        "A's dot-repeat target must survive a remote dispatch on B"
    );
    assert_eq!(
        ed.state.panes.jumps[pid_a].len(),
        jump_len_before,
        "A's own jump list must be untouched by a dispatch that targeted B"
    );
    // `.` on A must still replay its own staged delete, not anything from B.
    ed.feed_key(key('.'));
    assert_eq!(
        ed.state.buffers.get(bid_a).text().to_string(),
        "a\n",
        "`.` on A must still replay A's own staged delete"
    );
}

// ── Jump list follows the target pane, not focus ────────────────────────────

#[test]
fn remote_call_jump_list_follows_the_target_pane() {
    let mut ed = editor_from("-[a]>bc\n");
    let (pid_a, _bid_a, pid_b, bid_b) = split_two_buffers(&mut ed, &"line\n".repeat(10));
    let jump_len_a_before = ed.state.panes.jumps[pid_a].len();
    let jump_len_b_before = ed.state.panes.jumps[pid_b].len();

    live_host!(ed)
        .run_command_sync(
            "goto-last-line",
            PaneHandle::with_pane(bid_b, pid_b),
            None,
            false,
            None,
        )
        .expect("goto-last-line on B must not error");

    assert_eq!(
        ed.state.panes.jumps[pid_b].len(),
        jump_len_b_before + 1,
        "the jump this remote dispatch recorded must land on B's own pane"
    );
    assert_eq!(
        ed.state.panes.jumps[pid_a].len(),
        jump_len_a_before,
        "A's jump list must be untouched"
    );
}

// ── MockHost has no pane model: it accepts any pane unconditionally ────────

#[test]
fn mock_host_records_a_non_focused_pane_unchanged() {
    use crate::testing::MockHost;

    let mut mock = MockHost::new();
    mock.native_names.insert("delete".to_string());
    let pane = PaneHandle::buffer_only(BufferId::default());

    let ran = mock
        .run_command_sync("delete", pane, Some(1), false, None)
        .expect("MockHost must accept any pane — it has no pane model to resolve against");
    assert!(ran);
    assert_eq!(mock.dispatched_native.len(), 1);
    assert_eq!(mock.dispatched_native[0].1, pane);
}
