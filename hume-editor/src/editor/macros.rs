//! Keyboard macros: the recording in progress, and the replay queue drained
//! through the normal event path.

use std::collections::VecDeque;

use termina::event::{Event as TerminalEvent, KeyEvent};

use super::{Editor, EditorState};

#[derive(Default)]
pub(crate) struct MacroState {
    recording: Option<Recording>,
    queue: VecDeque<KeyEvent>,
    /// `true` from the first replayed key's dispatch until the drain ends, so
    /// a macro whose last key is `Q` or `q` cannot start a nested session
    /// even though the queue is already empty while that key runs.
    replaying: bool,
}

struct Recording {
    register: char,
    keys: Vec<KeyEvent>,
    /// The key that named the register started the recording but is not
    /// part of it.
    skip_next: bool,
}

impl MacroState {
    pub(in crate::editor) fn start_recording(&mut self, register: char) {
        self.recording = Some(Recording {
            register,
            keys: Vec::new(),
            skip_next: true,
        });
    }

    /// Ends the recording, returning its register and keys.
    pub(in crate::editor) fn stop_recording(&mut self) -> Option<(char, Vec<KeyEvent>)> {
        self.recording.take().map(|r| (r.register, r.keys))
    }

    pub(crate) fn recording_register(&self) -> Option<char> {
        self.recording.as_ref().map(|r| r.register)
    }

    /// Appends a dispatched key to the recording, if one is in progress.
    pub(in crate::editor) fn record(&mut self, key: KeyEvent) {
        if let Some(r) = &mut self.recording {
            if r.skip_next {
                r.skip_next = false;
            } else {
                r.keys.push(key);
            }
        }
    }

    pub(in crate::editor) fn enqueue(&mut self, keys: &[KeyEvent], count: usize) {
        for _ in 0..count {
            self.queue.extend(keys.iter().copied());
        }
    }

    #[cfg(test)]
    pub(in crate::editor) fn has_queued_keys(&self) -> bool {
        !self.queue.is_empty()
    }

    pub(in crate::editor) fn is_replaying(&self) -> bool {
        self.replaying
    }

    /// The next queued key, marking a replay in progress until
    /// [`Self::end_replay`].
    pub(in crate::editor) fn next_replay_key(&mut self) -> Option<KeyEvent> {
        let key = self.queue.pop_front();
        if key.is_some() {
            self.replaying = true;
        }
        key
    }

    pub(in crate::editor) fn end_replay(&mut self) {
        self.replaying = false;
    }

    /// Drop the keys still queued for replay.
    pub(in crate::editor) fn abort_replay(&mut self) {
        self.queue.clear();
    }
}

impl EditorState {
    /// Queue the keys of the macro in `reg` for replay, `count` times over.
    /// No-op when the register is empty or holds text rather than a macro.
    pub(in crate::editor) fn enqueue_macro_replay(&mut self, reg: char, count: usize) {
        if let Some(keys) = self.registers.read(reg).and_then(|r| r.as_macro()) {
            self.macros.enqueue(keys, count);
        }
    }
}

impl Editor {
    /// Drain the macro replay queue, executing each key in order, waiting for
    /// a pipe run it started, and settling after each one. A run the user
    /// cancels drops the keys still queued ([`EditorState::stop_replays`]):
    /// `Ctrl-c` stops the macro too.
    ///
    /// Marks a replay in progress for the duration so that `Q`/`q` intercepts
    /// inside replayed keys cannot start nested recording or replay sessions,
    /// including when the macro's last key is `Q` and the queue is already
    /// empty.
    ///
    /// Replayed keys record changes as typed keys do: `.` inside a macro
    /// repeats the latest change, the one made before the macro included,
    /// and `.` after a macro repeats the last change it made.
    ///
    /// Settling once per replayed key, not once after the whole queue, keeps a
    /// macro's own hooks in the loop: a plugin that reconfigures the buffer on
    /// `on-buffer-open` (indent width, a language keymap) has that reaction run
    /// before the macro's remaining keys type into the buffer, as it would for
    /// keys typed by hand. The replay mark stays set across these settles,
    /// so `confirm_permit` still holds back a confirm no queued key could
    /// answer; it waits for the next real buffer-enter.
    ///
    /// `message_logged_this_input` is OR'd back in every iteration so a
    /// stale-buffer warning from a later key's settle doesn't lose an earlier
    /// message's shadowing guard.
    pub(in crate::editor) fn drain_replay_queue(&mut self) {
        let message_already_logged = self.state.message_logged_this_input;
        while let Some(key) = self.state.macros.next_replay_key() {
            let runs_from = self.state.pipe_runs.mark();
            self.handle_input(TerminalEvent::Key(key));
            self.wait_pipe_runs_from(runs_from);
            self.state.message_logged_this_input |= message_already_logged;
            self.settle();
            if self.state.should_quit {
                break;
            }
        }
        self.state.macros.end_replay();
    }
}
