use super::*;
use crate::editor::doc_ops;
use crate::editor::position_stores::PositionStores;
use hume_scripting::host::CommandHost;
use pretty_assertions::assert_eq;

/// One `undo_n(1)` walk through the history funnel, on the focused pane.
fn undo_through_funnel(
    ed: &mut Editor,
) -> Result<doc_ops::HistoryWalk, crate::editor::error::CommandError> {
    let pane = ed.state.focus.id();
    let buffer = ed.focused_buffer_id();
    doc_ops::apply_doc_history_walk(
        &mut ed.state.buffers,
        &mut PositionStores::new(
            &mut ed.state.panes,
            &mut ed.state.input,
            &mut ed.state.buffer_positions,
            &mut ed.state.config.decorations,
        ),
        &mut ed.state.active_session,
        pane,
        buffer,
        |b, id, stores, acting| b.undo_n(id, stores, acting, 1),
    )
}

#[test]
fn history_walk_commits_own_paste_session_first() {
    let mut ed = editor_from("-[h]>ello\n");
    ed.feed_key(key('y'));
    ed.feed_key(key('p'));
    let (pane, buffer) = (ed.state.focus.id(), ed.focused_buffer_id());
    assert!(
        ed.state
            .active_session
            .as_ref()
            .is_some_and(|s| s.is_paste_at(pane, buffer)),
        "setup: `p` leaves a paste session open"
    );
    assert_eq!(ed.doc().text().to_string(), "hhello\n");

    let walk = undo_through_funnel(&mut ed).expect("an own paste session is not a conflict");

    assert_eq!(walk, doc_ops::HistoryWalk::Took(1));
    assert_eq!(
        ed.doc().text().to_string(),
        "hello\n",
        "the committed paste is the one revision the walk undoes"
    );
    assert!(ed.state.active_session.is_none());
}

#[test]
fn history_walk_refuses_own_insert_session() {
    let mut ed = editor_from("-[h]>ello\n");
    ed.feed_key(key('i'));
    ed.feed_key(key('x'));
    let (pane, buffer) = (ed.state.focus.id(), ed.focused_buffer_id());
    assert!(
        ed.state
            .active_session
            .as_ref()
            .is_some_and(|s| s.is_insert_at(pane, buffer)),
        "setup: `i` leaves an insert session open"
    );

    let walk = undo_through_funnel(&mut ed);

    assert!(
        walk.is_err(),
        "a walk under an open insert group must be refused"
    );
    assert_eq!(ed.doc().text().to_string(), "xhello\n");
    assert!(
        ed.state
            .active_session
            .as_ref()
            .is_some_and(|s| s.is_insert_at(pane, buffer)),
        "the refused walk leaves the session open"
    );
}

// ── goto_revision ─────────────────────────────────────────────────────────────

/// `commands::edit::goto_revision` on the focused pane.
fn goto(
    ed: &mut Editor,
    revision: usize,
) -> Result<doc_ops::HistoryWalk, crate::editor::error::CommandError> {
    let pane = crate::editor::commands::CommandPane::existing(&ed.view, ed.state.focus.id())
        .expect("the focused pane exists");
    crate::editor::commands::goto_revision(&mut ed.state, &ed.view, pane, revision)
}

/// Two branches off the root: `a` deletes the first char, `b` the second.
/// Leaves the editor on branch `b`, and returns each branch's state string
/// and revision number alongside the root's.
struct Branches {
    root: (String, usize),
    a: (String, usize),
    b: (String, usize),
}

fn two_branches(ed: &mut Editor) -> Branches {
    let snapshot = |ed: &Editor| (state(ed), ed.doc().revision_id().index());
    let root = snapshot(ed);
    ed.feed_key(key('d'));
    let a = snapshot(ed);
    ed.feed_key(key('u'));
    ed.feed_key(key('l'));
    ed.feed_key(key('d'));
    let b = snapshot(ed);
    Branches { root, a, b }
}

#[test]
fn goto_revision_jumps_across_branches_through_the_funnel() {
    let mut ed = editor_from("-[h]>ello\n");
    let br = two_branches(&mut ed);
    assert_ne!(br.a.0, br.b.0, "setup: the branches differ");

    let walk = goto(&mut ed, br.a.1).expect("revision a is live");
    assert_eq!(
        walk,
        doc_ops::HistoryWalk::Took(2),
        "up to the root, down to a"
    );
    assert_eq!(state(&ed), br.a.0);
    assert_eq!(ed.doc().revision_id().index(), br.a.1);

    goto(&mut ed, br.b.1).expect("revision b is live");
    assert_eq!(state(&ed), br.b.0);

    // A walk restores the selections stored on the edge it last crossed, so
    // only the root's text is fixed, not where the cursor lands on it.
    goto(&mut ed, br.root.1).expect("the root is live");
    assert_eq!(ed.doc().text().to_string(), "hello\n");
}

#[test]
fn goto_revision_to_current_is_a_zero_step_walk() {
    let mut ed = editor_from("-[h]>ello\n");
    let br = two_branches(&mut ed);
    let generation = ed.doc().text().generation();

    let walk = goto(&mut ed, br.b.1).expect("the current revision is live");

    assert_eq!(walk, doc_ops::HistoryWalk::Took(0));
    assert_eq!(state(&ed), br.b.0);
    assert_eq!(ed.doc().text().generation(), generation);
}

