//! A closed buffer's confirm, one confirm at a time, and confirms under replay.

use super::*;
use pretty_assertions::assert_eq;

// ── Confirm hygiene: a closed buffer's confirm, and one confirm at a time ────

/// Closing a buffer that an open confirm targets must retire the confirm:
/// `reload_buffer_from_disk`'s `try_get` bail turns a `[r]eload` answer for
/// a closed buffer into a no-op. Mirrors a non-interactive close (Steel's
/// `close-buffer!`), which never routes through input dispatch, so the
/// `Confirm` layer never gets a chance to dismiss it for us.
///
/// `forget_closed_buffer` retires any confirm that `targets_buffer(id)`,
/// which also frees the `confirm.is_none()` guard for every later prompt.
#[test]
fn closing_a_buffer_retires_its_open_reload_confirm() {
    let (mut ed, tmp) = editor_with_file("-[h]>ello\n", "hello\n");
    let bid = ed.focused_buffer_id();
    rewrite_externally(&tmp, "hello, externally changed!\n");
    ed.check_buffer_disk_state(bid, DiskCheckTrigger::Ambient);
    assert!(
        ed.state.input.confirm().is_some(),
        "setup: confirm must be open"
    );

    ed.close_buffer(bid)
        .expect("the buffer has no unsaved changes and runs no pipe");

    assert!(
        ed.state.input.confirm().is_none(),
        "closing the confirm's target buffer must retire the confirm"
    );
}

/// `enter_buffer_disk_check` retiring a mismatched confirm must leave
/// whatever else was stacked above it completely untouched: a `Search`
/// session landed there stands in for a `prompt!` a timer would push the
/// same way (`push_mode_layer` only truncates the outgoing *mode* layer,
/// never an overlay sitting on `Base`); `Search` is used here because its
/// `tear_down` has an easily observed effect (restores selections, clears
/// the live pattern), unlike `Prompt`'s (a history-nav reset with no
/// externally visible signal).
///
/// `retire_stale_confirm` removes the confirm alone with `excise_layer`
/// rather than truncating the stack at its ref, so Search's `tear_down`
/// never runs on the pattern and selection the user is still editing.
#[test]
fn enter_buffer_disk_check_leaves_a_layer_stacked_above_the_confirm_untouched() {
    let (mut ed, tmp_a) = editor_with_file("-[h]>ello world\n", "hello world\n");
    let bid_a = ed.focused_buffer_id();
    let (tmp_b, _tmp_b_guard) = temp_file("other\n");
    type_cmd(&mut ed, &format!(":e {}", tmp_b.display()));
    let bid_b = ed.focused_buffer_id();
    assert_ne!(bid_a, bid_b, "setup: :e must open a distinct second buffer");
    type_cmd(&mut ed, ":b #");
    assert_eq!(ed.focused_buffer_id(), bid_a, "setup: back on A");

    rewrite_externally(&tmp_a, "hello world, externally changed!\n");
    ed.check_buffer_disk_state(bid_a, DiskCheckTrigger::Ambient);
    assert!(
        ed.state.input.confirm().is_some(),
        "setup: confirm open on A"
    );

    // Lands above the confirm the same ungated way a timer's `prompt!`
    // would (`push_mode_layer` never gates on what's already open).
    ed.execute_keymap_command("search-forward".into(), Count::typed(1), false);
    for ch in "world".chars() {
        ed.handle_key(key(ch));
    }
    assert_eq!(
        state(&ed),
        "hello -[world]>\n",
        "sanity: live search moved the selection"
    );
    assert!(
        ed.search_pattern().is_some(),
        "sanity: a live pattern is armed"
    );

    ed.enter_buffer_disk_check(bid_b);

    assert!(
        ed.state.input.confirm().is_none(),
        "the mismatched confirm must be retired"
    );
    assert!(
        ed.state
            .input
            .find::<crate::editor::input_stack::SearchLayer>()
            .is_some(),
        "the layer stacked above it must survive"
    );
    assert!(
        ed.search_pattern().is_some(),
        "Search's own tear_down must not have run: the live pattern must stay armed"
    );
    assert_eq!(
        state(&ed),
        "hello -[world]>\n",
        "the live-search preview selection must be untouched"
    );
}

