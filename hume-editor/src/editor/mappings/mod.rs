use termina::event::KeyEvent;

use super::Editor;
use super::input_stack::{
    BaseLayer, CommandLayer, CompletionLayer, InputEvent, InsertLayer, LayerRef, PickerLayer,
    PromptLayer, SearchLayer, SiftLayer,
};
use super::minibuf::MiniBufferEvent;
use super::overlay_models::{ConfirmModel, DrawerModel, MenuModel, PopupModel};
use super::replay::InsertInput;

mod bracketed_paste;
pub(super) mod command_mode;
mod completion_menu;
mod execute;
mod insert;
mod lazy;
mod normal;
mod search_mode;
mod sift_mode;
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
    /// see `input_stack/stack.rs`'s own doc for why: each handler still
    /// lives beside its old neighbors (`mappings::widgets`,
    /// `mappings::completion_menu`, this very `impl` block) rather than
    /// beside its own `Layer` impl, so there is no `handler()` fn pointer to
    /// call yet. This chain collapses to one as each layer's handler moves
    /// into its own file and starts implementing `handler()` instead.
    fn dispatch_at(&mut self, r: LayerRef, ev: InputEvent) {
        let input = &self.state.input;
        if input.is::<BaseLayer>(r) {
            self.base_input(r, ev)
        } else if input.is::<InsertLayer>(r) {
            self.insert_input(r, ev)
        } else if input.is::<CommandLayer>(r) {
            self.command_input(r, ev)
        } else if input.is::<SearchLayer>(r) {
            self.search_input(r, ev)
        } else if input.is::<SiftLayer>(r) {
            self.sift_input(r, ev)
        } else if input.is::<PromptLayer>(r) {
            self.prompt_input(r, ev)
        } else if input.is::<DrawerModel>(r) {
            self.drawer_input(r, ev)
        } else if input.is::<MenuModel>(r) {
            self.menu_input(r, ev)
        } else if input.is::<PickerLayer>(r) {
            self.picker_input(r, ev)
        } else if input.is::<ConfirmModel>(r) {
            self.confirm_input(r, ev)
        } else if input.is::<CompletionLayer>(r) {
            self.completion_input(r, ev)
        } else if input.is::<PopupModel>(r) {
            self.popup_input(r, ev)
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

    /// The base layer's own policy. `r` is unused: `Base` never falls
    /// through further (there is nothing below it) and never truncates
    /// itself (it is never removed). Only ever runs for Normal/Extend —
    /// every other mode is its own layer type, routed directly by
    /// `dispatch_at`; `handle_normal` still reads `state.mode()` internally
    /// to tell the two apart.
    fn base_input(&mut self, _r: LayerRef, ev: InputEvent) {
        match ev {
            InputEvent::Key(key) => self.handle_normal(key),
            InputEvent::Paste(text) => self.apply_normal_mode_paste(&text),
            InputEvent::Mouse(mouse) => self.base_mouse(mouse),
        }
    }

    fn insert_input(&mut self, r: LayerRef, ev: InputEvent) {
        match ev {
            InputEvent::Key(key) => self.handle_insert(key),
            InputEvent::Paste(text) => {
                self.apply_insert_mode_paste(&text);
                if let Some(session) = self.state.insert_session.as_mut() {
                    session.keystrokes.push(InsertInput::Paste(text));
                }
            }
            // A click or a wheel notch is Base's own action to run — most
            // visibly, a click's `focus_pane` ends this very Insert session
            // before resolving the click (see `focus::focus_pane`'s doc).
            InputEvent::Mouse(mouse) => self.fall_through(r, InputEvent::Mouse(mouse)),
        }
    }

    /// Converts one `InputEvent` into a `MiniBufferEvent` for whichever
    /// minibuf-mode layer (`Command`/`Search`/`Sift`/`Prompt`) is dispatching
    /// — the single place a key or a paste becomes an edit to the
    /// minibuffer, shared by all four so a paste runs the same `Edited`
    /// follow-up a typed character would (see each mode's own event match).
    /// A mouse event has no minibuffer edit to become — it falls through
    /// (the cursor still moves under an open `:`/`/`/`s` prompt) and
    /// returns `None` either way. `None` also covers
    /// the case where no minibuffer is live (dispatch reached this layer
    /// kind but its payload was already torn down mid-call — matches every
    /// other handler's own liveness discipline).
    fn minibuf_input(&mut self, r: LayerRef, ev: InputEvent) -> Option<MiniBufferEvent> {
        match ev {
            InputEvent::Mouse(mouse) => {
                self.fall_through(r, InputEvent::Mouse(mouse));
                None
            }
            InputEvent::Key(key) => Some(self.state.input.minibuf_mut()?.handle_key(key)),
            InputEvent::Paste(text) => Some(
                self.state
                    .input
                    .minibuf_mut()?
                    .insert_str(&bracketed_paste::flatten_single_line(&text)),
            ),
        }
    }

    fn command_input(&mut self, r: LayerRef, ev: InputEvent) {
        if let Some(event) = self.minibuf_input(r, ev) {
            self.handle_command_event(r, event);
        }
    }

    fn search_input(&mut self, r: LayerRef, ev: InputEvent) {
        if let Some(event) = self.minibuf_input(r, ev) {
            self.handle_search_event(r, event);
        }
    }

    fn sift_input(&mut self, r: LayerRef, ev: InputEvent) {
        if let Some(event) = self.minibuf_input(r, ev) {
            self.handle_sift_event(r, event);
        }
    }

    fn prompt_input(&mut self, r: LayerRef, ev: InputEvent) {
        if let Some(event) = self.minibuf_input(r, ev) {
            self.handle_steel_prompt_event(r, event);
        }
    }
}

// ── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests;
