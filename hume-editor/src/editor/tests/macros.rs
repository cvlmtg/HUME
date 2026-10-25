use super::*;
use pretty_assertions::assert_eq;

// ── Keyboard macros ───────────────────────────────────────────────────────────

/// `QQ` starts recording into register `q`, second `Q` stops.
/// Keys typed during recording are stored as a macro.
#[test]
fn macro_qq_records_into_register_q() {
    let mut ed = editor_from("-[a]>bcd\n");
    // First `Q` sets the pending state; recording hasn't started yet.
    ed.handle_key(key('Q'));
    assert!(
        ed.state.macros.recording_register().is_none(),
        "recording not started until register name given"
    );
    assert!(reading_char(&ed), "pending should be set after Q");

    // Second `Q` is consumed as the register name; recording starts now.
    ed.handle_key(key('Q'));
    assert!(
        ed.state.macros.recording_register().is_some(),
        "recording should start after Q<reg>"
    );
    assert_eq!(ed.state.macros.recording_register().unwrap(), 'q');

    // Record a motion: j (move down)
    ed.handle_key(key('j'));

    // Stop recording: Q
    ed.handle_key(key('Q'));
    assert!(
        ed.state.macros.recording_register().is_none(),
        "recording should stop after stop-Q"
    );

    // Register 'q' should now hold a macro with [j] (not the register-name Q or stop Q)
    let keys = ed
        .state
        .registers
        .read('q')
        .and_then(|r| r.as_macro())
        .map(|k| k.to_vec());
    assert!(keys.is_some(), "register q should hold a macro");
    let keys = keys.unwrap();
    assert_eq!(
        keys.len(),
        1,
        "only the j key should be recorded, not Q keys"
    );
    assert_eq!(keys[0].code, KeyCode::Char('j'));
}

/// `Q0` records into register `0`.
#[test]
fn macro_q_digit_records_into_named_register() {
    let mut ed = editor_from("-[a]>bcd\n");
    ed.handle_key(key('Q'));
    ed.handle_key(key('0'));
    assert!(ed.state.macros.recording_register().is_some());
    assert_eq!(ed.state.macros.recording_register().unwrap(), '0');
    ed.handle_key(key('j'));
    ed.handle_key(key('Q'));
    assert!(ed.state.macros.recording_register().is_none());
    let keys = ed
        .state
        .registers
        .read('0')
        .and_then(|r| r.as_macro())
        .map(|k| k.to_vec());
    assert!(keys.is_some());
    assert_eq!(keys.unwrap()[0].code, KeyCode::Char('j'));
}

/// `Q Esc` cancels: no recording starts.
#[test]
fn macro_q_esc_cancels() {
    let mut ed = editor_from("-[a]>bcd\n");
    ed.handle_key(key('Q'));
    assert!(reading_char(&ed), "pending should be set after Q");
    ed.handle_key(key_esc());
    assert!(!reading_char(&ed), "pending should be cleared after Esc");
    assert!(
        ed.state.macros.recording_register().is_none(),
        "no recording should have started"
    );
}

/// `q Esc` cancels: no replay is queued.
#[test]
fn macro_big_q_esc_cancels() {
    let mut ed = editor_from("-[a]>bcd\n");
    ed.handle_key(key('q'));
    assert!(reading_char(&ed));
    ed.handle_key(key_esc());
    assert!(!reading_char(&ed));
    assert!(!ed.state.macros.has_queued_keys());
}

/// `qq` replays from the default register `q`. The cursor should move down one line.
#[test]
fn macro_big_q_replays_from_register() {
    // 3 lines: cursor starts on first line
    let mut ed = editor_from("-[a]>\nb\nc\n");

    // Record `j` into register `q`
    ed.handle_key(key('Q'));
    ed.handle_key(key('Q'));
    ed.handle_key(key('j'));
    ed.handle_key(key('Q'));

    let before = ed.current_view().primary().head().offset();

    // `qq` replays from the default register, no extra key needed.
    ed.handle_key(key('q'));
    ed.handle_key(key('q'));

    ed.drain_replay_queue();

    let after = ed.current_view().primary().head().offset();
    assert!(after > before, "cursor should have moved down after replay");
}

