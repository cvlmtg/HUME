use crossterm::event::KeyEvent;

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
        // Any keypress dismisses the previous transient status message.
        self.state.status_msg = None;

        match self.state.mode() {
            Mode::Normal | Mode::Extend => self.handle_normal(key),
            Mode::Insert => self.handle_insert(key),
            Mode::Command => self.handle_command(key),
            Mode::Search => self.handle_search(key),
            Mode::Select => self.handle_select(key),
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
        self.drain_pending_repeat();

    }
}

// ── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    /// Guard: every jump command has `is_jump() == true` in the registry.
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
        ];
        for name in must_be_jump {
            assert!(
                reg.get_mappable(name).expect(name).is_jump(),
                "'{name}' should have jump: true"
            );
        }

        let must_be_visual_move = ["move-down", "move-up"];
        for name in must_be_visual_move {
            assert!(
                reg.get_mappable(name).expect(name).is_visual_move(),
                "'{name}' should have visual_move: true"
            );
        }

        // Spot-check non-jump commands.
        for name in ["move-left", "move-right", "delete", "undo", "insert-before"] {
            assert!(
                !reg.get_mappable(name).expect(name).is_jump(),
                "'{name}' should have jump: false"
            );
            assert!(
                !reg.get_mappable(name).expect(name).is_visual_move(),
                "'{name}' should have visual_move: false"
            );
        }
    }

    #[test]
    fn parse_typed_command_table() {
        use super::command_mode::parse_typed_command;
        let cases: &[(&str, &str, bool, Option<&str>)] = &[
            ("",              "",             false, None),           // empty
            ("!",             "",             true,  None),           // lone bang
            ("e",             "e",            false, None),           // bare command
            ("e!",            "e",            true,  None),           // force, no arg
            ("e!path",        "e",            true,  Some("path")),   // force adjacent to arg
            ("e foo",         "e",            false, Some("foo")),    // space-separated arg
            ("e   foo  ",     "e",            false, Some("foo")),    // arg trimming
            ("list-buffers",  "list-buffers", false, None),           // hyphenated name
            ("b#",            "b",            false, Some("#")),      // non-alpha arg
            ("b#alt",         "b",            false, Some("#alt")),   // alternate-buffer style
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
