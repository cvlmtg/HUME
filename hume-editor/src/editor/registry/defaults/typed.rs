use std::borrow::Cow;

use crate::editor::commands::*;
use crate::editor::registry::{ArgCompleter, CommandRegistry, TypedCommand};

impl CommandRegistry {
    pub(super) fn register_typed_commands(&mut self) {
        // ── Typed commands (`:` command line) ─────────────────────────────────
        macro_rules! typed_cmd {
            ($name:literal, $doc:literal, $aliases:expr, $fun:expr) => {
                typed_cmd!($name, $doc, $aliases, $fun, completer: None)
            };
            ($name:literal, $doc:literal, $aliases:expr, $fun:expr, completer: $completer:expr) => {
                self.register_typed(TypedCommand {
                    name: Cow::Borrowed($name),
                    doc: Cow::Borrowed($doc),
                    aliases: $aliases,
                    fun: $fun,
                    completer: $completer,
                })
            };
        }

        typed_cmd!("quit", "Close the editor.", &["q"], typed_quit);
        typed_cmd!(
            "quit-all",
            "Quit the editor, closing all buffers.",
            &["qa"],
            typed_quit_all
        );
        typed_cmd!(
            "write",
            "Write changes to disk.",
            &["w"],
            typed_write,
            completer: Some(ArgCompleter::Path { dirs_only: false })
        );
        typed_cmd!(
            "write-quit",
            "Write changes and quit.",
            &["wq"],
            typed_write_quit,
            completer: Some(ArgCompleter::Path { dirs_only: false })
        );
        typed_cmd!(
            "write-all",
            "Write all modified buffers to disk.",
            &["wa"],
            typed_write_all
        );
        typed_cmd!(
            "toggle-soft-wrap",
            "Toggle soft line wrapping.",
            &["wrap"],
            typed_toggle_soft_wrap
        );
        typed_cmd!(
            "set",
            "Set a configuration value: :set global|buffer|pane key=value.",
            &[],
            typed_set,
            completer: Some(ArgCompleter::Set)
        );
        typed_cmd!(
            "messages",
            "Show the message log in a read-only scratch buffer.",
            &["mes"],
            typed_messages
        );
        typed_cmd!(
            "reload-config",
            "Reload init.scm from scratch.",
            &[],
            crate::editor::reload::typed_reload_config
        );
        typed_cmd!(
            "edit",
            "Open a file or reload current file.",
            &["e"],
            typed_edit,
            completer: Some(ArgCompleter::Path { dirs_only: false })
        );
        typed_cmd!(
            "checktime",
            "Check every open buffer against its file on disk, right now.",
            &[],
            typed_checktime
        );
        typed_cmd!(
            "buffer-delete",
            "Close the focused buffer.",
            &["bd"],
            typed_buffer_delete
        );
        typed_cmd!(
            "bnext",
            "Switch to next buffer in open-order.",
            &["bn"],
            typed_bnext
        );
        typed_cmd!(
            "bprev",
            "Switch to previous buffer in open-order.",
            &["bp"],
            typed_bprev
        );
        typed_cmd!(
            "split",
            "Split the focused pane, stacking the new pane below it.",
            &["sp"],
            typed_split
        );
        typed_cmd!(
            "vsplit",
            "Split the focused pane side by side.",
            &["vsp"],
            typed_vsplit
        );
        typed_cmd!(
            "theme",
            "Load a theme by name: :theme <name>. No arg shows current theme.",
            &[],
            typed_theme,
            completer: Some(ArgCompleter::Theme)
        );
        typed_cmd!(
            "theme-debug",
            "Show resolved styles for key UI scopes of the active theme.",
            &[],
            typed_theme_debug
        );
        typed_cmd!(
            "change-directory",
            "Change the working directory.",
            &["cd"],
            typed_cd,
            completer: Some(ArgCompleter::Path { dirs_only: true })
        );
        typed_cmd!(
            "print-working-directory",
            "Print the current working directory.",
            &["pwd"],
            typed_pwd
        );
        typed_cmd!(
            "list-buffers",
            "List all open buffers.",
            &["ls"],
            typed_list_buffers
        );
        typed_cmd!(
            "plugin-status",
            "Show declared plugins and their load state.",
            &["plugins"],
            typed_plugin_status
        );
        typed_cmd!(
            "buffer",
            "Switch to an open buffer.",
            &["b"],
            typed_buffer,
            completer: Some(ArgCompleter::Buffer)
        );
        typed_cmd!(
            "version",
            "Print the editor version.",
            &["ver"],
            typed_version
        );
        typed_cmd!("tutor", "Open the interactive tutorial.", &[], typed_tutor);
        typed_cmd!(
            "goto",
            "Jump to a 1-based line number: :goto 42.",
            &[],
            typed_goto_line
        );
        typed_cmd!(
            "sort",
            "Sort adjacent rows by their selected text. Flags: -r, -i.",
            &[],
            typed_sort
        );
    }
}
