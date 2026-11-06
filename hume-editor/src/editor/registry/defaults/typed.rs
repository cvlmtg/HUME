use super::builder::typed;
use crate::editor::commands::*;
use crate::editor::completion;
use crate::editor::jump_list::JumpRule;
use crate::editor::registry::{CommandRegistry, NativeTyped};

impl CommandRegistry {
    pub(super) fn register_typed_commands(&mut self) {
        // ── Typed commands (`:` command line) ─────────────────────────────────
        typed(
            "quit",
            "Close the editor.",
            &["q"],
            NativeTyped::Force(typed_quit),
        )
        .reg(self);
        typed(
            "quit-all",
            "Quit the editor, closing all buffers.",
            &["qa"],
            NativeTyped::Force(typed_quit_all),
        )
        .jump(JumpRule::Selections)
        .reg(self);
        typed(
            "write",
            "Write changes to disk.",
            &["w"],
            NativeTyped::ArgForce(typed_write),
        )
        .completer(completion::PATH_SOURCE)
        .reg(self);
        typed(
            "write-quit",
            "Write changes and quit.",
            &["wq"],
            NativeTyped::ArgForce(typed_write_quit),
        )
        .completer(completion::PATH_SOURCE)
        .reg(self);
        typed(
            "write-all",
            "Write all modified buffers to disk.",
            &["wa"],
            NativeTyped::Force(typed_write_all),
        )
        .reg(self);
        typed(
            "toggle-soft-wrap",
            "Toggle soft line wrapping.",
            &["wrap"],
            NativeTyped::Bare(typed_toggle_soft_wrap),
        )
        .reg(self);
        typed(
            "set",
            "Set a configuration value: :set global|buffer|pane key=value.",
            &[],
            NativeTyped::ArgRequired(typed_set, "global|buffer|pane key=value"),
        )
        .completer(completion::SET_SOURCE)
        .reg(self);
        typed(
            "get",
            "Show a configuration value: :get global|buffer|pane key.",
            &[],
            NativeTyped::ArgRequired(typed_get, "global|buffer|pane key"),
        )
        .completer(completion::GET_SOURCE)
        .reg(self);
        typed(
            "messages",
            "Show the message log in a read-only scratch buffer.",
            &["mes"],
            NativeTyped::Bare(typed_messages),
        )
        .reg(self);
        typed(
            "reload-config",
            "Reload init.scm from scratch.",
            &[],
            NativeTyped::Bare(crate::editor::reload::typed_reload_config),
        )
        .reg(self);
        typed(
            "save-session",
            "Save the session now; quitting saves it again.",
            &[],
            NativeTyped::Bare(crate::editor::session::typed_save_session),
        )
        .reg(self);
        typed(
            "edit",
            "Open a file or reload current file.",
            &["e"],
            NativeTyped::ArgOrForce(typed_edit),
        )
        .completer(completion::PATH_SOURCE)
        .jump(JumpRule::Selections)
        .reg(self);
        typed(
            "checktime",
            "Check every open buffer against its file on disk, right now.",
            &[],
            NativeTyped::Bare(typed_checktime),
        )
        .reg(self);
        typed(
            "buffer-delete",
            "Close the focused buffer.",
            &["bd"],
            NativeTyped::Force(typed_buffer_delete),
        )
        .reg(self);
        typed(
            "bnext",
            "Switch to next buffer in open-order.",
            &["bn"],
            NativeTyped::Bare(typed_bnext),
        )
        .jump(JumpRule::Selections)
        .reg(self);
        typed(
            "bprev",
            "Switch to previous buffer in open-order.",
            &["bp"],
            NativeTyped::Bare(typed_bprev),
        )
        .jump(JumpRule::Selections)
        .reg(self);
        typed(
            "new",
            "Open a new empty scratch buffer.",
            &["n"],
            NativeTyped::Bare(typed_new),
        )
        .jump(JumpRule::Selections)
        .reg(self);
        typed(
            "split",
            "Split the focused pane, stacking the new pane below it.",
            &["sp"],
            NativeTyped::Arg(typed_split),
        )
        .reg(self);
        typed(
            "vsplit",
            "Split the focused pane side by side.",
            &["vsp"],
            NativeTyped::Arg(typed_vsplit),
        )
        .reg(self);
        typed(
            "tabnew",
            "Open a new tab.",
            &["tabe"],
            NativeTyped::Arg(typed_tabnew),
        )
        .completer(completion::PATH_SOURCE)
        .reg(self);
        typed(
            "tabclose",
            "Close the current tab and every pane it owns.",
            &["tabc"],
            NativeTyped::Force(typed_tabclose),
        )
        .reg(self);
        typed(
            "tabnext",
            "Switch to the next tab in display order.",
            &["tabn"],
            NativeTyped::Bare(typed_tabnext),
        )
        .reg(self);
        typed(
            "tabprev",
            "Switch to the previous tab in display order.",
            &["tabp"],
            NativeTyped::Bare(typed_tabprev),
        )
        .reg(self);
        typed(
            "theme",
            "Load a theme by name: :theme <name>. No arg shows current theme.",
            &[],
            NativeTyped::Arg(typed_theme),
        )
        .completer(completion::THEME_SOURCE)
        .reg(self);
        typed(
            "theme-debug",
            "Show resolved styles for key UI scopes of the active theme.",
            &[],
            NativeTyped::Bare(typed_theme_debug),
        )
        .reg(self);
        typed(
            "change-directory",
            "Change the working directory.",
            &["cd"],
            NativeTyped::Arg(typed_cd),
        )
        .completer(completion::PATH_DIRS_ONLY_SOURCE)
        .reg(self);
        typed(
            "print-working-directory",
            "Print the current working directory.",
            &["pwd"],
            NativeTyped::Bare(typed_pwd),
        )
        .reg(self);
        typed(
            "list-buffers",
            "List all open buffers.",
            &["ls"],
            NativeTyped::Bare(typed_list_buffers),
        )
        .reg(self);
        typed(
            "plugin-status",
            "Show declared plugins and their load state.",
            &["plugins"],
            NativeTyped::Bare(typed_plugin_status),
        )
        .reg(self);
        // `#` is the alternate-buffer specifier itself, not a filename token:
        // expanding it to a path would lose pathless alternates (scratch,
        // [messages], [plugin-status]).
        typed(
            "buffer",
            "Switch to an open buffer.",
            &["b"],
            NativeTyped::ArgRequired(typed_buffer, "<name|#|index>"),
        )
        .completer(completion::BUFFER_NAME_SOURCE)
        .verbatim()
        .jump(JumpRule::Selections)
        .reg(self);
        typed(
            "version",
            "Print the editor version.",
            &["ver"],
            NativeTyped::Bare(typed_version),
        )
        .reg(self);
        typed(
            "tutor",
            "Open the interactive tutorial.",
            &[],
            NativeTyped::Bare(typed_tutor),
        )
        .jump(JumpRule::Selections)
        .reg(self);
        typed(
            "goto",
            "Jump to a 1-based line number: :goto 42.",
            &[],
            NativeTyped::ArgRequired(typed_goto_line, "<line>"),
        )
        .jump(JumpRule::Selections)
        .reg(self);
        typed(
            "sort",
            "Sort adjacent rows by their selected text. Flags: -r, -i.",
            &[],
            NativeTyped::Arg(typed_sort),
        )
        .reg(self);
        typed(
            "earlier",
            "Step back N revisions (:earlier 3) or to the state as of an age ago (:earlier 5m).",
            &[],
            NativeTyped::Arg(typed_earlier),
        )
        .reg(self);
        typed(
            "later",
            "Step forward N revisions (:later 3) or to the state as of an age ago (:later 5m).",
            &[],
            NativeTyped::Arg(typed_later),
        )
        .reg(self);
    }
}
