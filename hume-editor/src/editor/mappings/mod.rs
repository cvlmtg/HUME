use termina::event::KeyEvent;

use super::Editor;
use super::input_stack::{
    BaseLayer, CommandLayer, CompletionLayer, ConfirmLayer, DrawerLayer, InputEvent, InsertLayer,
    LayerRef, MenuLayer, PickerLayer, PopupLayer, PromptLayer, SearchLayer, SiftLayer,
};
use super::input_stack::{
    base, command, completion, confirm, drawer, insert, menu, popup, prompt, search, sift,
};

mod bracketed_paste;
mod execute;
mod lazy;
mod widgets;

impl Editor {
    // ── Input dispatch ────────────────────────────────────────────────────────

    pub(in crate::editor) fn handle_key(&mut self, key: KeyEvent) {
        // How many keystrokes the message-log summary stays visible after
        // `status_msg` clears. Chosen for UX, not technical constraint.
        const SUMMARY_TTL: u8 = 3;

        // Any keypress dismisses the previous transient status message.
        // When it clears with unseen log entries the summary becomes visible —
        // arm its countdown. On later keypresses tick it down and auto-dismiss
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
        // into the macro is whatever the topmost layer actually consumed —
        // Insert, Command, Search, or any overlay layer above them.
        // `skip_macro_record` excludes the stop `Q` itself.
        if let Some((_, ref mut keys)) = self.state.macro_recording
            && !self.state.skip_macro_record
        {
            keys.push(key);
        }
        self.state.skip_macro_record = false;

        // Replay a pending dot-repeat action, if cmd_repeat set one.
        // Runs after macro recording so the `.` key itself is captured,
        // but the replayed command executes as a fresh dispatch with &mut Editor.
        if let Some(pending) = self.state.pending_repeat.take() {
            self.replay_dot(pending.count);
        }
    }

    /// Bare stack walk, no cross-cutting bookkeeping — the single entry
    /// point every input event reaches the stack through, whatever its
    /// source. `handle_key` wraps it with the status/summary bookkeeping,
    /// macro recording, and dot-repeat replay above and below, for a key
    /// from the terminal; `handle_input`'s mouse arm and
    /// `handle_terminal_paste` call it directly for those two event kinds,
    /// which carry none of that key-only bookkeeping. `replay_dot` also
    /// calls it directly for a replayed key instead of going through
    /// `handle_key`, since a replay must skip all of that (recording it
    /// again, re-arming the summary countdown).
    pub(in crate::editor) fn dispatch_input(&mut self, ev: InputEvent) {
        let top = self.state.input.top();
        self.dispatch_at(top, ev);
    }

    /// Routes to whichever handler owns `r`'s concrete type. A chain of
    /// `is::<L>()` checks, not a match on a closed `enum`'s discriminant —
    /// see `input_stack/stack.rs`'s own doc for why: `Picker` (still in
    /// `mappings::widgets`, called as an `Editor` method below) hasn't moved
    /// yet, so there's no `handler()` fn pointer to call uniformly. This
    /// chain collapses to one once it does.
    fn dispatch_at(&mut self, r: LayerRef, ev: InputEvent) {
        let input = &self.state.input;
        if input.is::<BaseLayer>(r) {
            base::base_input(self, r, ev)
        } else if input.is::<InsertLayer>(r) {
            insert::insert_input(self, r, ev)
        } else if input.is::<CommandLayer>(r) {
            command::command_input(self, r, ev)
        } else if input.is::<SearchLayer>(r) {
            search::search_input(self, r, ev)
        } else if input.is::<SiftLayer>(r) {
            sift::sift_input(self, r, ev)
        } else if input.is::<PromptLayer>(r) {
            prompt::prompt_input(self, r, ev)
        } else if input.is::<DrawerLayer>(r) {
            drawer::drawer_input(self, r, ev)
        } else if input.is::<MenuLayer>(r) {
            menu::menu_input(self, r, ev)
        } else if input.is::<PickerLayer>(r) {
            self.picker_input(r, ev)
        } else if input.is::<ConfirmLayer>(r) {
            confirm::confirm_input(self, r, ev)
        } else if input.is::<CompletionLayer>(r) {
            completion::completion_input(self, r, ev)
        } else if input.is::<PopupLayer>(r) {
            popup::popup_input(self, r, ev)
        } else {
            unreachable!("dispatch target is live: top(), or below(r) under the index invariant")
        }
    }

    /// Hand `ev` to the layer directly below `r`. The caller never inspects
    /// what is there — every index below `r.depth` is stable for the whole
    /// duration of the handler running at `r` (the index invariant), so the
    /// target is live by construction whether `r` itself is still on the
    /// stack or was just truncated by the caller (self-removal is always
    /// `truncate(r)`, never "pop the top", so `below(r)` works either way).
    /// `LayerRef`'s fields are private to `InputStack`'s own module, so the
    /// "never fall through from `Base`" invariant is enforced there instead
    /// — `InputStack::below` panics unconditionally (not just in debug
    /// builds) if `r` is already `Base`, which is strictly louder than a
    /// `debug_assert!` here could be.
    pub(in crate::editor) fn fall_through(&mut self, r: LayerRef, ev: InputEvent) {
        let below = self.state.input.below(r);
        self.dispatch_at(below, ev);
    }
}

// ── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests;
