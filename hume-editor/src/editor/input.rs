//! Where terminal input enters the editor: a key, a mouse event or a
//! paste, interactively or one key at a time from the headless runner.

use termina::event::{Event as TerminalEvent, KeyEvent, KeyEventKind};

use super::Editor;
use super::event::EditorEvent;
use super::input_stack::{InputEvent, PastedText};

impl Editor {
    /// Process one key event: dispatch it, wait for a pipe run it started,
    /// sync the search cache, drain any macro replay, sync again.
    ///
    /// This is the single, non-test path for feeding one keystroke to the editor
    /// from outside the interactive event loop (e.g. headless key-runner). Unlike
    /// [`Self::handle_input`], `step` does not itself call [`Self::settle`]. The
    /// caller (`hume_editor::run_keys`) calls it once per key, mirroring how the
    /// interactive loop settles once per iteration rather than once per input
    /// handler. The one exception is [`Self::drain_replay_queue`], which settles
    /// once per replayed key while the replay mark is still set (see that
    /// function's doc). Here we use
    /// [`Self::handle_key`] directly so the caller doesn't need a scripting host.
    pub(crate) fn step(&mut self, key: KeyEvent) {
        let runs_from = self.state.pipe_runs.mark();
        self.handle_key(key);
        self.wait_pipe_runs_from(runs_from);
        self.sync_search_cache();
        self.drain_replay_queue();
        self.sync_search_cache();
    }

    /// Single interactive input boundary: dispatch one terminal event.
    ///
    /// All interactive input flows through here: key events and mouse events
    /// alike. This does not drain queued work itself, nor diff focus
    /// itself (see `tests/sync_dispatch.rs`'s
    /// `mouse_click_leaves_hook_queued_until_the_next_settle`, which pins
    /// it): `Editor::run`'s loop calls `settle()` once per
    /// iteration, at the top, and `settle()`'s own fixpoint is where a focus
    /// change made here (or by a hook handler, or by non-interactive Steel/
    /// LSP code) is observed and turned into `OnBufferEnter`. New input
    /// paths should still route through here: it's what marks
    /// `message_logged_this_input` for `settle()`'s disk check to honour
    /// below.
    ///
    /// Sets `message_logged_this_input` whenever this dispatch itself logged
    /// a new warning or error: a command that fails after moving focus
    /// (`:qa` landing on the first dirty buffer) must keep its own message on
    /// screen instead of losing it to an unrelated disk-change confirm the
    /// next `settle()` might open. `settle()` clears the flag once that
    /// drain has run; see `EditorState::message_logged_this_input`'s doc for
    /// why the window spans both calls.
    pub(crate) fn handle_input(&mut self, ev: TerminalEvent) {
        let totals_before = self.state.message_log.totals();
        match ev {
            TerminalEvent::Key(k) => self.handle_key(k),
            TerminalEvent::Mouse(m) => self.dispatch_input(InputEvent::Mouse(m)),
            TerminalEvent::Paste(s) => self.handle_terminal_paste(s),
            // Regaining focus is one of the external-file-change check's
            // trigger points (alongside buffer-enter and `:checktime`); see
            // `DiskCheckTrigger::Ambient`. `FocusOut` needs no handling:
            // there's nothing to check until focus returns.
            TerminalEvent::FocusIn => self.state.queue_event(EditorEvent::OnFocusGained),
            _ => {}
        }
        self.state.message_logged_this_input = self.state.message_log.totals() != totals_before;
    }

    /// Handle one non-resize terminal event read by [`Self::run_loop`].
    pub(in crate::editor) fn dispatch_terminal_event(&mut self, ev: TerminalEvent) {
        match ev {
            // Release events arrive only with kitty keyboard protocol
            // (REPORT_EVENT_TYPES flag). Ignore them; we act on Press and
            // Repeat (held key). Without kitty all events are Press anyway.
            TerminalEvent::Key(key) if key.kind != KeyEventKind::Release => {
                self.handle_input(TerminalEvent::Key(key));
                self.sync_search_cache();
            }
            TerminalEvent::Paste(text) => {
                self.handle_input(TerminalEvent::Paste(text));
                self.sync_search_cache();
            }
            // Regaining focus raises `OnFocusGained` (see `handle_input`,
            // which is what actually queues it). The external-file-change
            // sweep is that event's Rust reaction, not a direct call here.
            ev @ (TerminalEvent::Mouse(_) | TerminalEvent::FocusIn) => self.handle_input(ev),
            // CSI/OSC/DCS protocol responses: nothing in the run loop
            // needs them. The `|_| true` filter guarantees they can't
            // pile up unread in the reader's buffer either way.
            _ => {}
        }
    }

    pub(in crate::editor) fn handle_key(&mut self, key: KeyEvent) {
        debug_assert!(
            !self.state.recording_scope.is_set(),
            "nothing in the scope slot outlives the call that set it"
        );
        debug_assert!(
            self.state.pipe_runs.foreground().is_none(),
            "a foreground pipe run is waited for before the next key"
        );
        // How many keystrokes the message-log summary stays visible after
        // `status_msg` clears. Chosen for UX, not technical constraint.
        const SUMMARY_TTL: u8 = 3;

        // Any keypress dismisses the previous transient status message.
        // When it clears with unseen log entries the summary becomes visible,
        // so arm its countdown. On later keypresses tick it down and auto-dismiss
        // at zero. The countdown only runs when the minibuffer is closed so
        // typing a long `:` command doesn't burn the budget invisibly.
        let had_status = self.state.status_msg.take().is_some();
        if self.state.input.minibuf().is_none() {
            if had_status {
                if self.state.message_log.has_unseen() {
                    self.state.summary_ttl = SUMMARY_TTL;
                }
            } else if self.state.summary_ttl > 0 {
                self.state.summary_ttl -= 1;
                if self.state.summary_ttl == 0 {
                    self.state.message_log.mark_all_seen();
                }
            }
        }

        self.dispatch_input(InputEvent::Key(key));

        // ── Macro recording ───────────────────────────────────────────────────
        // Runs after the whole `dispatch_input` stack walk, so a key captured
        // into the macro is whatever the topmost layer actually consumed:
        // Insert, Command, Search, or any overlay layer above them.
        self.state.macros.record(key);

        // Replay a pending dot-repeat action, if cmd_repeat set one.
        // Runs after macro recording so the `.` key itself is captured,
        // but the replayed command executes as a fresh dispatch with &mut Editor.
        if let Some(pending) = self.state.pending_repeat.take() {
            self.replay_dot(pending.count);
        }
        debug_assert!(
            !self.state.recording_scope.is_set(),
            "nothing in the scope slot outlives the call that set it"
        );
    }

    /// Handle a whole pasted string arriving as one terminal event: the
    /// terminal's entry to [`EditorState::paste`](super::EditorState::paste).
    /// Named apart from the `p`/`P` register paste commands
    /// (`commands/paste.rs`), an unrelated feature.
    pub(in crate::editor) fn handle_terminal_paste(&mut self, text: String) {
        let Some(text) = PastedText::new(vec![text]) else {
            return;
        };
        // Mirrors `handle_key`'s status-message dismissal, minus the
        // summary-TTL bookkeeping: a paste is a real input event but not a
        // keystroke the TTL countdown should tick against. Unconditional,
        // since the paste itself decides whether it does anything (a
        // swallow, e.g. under Confirm, still dismisses the previous message
        // as a real input event).
        self.state.status_msg.take();
        if let Err(e) = self.state.paste(&self.view, text) {
            self.state.report_error(&e);
        }
    }
}

#[cfg(test)]
mod tests;
