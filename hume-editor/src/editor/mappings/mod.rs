use termina::event::{KeyCode, KeyEvent, Modifiers};

use hume_scripting::host::PopupKind;

use super::{Editor, Mode};

pub(super) mod command_mode;
mod execute;
mod insert;
mod lazy;
mod normal;
mod paste;
mod search_mode;
mod select_mode;
mod widgets;

impl Editor {
    // ── Key dispatch ──────────────────────────────────────────────────────────

    pub(crate) fn handle_key(&mut self, key: KeyEvent) {
        // How many keystrokes the message-log summary stays visible after
        // `status_msg` clears. Chosen for UX, not technical constraint.
        const SUMMARY_TTL: u8 = 3;

        // Any keypress dismisses the previous transient status message.
        // When it clears with unseen log entries the summary becomes visible —
        // arm its countdown. On later keypresses tick it down and auto-dismiss
        // at zero. The countdown only runs when the minibuffer is closed so
        // typing a long `:` command doesn't burn the budget invisibly.
        let had_status = self.state.status_msg.take().is_some();
        if self.state.minibuf.is_none() {
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

        // Popup dismissal/scroll, before mode dispatch — see `PopupKind`.
        // `Scrollable` (scrollable hover, `gn`/`gp`'s diagnostic overlay)
        // consumes Ctrl+u/Ctrl+d to scroll when there's actually content past
        // one screenful; otherwise (and for any other key) it closes the
        // popup and falls through to normal dispatch this same call, so a
        // short popup never blocks buffer half-page scroll.
        match self.state.config.popup.as_ref().map(|p| p.kind) {
            Some(PopupKind::Scrollable) => {
                let ctrl = key.modifiers.contains(Modifiers::CONTROL);
                match key.code {
                    KeyCode::Char('d') if ctrl => {
                        if self.scroll_popup(true) {
                            return;
                        }
                        self.state.config.popup = None;
                    }
                    KeyCode::Char('u') if ctrl => {
                        if self.scroll_popup(false) {
                            return;
                        }
                        self.state.config.popup = None;
                    }
                    _ => self.state.config.popup = None,
                }
            }
            Some(PopupKind::Sticky) | None => {}
        }

        // ── Confirm intercept ──────────────────────────────────────────────
        // Ahead of every other intercept. `check_buffer_disk_state` gates
        // when one can open — never from Insert/Search/Select, and from
        // Command only as the direct result of a fully-submitted `:`
        // command (`:e`/`:b`/`:bn`/`:bp`/`:checktime`) — so a confirm never
        // steals a keystroke from an Insert session or a half-typed
        // command/search line. No mode check is needed here regardless:
        // once a confirm is open, every key routes here first, so nothing
        // can change mode out from under it before it's answered. Always
        // fully consumes, like the picker.
        let confirm_consumed = self.state.config.confirm.is_some();
        if confirm_consumed {
            self.handle_confirm_key(key);
        }

        // ── Picker intercept ──────────────────────────────────────────────
        // Sits above the menu/drawer intercepts and is mode-agnostic (Q-B7,
        // `docs/FUZZY-FINDERS.md`: the picker opens from any mode, unlike
        // the menu/drawer's Normal/Extend-only gate) — key ownership mirrors
        // the picker's top z-order registration (`ui/mod.rs`'s `build_pane`),
        // the most action-relevant surface when more than one could be
        // visible. Full-modal: `handle_picker_key` always consumes, so while
        // a picker is open `handle_insert`'s own completion intercept never
        // runs — no conflict between the two.
        let picker_consumed = !confirm_consumed
            && self.state.config.picker.is_some()
            && self.handle_picker_key(key);

        // ── Selection menu intercept ─────────────────────────────────────
        // Guarded early-return before mode dispatch, not a new `Mode` — a
        // menu is transient chrome, not an editing mode (no `on-mode-change`,
        // no statusline/cursor-shape changes). Normal/Extend only: menus
        // don't open from Insert in v1.
        let menu_consumed = !confirm_consumed
            && !picker_consumed
            && self.state.config.menu.is_some()
            && matches!(self.state.mode(), Mode::Normal | Mode::Extend)
            && self.handle_menu_key(key);

        // ── Bottom drawer intercept ──────────────────────────────────────
        // Same guarded-early-return shape as the menu's, but unlike the menu
        // a stray key neither closes the drawer nor invokes its callback —
        // it falls through untouched, leaving the drawer open while focus
        // stays on the pane (Helix-style browse-while-editing).
        let drawer_consumed = !confirm_consumed
            && !picker_consumed
            && !menu_consumed
            && self.state.config.drawer.is_some()
            && matches!(self.state.mode(), Mode::Normal | Mode::Extend)
            && self.handle_drawer_key(key);

        if !confirm_consumed && !picker_consumed && !menu_consumed && !drawer_consumed {
            match self.state.mode() {
                Mode::Normal | Mode::Extend => self.handle_normal(key),
                Mode::Insert => self.handle_insert(key),
                Mode::Command => self.handle_command(key),
                Mode::Search => self.handle_search(key),
                Mode::Select => self.handle_select(key),
            }
        }

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

        // Any mode change this dispatch made (`set_mode`'s Insert-exit arm)
        // dismisses a completion session synchronously, before this
        // function returns — same timing tests already assert on.
        self.take_pending_lsp_completion_dismiss();
    }
}

// ── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests;