/// A confirm already open for one buffer must not be replaced by a second
/// buffer's deferred prompt, so the user would answer a question they never
/// saw. B's deferred change stays a warning instead, same as any other
/// blocked-confirm case.
///
/// The `confirm.is_none()` check in `confirm_permit` keeps a second
/// `open_disk_change_confirm` call from replacing the live model with B's.
#[test]
fn a_second_confirm_never_replaces_a_live_one() {
    // Both buffers opened up front: once A's confirm is live, the very
    // first key of a typed `:e` would dismiss it (a stray key, not `r`/`k`)
    // rather than opening B through the command line, so B has to already
    // exist before that point.
    let (mut ed, tmp_a) = editor_with_file("-[h]>ello\n", "hello\n");
    let bid_a = ed.focused_buffer_id();
    let (tmp_b, _tmp_b_guard) = temp_file("world\n");
    type_cmd(&mut ed, &format!(":e {}", tmp_b.display()));
    let bid_b = ed.focused_buffer_id();
    assert_ne!(bid_a, bid_b, "setup: :e must open a distinct second buffer");
    type_cmd(&mut ed, ":b #");
    assert_eq!(ed.focused_buffer_id(), bid_a, "setup: back on A");

    rewrite_externally(&tmp_a, "hello, externally changed!\n");
    ed.check_buffer_disk_state(bid_a, DiskCheckTrigger::Ambient);
    assert!(
        ed.state.input.confirm().is_some(),
        "setup: A's confirm must be open"
    );

    // Switch focus to B directly, bypassing input dispatch (and so the live
    // `Confirm` layer), the same way a Steel `switch-to-buffer!` would.
    ed.switch_to_buffer_without_jump(FocusedPane::current(&ed.view), bid_b);
    rewrite_externally(&tmp_b, "world, externally changed!\n");

    let (_, warnings_before) = ed.state.message_log.totals();
    ed.check_buffer_disk_state(bid_b, DiskCheckTrigger::BufferEnter);
    let (_, warnings_after) = ed.state.message_log.totals();

    assert!(
        matches!(
            ed.state.input.confirm().unwrap().action,
            crate::editor::input_stack::ConfirmAction::ReloadBuffer(id) if id == bid_a
        ),
        "A's confirm must survive B's check untouched"
    );
    assert_eq!(
        warnings_after,
        warnings_before + 1,
        "B's blocked change must warn instead"
    );
}

/// A change hit mid macro-replay must never open a confirm: a `Confirm`
/// layer on top would consume the next replayed key, truncating the macro.
/// Blocked during replay, deferred prompt still honoured afterward.
///
/// `confirm_permit`'s replay check holds the confirm back for the whole of
/// `drain_replay_queue`, so none of the macro's remaining keys are eaten
/// before focus reaches B.
#[test]
fn confirm_does_not_open_during_macro_replay() {
    let (mut ed, _tmp_a) = editor_with_file("-[h]>ello\n", "hello\n");
    let bid_a = ed.focused_buffer_id();
    let (tmp_b, _tmp_b_guard) = temp_file("world\n");
    type_cmd(&mut ed, &format!(":e {}", tmp_b.display()));
    let bid_b = ed.focused_buffer_id();
    type_cmd(&mut ed, ":b #");
    assert_eq!(
        ed.focused_buffer_id(),
        bid_a,
        "setup: back on A, B is the alternate"
    );

    // Record a macro into register 'q' that switches to the alternate (B).
    ed.handle_key(key('Q'));
    ed.handle_key(key('Q'));
    for ch in ":b #".chars() {
        ed.handle_key(key(ch));
    }
    ed.handle_key(key_enter());
    ed.handle_key(key('Q'));
    assert_eq!(
        ed.focused_buffer_id(),
        bid_b,
        "setup: recording a live macro also executes it once"
    );

    // Back on A so the replay below has somewhere to switch away from.
    type_cmd(&mut ed, ":b #");
    assert_eq!(ed.focused_buffer_id(), bid_a);

    rewrite_externally(&tmp_b, "world, externally changed!\n");
    let (_, warnings_before) = ed.state.message_log.totals();

    ed.handle_key(key('q'));
    ed.handle_key(key('q'));
    ed.drain_replay_queue();

    let (_, warnings_after) = ed.state.message_log.totals();
    assert!(
        ed.state.input.confirm().is_none(),
        "no confirm may open mid-replay"
    );
    assert_eq!(
        ed.focused_buffer_id(),
        bid_b,
        "the whole macro must still have run, unswallowed by an intercept"
    );
    assert_eq!(warnings_after, warnings_before + 1, "must warn instead");

    // Outside replay, the deferred prompt still arrives on the next real
    // buffer-enter, the same deferral rule as any other blocked case.
    ed.check_buffer_disk_state(bid_b, DiskCheckTrigger::BufferEnter);
    assert!(
        ed.state.input.confirm().is_some(),
        "the deferred prompt must arrive once replay is over"
    );
}

