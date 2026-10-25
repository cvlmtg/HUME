//! Detecting an external change, and the confirm's choices.

use super::*;
use pretty_assertions::assert_eq;

// ── Detection ─────────────────────────────────────────────────────────────────

/// An external rewrite with different content/length is detected as
/// `Changed`, and (since `file-reload-prompt` defaults to `true` and this is the
/// focused buffer) opens a reload confirm.
#[test]
fn external_rewrite_is_detected_and_opens_confirm() {
    let (mut ed, tmp) = editor_with_file("-[h]>ello\n", "hello\n");
    rewrite_externally(&tmp, "hello, world!\n");

    let bid = ed.focused_buffer_id();
    ed.check_buffer_disk_state(bid, DiskCheckTrigger::Ambient);

    // A mtime-only comparison could miss this on a filesystem whose mtime
    // resolution is coarser than the test's wall-clock delta.
    assert!(ed.doc().is_disk_stale());
    assert!(ed.state.input.confirm().is_some());
}

/// `file-reload-prompt=false` only warns: no confirm, but the stale flag is still set
/// so `:w` still refuses until the buffer is reloaded or the write is forced.
#[test]
fn auto_read_false_warns_without_opening_confirm() {
    let (mut ed, tmp) = editor_with_file("-[h]>ello\n", "hello\n");
    ed.state.settings.file_reload_prompt = false;
    rewrite_externally(&tmp, "hello, world!\n");

    let (_, warnings_before) = ed.state.message_log.totals();
    let bid = ed.focused_buffer_id();
    ed.check_buffer_disk_state(bid, DiskCheckTrigger::Ambient);
    let (_, warnings_after) = ed.state.message_log.totals();

    assert_eq!(warnings_after, warnings_before + 1);
    assert!(ed.doc().is_disk_stale());
    assert!(ed.state.input.confirm().is_none());
}

/// A deleted file reads as `Vanished`: warns, marks the buffer stale, never
/// opens a confirm: there is nothing to reload from.
#[test]
fn deleted_file_warns_and_never_prompts() {
    let (mut ed, tmp) = editor_with_file("-[h]>ello\n", "hello\n");
    let name = ed.doc().display_name();
    std::fs::remove_file(&tmp).unwrap();

    let bid = ed.focused_buffer_id();
    ed.check_buffer_disk_state(bid, DiskCheckTrigger::Ambient);

    assert!(ed.doc().is_disk_stale());
    assert!(ed.state.input.confirm().is_none());
    assert_eq!(
        ed.state.status_msg.as_deref(),
        Some(format!("{name}: file no longer exists on disk").as_str())
    );
}

/// A deleted file warns once; a second check with the file still gone must
/// stay silent, the same "don't nag again for the same thing" rule `Changed`
/// follows, just with no signature to compare (there's only one "vanished").
///
/// If the `Vanished` arm never recorded that it had already reported, every
/// later ambient trigger (each terminal focus regain) would warn again.
#[test]
fn vanished_file_does_not_refire_on_every_trigger() {
    let (mut ed, tmp) = editor_with_file("-[h]>ello\n", "hello\n");
    std::fs::remove_file(&tmp).unwrap();
    let bid = ed.focused_buffer_id();

    ed.check_buffer_disk_state(bid, DiskCheckTrigger::Ambient);
    let (_, warnings_after_first) = ed.state.message_log.totals();

    ed.check_buffer_disk_state(bid, DiskCheckTrigger::Ambient);
    let (_, warnings_after_second) = ed.state.message_log.totals();

    assert_eq!(
        warnings_after_second, warnings_after_first,
        "a vanished file already reported must stay silent on a later check"
    );
}