/// `q` followed by a non-register key cancels replay; key is swallowed.
#[test]
fn macro_big_q_non_register_key_cancels() {
    let mut ed = editor_from("-[a]>\nb\nc\n");

    // Record `j` into register `q` so there's something to (not) replay.
    ed.handle_key(key('Q'));
    ed.handle_key(key('Q'));
    ed.handle_key(key('j'));
    ed.handle_key(key('Q'));

    let before = ed.current_view().primary().head().offset();

    // `q` then `Q` (uppercase, not a valid register): cancelled, cursor stays put.
    ed.handle_key(key('q'));
    ed.handle_key(key('Q'));

    ed.drain_replay_queue();

    let after = ed.current_view().primary().head().offset();
    assert_eq!(before, after, "cancelled replay should not move cursor");
}

/// Replay of an empty/nonexistent register is a no-op.
#[test]
fn macro_replay_empty_register_is_noop() {
    let mut ed = editor_from("-[a]>bcd\n");
    let before = state(&ed);

    // `q` must start the register-name read, proving the dispatch path ran.
    ed.handle_key(key('q'));
    assert!(reading_char(&ed), "q should start the register-name read");

    // Register 'z' has never been written: the read ends but
    // no keys are queued and state is unchanged.
    ed.handle_key(key('z'));
    assert!(!reading_char(&ed), "the register key should end the read");
    assert!(
        !ed.state.macros.has_queued_keys(),
        "no keys queued for unset register"
    );
    assert_eq!(state(&ed), before, "state unchanged");
}

/// `Q` during replay does not start recording (nested recording suppressed).
#[test]
fn macro_no_nested_recording_during_replay() {
    // Record a macro that would press `Q Q` (try to start recording).
    // During replay, the `Q` intercept should be suppressed.
    let mut ed = editor_from("-[a]>bcd\n");

    // Manually seed a macro that contains `Q Q j` into register 'q'.
    // We can't record this via the normal path (Q would stop recording),
    // so we write directly to the register.
    ed.state.registers.write_macro(
        'q',
        vec![
            KeyEvent::new(KeyCode::Char('Q'), Modifiers::NONE),
            KeyEvent::new(KeyCode::Char('Q'), Modifiers::NONE),
            KeyEvent::new(KeyCode::Char('j'), Modifiers::NONE),
        ],
    );

    // Trigger replay: qq
    ed.handle_key(key('q'));
    ed.handle_key(key('q'));

    ed.drain_replay_queue();

    // Recording should NOT have started: the Q intercept is suppressed during replay
    assert!(
        ed.state.macros.recording_register().is_none(),
        "nested recording must be suppressed"
    );
    assert!(
        !reading_char(&ed),
        "no register-name read may be left after replay"
    );
}

/// A macro whose last key is `Q` must not start a register-name read after replay.
///
/// The suppression must key off the replay mark, not an empty queue:
/// the queue is empty at the exact moment the last key is processed, which would
/// let a trailing `Q` slip through and start a register-name read.
#[test]
fn macro_trailing_q_does_not_arm_pending() {
    let mut ed = editor_from("-[a]>\nb\nc\n");

    // Seed a macro ending with Q (can't be recorded normally).
    ed.state.registers.write_macro(
        'q',
        vec![
            KeyEvent::new(KeyCode::Char('j'), Modifiers::NONE),
            KeyEvent::new(KeyCode::Char('Q'), Modifiers::NONE),
        ],
    );

    // Replay: qq
    ed.handle_key(key('q'));
    ed.handle_key(key('q'));

    ed.drain_replay_queue();

    assert!(
        ed.state.macros.recording_register().is_none(),
        "recording must not have started"
    );
    assert!(
        !reading_char(&ed),
        "a trailing Q must not start a register-name read"
    );
}

