use super::*;
use pretty_assertions::assert_eq;

// ── Keyboard macros ───────────────────────────────────────────────────────────

/// `QQ` starts recording into register `q`, second `Q` stops.
/// Keys typed during recording are stored as a macro.
#[test]
fn macro_qq_records_into_register_q() {
    let mut ed = editor_from("-[a]>bcd\n");
    // First `Q` sets the pending state — recording hasn't started yet.
    ed.handle_key(key('Q'));
    assert!(
        ed.macro_recording.is_none(),
        "recording not started until register name given"
    );
    assert!(ed.macro_pending.is_some(), "pending should be set after Q");

    // Second `Q` is consumed as the register name — recording starts now.
    ed.handle_key(key('Q'));
    assert!(
        ed.macro_recording.is_some(),
        "recording should start after Q<reg>"
    );
    assert_eq!(ed.macro_recording.as_ref().unwrap().0, 'q');

    // Record a motion: j (move down)
    ed.handle_key(key('j'));

    // Stop recording: Q
    ed.handle_key(key('Q'));
    assert!(
        ed.macro_recording.is_none(),
        "recording should stop after stop-Q"
    );

    // Register 'q' should now hold a macro with [j] (not the register-name Q or stop Q)
    let keys = ed
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
    assert!(ed.macro_recording.is_some());
    assert_eq!(ed.macro_recording.as_ref().unwrap().0, '0');
    ed.handle_key(key('j'));
    ed.handle_key(key('Q'));
    assert!(ed.macro_recording.is_none());
    let keys = ed
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
    assert!(ed.macro_pending.is_some(), "pending should be set after Q");
    ed.handle_key(key_esc());
    assert!(
        ed.macro_pending.is_none(),
        "pending should be cleared after Esc"
    );
    assert!(
        ed.macro_recording.is_none(),
        "no recording should have started"
    );
}

/// `q Esc` cancels: no replay is queued.
#[test]
fn macro_big_q_esc_cancels() {
    let mut ed = editor_from("-[a]>bcd\n");
    ed.handle_key(key('q'));
    assert!(ed.macro_pending.is_some());
    ed.handle_key(key_esc());
    assert!(ed.macro_pending.is_none());
    assert!(ed.replay_queue.is_empty());
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

    let before = ed.current_selections().primary().head;

    // `qq` replays from the default register — no extra key needed.
    ed.handle_key(key('q'));
    ed.handle_key(key('q'));

    ed.drain_replay_queue();

    let after = ed.current_selections().primary().head;
    assert!(after > before, "cursor should have moved down after replay");
}

/// `q` followed by a non-register key cancels replay — key is swallowed.
#[test]
fn macro_big_q_non_register_key_cancels() {
    let mut ed = editor_from("-[a]>\nb\nc\n");

    // Record `j` into register `q` so there's something to (not) replay.
    ed.handle_key(key('Q'));
    ed.handle_key(key('Q'));
    ed.handle_key(key('j'));
    ed.handle_key(key('Q'));

    let before = ed.current_selections().primary().head;

    // `q` then `Q` (uppercase, not a valid register) — cancelled, cursor stays put.
    ed.handle_key(key('q'));
    ed.handle_key(key('Q'));

    ed.drain_replay_queue();

    let after = ed.current_selections().primary().head;
    assert_eq!(before, after, "cancelled replay should not move cursor");
}