/// A buffer with no backing file (scratch, or a synthetic view like
/// `[messages]`) has nothing to compare against and is always `Unchanged`:
/// checking it must never warn or mark the buffer stale.
#[test]
fn pathless_buffers_are_never_flagged() {
    let mut ed = editor_from("-[h]>ello\n"); // scratch buffer, no file
    let bid = ed.focused_buffer_id();
    let (_, warnings_before) = ed.state.message_log.totals();

    ed.check_buffer_disk_state(bid, DiskCheckTrigger::Ambient);

    let (_, warnings_after) = ed.state.message_log.totals();
    assert_eq!(warnings_after, warnings_before);
    assert!(!ed.doc().is_disk_stale());
}

/// Once a change has been reported, a second check with nothing further
/// changed must stay silent. Re-warning on every trigger (terminal focus,
/// buffer-enter) for a condition the user already saw once would be pure
/// noise. A *further*, distinct external change must still fire.
///
/// If `check_buffer_disk_state` left the buffer's stored signature untouched
/// after reporting, the second check would compare against the same stale
/// signature and warn again.
#[test]
fn unactioned_change_does_not_refire_until_a_further_change_happens() {
    let (mut ed, tmp) = editor_with_file("-[h]>ello\n", "hello\n");
    ed.state.settings.file_reload_prompt = false; // isolate the warning count from the confirm
    rewrite_externally(&tmp, "hello, world!\n");
    let bid = ed.focused_buffer_id();

    ed.check_buffer_disk_state(bid, DiskCheckTrigger::Ambient);
    let (_, warnings_after_first) = ed.state.message_log.totals();

    ed.check_buffer_disk_state(bid, DiskCheckTrigger::Ambient);
    let (_, warnings_after_second) = ed.state.message_log.totals();
    assert_eq!(
        warnings_after_second, warnings_after_first,
        "an unactioned, unchanged-since-last-report disk state must stay silent"
    );

    rewrite_externally(&tmp, "a third, distinctly different revision\n");
    ed.check_buffer_disk_state(bid, DiskCheckTrigger::Ambient);
    let (_, warnings_after_third) = ed.state.message_log.totals();
    assert_eq!(
        warnings_after_third,
        warnings_after_second + 1,
        "a genuinely new external change must still fire"
    );
}

/// `:bn`/`:bp` run the same buffer-enter disk check as `:e`/`:b`. Landing on
/// an externally-changed buffer this way must not silently show stale
/// content with `:w` free to clobber the external edit.
///
/// Without `detect_buffer_enter`'s focus diff (`event/pump.rs`)
/// observing the pane's new buffer and queuing `OnBufferEnter`, the target
/// buffer's disk state would stay `InSync` no matter what changed externally.
/// This holds for any switch primitive `:bnext`/`:bprev` route through, since
/// the diff is keyed on the focused (pane, buffer) pair, not on the call
/// site. Driven via `type_cmd_event`, not `type_cmd`: the check is
/// `OnBufferEnter`'s Rust reaction, observed only once `Editor::settle()`
/// runs its focus diff: `type_cmd_event` settles after the command,
/// `type_cmd` does not.
#[test]
fn bnext_and_bprev_run_the_buffer_enter_disk_check() {
    let (mut ed, tmp_a) = editor_with_file("-[h]>ello\n", "hello\n");
    let bid_a = ed.focused_buffer_id();

    let (tmp_b, _tmp_b_guard) = temp_file("world\n");
    type_cmd(&mut ed, &format!(":e {}", tmp_b.display()));
    let bid_b = ed.focused_buffer_id();
    assert_ne!(bid_a, bid_b, "setup: :e must open a distinct second buffer");

    // B is focused; rewrite A externally.
    rewrite_externally(&tmp_a, "hello, externally changed!\n");

    // With only two buffers open, either direction lands back on A.
    type_cmd_event(&mut ed, ":bp");
    assert_eq!(ed.focused_buffer_id(), bid_a);
    assert!(
        ed.state.input.confirm().is_some(),
        ":bp must run the buffer-enter disk check as well as switching"
    );
}