/// Status bar shows `[recording @q]` during recording and nothing when idle.
///
/// Tests that `StatusElement::MacroRecording` is in the default config and that
/// the actual `render_element` path (in `statusline.rs`) produces the right text.
/// This test lives here for access to `editor_from`; the rendering assertion
/// is in `statusline.rs::tests::macro_recording_element_renders`.
#[test]
fn macro_status_indicator() {
    use crate::statusline::StatusElement;

    let ed = editor_from("-[a]>bcd\n");
    let config = ed.state.settings.statusline();
    assert!(
        config.right.contains(&StatusElement::MacroRecording),
        "MacroRecording should be in the default right section"
    );
}

/// Recording works across mode transitions: insert-mode keys are captured.
#[test]
fn macro_records_insert_mode_keys() {
    let mut ed = editor_from("-[a]>bcd\n");

    // Start recording
    ed.handle_key(key('Q'));
    ed.handle_key(key('Q'));

    // Enter insert mode, type 'x', exit
    ed.handle_key(key('i'));
    ed.handle_key(key('x'));
    ed.handle_key(key_esc());

    // Stop recording
    ed.handle_key(key('Q'));

    // The recorded macro should contain: i, x, Esc (3 keys)
    let keys = ed
        .state
        .registers
        .read('q')
        .and_then(|r| r.as_macro())
        .map(|k| k.to_vec())
        .unwrap();
    assert_eq!(
        keys.len(),
        3,
        "expected i, x, Esc; got {} keys: {:?}",
        keys.len(),
        keys
    );
    assert_eq!(keys[0].code, KeyCode::Char('i'));
    assert_eq!(keys[1].code, KeyCode::Char('x'));
    assert_eq!(keys[2].code, KeyCode::Escape);
}

// ── New edge-case tests ───────────────────────────────────────────────────────

/// `3qq` replays the macro 3 times. With a `j` macro and enough lines, the
/// cursor should end up exactly 3 lines below its position at replay start.
#[test]
fn macro_replay_with_count() {
    // 5 lines so we can move down 3 from line 0 without hitting the buffer end.
    let mut ed = editor_from("-[a]>\nb\nc\nd\ne\n");

    // Record `j` into register 'q'. The cursor moves to line 1 during recording.
    ed.handle_key(key('Q'));
    ed.handle_key(key('Q'));
    ed.handle_key(key('j'));
    ed.handle_key(key('Q'));

    // Go back to line 0 (gg = goto-first-line) so replay has room to move 3 lines.
    ed.handle_key(key('g'));
    ed.handle_key(key('g'));

    let start = ed.current_view().primary().head().offset();
    let start_line = ed.doc().text().char_to_line(start);
    assert_eq!(
        start_line,
        hume_rope::line::ContentLine::new(0),
        "cursor should be on line 0 before replay"
    );

    // `3qq`: count 3, replay from register 'q'.
    ed.handle_key(key('3'));
    ed.handle_key(key('q'));
    ed.handle_key(key('q'));
    ed.drain_replay_queue();

    let end_line = ed
        .doc()
        .text()
        .char_to_line(ed.current_view().primary().head().offset());
    assert_eq!(
        end_line,
        hume_rope::line::ContentLine::new(3),
        "expected cursor on line 3, got line {}",
        end_line.index()
    );
}

/// Replaying a register that holds text (not a macro) is a no-op.
///
/// `enqueue_macro_replay` calls `as_macro()` which returns `None` for text
/// registers. The queue must stay empty and the state unchanged.
#[test]
fn macro_replay_of_text_register_is_noop() {
    let mut ed = editor_from("-[a]>bcd\n");
    let before = state(&ed);

    // Write text (not a macro) directly into register '0', then try to replay.
    ed.state.registers.write_text('0', vec!["some text"]);
    ed.handle_key(key('q'));
    assert!(reading_char(&ed));
    ed.handle_key(key('0'));
    assert!(!reading_char(&ed));
    assert!(
        !ed.state.macros.has_queued_keys(),
        "text register must not enqueue any keys"
    );
    assert_eq!(state(&ed), before, "state must be unchanged");
}