/// A macro that lands on a buffer whose change was already warned about
/// *before* the macro ever ran must still say something. Going completely
/// silent would leave the user editing content that no longer matches disk
/// with no indication anywhere until `:w` refuses.
///
/// The `is_buffer_enter && promptable` fallback covers this. The
/// `!already_reported` warn clause alone is not enough: `already_reported`
/// is `true` from the pre-replay ambient check below, so the
/// replay-triggered `BufferEnter` check would find nothing new to say.
#[test]
fn macro_replay_onto_an_already_warned_stale_buffer_still_warns() {
    let (mut ed, _tmp_a) = editor_with_file("-[h]>ello\n", "hello\n");
    let bid_a = ed.focused_buffer_id();
    let (tmp_b, _tmp_b_guard) = temp_file("world\n");
    type_cmd(&mut ed, &format!(":e {}", tmp_b.display()));
    let bid_b = ed.focused_buffer_id();
    type_cmd(&mut ed, ":b #");
    assert_eq!(
        ed.focused_buffer_id(),
        bid_a,
        "setup: back on A, B is the alternate"
    );

    // Record a macro into register 'q' that switches to the alternate (B).
    ed.handle_key(key('Q'));
    ed.handle_key(key('Q'));
    for ch in ":b #".chars() {
        ed.handle_key(key(ch));
    }
    ed.handle_key(key_enter());
    ed.handle_key(key('Q'));

    // Back on A so the replay below has somewhere to switch away from.
    type_cmd(&mut ed, ":b #");
    assert_eq!(ed.focused_buffer_id(), bid_a);

    // B changes externally and is warned about once, ambiently, *before*
    // replay ever runs: `already_reported` is true by the time replay hits.
    rewrite_externally(&tmp_b, "world, externally changed!\n");
    ed.check_buffer_disk_state(bid_b, DiskCheckTrigger::Ambient);
    let (_, warnings_before) = ed.state.message_log.totals();

    ed.handle_key(key('q'));
    ed.handle_key(key('q'));
    ed.drain_replay_queue();

    let (_, warnings_after) = ed.state.message_log.totals();
    assert_eq!(ed.focused_buffer_id(), bid_b);
    assert!(
        ed.state.input.confirm().is_none(),
        "no confirm may open mid-replay"
    );
    assert_eq!(
        warnings_after,
        warnings_before + 1,
        "landing on an already-warned stale buffer during replay must still \
         warn, not go completely silent"
    );
}

