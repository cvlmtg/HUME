//! Keyboard macro state: the recording in progress and the replay queue.

use std::collections::VecDeque;

use termina::event::KeyEvent;

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
}