/// Record `f` + `x` (find-char). Both keys must be captured in the macro.
/// Replay must move the cursor to the next `x` on the line.
#[test]
fn macro_with_find_char() {
    // Two `x` chars so we can move from the first to the second.
    let mut ed = editor_from("-[a]>bxcxd\n");

    // Record `f` then `x` into register 'q'.
    ed.handle_key(key('Q'));
    ed.handle_key(key('Q'));
    ed.handle_key(key('f'));
    ed.handle_key(key('x'));
    ed.handle_key(key('Q'));

    let keys = ed
        .state
        .registers
        .read('q')
        .and_then(|r| r.as_macro())
        .map(|k| k.to_vec())
        .unwrap();
    assert_eq!(
        keys.len(),
        2,
        "macro should contain exactly 2 keys (f and x), got {:?}",
        keys
    );
    assert_eq!(keys[0].code, KeyCode::Char('f'));
    assert_eq!(keys[1].code, KeyCode::Char('x'));

    // After recording, cursor is on first 'x'. Move to 'c' so replay can find next 'x'.
    ed.handle_key(key('l')); // step right to 'c'

    let before_pos = ed.current_view().primary().head().offset();
    let before_char = ed.doc().text().char_at(before_pos);

    // Replay: `f x` from 'c' should land on the second 'x'.
    ed.handle_key(key('q'));
    ed.handle_key(key('q'));
    ed.drain_replay_queue();

    let after_pos = ed.current_view().primary().head().offset();
    assert!(after_pos > before_pos, "cursor should have moved right");
    assert_eq!(
        ed.doc().text().char_at(after_pos),
        Some('x'),
        "cursor should be on 'x' after replay"
    );
    let _ = before_char;
}

/// Record `i x Esc` (insert 'x' then exit insert mode) into a register.
/// Replay on a different cursor position should insert 'x' there.
#[test]
fn macro_insert_mode_round_trip() {
    let mut ed = editor_from("ab-[c]>d\n");

    // Record: insert 'x' before the current cursor, then Esc.
    ed.handle_key(key('Q'));
    ed.handle_key(key('Q'));
    ed.handle_key(key('i'));
    ed.handle_key(key('x'));
    ed.handle_key(key_esc());
    ed.handle_key(key('Q'));

    // Move to 'd' (one position right) so replay applies at a different spot.
    ed.handle_key(key('l'));
    let before = state(&ed);

    ed.handle_key(key('q'));
    ed.handle_key(key('q'));
    ed.drain_replay_queue();

    let after = state(&ed);
    assert_ne!(after, before, "replay should have modified the buffer");
    assert!(
        ed.doc().text().to_string().matches('x').count() == 2,
        "there should be two 'x' chars: one from recording, one from replay"
    );
}

/// A macro that makes no change leaves the change `.` repeats as it was.
#[test]
fn macro_replay_preserves_dot_repeat() {
    let mut ed = editor_from("-[a]>bc\nxyz\n");

    // Perform a `d` (delete) to establish the recorded change "delete".
    ed.handle_key(key('d'));
    let action_after_delete = ed.state.last_action().map(|a| a.command.as_ref());
    assert_eq!(
        action_after_delete,
        Some("delete"),
        "the recorded change should be 'delete'"
    );

    // Record a `j` motion macro (not repeatable, should not overwrite the recorded change).
    ed.handle_key(key('Q'));
    ed.handle_key(key('Q'));
    ed.handle_key(key('j'));
    ed.handle_key(key('Q'));

    // Replay the macro.
    ed.handle_key(key('q'));
    ed.handle_key(key('q'));
    ed.drain_replay_queue();

    // The macro only moved, so the recorded change is still "delete".
    let action_after_replay = ed.state.last_action().map(|a| a.command.as_ref());
    assert_eq!(
        action_after_replay,
        Some("delete"),
        "dot-repeat must survive macro replay; got {:?}",
        action_after_replay
    );
}

/// `.` inside a macro repeats the change made before the macro ran.
#[test]
fn dot_inside_a_macro_repeats_the_change_made_before_it() {
    let mut ed = editor_from("-[a]>b\ncd\n");
    ed.feed_key(key('d'));
    assert_eq!(ed.doc().text().to_string(), "b\ncd\n");

    ed.state
        .registers
        .write_macro('q', vec![key('j'), key('.')]);
    ed.feed_key(key('q'));
    ed.feed_key(key('q'));
    assert_eq!(ed.doc().text().to_string(), "b\nd\n");
}