/// A message already on screen when a macro replay starts must still shadow
/// the disk-change warning `drain_replay_queue`'s own trailing `settle()`
/// produces, even though every key `handle_key` dispatches clears
/// `status_msg` at its own start (see `input.rs`), so the sentinel
/// message itself is long gone from `status_msg` by the time the replayed
/// keys finish; what must survive is the *shadowing flag*, not the message
/// text. `drain_replay_queue` saves the flag and ORs it back after every
/// replayed key, so `report_disk_state` pushes to `message_log` alone rather
/// than calling `Editor::report`, which writes the disk warning over whatever
/// the last replayed key left in `status_msg`. No built-in mapping both logs
/// a message and enqueues a replay in the same dispatch, so this drives
/// `EditorState` directly rather than through a real key.
#[test]
fn message_logged_before_a_macro_replay_survives_its_trailing_settle() {
    let (mut ed, _tmp_a) = editor_with_file("-[h]>ello\n", "hello\n");
    let bid_a = ed.focused_buffer_id();
    let (tmp_b, _tmp_b_guard) = temp_file("world\n");
    type_cmd(&mut ed, &format!(":e {}", tmp_b.display()));
    let bid_b = ed.focused_buffer_id();
    type_cmd(&mut ed, ":b #");
    assert_eq!(
        ed.focused_buffer_id(),
        bid_a,
        "setup: back on A, B is the alternate"
    );

    rewrite_externally(&tmp_b, "world, externally changed!\n");

    // Simulate a dispatch that both logged its own message and populated
    // replay queue: the effect a hypothetical mapping doing both in one
    // `handle_input` call would leave behind.
    ed.report(Severity::Warning, "sentinel message".to_string());
    ed.state.message_logged_this_input = true;
    for ch in ":b #".chars() {
        ed.state.macros.enqueue(&[key(ch)], Count::typed(1));
    }
    ed.state.macros.enqueue(&[key_enter()], Count::typed(1));

    let (_, warnings_before) = ed.state.message_log.totals();
    ed.drain_replay_queue();

    assert_eq!(
        ed.focused_buffer_id(),
        bid_b,
        "setup: the replayed `:b #` must have switched onto stale B"
    );
    let (_, warnings_after) = ed.state.message_log.totals();
    assert_eq!(
        warnings_after,
        warnings_before + 1,
        "B's stale-file warning must still land in :messages"
    );
    assert_eq!(
        ed.state.status_msg, None,
        "the disk warning must land in :messages only (report_disk_state's \
         silent-push branch), not overwrite status_msg via Editor::report. \
         status_msg must stay whatever the last replayed key (`:b #`'s own \
         Enter, which reports nothing on success) left it as"
    );
}

/// A command that fails after moving focus (`:qa` jumping to the first
/// dirty buffer to name it in its error) must keep its own error on screen:
/// a reload confirm for that same buffer's external change must not replace
/// it, and must not steal the keystroke meant to answer the error (e.g.
/// retrying with `:qa!`).
///
/// The `message_logged_this_input` guard in `confirm_permit` (set by
/// `Editor::handle_input` for the duration of its post-dispatch
/// focus-change check) stops `check_buffer_disk_state` from opening a reload
/// confirm over the "has unsaved changes" error after the focus diff
/// `typed_quit_all` produces. `open_disk_change_confirm` never writes
/// `status_msg`, so the first assertion doesn't distinguish the two cases;
/// it documents the desired behaviour, not this guard specifically. See
/// `quit_all_focus_move_still_logs_the_disk_change_without_shadowing_the_error`
/// for the guard that does.
#[test]
fn quit_all_error_is_not_shadowed_by_a_disk_confirm() {
    let (mut ed, tmp_a) = editor_with_file("-[h]>ello\n", "hello\n");
    let bid_a = ed.focused_buffer_id();
    ed.handle_key(key('i'));
    ed.handle_key(key('!'));
    ed.handle_key(key_esc());
    assert!(focused_unsaved(&ed), "setup: A is dirty");

    let (tmp_b, _tmp_b_guard) = temp_file("world\n");
    type_cmd(&mut ed, &format!(":e {}", tmp_b.display()));
    assert_ne!(ed.focused_buffer_id(), bid_a, "setup: B is focused, clean");

    rewrite_externally(&tmp_a, "hello, externally changed!\n");

    type_cmd_event(&mut ed, ":qa");

    assert_eq!(
        ed.focused_buffer_id(),
        bid_a,
        "setup: :qa must jump to the dirty buffer to name it"
    );
    assert!(
        ed.state
            .status_msg
            .as_deref()
            .is_some_and(|m| m.contains("has unsaved changes")),
        "the :qa error must stay on screen, got: {:?}",
        ed.state.status_msg
    );
    assert!(
        ed.state.input.confirm().is_none(),
        "no reload confirm may shadow the :qa error"
    );
}