/// Replay of an empty/nonexistent register is a no-op.
#[test]
fn macro_replay_empty_register_is_noop() {
    let mut ed = editor_from("-[a]>bcd\n");
    let before = state(&ed);

    // `q` must arm macro_pending — proving the dispatch path ran.
    ed.handle_key(key('q'));
    assert!(
        ed.macro_pending.is_some(),
        "macro_pending should be set after q"
    );

    // Register 'z' has never been written — macro_pending is consumed but
    // no keys are queued and state is unchanged.
    ed.handle_key(key('z'));
    assert!(
        ed.macro_pending.is_none(),
        "macro_pending should be consumed after register key"
    );
    assert!(
        ed.replay_queue.is_empty(),
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
    ed.registers.write_macro(
        'q',
        vec![
            KeyEvent::new(KeyCode::Char('Q'), KeyModifiers::NONE),
            KeyEvent::new(KeyCode::Char('Q'), KeyModifiers::NONE),
            KeyEvent::new(KeyCode::Char('j'), KeyModifiers::NONE),
        ],
    );

    // Trigger replay: qq
    ed.handle_key(key('q'));
    ed.handle_key(key('q'));

    ed.drain_replay_queue();

    // Recording should NOT have started — the Q intercept is suppressed during replay
    assert!(
        ed.macro_recording.is_none(),
        "nested recording must be suppressed"
    );
    assert!(
        ed.macro_pending.is_none(),
        "macro_pending must not be armed after replay"
    );
}

/// A macro whose last key is `Q` must not arm `macro_pending` after replay.
///
/// Previously, the suppression checked `replay_queue.is_empty()`, which becomes
/// `true` at the exact moment the last key is processed — causing a trailing `Q`
/// to slip through and arm `macro_pending`. The fix uses `is_replaying` instead.
#[test]
fn macro_trailing_q_does_not_arm_pending() {
    let mut ed = editor_from("-[a]>\nb\nc\n");

    // Seed a macro ending with Q (can't be recorded normally).
    ed.registers.write_macro(
        'q',
        vec![
            KeyEvent::new(KeyCode::Char('j'), KeyModifiers::NONE),
            KeyEvent::new(KeyCode::Char('Q'), KeyModifiers::NONE),
        ],
    );

    // Replay: qq
    ed.handle_key(key('q'));
    ed.handle_key(key('q'));

    ed.drain_replay_queue();

    assert!(
        ed.macro_recording.is_none(),
        "recording must not have started"
    );
    assert!(
        ed.macro_pending.is_none(),
        "macro_pending must not be armed by trailing Q"
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
    use crate::ui::statusline::StatusElement;

    let ed = editor_from("-[a]>bcd\n");
    let config = &ed.settings.statusline;
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
        .registers
        .read('q')
        .and_then(|r| r.as_macro())
        .map(|k| k.to_vec())
        .unwrap();
    assert_eq!(
        keys.len(),
        3,
        "expected i, x, Esc — got {} keys: {:?}",
        keys.len(),
        keys
    );
    assert_eq!(keys[0].code, KeyCode::Char('i'));
    assert_eq!(keys[1].code, KeyCode::Char('x'));
    assert_eq!(keys[2].code, KeyCode::Esc);
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

    let start = ed.current_selections().primary().head;
    let start_line = ed.doc().text().char_to_line(start);
    assert_eq!(start_line, 0, "cursor should be on line 0 before replay");

    // `3qq` — count 3, replay from register 'q'.
    ed.handle_key(key('3'));
    ed.handle_key(key('q'));
    ed.handle_key(key('q'));
    ed.drain_replay_queue();

    let end_line = ed
        .doc()
        .text()
        .char_to_line(ed.current_selections().primary().head);
    assert_eq!(
        end_line, 3,
        "expected cursor on line 3, got line {}",
        end_line
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
    ed.registers.write_text('0', vec!["some text".into()]);
    ed.handle_key(key('q'));
    assert!(ed.macro_pending.is_some());
    ed.handle_key(key('0'));
    assert!(ed.macro_pending.is_none());
    assert!(
        ed.replay_queue.is_empty(),
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

    let before_pos = ed.current_selections().primary().head;
    let before_char = ed.doc().text().char_at(before_pos);

    // Replay: `f x` from 'c' should land on the second 'x'.
    ed.handle_key(key('q'));
    ed.handle_key(key('q'));
    ed.drain_replay_queue();

    let after_pos = ed.current_selections().primary().head;
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
        "there should be two 'x' chars — one from recording, one from replay"
    );
}

/// After replaying a macro, `.` must repeat the last *editing* action, not
/// the macro itself. `last_repeatable_action` is saved/restored around the replay drain.
#[test]
fn macro_replay_preserves_dot_repeat() {
    let mut ed = editor_from("-[a]>bc\nxyz\n");

    // Perform a `d` (delete) to establish last_repeatable_action = "delete".
    ed.handle_key(key('d'));
    let action_after_delete = ed.last_repeatable_action.as_ref().map(|a| a.command.as_ref());
    assert_eq!(
        action_after_delete,
        Some("delete"),
        "last_repeatable_action should be 'delete'"
    );

    // Record a `j` motion macro (not repeatable — should not overwrite last_repeatable_action).
    ed.handle_key(key('Q'));
    ed.handle_key(key('Q'));
    ed.handle_key(key('j'));
    ed.handle_key(key('Q'));

    // Replay the macro.
    ed.handle_key(key('q'));
    ed.handle_key(key('q'));
    ed.drain_replay_queue();

    // last_repeatable_action must still be "delete", not whatever the macro did.
    let action_after_replay = ed.last_repeatable_action.as_ref().map(|a| a.command.as_ref());
    assert_eq!(
        action_after_replay,
        Some("delete"),
        "dot-repeat must survive macro replay; got {:?}",
        action_after_replay
    );
}

/// Pressing `q` while recording should be silently captured as a recorded key
/// — it must not arm macro_pending or trigger replay.
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
        ed.macro_pending.is_none(),
        "q during recording must not arm macro_pending"
    );
    assert!(
        ed.replay_queue.is_empty(),
        "q during recording must not enqueue replay"
    );

    // Stop recording.
    ed.handle_key(key('Q'));

    // The `q` must have been captured as a recorded key.
    let keys = ed
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

/// A macro containing `qq` (self-replay) must not actually replay during replay —
/// the `is_replaying` guard must suppress the nested `q` intercept.
#[test]
fn macro_recursive_replay_suppressed() {
    let mut ed = editor_from("-[a]>\nb\nc\n");

    // Seed a macro `[q, q]` (self-replay) into 'q' manually.
    ed.registers.write_macro(
        'q',
        vec![
            KeyEvent::new(KeyCode::Char('q'), KeyModifiers::NONE),
            KeyEvent::new(KeyCode::Char('q'), KeyModifiers::NONE),
        ],
    );

    // Replay: qq. The macro contains `qq` which should be suppressed.
    ed.handle_key(key('q'));
    ed.handle_key(key('q'));
    ed.drain_replay_queue();

    // Neither recording nor pending should be armed.
    assert!(
        ed.macro_recording.is_none(),
        "nested recording must not start"
    );
    assert!(
        ed.macro_pending.is_none(),
        "macro_pending must not be armed after replay"
    );
    assert!(
        ed.replay_queue.is_empty(),
        "replay queue must be empty after drain"
    );
}

/// `QQ Q` — record with zero keys, then stop. The register should hold an
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
    assert!(ed.macro_recording.is_some());

    // Press Esc — this should be recorded, not stop the session.
    ed.handle_key(key_esc());
    assert!(ed.macro_recording.is_some(), "Esc must not stop recording");

    // Record a motion to confirm the session is still open.
    ed.handle_key(key('j'));

    // Stop recording.
    ed.handle_key(key('Q'));
    assert!(ed.macro_recording.is_none());

    // Macro should contain Esc and j.
    let keys = ed
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
    assert_eq!(keys[0].code, KeyCode::Esc);
    assert_eq!(keys[1].code, KeyCode::Char('j'));
}

/// A count prefix before `Q` (e.g. `3Qq`) must not leak into the recording
/// session — the count is consumed by the `Q` intercept and not stored.
#[test]
fn macro_count_prefix_before_record_does_not_leak() {
    let mut ed = editor_from("-[a]>bcd\n");

    // `3` then `Q` then `q` — count prefix before start-record sequence.
    ed.handle_key(key('3'));
    ed.handle_key(key('Q'));
    ed.handle_key(key('q')); // register name

    // Recording should have started cleanly.
    assert!(
        ed.macro_recording.is_some(),
        "recording should start after Q<reg>"
    );
    // Count must be consumed/cleared.
    assert!(
        ed.count.is_none(),
        "count must be cleared after Q<reg> sequence"
    );

    ed.handle_key(key('Q')); // stop
    assert!(ed.macro_recording.is_none());
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

/// Record into register 1, undo, then replay — the edit should be reapplied.
#[test]
fn macro_q1_replay_after_undo() {
    let mut ed = editor_from("-[h]>ello world\nhello world\n");

    // Q 1: start recording into register '1'
    ed.handle_key(key('Q'));
    ed.handle_key(key('1'));
    assert!(ed.macro_recording.is_some(), "recording into register 1");

    // Record: w (select word) → c (change) → x (insert 'x') → Esc
    ed.handle_key(key('w'));
    ed.handle_key(key('c'));
    ed.handle_key(key('x'));
    ed.handle_key(key_esc());

    // Q: stop recording
    ed.handle_key(key('Q'));
    assert!(ed.macro_recording.is_none());
    assert!(
        ed.registers.read('1').and_then(|r| r.as_macro()).is_some(),
        "macro saved"
    );

    // Undo the edit
    ed.handle_key(key('u'));
    let before = state(&ed);

    // q 1: replay from register '1'
    ed.handle_key(key('q'));
    ed.handle_key(key('1'));
    assert!(!ed.replay_queue.is_empty(), "replay queue populated");

    ed.drain_replay_queue();

    assert_ne!(state(&ed), before, "replay should have changed the state");
}