/// `.` after a macro repeats the last change the macro made.
#[test]
fn dot_after_a_macro_repeats_the_macros_last_change() {
    let mut ed = editor_from("-[a]>bcd\n");
    ed.state.registers.write_macro('q', vec![key('d')]);
    ed.feed_key(key('q'));
    ed.feed_key(key('q'));
    assert_eq!(ed.doc().text().to_string(), "bcd\n");

    ed.feed_key(key('.'));
    assert_eq!(ed.doc().text().to_string(), "cd\n");
}

/// Pressing `q` while recording should be silently captured as a recorded key.
/// It must not start a register-name read or trigger replay.
#[test]
fn macro_q_during_recording_is_captured() {
    let mut ed = editor_from("-[a]>bcd\n");

    // Start recording into 'q'.
    ed.handle_key(key('Q'));
    ed.handle_key(key('Q'));

    // Press `q` (replay trigger) while recording.
    ed.handle_key(key('q'));

    // Must not have changed pending or replay state.
    assert!(
        !reading_char(&ed),
        "q during recording must not start a register-name read"
    );
    assert!(
        !ed.state.macros.has_queued_keys(),
        "q during recording must not enqueue replay"
    );

    // Stop recording.
    ed.handle_key(key('Q'));

    // The `q` must have been captured as a recorded key.
    let keys = ed
        .state
        .registers
        .read('q')
        .and_then(|r| r.as_macro())
        .map(|k| k.to_vec())
        .unwrap();
    assert_eq!(
        keys.len(),
        1,
        "macro should contain exactly 1 key (the q), got {:?}",
        keys
    );
    assert_eq!(keys[0].code, KeyCode::Char('q'));
}

/// A macro containing `qq` (self-replay) must not actually replay during replay:
/// the replay guard must suppress the nested `q` intercept.
#[test]
fn macro_recursive_replay_suppressed() {
    let mut ed = editor_from("-[a]>\nb\nc\n");

    // Seed a macro `[q, q]` (self-replay) into 'q' manually.
    ed.state.registers.write_macro(
        'q',
        vec![
            KeyEvent::new(KeyCode::Char('q'), Modifiers::NONE),
            KeyEvent::new(KeyCode::Char('q'), Modifiers::NONE),
        ],
    );

    // Replay: qq. The macro contains `qq` which should be suppressed.
    ed.handle_key(key('q'));
    ed.handle_key(key('q'));
    ed.drain_replay_queue();

    // Neither recording nor pending should be armed.
    assert!(
        ed.state.macros.recording_register().is_none(),
        "nested recording must not start"
    );
    assert!(
        !reading_char(&ed),
        "no register-name read may be left after replay"
    );
    assert!(
        !ed.state.macros.has_queued_keys(),
        "replay queue must be empty after drain"
    );
}

/// `QQ Q`: record with zero keys, then stop. The register should hold an
/// empty macro. Replaying it is a no-op.
#[test]
fn macro_empty_recording() {
    let mut ed = editor_from("-[a]>bcd\n");
    let before = state(&ed);

    // Start and immediately stop recording.
    ed.handle_key(key('Q'));
    ed.handle_key(key('Q'));
    ed.handle_key(key('Q'));

    // Register 'q' should hold an empty macro (Some, but zero keys).
    let keys = ed
        .state
        .registers
        .read('q')
        .and_then(|r| r.as_macro())
        .map(|k| k.to_vec());
    assert!(keys.is_some(), "register should hold a macro (not None)");
    assert!(keys.unwrap().is_empty(), "macro should have zero keys");

    // Replay: no-op.
    ed.handle_key(key('q'));
    ed.handle_key(key('q'));
    ed.drain_replay_queue();
    assert_eq!(
        state(&ed),
        before,
        "replay of empty macro must not change state"
    );
}