/// The disk change blocked by `quit_all_error_is_not_shadowed_by_a_disk_confirm`
/// must not vanish outright: it lands in `:messages` (bumping the warning
/// total) without touching `status_msg`, so `:qa`'s own error keeps the
/// status line but the change is still discoverable, matching every other
/// blocked-confirm case (mode-blocked, mid-replay).
///
/// While `message_logged_this_input` is set, `report_disk_state` uses the
/// log-only `message_log.push` in place of `Editor::report`, which also
/// writes `status_msg`.
#[test]
fn quit_all_focus_move_still_logs_the_disk_change_without_shadowing_the_error() {
    let (mut ed, tmp_a) = editor_with_file("-[h]>ello\n", "hello\n");
    let bid_a = ed.focused_buffer_id();
    ed.handle_key(key('i'));
    ed.handle_key(key('!'));
    ed.handle_key(key_esc());

    let (tmp_b, _tmp_b_guard) = temp_file("world\n");
    type_cmd(&mut ed, &format!(":e {}", tmp_b.display()));

    rewrite_externally(&tmp_a, "hello, externally changed!\n");
    let (_, warnings_before) = ed.state.message_log.totals();

    type_cmd_event(&mut ed, ":qa");

    assert_eq!(ed.focused_buffer_id(), bid_a, "setup: :qa lands on A");
    let (_, warnings_after) = ed.state.message_log.totals();
    assert_eq!(
        warnings_after,
        warnings_before + 1,
        "the blocked disk-change warning must still land in :messages"
    );
    assert!(
        ed.state
            .status_msg
            .as_deref()
            .is_some_and(|m| m.contains("has unsaved changes")),
        "status_msg must still be :qa's own error, not the disk warning"
    );
}

/// Declining a reload confirm (`[k]eep`) must not reopen the identical
/// question on a later, unrelated focus change. Only a further external
/// change (a new signature) should ask again.
///
/// `[k]eep` records `DiskState::Declined` for the signature, so cycling away
/// and back finds no `Changed` state for the `BufferEnter` re-prompt rule to
/// reopen.
#[test]
fn declined_confirm_does_not_reopen_on_later_focus_change() {
    let (mut ed, tmp_a, _tmp_b_guard, bid_a, bid_b) = two_panes_with_b_focused();
    assert_eq!(ed.focused_buffer_id(), bid_b, "setup: right pane focused");

    rewrite_externally(&tmp_a, "hello, externally changed!\n");

    // Cycle onto A: the deferred prompt opens for the first time.
    ed.feed_event(key_ctrl('p'));
    ed.feed_event(key('p'));
    assert_eq!(ed.focused_buffer_id(), bid_a);
    assert!(
        ed.state.input.confirm().is_some(),
        "setup: A's confirm opens"
    );

    // Decline it.
    ed.feed_event(key('k'));
    assert!(ed.state.input.confirm().is_none());

    // Cycle away to B and back to A: must stay silent for the same change.
    ed.feed_event(key_ctrl('p'));
    ed.feed_event(key('p'));
    assert_eq!(ed.focused_buffer_id(), bid_b);
    ed.feed_event(key_ctrl('p'));
    ed.feed_event(key('p'));
    assert_eq!(ed.focused_buffer_id(), bid_a);
    assert!(
        ed.state.input.confirm().is_none(),
        "a declined change must not re-prompt on a later, unrelated focus change"
    );

    // A further external change (a different signature) still asks again.
    rewrite_externally(&tmp_a, "hello, changed again!\n");
    ed.feed_event(key_ctrl('p'));
    ed.feed_event(key('p'));
    ed.feed_event(key_ctrl('p'));
    ed.feed_event(key('p'));
    assert_eq!(ed.focused_buffer_id(), bid_a);
    assert!(
        ed.state.input.confirm().is_some(),
        "a further external change must still prompt"
    );
}

/// A decline silences the *automatic* nagging (`Ambient`/`BufferEnter`), but
/// a direct `:checktime` (a check the user asked for on purpose) must
/// still say something about it. Without this, a buffer with no further
/// focus changes or terminal-focus events to trigger on could sit silently
/// out of sync with disk indefinitely, with only `:w`'s bare refusal (no
/// explanation) as a clue.
///
/// The `Changed` arm's `Declined` early-return still reports for
/// `DiskCheckTrigger::Explicit` before it returns.
#[test]
fn declined_change_still_warns_on_explicit_checktime() {
    let (mut ed, tmp) = editor_with_file("-[h]>ello\n", "hello\n");
    let bid = ed.focused_buffer_id();
    rewrite_externally(&tmp, "hello, externally changed!\n");
    ed.check_buffer_disk_state(bid, DiskCheckTrigger::Ambient);
    assert!(ed.state.input.confirm().is_some(), "setup: confirm opens");

    ed.feed_event(key('k'));
    assert!(ed.state.input.confirm().is_none(), "setup: declined");

    // Ambient stays silent for the declined signature.
    let (_, warnings_before) = ed.state.message_log.totals();
    ed.check_buffer_disk_state(bid, DiskCheckTrigger::Ambient);
    let (_, warnings_mid) = ed.state.message_log.totals();
    assert_eq!(warnings_mid, warnings_before, "setup: Ambient stays silent");

    // :checktime, a direct request, still warns, and still doesn't prompt.
    type_cmd(&mut ed, ":checktime");
    let (_, warnings_after) = ed.state.message_log.totals();
    assert_eq!(
        warnings_after,
        warnings_before + 1,
        ":checktime must still warn about a declined change, not stay silent forever"
    );
    assert!(
        ed.state.input.confirm().is_none(),
        ":checktime must warn, not reopen the confirm the user already answered"
    );
}

