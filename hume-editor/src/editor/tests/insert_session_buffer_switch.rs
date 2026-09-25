//! An open Insert or paste session on the focused pane must be torn down
//! before that pane's buffer changes out from under it — `focus::
//! end_focus_sessions`, called from `buffer::lifecycle::switch_pane_to_buffer`
//! (gated to the focused pane and a genuine buffer change) and from
//! `Editor::reset_config_state`. Left open, the session's `edit_group`/
//! `paste_group` stays keyed to the buffer the pane no longer shows, and the
//! next dispatch panics in `Buffer::commit_edit_group`'s or
//! `EditorState::commit_paste_session`'s own consistency check.
//!
//! Covers every path that can swap the focused pane's buffer mid-session
//! (`switch-to-buffer!`, `goto-location!`, `close-buffer!`, a Pane-category
//! `call!`, `:reload-config`), plus the two cases teardown must *not* fire:
//! a remote pane's own switch, and a `goto-location!` that lands back in the
//! buffer already focused.

use super::*;
use hume_scripting::PaneHandle;
use hume_scripting::host::{BufferHost, CommandHost, EditHost};
use pretty_assertions::assert_eq;

// ── switch-to-buffer! ───────────────────────────────────────────────────────

#[test]
fn switch_to_buffer_ends_insert_session_on_focused_pane() {
    let mut ed = editor_from("-[a]>bc\n");
    let old_bid = ed.focused_buffer_id();
    let new_bid = ed.open_buffer(Buffer::new(
        BufferText::from("xyz\n"),
        SelectionSet::default(),
    ));

    ed.feed_key(key('i'));
    type_chars(&mut ed, "Q");
    assert_eq!(ed.state.mode(), Mode::Insert, "sanity: Insert is open");

    let pane = focused_pane(&ed);
    live_host!(ed)
        .switch_to_buffer(pane, new_bid)
        .expect("switch must succeed");

    assert_eq!(
        ed.state.mode(),
        Mode::Normal,
        "the switch must end the open Insert session, not leave it dangling on the old buffer"
    );
    assert_eq!(
        ed.state.buffers.get(old_bid).text().to_string(),
        "Qabc\n",
        "the typed char must have committed as its own undo step on the old buffer"
    );
    assert_eq!(ed.focused_buffer_id(), new_bid);

    // A follow-up insert session in the new buffer must apply cleanly — no
    // stale `edit_group` left over to panic `commit_edit_group`'s `.expect()`.
    ed.feed_key(key('i'));
    type_chars(&mut ed, "Z");
    ed.feed_key(key_esc());
    assert_eq!(ed.state.buffers.get(new_bid).text().to_string(), "Zxyz\n");
}

// ── goto-location! ───────────────────────────────────────────────────────────

#[test]
fn goto_location_to_another_buffer_ends_insert_session_on_focused_pane() {
    let mut ed = editor_from("-[a]>bcdef\n");
    let old_bid = ed.focused_buffer_id();
    let new_bid = ed.open_buffer(Buffer::new(
        BufferText::from("xyz\n"),
        SelectionSet::default(),
    ));

    ed.feed_key(key('i'));
    type_chars(&mut ed, "Q");
    assert_eq!(ed.state.mode(), Mode::Insert, "sanity: Insert is open");

    let pane = focused_pane(&ed);
    live_host!(ed)
        .goto_location_buffer(pane, new_bid, hume_rope::line::RopeyLine::new(0), 1)
        .expect("goto-location! must succeed");

    assert_eq!(
        ed.state.mode(),
        Mode::Normal,
        "goto-location! into another buffer must end the open Insert session"
    );
    assert_eq!(
        ed.state.buffers.get(old_bid).text().to_string(),
        "Qabcdef\n",
        "the typed char must have committed on the old buffer"
    );
    assert_eq!(ed.focused_buffer_id(), new_bid);
}