/// `Esc` while recording should be captured as a key (stopping insert/extend),
/// not stop the recording session itself. Recording continues after Esc.
#[test]
fn macro_esc_during_recording_is_captured() {
    let mut ed = editor_from("-[a]>bcd\n");

    // Start recording.
    ed.handle_key(key('Q'));
    ed.handle_key(key('Q'));
    assert!(ed.state.macros.recording_register().is_some());

    // Press Esc: this should be recorded, not stop the session.
    ed.handle_key(key_esc());
    assert!(
        ed.state.macros.recording_register().is_some(),
        "Esc must not stop recording"
    );

    // Record a motion to confirm the session is still open.
    ed.handle_key(key('j'));

    // Stop recording.
    ed.handle_key(key('Q'));
    assert!(ed.state.macros.recording_register().is_none());

    // Macro should contain Esc and j.
    let keys = ed
        .state
        .registers
        .read('q')
        .and_then(|r| r.as_macro())
        .map(|k| k.to_vec())
        .unwrap();
    assert_eq!(
        keys.len(),
        2,
        "expected [Esc, j], got {} keys: {:?}",
        keys.len(),
        keys
    );
    assert_eq!(keys[0].code, KeyCode::Escape);
    assert_eq!(keys[1].code, KeyCode::Char('j'));
}

/// A count prefix before `Q` (e.g. `3Qq`) must not leak into the recording
/// session: the count is consumed by the `Q` intercept and not stored.
#[test]
fn macro_count_prefix_before_record_does_not_leak() {
    let mut ed = editor_from("-[a]>bcd\n");

    // `3` then `Q` then `q`: count prefix before start-record sequence.
    ed.handle_key(key('3'));
    ed.handle_key(key('Q'));
    ed.handle_key(key('q')); // register name

    // Recording should have started cleanly.
    assert!(
        ed.state.macros.recording_register().is_some(),
        "recording should start after Q<reg>"
    );
    // Count must be consumed/cleared.
    assert!(
        ed.state.count.is_none(),
        "count must be cleared after Q<reg> sequence"
    );

    ed.handle_key(key('Q')); // stop
    assert!(ed.state.macros.recording_register().is_none());
}

/// After replaying a macro, `u` should undo the edits made by the macro.
#[test]
fn macro_replay_undo() {
    let mut ed = editor_from("-[f]>oo\nbar\n");

    // Record `d` (delete selection) into 'q'.
    ed.handle_key(key('Q'));
    ed.handle_key(key('Q'));
    ed.handle_key(key('d'));
    ed.handle_key(key('Q'));

    let before_replay = state(&ed);

    // Replay: cursor deletes its selection.
    ed.handle_key(key('q'));
    ed.handle_key(key('q'));
    ed.drain_replay_queue();

    let after_replay = state(&ed);
    assert_ne!(
        after_replay, before_replay,
        "replay should have changed state"
    );

    // Undo should restore to the pre-replay state.
    ed.handle_key(key('u'));
    assert_eq!(
        state(&ed),
        before_replay,
        "undo after replay should restore pre-replay state"
    );
}

/// Record into register 1, undo, then replay; the edit should be reapplied.
#[test]
fn macro_q1_replay_after_undo() {
    let mut ed = editor_from("-[h]>ello world\nhello world\n");

    // Q 1: start recording into register '1'
    ed.handle_key(key('Q'));
    ed.handle_key(key('1'));
    assert!(
        ed.state.macros.recording_register().is_some(),
        "recording into register 1"
    );

    // Record: w (select word) → c (change) → x (insert 'x') → Esc
    ed.handle_key(key('w'));
    ed.handle_key(key('c'));
    ed.handle_key(key('x'));
    ed.handle_key(key_esc());

    // Q: stop recording
    ed.handle_key(key('Q'));
    assert!(ed.state.macros.recording_register().is_none());
    assert!(
        ed.state
            .registers
            .read('1')
            .and_then(|r| r.as_macro())
            .is_some(),
        "macro saved"
    );

    // Undo the edit
    ed.handle_key(key('u'));
    let before = state(&ed);

    // q 1: replay from register '1'
    ed.handle_key(key('q'));
    ed.handle_key(key('1'));
    assert!(ed.state.macros.has_queued_keys(), "replay queue populated");

    ed.drain_replay_queue();

    assert_ne!(state(&ed), before, "replay should have changed the state");
}

