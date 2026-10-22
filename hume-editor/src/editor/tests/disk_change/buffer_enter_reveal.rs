//! The buffer-enter disk check when a close, cycle or click reveals a buffer.

use super::*;
use pretty_assertions::assert_eq;

// ── `OnBufferEnter`'s disk check: closing/cycling/clicking reveals a
//    buffer with a deferred change ──────────────────────────────────────────
//
// The check is `OnBufferEnter`'s Rust reaction (`Editor::react_to_event`),
// raised by `Editor::settle()`'s own diff against `focused_buffer_id()`, not
// wired per command. Closing a buffer or a pane, cycling pane focus, and
// clicking into another pane all move focus with no write to `buffer_id`
// itself, so a diff taken inside `settle()`'s fixpoint is what catches them,
// not a call any of those commands makes. These tests drive input via
// `feed_event`/`type_cmd_event`, which call `settle()` after dispatch, not
// the usual `feed_key`/`type_cmd`, which leave it queued.

/// A and B both change on disk; B's confirm is answered
/// (implicitly, by the test just moving on); `:q` closes the focused buffer
/// (B) and reveals A, which must re-prompt for its own already-warned
/// change.
///
/// `:q` moves the focused pane onto A through `Editor::close_buffer`,
/// `lifecycle::close_buffer` and `switch_pane_to_buffer`, and none of them
/// runs a `BufferEnter` check. A's already-warned `Changed` state is
/// re-surfaced only by the focus diff in `settle()`. The setup also pins
/// the *trigger*: an `Ambient` check on the unfocused A warns once (the
/// first assertion) and opens no confirm.
#[test]
fn quit_closing_a_buffer_prompts_the_revealed_one() {
    let (mut ed, tmp_a) = editor_with_file("-[h]>ello\n", "hello\n");
    let bid_a = ed.focused_buffer_id();

    let (tmp_b, _tmp_b_guard) = temp_file("world\n");
    type_cmd(&mut ed, &format!(":e {}", tmp_b.display()));
    let bid_b = ed.focused_buffer_id();
    assert_ne!(bid_a, bid_b, "setup: :e must open a distinct second buffer");

    rewrite_externally(&tmp_a, "hello, externally changed!\n");
    let (_, warnings_before) = ed.state.message_log.totals();
    ed.check_buffer_disk_state(bid_a, DiskCheckTrigger::Ambient);
    let (_, warnings_after) = ed.state.message_log.totals();
    assert_eq!(
        warnings_after,
        warnings_before + 1,
        "setup: A only warns while B is focused"
    );
    assert!(
        ed.state.input.confirm().is_none(),
        "setup: A is not focused, so no confirm yet"
    );

    type_cmd_event(&mut ed, ":q");

    assert_eq!(
        ed.focused_buffer_id(),
        bid_a,
        "setup: closing B must reveal A, the only other real buffer"
    );
    assert!(
        ed.state.input.confirm().is_some(),
        "landing on A via :q must re-open its deferred reload confirm"
    );
}

/// `:bd` reaches the reveal through the same `Editor::close_buffer` as `:q`,
/// but is a distinct command: a fix wired only into `typed_quit` would
/// leave this silent.
#[test]
fn buffer_delete_prompts_the_revealed_buffer() {
    let (mut ed, tmp_a) = editor_with_file("-[h]>ello\n", "hello\n");
    let bid_a = ed.focused_buffer_id();

    let (tmp_b, _tmp_b_guard) = temp_file("world\n");
    type_cmd(&mut ed, &format!(":e {}", tmp_b.display()));
    let bid_b = ed.focused_buffer_id();
    assert_ne!(bid_a, bid_b, "setup: :e must open a distinct second buffer");

    rewrite_externally(&tmp_a, "hello, externally changed!\n");
    ed.check_buffer_disk_state(bid_a, DiskCheckTrigger::Ambient);
    assert!(
        ed.state.input.confirm().is_none(),
        "setup: A is not focused"
    );

    type_cmd_event(&mut ed, ":bd");

    assert_eq!(ed.focused_buffer_id(), bid_a);
    assert!(
        ed.state.input.confirm().is_some(),
        ":bd must run the same buffer-enter check as :q"
    );
}