/// `goto-location!` landing back in the buffer already focused must leave a
/// still-open Insert session alone — the buffer never actually changes, so
/// the open `edit_group` is still valid.
#[test]
fn goto_location_within_the_focused_buffer_leaves_insert_session_open() {
    let mut ed = editor_from("-[a]>bcdef\n");
    let bid = ed.focused_buffer_id();

    ed.feed_key(key('i'));
    type_chars(&mut ed, "Q");
    assert_eq!(ed.state.mode(), Mode::Insert, "sanity: Insert is open");

    let pane = focused_pane(&ed);
    live_host!(ed)
        .goto_location_buffer(pane, bid, hume_rope::line::RopeyLine::new(0), 3)
        .expect("goto-location! must succeed");

    assert_eq!(
        ed.state.mode(),
        Mode::Insert,
        "a same-buffer goto must not end the still-valid open Insert session"
    );

    // The still-open session must compose and commit cleanly.
    type_chars(&mut ed, "R");
    ed.feed_key(key_esc());
    assert_eq!(
        ed.state.buffers.get(bid).text().to_string(),
        "QabRcdef\n",
        "both typed chars must have composed into a single insert session"
    );
}

// ── close-buffer! ────────────────────────────────────────────────────────────

#[test]
fn close_buffer_ends_insert_session_on_focused_pane() {
    let mut ed = editor_from("-[a]>bc\n");
    let old_bid = ed.focused_buffer_id();
    let other_bid = ed.open_buffer(Buffer::new(
        BufferText::from("xyz\n"),
        SelectionSet::default(),
    ));

    ed.feed_key(key('i'));
    type_chars(&mut ed, "Q");
    assert_eq!(ed.state.mode(), Mode::Insert, "sanity: Insert is open");

    live_host!(ed)
        .close_buffer(old_bid)
        .expect("close must succeed");

    assert_eq!(
        ed.state.mode(),
        Mode::Normal,
        "closing the focused buffer must end its open Insert session"
    );
    assert_eq!(ed.focused_buffer_id(), other_bid);

    // A follow-up insert session in the redirected buffer must apply cleanly.
    ed.feed_key(key('i'));
    type_chars(&mut ed, "Z");
    ed.feed_key(key_esc());
    assert_eq!(ed.state.buffers.get(other_bid).text().to_string(), "Zxyz\n");
}

// ── A Pane-category `call!` (goto-alternate-buffer) ─────────────────────────

#[test]
fn goto_alternate_buffer_call_ends_insert_session_on_focused_pane() {
    let mut ed = editor_from("-[a]>bc\n");
    let old_bid = ed.focused_buffer_id();
    let other_bid = ed.open_buffer(Buffer::new(
        BufferText::from("xyz\n"),
        SelectionSet::default(),
    ));
    // `open_buffer` seeds `mru` at open time, making `other_bid` the most
    // recent entry; re-touch `old_bid` (the focused buffer) so it becomes
    // most recent again and `other_bid` is the alternate (second-most-recent).
    ed.state.buffers.touch_mru(old_bid);

    ed.feed_key(key('i'));
    type_chars(&mut ed, "Q");
    assert_eq!(ed.state.mode(), Mode::Insert, "sanity: Insert is open");

    let pane = focused_pane(&ed);
    live_host!(ed)
        .run_command_sync("goto-alternate-buffer", pane, None, false, None)
        .expect("goto-alternate-buffer must run");

    assert_eq!(
        ed.state.mode(),
        Mode::Normal,
        "a Pane-category call! that switches the focused pane's buffer must end its Insert session"
    );
    assert_eq!(ed.focused_buffer_id(), other_bid);
    assert_eq!(
        ed.state.buffers.get(old_bid).text().to_string(),
        "Qabc\n",
        "the typed char must have committed on the old buffer"
    );
}

// ── :reload-config ───────────────────────────────────────────────────────────

#[test]
fn reload_config_ends_insert_session_on_focused_pane() {
    let mut ed = editor_from("-[a]>bc\n");
    let bid = ed.focused_buffer_id();

    ed.feed_key(key('i'));
    type_chars(&mut ed, "Q");
    assert_eq!(ed.state.mode(), Mode::Insert, "sanity: Insert is open");

    ed.reset_config_state();

    assert_eq!(
        ed.state.mode(),
        Mode::Normal,
        "reload must end the open Insert session before dropping its mode layers"
    );
    let pid = ed.state.focus.id();
    assert!(
        ed.state.panes.state[pid][bid].edit_group.is_none(),
        "reload must not leave a stale open edit_group behind"
    );

    // A follow-up insert session must work cleanly — no stale group left to
    // panic `commit_edit_group`'s `.expect()` on the next Esc.
    ed.feed_key(key('i'));
    type_chars(&mut ed, "Z");
    ed.feed_key(key_esc());
    assert_eq!(ed.state.buffers.get(bid).text().to_string(), "ZQabc\n");
}