/// Regression: a macro containing two consecutive `p` commands must not panic
/// or corrupt undo. `commit_paste_session` must run during replay too, or
/// the second `p` hits `begin_edit_group` on an already-open `paste_group`
/// and trips a `debug_assert`.
#[test]
fn macro_with_two_pastes_does_not_panic() {
    let mut ed = editor_from("-[AB]>CD\n");
    // Push "AB" onto the ring by deleting it.
    ed.handle_key(key('d')); // kill "AB"; ring head = "AB"; buffer = "-[C]>D\n"

    // Seed a macro [p, p] (two consecutive pastes) into register 'q'.
    ed.state
        .registers
        .write_macro('q', vec![key('p'), key('p')]);

    // Replay: qq + drain. Must not panic.
    ed.handle_key(key('q'));
    ed.handle_key(key('q'));
    ed.drain_replay_queue();

    // Both pastes must have fired: "AB" appears exactly twice.
    let buf = ed.doc().text().to_string();
    assert_eq!(
        buf.matches("AB").count(),
        2,
        "both pastes in the macro must fire; got: {buf:?}"
    );
}

/// `.` inside a macro replay fires `replay_dot` within the replay loop.
///
/// Macro `['d', '.']`: the inner `d` records the change inside the loop, then
/// `.` reads it and fires `replay_dot` at the tail of the inner `handle_key`
/// call, within `drain_replay_queue`'s loop, not after.
///
/// If `replay_dot` ran only after the replay loop, the `.` would set
/// `pending_repeat` on the last iteration without draining it, leaving the
/// buffer with only one `d` applied.
#[test]
fn dot_inside_macro_replay_fires_drain() {
    // Four chars so deletes leave visible residue.
    let mut ed = editor_from("-[a]>bcde\n");

    // Record a macro ["d", "."]: d deletes, then . repeats the delete.
    // Seed directly to avoid mutating the buffer during the recording phase.
    ed.state
        .registers
        .write_macro('q', vec![key('d'), key('.')]);

    // Replay: qq + drain. The doc comment above explains what each replayed
    // key does inside `drain_replay_queue`'s loop.
    ed.handle_key(key('q'));
    ed.handle_key(key('q'));
    ed.drain_replay_queue();

    // Both deletes must have fired ('a' by d, 'b' by the in-loop drain of '.').
    let buf = ed.doc().text().to_string();
    assert_eq!(
        buf, "cde\n",
        "dot inside macro replay must fire drain within loop; got: {buf:?}"
    );

    // The macro's own change stays recorded for a `.` after it.
    assert_eq!(
        ed.state.last_action().map(|a| a.command.as_ref()),
        Some("delete")
    );
}

/// A macro register can hold a raw `KeyEvent` from a non-conformant delivery
/// (the uppercase codepoint with SHIFT still set, per `handle_normal`'s doc
/// comment), however it got recorded. Replaying it must resolve identically
/// to a live press of the same key delivered cleanly: the stored key routes
/// back through `handle_key`, which applies the same SHIFT-strip and trie
/// canonicalization it would for any live keypress.
///
/// The register is written directly (rather than via `QQ...Q` recording) so
/// this test isolates the replay path from live dispatch during recording:
/// `non_conformant_i` here is never dispatched except through replay.
#[test]
fn macro_replay_resolves_non_conformant_shift_delivery_like_clean_press() {
    use termina::event::{KeyCode, KeyEvent, Modifiers};

    let non_conformant_i = KeyEvent::new(KeyCode::Char('I'), Modifiers::SHIFT);

    let mut recorded = editor_from("-[a]>bcd\n");
    recorded
        .state
        .registers
        .write_macro('q', vec![non_conformant_i]);
    recorded.handle_key(key('q'));
    recorded.handle_key(key('q'));
    recorded.drain_replay_queue();

    let mut direct = editor_from("-[a]>bcd\n");
    direct.handle_key(key('I'));

    assert_eq!(
        recorded.state.mode(),
        direct.state.mode(),
        "mode must match"
    );
    assert_eq!(
        recorded.current_view().primary().head().offset(),
        direct.current_view().primary().head().offset(),
        "cursor position must match"
    );
}