/// A non-focused buffer's external change only warns: no confirm opens
/// off-focus. But switching into that buffer afterwards must still open the
/// confirm, even though nothing has changed on disk since the warning: this
/// is the documented "asked about on its own next buffer-enter" promise.
///
/// If `BufferEnter` deduped an already-reported `Changed` state the way
/// `Ambient` does, `:b #` landing on the buffer would find nothing new to
/// report and stay silent, breaking that promise.
/// The final `:b #` is driven via `type_cmd_event`: the disk check is
/// `OnBufferEnter`'s Rust reaction, observed only once `Editor::settle()`
/// runs its focus diff after the switch.
#[test]
fn deferred_change_on_non_focused_buffer_prompts_on_buffer_enter() {
    let (mut ed, _tmp_a) = editor_with_file("-[h]>ello\n", "hello\n");
    let bid_a = ed.focused_buffer_id();

    let (tmp_b, _tmp_b_guard) = temp_file("world\n");
    type_cmd(&mut ed, &format!(":e {}", tmp_b.display()));
    let bid_b = ed.focused_buffer_id();
    assert_ne!(bid_a, bid_b, "setup: :e must open a distinct second buffer");

    // Switch back to A. B stays open as the alternate, unfocused.
    type_cmd(&mut ed, ":b #");
    assert_eq!(
        ed.focused_buffer_id(),
        bid_a,
        "setup: :b # must return to A"
    );

    rewrite_externally(&tmp_b, "world, externally changed!\n");
    let (_, warnings_before) = ed.state.message_log.totals();
    ed.check_buffer_disk_state(bid_b, DiskCheckTrigger::Ambient);
    let (_, warnings_after) = ed.state.message_log.totals();
    assert_eq!(
        warnings_after,
        warnings_before + 1,
        "a non-focused change only warns"
    );
    assert!(ed.state.input.confirm().is_none());

    // Enter B via :b: the deferred prompt must appear now.
    type_cmd_event(&mut ed, ":b #");
    assert_eq!(ed.focused_buffer_id(), bid_b);
    assert!(
        ed.state.input.confirm().is_some(),
        "buffer-enter must prompt a pending change even if already warned about"
    );
}

/// A pending change detected while the editor is in Insert never opens a
/// confirm, since it would steal the very next keystroke from whatever the user
/// is mid-typing. It warns instead, same as a non-focused buffer or
/// `file-reload-prompt` off, and, like that non-focused case, only a `BufferEnter`
/// check reopens the deferred prompt; a further `Ambient` recheck stays
/// silent for the same already-reported state.
#[test]
fn change_detected_mid_insert_warns_instead_of_prompting() {
    let (mut ed, tmp) = editor_with_file("-[h]>ello\n", "hello\n");
    ed.handle_key(key('i'));
    rewrite_externally(&tmp, "hello, externally changed!\n");

    let (_, warnings_before) = ed.state.message_log.totals();
    let bid = ed.focused_buffer_id();
    ed.check_buffer_disk_state(bid, DiskCheckTrigger::Ambient);
    let (_, warnings_after) = ed.state.message_log.totals();

    assert_eq!(
        warnings_after,
        warnings_before + 1,
        "must warn instead of prompting"
    );
    assert!(ed.state.input.confirm().is_none());

    // Back in Normal, only a buffer-enter check reopens the deferred prompt,
    // the same deferral rule as a non-focused buffer's warning (see
    // `deferred_change_on_non_focused_buffer_prompts_on_buffer_enter`).
    ed.handle_key(key_esc());
    ed.check_buffer_disk_state(bid, DiskCheckTrigger::BufferEnter);
    assert!(
        ed.state.input.confirm().is_some(),
        "a mode-blocked Changed must still prompt on the next buffer-enter"
    );
}

// ── Writing the buffer must not look like an external change ─────────────────