// ── Open paste-cycle session ─────────────────────────────────────────────────

#[test]
fn switch_to_buffer_commits_open_paste_session_on_focused_pane() {
    let mut ed = editor_from("-[a]>bc\n");
    let old_bid = ed.focused_buffer_id();
    ed.feed_key(key('d')); // delete "a" → ring head = ["a"], buffer now "bc\n"
    let new_bid = ed.open_buffer(Buffer::new(
        BufferText::from("xyz\n"),
        SelectionSet::default(),
    ));

    ed.feed_key(key('p')); // bare paste-after: ring head, opens paste_group
    let pid = ed.state.focus.id();
    assert!(
        ed.state.panes.state[pid][old_bid].paste_group.is_some(),
        "sanity: 'p' must leave an open, uncommitted paste session"
    );

    let pane = focused_pane(&ed);
    live_host!(ed)
        .switch_to_buffer(pane, new_bid)
        .expect("switch must succeed");

    assert!(
        ed.state.panes.state[pid][old_bid].paste_group.is_none(),
        "the switch must commit the open paste session on the old buffer, not leave it dangling"
    );

    // A follow-up paste must not trip the debug_assert in
    // `commit_paste_session` (an open session surviving outside the focused
    // (pane, buffer) pair) — reaching this assertion at all is the point.
    ed.feed_key(key('p'));
    assert_ne!(
        ed.state.buffers.get(new_bid).text().to_string(),
        "xyz\n",
        "the follow-up paste must have applied to the new buffer"
    );
}

// ── Remote pane switch must not touch the focused pane's own session ────────

#[test]
fn switch_to_buffer_on_a_non_focused_pane_leaves_focused_insert_session_open() {
    use crate::editor::commands::open_pane_in_layout;
    use hume_engine::pipeline::Direction;

    let mut ed = editor_from("-[a]>bc\n");
    let pid_a = ed.state.focus.id();
    let bid_a = ed.focused_buffer_id();
    let bid_b = ed.open_buffer(Buffer::new(
        BufferText::from("baz\n"),
        SelectionSet::default(),
    ));
    let pid_b = open_pane_in_layout(
        &mut ed.state,
        &mut ed.view,
        pid_a,
        bid_b,
        Direction::Horizontal,
    )
    .expect("split must succeed");
    // `open_pane_in_layout` does not move focus itself — pin it back to A
    // explicitly so the premise below holds regardless.
    ed.state.focus.set_for_test(pid_a);
    let bid_c = ed.open_buffer(Buffer::new(
        BufferText::from("qux\n"),
        SelectionSet::default(),
    ));

    ed.feed_key(key('i'));
    type_chars(&mut ed, "Q");
    assert_eq!(ed.state.mode(), Mode::Insert, "sanity: Insert is open on A");

    // A hook or remote call redirects B, the *non-focused* pane — must not
    // touch A's own open Insert session.
    let pane_b = PaneHandle::with_pane(bid_b, pid_b);
    live_host!(ed)
        .switch_to_buffer(pane_b, bid_c)
        .expect("remote switch must succeed");

    assert_eq!(
        ed.state.mode(),
        Mode::Insert,
        "a remote pane's own buffer switch must not end focus's own Insert session"
    );
    assert_eq!(
        ed.view.panes[pid_b].buffer_id, bid_c,
        "B must have redirected"
    );
    assert_eq!(ed.focused_buffer_id(), bid_a, "focus must be untouched");

    // The still-open session must compose and commit cleanly.
    type_chars(&mut ed, "R");
    ed.feed_key(key_esc());
    assert_eq!(ed.state.mode(), Mode::Normal);
    assert_eq!(
        ed.state.buffers.get(bid_a).text().to_string(),
        "QRabc\n",
        "both typed chars must have composed into a single insert session"
    );
}