/// A hook a macro's own early key queues must run before the macro's later
/// keys dispatch: `drain_replay_queue` settles after every replayed key,
/// not once after the whole queue, so a replayed `@q` matches typing the
/// same keys by hand.
///
/// `i` queues `OnModeChange` (deferred: fired by the next `settle()`, not
/// synchronously); the hook it drives writes a buffer-scoped `indent-style`
/// override that the very next key, `<tab>`, reads. `recorded` replays the
/// macro in one shot; `direct` uses `feed_event` to mirror `Editor::run`'s
/// loop (dispatch, then settle, per key), as the reference for what "typed
/// by hand" produces.
///
/// If `drain_replay_queue` settled only once at the end of the loop,
/// `recorded`'s `<tab>` would run while `indent-style` is still the compiled-in
/// `Hard` and insert a literal `\t` where `direct` inserts spaces.
#[test]
fn macro_replay_runs_a_replayed_keys_own_hook_before_the_next_replayed_key() {
    use crate::editor::tests::language::attach_host;

    const HOOK: &str = r#"(register-hook! 'on-mode-change (lambda (from to)
        (when (equal? to 'insert)
          (set-buffer-option! (focused-pane) "indent-style" "soft"))))"#;

    let mut recorded = editor_from("-[a]>b\n");
    attach_host(&mut recorded, HOOK);
    recorded
        .state
        .registers
        .write_macro('q', vec![key('i'), key_tab(), key_esc()]);
    recorded.handle_key(key('q'));
    recorded.handle_key(key('q'));
    recorded.drain_replay_queue();

    let mut direct = editor_from("-[a]>b\n");
    attach_host(&mut direct, HOOK);
    direct.feed_event(key('i'));
    direct.feed_event(key_tab());
    direct.feed_event(key_esc());

    assert_eq!(
        state(&recorded),
        state(&direct),
        "replaying a macro must match typing the same keys by hand"
    );
    assert!(
        !recorded.doc().text().to_string().contains('\t'),
        "the on-mode-change hook must have applied soft indent-style before \
         <tab> dispatched during replay, got: {:?}",
        recorded.doc().text().to_string()
    );
}

/// A mouse click ends the wait for `q`'s or `"`'s register name, so the
/// next key runs its own binding instead of being read as one.
#[test]
fn mouse_click_cancels_a_register_name_read() {
    for prefix in ['q', '"'] {
        let mut ed = editor_from("-[h]>ello\n");
        ed.handle_key(key(prefix));
        ed.handle_input(mouse_left_down(0, 0));
        ed.handle_key(key('l'));
        assert_eq!(state(&ed), "h-[e]>llo\n", "after {prefix:?}");
    }
}

/// A macro that ends with its command's read still pending keeps that
/// command's change: the read's answer and the callback's edits belong to
/// it, so `.` repeats the whole command.
#[test]
fn macro_ending_mid_read_keeps_its_change() {
    let tmp = tempfile::tempdir().unwrap();
    let mut ed = editor_from("-[h]>ello\nworld\n");
    run(
        &mut ed,
        tmp.path(),
        r#"(define-command! "rd" ""
             (lambda (pane)
               (on-next-char!
                 (lambda (c)
                   (when c (call! "replace" pane c) (call! "surround-add" pane #\()))))
             #:repeatable #t)
           (bind-key! 'normal "ctrl-x" "rd")"#,
    );
    ed.state.macros.enqueue(&[key_ctrl('x')], 1);
    ed.drain_replay_queue();
    assert!(reading_char(&ed));
    ed.feed_key(key('z'));
    ed.settle();
    assert_eq!(ed.doc().text().to_string(), "(z)ello\nworld\n");
    ed.feed_key(key('j'));
    ed.feed_key(key('g'));
    ed.feed_key(key('h'));
    ed.feed_key(key('.'));
    assert_eq!(ed.doc().text().to_string(), "(z)ello\n(z)orld\n");
}
