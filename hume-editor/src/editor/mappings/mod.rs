use crossterm::event::{KeyCode, KeyEvent};

use super::{Editor, Mode};

pub(super) mod command_mode;
mod execute;
mod insert;
mod lazy;
mod normal;
mod search_mode;
mod select_mode;

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

        // ── Selection menu intercept (U5) ─────────────────────────────────────
        // Guarded early-return before mode dispatch, not a new `Mode` — a
        // menu is transient chrome, not an editing mode (no `on-mode-change`,
        // no statusline/cursor-shape changes). Normal/Extend only: menus
        // don't open from Insert in v1.
        let menu_consumed = self.state.menu.is_some()
            && matches!(self.state.mode(), Mode::Normal | Mode::Extend)
            && self.handle_menu_key(key);

        if !menu_consumed {
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
    }

    /// Handles one key while a selection menu (U5) is open. Returns `true`
    /// if the key was fully consumed (movement, `Enter`, `Esc`) — `false` if
    /// a stray key dismissed the menu but should still fall through to
    /// normal dispatch this same call: a stray key both closes the menu
    /// (with a `#f` callback) *and* executes its usual effect.
    ///
    /// The callback fires exactly once (one-shot `.take()` discipline) —
    /// `queue_steel_call` never invokes it inline, matching every other
    /// Rust→Steel callback in this codebase.
    fn handle_menu_key(&mut self, key: KeyEvent) -> bool {
        let Some(menu) = self.state.menu.as_mut() else {
            return false;
        };
        match key.code {
            KeyCode::Char('j') | KeyCode::Down => {
                if menu.selected + 1 < menu.items.len() {
                    menu.selected += 1;
                }
                true
            }
            KeyCode::Char('k') | KeyCode::Up => {
                menu.selected = menu.selected.saturating_sub(1);
                true
            }
            KeyCode::Enter => {
                let menu = self.state.menu.take().expect("checked by the caller above");
                let idx = steel::rvals::SteelVal::IntV(menu.selected as isize);
                self.queue_steel_call(menu.callback, vec![idx]);
                true
            }
            KeyCode::Esc => {
                let menu = self.state.menu.take().expect("checked by the caller above");
                self.queue_steel_call(menu.callback, vec![steel::rvals::SteelVal::BoolV(false)]);
                true
            }
            _ => {
                let menu = self.state.menu.take().expect("checked by the caller above");
                self.queue_steel_call(menu.callback, vec![steel::rvals::SteelVal::BoolV(false)]);
                false
            }
        }
    }
}

// ── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    /// Guard: every jump command has `meta().is_jump == true` in the registry.
    ///
    /// The registry is the single source of truth — there is no separate
    /// `JUMP_COMMANDS` list to keep in sync.
    #[test]
    fn jump_and_visual_move_flags_are_correct() {
        let reg = super::super::registry::CommandRegistry::with_defaults();

        let must_be_jump = [
            "goto-first-line",
            "goto-last-line",
            "search-next",
            "search-prev",
            "page-down",
            "page-up",
            "select-all",
        ];
        for name in must_be_jump {
            assert!(
                reg.get_mappable(name).expect(name).meta().is_jump,
                "'{name}' should have jump: true"
            );
        }

        let must_be_visual_move = ["move-down", "move-up"];
        for name in must_be_visual_move {
            assert!(
                reg.get_mappable(name).expect(name).meta().is_visual_move,
                "'{name}' should have visual_move: true"
            );
        }

        // Spot-check non-jump commands.
        for name in ["move-left", "move-right", "delete", "undo", "insert-before"] {
            assert!(
                !reg.get_mappable(name).expect(name).meta().is_jump,
                "'{name}' should have jump: false"
            );
            assert!(
                !reg.get_mappable(name).expect(name).meta().is_visual_move,
                "'{name}' should have visual_move: false"
            );
        }
    }

    /// The message-log summary auto-dismisses after exactly 3 keystrokes of visibility
    /// (`SUMMARY_TTL = 3`).
    ///
    /// Timeline:
    ///   - report() → status_msg set, summary hidden behind it
    ///   - key 1 → status_msg cleared, summary appears, TTL armed (3)
    ///   - key 2 → TTL ticks 3→2, summary still visible
    ///   - key 3 → TTL ticks 2→1, summary still visible
    ///   - key 4 → TTL ticks 1→0 → mark_all_seen() fires, summary gone
    #[test]
    fn message_log_summary_ttl() {
        use super::super::{Editor, Severity};
        use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

        let noop = KeyEvent::new(KeyCode::Char('h'), KeyModifiers::NONE);

        let (buf, sels) = crate::testing::parse_state("-[a]>\n");
        let mut ed = Editor::for_testing(crate::editor::buffer::Buffer::new(buf, sels));

        // report() sets status_msg AND logs to message_log.
        ed.report(Severity::Error, "boom".to_string());
        assert!(ed.state.status_msg.is_some());
        assert!(ed.state.message_log.has_unseen());

        // Key 1: status_msg clears, TTL armed — summary visible.
        ed.handle_key(noop);
        assert!(ed.state.status_msg.is_none());
        assert!(
            ed.state.message_log.has_unseen(),
            "summary should still be visible after key 1"
        );

        // Key 2: TTL ticks 3→2 — summary still visible.
        ed.handle_key(noop);
        assert!(
            ed.state.message_log.has_unseen(),
            "summary should still be visible after key 2"
        );

        // Key 3: TTL ticks 2→1 — summary still visible.
        ed.handle_key(noop);
        assert!(
            ed.state.message_log.has_unseen(),
            "summary should still be visible after key 3"
        );

        // Key 4: TTL ticks 1→0 → auto-dismissed.
        ed.handle_key(noop);
        assert!(
            !ed.state.message_log.has_unseen(),
            "summary should be gone after key 4"
        );
    }

    #[test]
    fn parse_typed_command_table() {
        use super::command_mode::parse_typed_command;
        let cases: &[(&str, &str, bool, Option<&str>)] = &[
            ("", "", false, None),                         // empty
            ("!", "", true, None),                         // lone bang
            ("e", "e", false, None),                       // bare command
            ("e!", "e", true, None),                       // force, no arg
            ("e!path", "e", true, Some("path")),           // force adjacent to arg
            ("e foo", "e", false, Some("foo")),            // space-separated arg
            ("e   foo  ", "e", false, Some("foo")),        // arg trimming
            ("list-buffers", "list-buffers", false, None), // hyphenated name
            ("b#", "b", false, Some("#")),                 // non-alpha arg
            ("b#alt", "b", false, Some("#alt")),           // alternate-buffer style
        ];
        for &(input, cmd, force, arg) in cases {
            let (got_cmd, got_force, got_arg) = parse_typed_command(input);
            assert_eq!(
                (got_cmd, got_force, got_arg),
                (cmd, force, arg),
                "parse_typed_command({input:?})"
            );
        }
    }
}