#[test]
fn goto_revision_unknown_id_errors_and_leaves_history() {
    let mut ed = editor_from("-[h]>ello\n");
    let br = two_branches(&mut ed);

    assert!(goto(&mut ed, 999).is_err());

    assert_eq!(state(&ed), br.b.0);
    assert_eq!(ed.doc().revision_id().index(), br.b.1);
}

#[test]
fn goto_revision_on_read_only_buffer_is_refused() {
    let mut ed = editor_from("-[h]>ello\n");
    let br = two_branches(&mut ed);
    ed.doc_mut().read_only = true;

    let walk = goto(&mut ed, br.a.1).expect("a refusal is not an error");

    assert_eq!(walk, doc_ops::HistoryWalk::RefusedReadOnly);
    assert_eq!(state(&ed), br.b.0);
    assert_eq!(ed.doc().revision_id().index(), br.b.1);
}

#[test]
fn goto_revision_carries_other_pane_selections() {
    let mut ed = editor_from("-[a]>bcdef\n");
    let pane_a = ed.state.focus.id();
    let bid = ed.focused_buffer_id();
    ed.feed_key(key('d'));
    assert_eq!(state(&ed), "-[b]>cdef\n");

    let handle = focused_pane(&ed);
    live_host!(ed)
        .run_command_sync("pane-vsplit", handle, Some(1), false, None)
        .expect("pane-vsplit must succeed");
    let pane_b = ed.state.focus.id();
    assert_ne!(pane_a, pane_b);
    for _ in 0..3 {
        ed.feed_key(key('l'));
    }
    assert_eq!(state(&ed), "bcd-[e]>f\n");

    ed.switch_focused_pane(pane_a);
    let root = 0;
    goto(&mut ed, root).expect("the root is live");

    assert_eq!(
        state(&ed),
        "-[a]>bcdef\n",
        "the acting pane gets the walk's selections"
    );
    ed.switch_focused_pane(pane_b);
    assert_eq!(
        state(&ed),
        "abcd-[e]>f\n",
        "the other pane's cursor stays on its character"
    );
    assert_eq!(ed.focused_buffer_id(), bid);
}

#[test]
fn redo_key_after_goto_follows_the_jumped_branch() {
    let mut ed = editor_from("-[h]>ello\n");
    let br = two_branches(&mut ed);

    goto(&mut ed, br.a.1).expect("revision a is live");
    ed.feed_key(key('u'));
    assert_eq!(state(&ed), br.root.0);
    ed.feed_key(key('U'));

    assert_eq!(
        state(&ed),
        br.a.0,
        "redo continues along the branch the jump entered, not the newest one"
    );
}

// ── Steel host seam ───────────────────────────────────────────────────────────

#[test]
fn buffer_undo_tree_reports_ids_parents_current_and_saved() {
    use hume_scripting::host::BufferHost;

    let mut ed = editor_from("-[h]>ello\n");
    let br = two_branches(&mut ed);
    let bid = ed.focused_buffer_id();

    let nodes = live_host!(ed)
        .buffer_undo_tree(bid)
        .expect("the buffer is open");

    let shape: Vec<_> = nodes.iter().map(|n| (n.id, n.parent)).collect();
    assert_eq!(
        shape,
        [
            (br.root.1, None),
            (br.a.1, Some(br.root.1)),
            (br.b.1, Some(br.root.1))
        ]
    );
    let current: Vec<_> = nodes.iter().filter(|n| n.current).map(|n| n.id).collect();
    assert_eq!(current, [br.b.1]);
    let saved: Vec<_> = nodes.iter().filter(|n| n.saved).map(|n| n.id).collect();
    assert_eq!(saved, [br.root.1], "the buffer was last saved at the root");
    assert!(
        live_host!(ed)
            .buffer_undo_tree(BufferId::default())
            .is_none()
    );
}

#[test]
fn goto_revision_host_errors_on_read_only_and_unknown_id() {
    use hume_scripting::host::EditHost;

    let mut ed = editor_from("-[h]>ello\n");
    let br = two_branches(&mut ed);
    let handle = focused_pane(&ed);

    assert!(live_host!(ed).goto_revision(handle, 999).is_err());
    assert_eq!(state(&ed), br.b.0);

    ed.doc_mut().read_only = true;
    assert!(live_host!(ed).goto_revision(handle, br.a.1).is_err());
    assert_eq!(state(&ed), br.b.0, "a read-only buffer is left alone");
}

#[test]
fn goto_revision_host_acts_on_the_given_pane_not_focus() {
    use hume_scripting::host::EditHost;

    let mut ed = editor_from("-[a]>bcdef\n");
    let pane_a = ed.state.focus.id();
    let bid = ed.focused_buffer_id();
    ed.feed_key(key('d'));
    let handle = focused_pane(&ed);
    live_host!(ed)
        .run_command_sync("pane-vsplit", handle, Some(1), false, None)
        .expect("pane-vsplit must succeed");
    let pane_b = ed.state.focus.id();
    for _ in 0..3 {
        ed.feed_key(key('l'));
    }

    let target_a = hume_scripting::PaneHandle::with_pane(bid, pane_a);
    live_host!(ed)
        .goto_revision(target_a, 0)
        .expect("the root is live");

    assert_eq!(ed.state.focus.id(), pane_b, "the walk does not move focus");
    ed.switch_focused_pane(pane_a);
    assert_eq!(
        state(&ed),
        "-[a]>bcdef\n",
        "the named pane gets the walk's selections"
    );
    ed.switch_focused_pane(pane_b);
    assert_eq!(
        state(&ed),
        "abcd-[e]>f\n",
        "the focused pane's cursor is mapped"
    );
}