/// `Declined` only silences the *specific declined signature*'s `Changed`
/// report: it must not swallow an unrelated later `Vanished` warning for
/// the same buffer.
///
/// The `Declined` short-circuit lives inside the `Changed` arm, so the
/// `Vanished` arm's warning never passes through it.
#[test]
fn declined_change_then_vanished_file_still_warns() {
    let (mut ed, tmp) = editor_with_file("-[h]>ello\n", "hello\n");
    let bid = ed.focused_buffer_id();
    rewrite_externally(&tmp, "hello, externally changed!\n");
    ed.check_buffer_disk_state(bid, DiskCheckTrigger::Ambient);
    ed.feed_event(key('k'));
    assert!(ed.state.input.confirm().is_none(), "setup: declined");

    std::fs::remove_file(&tmp).unwrap();
    let (_, warnings_before) = ed.state.message_log.totals();
    ed.check_buffer_disk_state(bid, DiskCheckTrigger::Ambient);
    let (_, warnings_after) = ed.state.message_log.totals();

    assert_eq!(
        warnings_after,
        warnings_before + 1,
        "a declined Changed signature must not swallow a later Vanished warning"
    );
    assert_eq!(ed.state.buffers.get(bid).disk_state, DiskState::Vanished);
}

/// A confirm left open for a buffer the user just clicked away from (a
/// mouse gesture has no confirm-key of its own to match, unlike a keypress)
/// must be retired rather than left on screen pointing at a pane no longer
/// in view, and the buffer the click actually landed on must get its own
/// deferred prompt, not be blocked by the stale one.
///
/// `enter_buffer_disk_check` (`OnBufferEnter`'s Rust reaction) retires a
/// confirm targeting a buffer other than the newly focused one, which clears
/// `confirm_permit`'s `confirm.is_none()` gate for A's own prompt.
#[test]
fn mouse_click_into_another_pane_retires_a_stale_confirm() {
    let (mut ed, tmp_a, tmp_b_guard, bid_a, bid_b) = two_panes_with_b_focused();
    assert_eq!(ed.focused_buffer_id(), bid_b, "setup: right pane focused");

    rewrite_externally(&tmp_a, "hello, externally changed!\n");
    ed.check_buffer_disk_state(bid_a, DiskCheckTrigger::Ambient);
    assert!(
        ed.state.input.confirm().is_none(),
        "setup: A is not focused yet"
    );

    rewrite_externally(&tmp_b_guard, "world, externally changed!\n");
    ed.check_buffer_disk_state(bid_b, DiskCheckTrigger::Ambient);
    assert!(
        matches!(
            ed.state.input.confirm().unwrap().action,
            crate::editor::input_stack::ConfirmAction::ReloadBuffer(id) if id == bid_b
        ),
        "setup: B's own confirm is open"
    );

    // 100×25, 0.5 split: pane A (left) spans x ∈ [0, 49); col 0 lands inside
    // it regardless of gutter width (see `clicking_into_another_pane_prompts_that_panes_buffer`).
    let mut ctx = hume_engine::pipeline::RenderContext::new();
    ed.resize_terminal(100, 25);
    ed.settle();
    ed.prepare_frame(&mut ctx);
    draw(&mut ed);
    ed.handle_input(mouse_left_down(0, 0));
    ed.settle();

    assert_eq!(ed.focused_buffer_id(), bid_a);
    assert!(
        matches!(
            ed.state.input.confirm().unwrap().action,
            crate::editor::input_stack::ConfirmAction::ReloadBuffer(id) if id == bid_a
        ),
        "B's orphaned confirm must be retired and A's own prompt opened in its place"
    );
}
