use termina::event::KeyEvent;

use super::Editor;
use super::input_stack::{InputEvent, LayerKind, LayerRef};
use super::minibuf::MiniBufferEvent;
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
    // ── Key dispatch ──────────────────────────────────────────────────────────

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
        // Runs after all mode handlers so Insert, Command, and Search keys
        // are captured. `skip_macro_record` excludes the stop `Q` itself.
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

    /// Bare stack walk, no cross-cutting bookkeeping — `handle_key` wraps it
    /// with the status/summary bookkeeping, macro recording, and dot-repeat
    /// replay above and below. `replay_dot` calls this
    /// directly instead of going through `handle_key`, since a replayed key
    /// must skip all of that (recording it again, re-arming the summary
    /// countdown).
    pub(in crate::editor) fn dispatch_input(&mut self, ev: InputEvent) {
        let top = self.state.input.top();
        self.dispatch_at(top, ev);
    }

    fn dispatch_at(&mut self, r: LayerRef, ev: InputEvent) {
        let kind = self
            .state
            .input
            .kind(r)
            .expect("dispatch target is live: top(), or below(r) under the index invariant");
        match kind {
            LayerKind::Base => self.base_input(r, ev),
            LayerKind::Insert => self.insert_input(r, ev),
            LayerKind::Command => self.command_input(r, ev),
            LayerKind::Search => self.search_input(r, ev),
            LayerKind::Sift => self.sift_input(r, ev),
            LayerKind::Prompt => self.prompt_input(r, ev),
            LayerKind::Drawer => self.drawer_input(r, ev),
            LayerKind::Menu => self.menu_input(r, ev),
            LayerKind::Picker => self.picker_input(r, ev),
            LayerKind::Confirm => self.confirm_input(r, ev),
            LayerKind::Completion => self.completion_input(r, ev),
            LayerKind::Popup => self.popup_input(r, ev),
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
    /// every other mode is its own `LayerKind` now, routed directly by
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
    /// (the cursor still moves under an open `:`/`/`/`s` prompt, as before
    /// the stack existed) and returns `None` either way. `None` also covers
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