/// The editor's own `:w` renames a fresh inode into place, which always
/// bumps mtime. Without `write_file_atomic` refreshing the stored
/// signature after a successful write, the very next check would
/// misreport the editor's own save as an external change.
#[test]
fn writing_the_buffer_does_not_flag_it_as_externally_changed() {
    let (mut ed, _tmp) = editor_with_file("-[h]>ello\n", "hello\n");
    // Dirty the buffer so `:w` actually writes instead of no-oping.
    ed.handle_key(key('i'));
    ed.handle_key(key('x'));
    ed.handle_key(key_esc());
    type_cmd(&mut ed, ":w");

    let bid = ed.focused_buffer_id();
    let (_, warnings_before) = ed.state.message_log.totals();
    ed.check_buffer_disk_state(bid, DiskCheckTrigger::Ambient);
    let (_, warnings_after) = ed.state.message_log.totals();

    assert_eq!(warnings_after, warnings_before);
    assert!(!ed.doc().is_disk_stale());
    assert!(ed.state.input.confirm().is_none());
}

// ── Confirm overlay choices ───────────────────────────────────────────────────

/// The confirm's `[r]eload` choice re-reads the file, clears the stale flag,
/// and preserves the cursor position (line 0, col 0 maps to the same char
/// index regardless of what the line's new content is).
#[test]
fn confirm_reload_choice_reloads_and_clears_disk_stale() {
    let (mut ed, tmp) = editor_with_file("-[h]>ello\n", "hello\n");
    rewrite_externally(&tmp, "HELLO!!\n");
    let bid = ed.focused_buffer_id();
    ed.check_buffer_disk_state(bid, DiskCheckTrigger::Ambient);
    assert!(
        ed.state.input.confirm().is_some(),
        "setup: confirm must be open"
    );

    ed.handle_key(key('r'));

    assert!(ed.state.input.confirm().is_none());
    assert_eq!(ed.doc().text().to_string(), "HELLO!!\n");
    assert_eq!(state(&ed), "-[H]>ELLO!!\n");
    assert!(!focused_unsaved(&ed));
    assert!(
        !ed.doc().is_disk_stale(),
        "reload must clear the stale flag, or :w would keep refusing forever"
    );
    assert!(
        ed.doc().can_undo(),
        "the reload must be a recorded revision, not a history reset"
    );
}

/// The confirm's `[k]eep` choice leaves the buffer's content untouched and
/// the stale flag still set: `:w` still refuses until reloaded or forced.
#[test]
fn confirm_keep_choice_leaves_buffer_untouched_and_still_stale() {
    let (mut ed, tmp) = editor_with_file("-[h]>ello\n", "hello\n");
    rewrite_externally(&tmp, "HELLO!!\n");
    let bid = ed.focused_buffer_id();
    ed.check_buffer_disk_state(bid, DiskCheckTrigger::Ambient);

    ed.handle_key(key('k'));

    assert!(ed.state.input.confirm().is_none());
    assert_eq!(ed.doc().text().to_string(), "hello\n");
    assert!(ed.doc().is_disk_stale());
}

/// `Esc` dismisses without answering: neither reload nor decline.
#[test]
fn confirm_esc_dismisses_without_answering() {
    let (mut ed, tmp) = editor_with_file("-[h]>ello\n", "hello\n");
    rewrite_externally(&tmp, "HELLO!!\n");
    let bid = ed.focused_buffer_id();
    ed.check_buffer_disk_state(bid, DiskCheckTrigger::Ambient);

    ed.handle_key(key_esc());

    assert!(ed.state.input.confirm().is_none());
    assert_eq!(ed.doc().text().to_string(), "hello\n");
    assert!(ed.doc().is_disk_stale());
}

