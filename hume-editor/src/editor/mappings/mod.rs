use termina::event::{KeyCode, KeyEvent, Modifiers};

use hume_scripting::host::PopupKind;

use super::Editor;
use super::input_stack::{InputEvent, LayerKind, LayerRef};

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

        // Popup dismissal/scroll, before the stack walk — see `PopupKind`.
        // `Scrollable` (scrollable hover, `gn`/`gp`'s diagnostic overlay)
        // consumes Ctrl-u/Ctrl-d to scroll when there's actually content past
        // one screenful; otherwise (and for any other key) it closes the
        // popup and falls through to normal dispatch this same call, so a
        // short popup never blocks buffer half-page scroll. The close itself
        // is `ConfigState::dismiss_scrollable_popup`, shared with
        // `handle_mouse` — any input event dismisses a `Scrollable` popup,
        // not just keys.
        if self.state.config.popup.as_ref().map(|p| p.kind) == Some(PopupKind::Scrollable)
            && key.modifiers.contains(Modifiers::CONTROL)
            && let KeyCode::Char(c @ ('u' | 'd')) = key.code
            && self.scroll_popup(c == 'd')
        {
            return;
        }
        self.state.config.dismiss_scrollable_popup();

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
    /// with the status/summary bookkeeping, popup pre-step, macro recording,
    /// and dot-repeat replay above and below. `replay_dot` calls this
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
        let InputEvent::Key(key) = ev;
        self.handle_normal(key);
    }

    fn insert_input(&mut self, _r: LayerRef, ev: InputEvent) {
        let InputEvent::Key(key) = ev;
        self.handle_insert(key);
    }

    fn command_input(&mut self, r: LayerRef, ev: InputEvent) {
        let InputEvent::Key(key) = ev;
        self.handle_command(r, key);
    }

    fn search_input(&mut self, r: LayerRef, ev: InputEvent) {
        let InputEvent::Key(key) = ev;
        self.handle_search(r, key);
    }

    fn sift_input(&mut self, r: LayerRef, ev: InputEvent) {
        let InputEvent::Key(key) = ev;
        self.handle_sift(r, key);
    }

    fn prompt_input(&mut self, r: LayerRef, ev: InputEvent) {
        let InputEvent::Key(key) = ev;
        self.handle_steel_prompt_key(r, key);
    }
}

// ── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests;