/// Multi-pane `:q` closes the focused pane and reveals its sibling, which
/// must re-prompt for its own deferred change.
///
/// `close_focused_pane` takes `(&mut EditorState, &mut EngineView)` and
/// cannot call an `Editor` method. A check living only in
/// `Editor::close_buffer` would leave multi-pane `:q` silent while
/// single-pane `:q` (`quit_closing_a_buffer_prompts_the_revealed_one`)
/// prompted correctly.
#[test]
fn multi_pane_quit_prompts_the_surviving_panes_buffer() {
    let (mut ed, tmp_a, _tmp_b_guard, bid_a, _bid_b) = two_panes_with_b_focused();

    rewrite_externally(&tmp_a, "hello, externally changed!\n");
    ed.check_buffer_disk_state(bid_a, DiskCheckTrigger::Ambient);
    assert!(
        ed.state.input.confirm().is_none(),
        "setup: A is not focused (right pane shows B)"
    );

    type_cmd_event(&mut ed, ":q");

    assert_eq!(
        ed.view.panes.len(),
        1,
        "setup: multi-pane :q must close the focused pane, not quit"
    );
    assert_eq!(ed.focused_buffer_id(), bid_a);
    assert!(
        ed.state.input.confirm().is_some(),
        "the surviving pane landing on A must re-open its deferred confirm"
    );
}

/// `Ctrl-p c` (`pane-close`) reaches the same reveal as multi-pane `:q`, but
/// as a keymap `EditorCmd` with no `&mut Editor` at all to call the check on.
/// Pins that the post-dispatch chokepoint covers a keymap command with no
/// per-command plumbing. Also pins that the pending sequence is cleared before the
/// `c` leaf runs (`input_stack/base.rs`'s Leaf arm), so `confirm_permit`
/// isn't blocked by the still-just-consumed `Ctrl-p` prefix.
#[test]
fn ctrl_p_c_pane_close_prompts_the_surviving_panes_buffer() {
    let (mut ed, tmp_a, _tmp_b_guard, bid_a, _bid_b) = two_panes_with_b_focused();

    rewrite_externally(&tmp_a, "hello, externally changed!\n");
    ed.check_buffer_disk_state(bid_a, DiskCheckTrigger::Ambient);
    assert!(
        ed.state.input.confirm().is_none(),
        "setup: A is not focused"
    );

    ed.feed_event(key_ctrl('p'));
    ed.feed_event(key('c'));

    assert_eq!(
        ed.view.panes.len(),
        1,
        "setup: Ctrl-p c must close the pane"
    );
    assert_eq!(ed.focused_buffer_id(), bid_a);
    assert!(
        ed.state.input.confirm().is_some(),
        "landing on A via pane-close must re-open its deferred confirm"
    );
}

/// Cycling pane focus (`Ctrl-p p`, `pane-focus-next`) routes through
/// `focus_pane` (`commands/jump.rs`), which changes which pane is focused but
/// has no notion of disk state. Without this check, cycling onto a pane
/// showing an externally-changed file would show stale content with `:w`
/// free to clobber the external edit.
#[test]
fn pane_focus_cycling_prompts_the_buffer_it_lands_on() {
    let (mut ed, tmp_a, _tmp_b_guard, bid_a, bid_b) = two_panes_with_b_focused();
    assert_eq!(ed.focused_buffer_id(), bid_b, "setup: right pane focused");

    rewrite_externally(&tmp_a, "hello, externally changed!\n");
    ed.check_buffer_disk_state(bid_a, DiskCheckTrigger::Ambient);
    assert!(
        ed.state.input.confirm().is_none(),
        "setup: A is not focused"
    );

    ed.feed_event(key_ctrl('p'));
    ed.feed_event(key('p'));

    assert_eq!(
        ed.focused_buffer_id(),
        bid_a,
        "setup: with only two panes, cycling lands on the other one"
    );
    assert!(
        ed.state.input.confirm().is_some(),
        "landing on A via pane-focus-next must re-open its deferred confirm"
    );
}

/// A click into another pane (`mouse_left_down`'s click-to-focus) routes
/// through the same `focus_pane` and never touches `handle_key` at all. The
/// focus diff in `settle()` sees the change whatever input caused it.
#[test]
fn clicking_into_another_pane_prompts_that_panes_buffer() {
    let (mut ed, tmp_a, _tmp_b_guard, bid_a, bid_b) = two_panes_with_b_focused();
    assert_eq!(ed.focused_buffer_id(), bid_b, "setup: right pane focused");

    rewrite_externally(&tmp_a, "hello, externally changed!\n");
    ed.check_buffer_disk_state(bid_a, DiskCheckTrigger::Ambient);
    assert!(
        ed.state.input.confirm().is_none(),
        "setup: A is not focused"
    );

    // 100×25, 0.5 split: pane A (left) spans x ∈ [0, 49); col 0 lands inside
    // it regardless of gutter width, since click-to-focus happens before
    // char-offset resolution (`mouse_left_down`, `mouse.rs`).
    let mut ctx = hume_engine::pipeline::RenderContext::new();
    ed.resize_terminal(100, 25);
    ed.settle();
    ed.prepare_frame(&mut ctx);
    draw(&mut ed);
    ed.handle_input(mouse_left_down(0, 0));
    ed.settle();

    assert_eq!(ed.focused_buffer_id(), bid_a);
    assert!(
        ed.state.input.confirm().is_some(),
        "clicking into A's pane must re-open its deferred confirm"
    );
}