/// A stray key that isn't `r`/`k`/`Esc` dismisses the confirm the same way,
/// but, unlike `Esc`, is not swallowed: it still runs its own binding in
/// the same dispatch. `/` opens the search line; a bug that ate the
/// keystroke would leave the mode at `Normal` with the search line unopened.
#[test]
fn confirm_stray_key_dismisses_and_still_runs_its_binding() {
    let (mut ed, tmp) = editor_with_file("-[h]>ello\n", "hello\n");
    rewrite_externally(&tmp, "HELLO!!\n");
    let bid = ed.focused_buffer_id();
    ed.check_buffer_disk_state(bid, DiskCheckTrigger::Ambient);

    ed.handle_key(key('/'));

    assert!(ed.state.input.confirm().is_none());
    assert_eq!(ed.doc().text().to_string(), "hello\n");
    assert!(ed.doc().is_disk_stale());
    assert_eq!(
        ed.state.mode(),
        Mode::Search,
        "the '/' must still open search"
    );
}

/// A choice key held with a modifier (`Ctrl-k`, the kitty one-shot extend
/// for `move-up`) must not answer the prompt. Only a bare `r`/`k` does.
///
/// The choice match has to look at `key.modifiers`. Matching on
/// `key.code == KeyCode::Char('k')` alone would make `Ctrl-k` decline
/// (`DiskState::Declined`) and never reach `move-up`.
#[test]
fn confirm_choice_key_with_ctrl_does_not_answer_the_prompt() {
    let (mut ed, tmp) = editor_with_file("-[h]>ello\n", "hello\n");
    rewrite_externally(&tmp, "HELLO!!\n");
    let bid = ed.focused_buffer_id();
    ed.check_buffer_disk_state(bid, DiskCheckTrigger::Ambient);

    ed.handle_key(key_ctrl('k'));

    assert!(ed.state.input.confirm().is_none());
    assert!(
        matches!(ed.state.buffers.get(bid).disk_state, DiskState::Changed(_)),
        "Ctrl-k must not decline: the choice match requires no modifiers"
    );
}

/// Accepting the confirm after focus moved away from the target buffer
/// (an async Steel/LSP callback calling `switch-to-buffer!` between frames,
/// not through input dispatch, which the `Confirm` layer would otherwise see
/// first) must not reload, and must not panic.
///
/// Without the focus guard, `reload_buffer_from_disk` would call
/// `reload_buffer_in_place(bid_a, ..)` while focus is on B, and its
/// `.expect("focused pane must view the reloaded buffer")` would panic,
/// since the now-focused pane holds no state for A's post-heads.
#[test]
fn reload_confirm_accept_after_focus_moved_away_does_not_panic() {
    let (mut ed, tmp_a) = editor_with_file("-[h]>ello\n", "hello\n");
    let bid_a = ed.focused_buffer_id();
    rewrite_externally(&tmp_a, "hello, externally changed!\n");
    ed.check_buffer_disk_state(bid_a, DiskCheckTrigger::Ambient);
    assert!(
        ed.state.input.confirm().is_some(),
        "setup: confirm must be open"
    );

    // Simulate an async callback moving focus without going through key
    // dispatch. The confirm is left open, still targeting A.
    let (tmp_b, _tmp_b_guard) = temp_file("world\n");
    let (bid_b, _) = ed.resolve_open_path(&tmp_b.display().to_string()).unwrap();
    ed.switch_to_buffer_without_jump(FocusedPane::current(&ed.view), bid_b);
    assert_ne!(
        ed.focused_buffer_id(),
        bid_a,
        "setup: focus must have moved off A"
    );

    let (_, warnings_before) = ed.state.message_log.totals();
    ed.handle_key(key('r'));
    let (_, warnings_after) = ed.state.message_log.totals();

    assert!(
        ed.state.input.confirm().is_none(),
        "confirm must still close"
    );
    assert_eq!(
        warnings_after,
        warnings_before + 1,
        "must warn instead of reloading"
    );
    assert_eq!(ed.focused_buffer_id(), bid_b, "focus must stay put");
    assert_eq!(
        ed.state.buffers.get(bid_a).text().to_string(),
        "hello\n",
        "A must not have been reloaded while unfocused"
    );
}
