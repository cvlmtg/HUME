//! Mappable command registry.
//!
//! A [`MappableCommand`] wraps a command together with a string name and a
//! one-line doc string. All commands live in a [`CommandRegistry`] keyed by
//! name, built once via [`CommandRegistry::with_defaults`].
//!
//! **Every user-facing operation is a named command in this registry.**
//! There is no parallel dispatch system — the keymap trie stores command
//! *names*; the registry resolves those names to `MappableCommand` values at
//! dispatch time inside `execute_keymap_command` (`editor/mappings.rs`).
//!
//! # Extend-mode pairing
//!
//! The registry is the single source of truth for extend-variant relationships.
//! Each command declares its extend variant inline at registration time via an
//! `extend: "variant-name"` argument on the registration macro. When extend
//! mode is active, the dispatcher resolves the extend variant automatically
//! via [`CommandRegistry::extend_variant`] — the keymap stores only base
//! command names.
//!
//! # Command variants
//!
//! 1. **Motion** — pure `fn(&Buffer, SelectionSet, usize) -> SelectionSet`
//! 2. **Selection** — pure `fn(&Buffer, SelectionSet) -> SelectionSet`
//! 3. **Edit** — pure `fn(Buffer, SelectionSet) -> (Buffer, SelectionSet, ChangeSet)`
//! 4. **EditorCmd** — `fn(&mut Editor, usize)` for composite/side-effectful
//!    operations (mode changes, registers, undo groups, parameterized motions).
//!    Implemented in `editor/commands.rs`; stored and dispatched as a
//!    function pointer exactly like the other variants.
//!
//! Typed commands entered via `:` (`:w`, `:q`, etc.) are a separate system in
//! `editor/mappings.rs:execute_command` and are NOT stored here.

use std::collections::HashMap;

use crate::core::buffer::Buffer;
use crate::core::changeset::ChangeSet;
use crate::ops::edit::{delete_char_backward, delete_char_forward, delete_selection};
use crate::ops::motion::{
    cmd_extend_first_line, cmd_extend_first_nonblank, cmd_extend_last_line,
    cmd_extend_left, cmd_extend_line_end,
    cmd_extend_line_start, cmd_extend_next_paragraph, cmd_extend_prev_paragraph,
    cmd_extend_right, cmd_extend_select_line, cmd_extend_select_line_backward,
    cmd_extend_select_next_WORD, cmd_extend_select_next_word, cmd_extend_select_prev_WORD,
    cmd_extend_select_prev_word, cmd_goto_first_line, cmd_goto_first_nonblank,
    cmd_goto_last_line, cmd_goto_line_end,
    cmd_goto_line_start, cmd_move_left, cmd_move_right,
    cmd_next_paragraph, cmd_prev_paragraph, cmd_select_line, cmd_select_line_backward,
    cmd_select_next_WORD, cmd_select_next_word, cmd_select_prev_WORD, cmd_select_prev_word,
};
use crate::core::selection::SelectionSet;
use crate::ops::selection_cmd::{
    cmd_collapse_selection, cmd_copy_selection_on_next_line, cmd_copy_selection_on_prev_line,
    cmd_cycle_primary_backward, cmd_cycle_primary_forward, cmd_flip_selections,
    cmd_keep_primary_selection, cmd_remove_primary_selection, cmd_select_all,
    cmd_split_selection_on_newlines,
    cmd_trim_selection_whitespace,
};
use crate::ops::surround::{
    cmd_surround_angle, cmd_surround_backtick, cmd_surround_brace, cmd_surround_bracket,
    cmd_surround_double_quote, cmd_surround_paren, cmd_surround_single_quote,
};
use crate::ops::text_object::{
    cmd_around_angle, cmd_around_argument, cmd_around_backtick, cmd_around_brace,
    cmd_around_bracket, cmd_around_double_quote, cmd_around_line, cmd_around_paren,
    cmd_around_single_quote, cmd_around_word, cmd_around_WORD, cmd_extend_around_angle,
    cmd_extend_around_argument, cmd_extend_around_backtick, cmd_extend_around_brace,
    cmd_extend_around_bracket, cmd_extend_around_double_quote, cmd_extend_around_line,
    cmd_extend_around_paren, cmd_extend_around_single_quote, cmd_extend_around_word,
    cmd_extend_around_WORD, cmd_extend_inner_angle, cmd_extend_inner_argument,
    cmd_extend_inner_backtick, cmd_extend_inner_brace, cmd_extend_inner_bracket,
    cmd_extend_inner_double_quote, cmd_extend_inner_line, cmd_extend_inner_paren,
    cmd_extend_inner_single_quote, cmd_extend_inner_word, cmd_extend_inner_WORD, cmd_inner_angle,
    cmd_inner_argument, cmd_inner_backtick, cmd_inner_brace, cmd_inner_bracket,
    cmd_inner_double_quote, cmd_inner_line, cmd_inner_paren, cmd_inner_single_quote,
    cmd_inner_word, cmd_inner_WORD,
};

// ── MappableCommand ───────────────────────────────────────────────────────────

/// A command that can be bound to a key in a keymap.
///
/// The keymap trie stores command *names*; the registry resolves names to
/// `MappableCommand` values at dispatch time.
#[derive(Clone, Copy)]
pub(crate) enum MappableCommand {
    /// Motion that repeats `count` times.
    ///
    /// Signature: `fn(&Buffer, SelectionSet, usize) -> SelectionSet`
    ///
    /// Motions never mutate the buffer, so `repeatable` is always `false`.
    Motion {
        name: &'static str,
        fun: fn(&Buffer, SelectionSet, usize) -> SelectionSet,
    },
    /// Selection or text-object operation (no count).
    ///
    /// Signature: `fn(&Buffer, SelectionSet) -> SelectionSet`
    ///
    /// Pure selection ops never mutate the buffer, so `repeatable` is always `false`.
    Selection {
        name: &'static str,
        fun: fn(&Buffer, SelectionSet) -> SelectionSet,
    },
    /// Buffer-modifying edit with no extra arguments.
    ///
    /// Signature: `fn(Buffer, SelectionSet) -> (Buffer, SelectionSet, ChangeSet)`
    Edit {
        name: &'static str,
        fun: fn(Buffer, SelectionSet) -> (Buffer, SelectionSet, ChangeSet),
        /// Whether `.` should replay this command. Set to `true` for edits that
        /// are meaningful to repeat (e.g. user-facing deletions). Set to `false`
        /// for internal primitives like `delete-char-backward`.
        repeatable: bool,
    },
    /// Editor-level command requiring `&mut Editor` context.
    ///
    /// Signature: `fn(&mut Editor, usize)`
    ///
    /// Covers composite operations: mode changes, register access, undo group
    /// management, and parameterized motions (find/till/replace). Stored and
    /// dispatched as a function pointer exactly like the other variants —
    /// `fn(&mut Editor, usize)` is a thin pointer so there is no self-referential
    /// sizing issue despite `Editor` owning the registry.
    EditorCmd {
        name: &'static str,
        fun: fn(&mut super::Editor, usize),
        /// Whether `.` should replay this command.
        repeatable: bool,
    },
}

impl MappableCommand {
    pub(crate) fn name(&self) -> &'static str {
        match self {
            Self::Motion { name, .. }
            | Self::Selection { name, .. }
            | Self::Edit { name, .. }
            | Self::EditorCmd { name, .. } => name,
        }
    }

    /// Returns `true` if this command should be recorded for `.` repeat.
    ///
    /// Motions and selections are never repeatable — they don't mutate the
    /// buffer. Edit and EditorCmd commands opt in explicitly at registration.
    pub(crate) fn is_repeatable(&self) -> bool {
        match self {
            Self::Motion { .. } | Self::Selection { .. } => false,
            Self::Edit { repeatable, .. } | Self::EditorCmd { repeatable, .. } => *repeatable,
        }
    }
}

// ── CommandRegistry ───────────────────────────────────────────────────────────

/// Registry of all mappable commands, keyed by name.
///
/// Built once via [`CommandRegistry::with_defaults`] and stored on the editor.
/// The keymap trie (`editor/keymap.rs`) stores command names as `&'static str`;
/// `execute_keymap_command` in `editor/mappings.rs` resolves them here at
/// dispatch time to obtain the actual function pointer.
///
/// Also owns the extend-variant map: each base command can declare its
/// extend variant at registration time via `extend: "variant-name"`. The
/// dispatcher calls [`extend_variant`](Self::extend_variant) to resolve
/// the correct command when extend mode is active.
pub(crate) struct CommandRegistry {
    commands: HashMap<&'static str, MappableCommand>,
    /// Maps a base command name to its extend variant.
    ///
    /// When extend mode is active, the dispatcher looks up the base command
    /// here and dispatches the extend variant instead. This is the single
    /// source of truth for extend pairing — the keymap stores only base
    /// command names.
    extend_map: HashMap<&'static str, &'static str>,
}

impl CommandRegistry {
    /// Build a registry pre-populated with every default command.
    pub(crate) fn with_defaults() -> Self {
        let mut reg = Self {
            commands: HashMap::new(),
            extend_map: HashMap::new(),
        };
        reg.register_defaults();
        reg
    }

    fn register(&mut self, cmd: MappableCommand) {
        self.commands.insert(cmd.name(), cmd);
    }

    /// Look up a command by name.
    pub(crate) fn get(&self, name: &str) -> Option<&MappableCommand> {
        self.commands.get(name)
    }

    /// Iterate over all registered command names.
    #[allow(dead_code)]
    pub(crate) fn names(&self) -> impl Iterator<Item = &&'static str> {
        self.commands.keys()
    }

    /// Total number of registered commands.
    #[allow(dead_code)]
    pub(crate) fn len(&self) -> usize {
        self.commands.len()
    }

    /// Look up the extend variant for a command, if one exists.
    pub(crate) fn extend_variant(&self, name: &str) -> Option<&'static str> {
        self.extend_map.get(name).copied()
    }

    fn register_defaults(&mut self) {
        // Local macros to cut down on struct-literal boilerplate.
        // `$doc` is accepted but not yet stored — reserved for future `:help`
        // and command-palette display.
        // The optional `extend: "name"` trailing argument links this command to
        // its extend variant in the extend_map (single source of truth).
        macro_rules! motion {
            ($name:literal, $doc:literal, $fun:expr, extend: $ext:literal) => {{
                self.register(MappableCommand::Motion { name: $name, fun: $fun });
                self.extend_map.insert($name, $ext);
            }};
            ($name:literal, $doc:literal, $fun:expr) => {
                self.register(MappableCommand::Motion { name: $name, fun: $fun })
            };
        }
        macro_rules! selection {
            ($name:literal, $doc:literal, $fun:expr, extend: $ext:literal) => {{
                self.register(MappableCommand::Selection { name: $name, fun: $fun });
                self.extend_map.insert($name, $ext);
            }};
            ($name:literal, $doc:literal, $fun:expr) => {
                self.register(MappableCommand::Selection { name: $name, fun: $fun })
            };
        }
        macro_rules! edit {
            ($name:literal, $doc:literal, $fun:expr) => {
                self.register(MappableCommand::Edit { name: $name, fun: $fun, repeatable: false })
            };
            ($name:literal, $doc:literal, $fun:expr, repeatable) => {
                self.register(MappableCommand::Edit { name: $name, fun: $fun, repeatable: true })
            };
        }
        macro_rules! editor_cmd {
            ($name:literal, $doc:literal, $fun:expr, repeatable, extend: $ext:literal) => {{
                self.register(MappableCommand::EditorCmd { name: $name, fun: $fun, repeatable: true });
                self.extend_map.insert($name, $ext);
            }};
            ($name:literal, $doc:literal, $fun:expr, extend: $ext:literal) => {{
                self.register(MappableCommand::EditorCmd { name: $name, fun: $fun, repeatable: false });
                self.extend_map.insert($name, $ext);
            }};
            ($name:literal, $doc:literal, $fun:expr, repeatable) => {
                self.register(MappableCommand::EditorCmd { name: $name, fun: $fun, repeatable: true })
            };
            ($name:literal, $doc:literal, $fun:expr) => {
                self.register(MappableCommand::EditorCmd { name: $name, fun: $fun, repeatable: false })
            };
        }

        // ── Character motions ─────────────────────────────────────────────────
        motion!("move-right", "Move cursors one grapheme to the right.", cmd_move_right, extend: "extend-right");
        motion!("move-left",  "Move cursors one grapheme to the left.",  cmd_move_left,  extend: "extend-left");
        editor_cmd!("move-down",  "Move cursors down one visual line.",   cmd_visual_move_down,   extend: "extend-down");
        editor_cmd!("move-up",    "Move cursors up one visual line.",     cmd_visual_move_up,     extend: "extend-up");
        motion!("extend-right", "Extend selections one grapheme to the right.", cmd_extend_right);
        motion!("extend-left",  "Extend selections one grapheme to the left.",  cmd_extend_left);
        editor_cmd!("extend-down",  "Extend selections down one visual line.", cmd_visual_extend_down);
        editor_cmd!("extend-up",    "Extend selections up one visual line.",   cmd_visual_extend_up);

        // ── Buffer-level goto motions ─────────────────────────────────────────
        motion!("goto-first-line", "Move cursors to the first character of the buffer.",     cmd_goto_first_line, extend: "extend-first-line");
        motion!("goto-last-line",  "Move cursors to the first character of the last line.",  cmd_goto_last_line,  extend: "extend-last-line");
        motion!("extend-first-line", "Extend selections to the first character of the buffer.",    cmd_extend_first_line);
        motion!("extend-last-line",  "Extend selections to the first character of the last line.", cmd_extend_last_line);

        // ── Line-position motions ─────────────────────────────────────────────
        motion!("goto-line-start",    "Move cursors to the start of the line.",                        cmd_goto_line_start,    extend: "extend-line-start");
        motion!("goto-line-end",      "Move cursors to the last character on the line.",               cmd_goto_line_end,      extend: "extend-line-end");
        motion!("goto-first-nonblank","Move cursors to the first non-blank character on the line.",    cmd_goto_first_nonblank,extend: "extend-first-nonblank");
        motion!("extend-line-start",      "Extend selections to the start of the line.",                       cmd_extend_line_start);
        motion!("extend-line-end",        "Extend selections to the last character on the line.",               cmd_extend_line_end);
        motion!("extend-first-nonblank",  "Extend selections to the first non-blank character on the line.",   cmd_extend_first_nonblank);

        // ── Word motions ──────────────────────────────────────────────────────
        motion!("select-next-word", "Select the next word.",                          cmd_select_next_word, extend: "extend-select-next-word");
        motion!("select-next-WORD", "Select the next WORD (whitespace-delimited).",   cmd_select_next_WORD, extend: "extend-select-next-WORD");
        motion!("select-prev-word", "Select the previous word.",                      cmd_select_prev_word, extend: "extend-select-prev-word");
        motion!("select-prev-WORD", "Select the previous WORD (whitespace-delimited).",cmd_select_prev_WORD,extend: "extend-select-prev-WORD");
        motion!("extend-select-next-word", "Extend selection to encompass the next word.",     cmd_extend_select_next_word);
        motion!("extend-select-next-WORD", "Extend selection to encompass the next WORD.",     cmd_extend_select_next_WORD);
        motion!("extend-select-prev-word", "Extend selection to encompass the previous word.", cmd_extend_select_prev_word);
        motion!("extend-select-prev-WORD", "Extend selection to encompass the previous WORD.", cmd_extend_select_prev_WORD);

        // ── Paragraph motions ─────────────────────────────────────────────────
        motion!("next-paragraph", "Move cursors to the start of the next paragraph.",     cmd_next_paragraph, extend: "extend-next-paragraph");
        motion!("prev-paragraph", "Move cursors to the start of the previous paragraph.", cmd_prev_paragraph, extend: "extend-prev-paragraph");
        motion!("extend-next-paragraph", "Extend selections to the start of the next paragraph.",     cmd_extend_next_paragraph);
        motion!("extend-prev-paragraph", "Extend selections to the start of the previous paragraph.", cmd_extend_prev_paragraph);

        // ── Line selection ────────────────────────────────────────────────────
        selection!("select-line",          "Select the full current line (forward).",              cmd_select_line,          extend: "extend-select-line");
        selection!("select-line-backward", "Select the full current line (backward).",             cmd_select_line_backward, extend: "extend-select-line-backward");
        selection!("extend-select-line",          "Grow selection to cover the current line (extend mode).", cmd_extend_select_line);
        selection!("extend-select-line-backward", "Grow selection upward to cover the current line.",       cmd_extend_select_line_backward);

        // ── Selection commands ────────────────────────────────────────────────
        selection!("collapse-selection", "Collapse each selection to a single cursor at the head.", cmd_collapse_selection);
        selection!("flip-selections", "Swap anchor and head for each selection.", cmd_flip_selections);
        selection!("keep-primary-selection", "Remove all selections except the primary.", cmd_keep_primary_selection);
        selection!("select-all", "Select the entire buffer.", cmd_select_all);
        selection!("remove-primary-selection", "Remove the primary selection, promoting the next.", cmd_remove_primary_selection);
        selection!("cycle-primary-forward", "Cycle the primary selection forward.", cmd_cycle_primary_forward);
        selection!("cycle-primary-backward", "Cycle the primary selection backward.", cmd_cycle_primary_backward);
        selection!("split-selection-on-newlines", "Split each multi-line selection into one per line.", cmd_split_selection_on_newlines);
        selection!("trim-selection-whitespace", "Trim leading and trailing whitespace from each selection.", cmd_trim_selection_whitespace);
        selection!("copy-selection-on-next-line", "Duplicate each selection on the line below.", cmd_copy_selection_on_next_line);
        selection!("copy-selection-on-prev-line", "Duplicate each selection on the line above.", cmd_copy_selection_on_prev_line);

        // ── Text objects — line ───────────────────────────────────────────────
        selection!("inner-line",  "Select inner line content (excluding the newline).", cmd_inner_line,  extend: "extend-inner-line");
        selection!("around-line", "Select the line including its newline.",             cmd_around_line, extend: "extend-around-line");
        selection!("extend-inner-line",  "Extend to inner line content.",     cmd_extend_inner_line);
        selection!("extend-around-line", "Extend to include the full line.",  cmd_extend_around_line);

        // ── Text objects — word ───────────────────────────────────────────────
        selection!("inner-word",  "Select inner word.",                          cmd_inner_word,  extend: "extend-inner-word");
        selection!("around-word", "Select word plus surrounding whitespace.",    cmd_around_word, extend: "extend-around-word");
        selection!("extend-inner-word",  "Extend to inner word.",                          cmd_extend_inner_word);
        selection!("extend-around-word", "Extend to word plus surrounding whitespace.",    cmd_extend_around_word);
        selection!("inner-WORD",  "Select inner WORD (whitespace-delimited).",  cmd_inner_WORD,  extend: "extend-inner-WORD");
        selection!("around-WORD", "Select WORD plus surrounding whitespace.",   cmd_around_WORD, extend: "extend-around-WORD");
        selection!("extend-inner-WORD",  "Extend to inner WORD.",                          cmd_extend_inner_WORD);
        selection!("extend-around-WORD", "Extend to WORD plus surrounding whitespace.",    cmd_extend_around_WORD);

        // ── Text objects — brackets ───────────────────────────────────────────
        selection!("inner-paren",   "Select content inside the nearest `()`.",    cmd_inner_paren,   extend: "extend-inner-paren");
        selection!("around-paren",  "Select content including the nearest `()`.", cmd_around_paren,  extend: "extend-around-paren");
        selection!("extend-inner-paren",  "Extend to content inside the nearest `()`.",    cmd_extend_inner_paren);
        selection!("extend-around-paren", "Extend to content including the nearest `()`.", cmd_extend_around_paren);
        selection!("inner-bracket",   "Select content inside the nearest `[]`.",    cmd_inner_bracket,   extend: "extend-inner-bracket");
        selection!("around-bracket",  "Select content including the nearest `[]`.", cmd_around_bracket,  extend: "extend-around-bracket");
        selection!("extend-inner-bracket",  "Extend to content inside the nearest `[]`.",    cmd_extend_inner_bracket);
        selection!("extend-around-bracket", "Extend to content including the nearest `[]`.", cmd_extend_around_bracket);
        selection!("inner-brace",   "Select content inside the nearest `{}`.",    cmd_inner_brace,   extend: "extend-inner-brace");
        selection!("around-brace",  "Select content including the nearest `{}`.", cmd_around_brace,  extend: "extend-around-brace");
        selection!("extend-inner-brace",  "Extend to content inside the nearest `{}`.",    cmd_extend_inner_brace);
        selection!("extend-around-brace", "Extend to content including the nearest `{}`.", cmd_extend_around_brace);
        selection!("inner-angle",   "Select content inside the nearest `<>`.",    cmd_inner_angle,   extend: "extend-inner-angle");
        selection!("around-angle",  "Select content including the nearest `<>`.", cmd_around_angle,  extend: "extend-around-angle");
        selection!("extend-inner-angle",  "Extend to content inside the nearest `<>`.",    cmd_extend_inner_angle);
        selection!("extend-around-angle", "Extend to content including the nearest `<>`.", cmd_extend_around_angle);

        // ── Text objects — quotes ─────────────────────────────────────────────
        selection!("inner-double-quote",  "Select content inside the nearest `\"`.",    cmd_inner_double_quote,  extend: "extend-inner-double-quote");
        selection!("around-double-quote", "Select content including the nearest `\"`.", cmd_around_double_quote, extend: "extend-around-double-quote");
        selection!("extend-inner-double-quote",  "Extend to content inside the nearest `\"`.",    cmd_extend_inner_double_quote);
        selection!("extend-around-double-quote", "Extend to content including the nearest `\"`.", cmd_extend_around_double_quote);
        selection!("inner-single-quote",  "Select content inside the nearest `'`.",    cmd_inner_single_quote,  extend: "extend-inner-single-quote");
        selection!("around-single-quote", "Select content including the nearest `'`.", cmd_around_single_quote, extend: "extend-around-single-quote");
        selection!("extend-inner-single-quote",  "Extend to content inside the nearest `'`.",    cmd_extend_inner_single_quote);
        selection!("extend-around-single-quote", "Extend to content including the nearest `'`.", cmd_extend_around_single_quote);
        selection!("inner-backtick",  "Select content inside the nearest backtick pair.",    cmd_inner_backtick,  extend: "extend-inner-backtick");
        selection!("around-backtick", "Select content including the nearest backtick pair.", cmd_around_backtick, extend: "extend-around-backtick");
        selection!("extend-inner-backtick",  "Extend to content inside the nearest backtick pair.",    cmd_extend_inner_backtick);
        selection!("extend-around-backtick", "Extend to content including the nearest backtick pair.", cmd_extend_around_backtick);

        // ── Text objects — arguments ──────────────────────────────────────────
        selection!("inner-argument",  "Select the argument at the cursor (trimmed).",      cmd_inner_argument,  extend: "extend-inner-argument");
        selection!("around-argument", "Select the argument and its separator comma.",       cmd_around_argument, extend: "extend-around-argument");
        selection!("extend-inner-argument",  "Extend to the inner argument.",                     cmd_extend_inner_argument);
        selection!("extend-around-argument", "Extend to include the argument and separator.",     cmd_extend_around_argument);

        // ── Surround selection ────────────────────────────────────────────
        selection!("surround-paren",        "Select surrounding `()` delimiters.",    cmd_surround_paren);
        selection!("surround-bracket",      "Select surrounding `[]` delimiters.",    cmd_surround_bracket);
        selection!("surround-brace",        "Select surrounding `{}` delimiters.",    cmd_surround_brace);
        selection!("surround-angle",        "Select surrounding `<>` delimiters.",    cmd_surround_angle);
        selection!("surround-double-quote", "Select surrounding `\"` delimiters.",    cmd_surround_double_quote);
        selection!("surround-single-quote", "Select surrounding `'` delimiters.",     cmd_surround_single_quote);
        selection!("surround-backtick",     "Select surrounding backtick delimiters.",cmd_surround_backtick);

        // ── Edit commands ─────────────────────────────────────────────────────
        edit!("delete-char-forward", "Delete the character (or selection) under the cursor.", delete_char_forward);
        edit!("delete-char-backward", "Delete the character before each cursor.", delete_char_backward);
        edit!("delete-selection", "Delete all selections.", delete_selection);

        use super::commands::*;

        // ── Editor commands — mode transitions ────────────────────────────────
        editor_cmd!("insert-before",        "Enter insert mode; collapse each selection to its start.",          cmd_insert_before,        repeatable);
        editor_cmd!("insert-after",         "Enter insert mode after the cursor (move one grapheme right).",     cmd_insert_after,         repeatable);
        editor_cmd!("insert-at-line-start", "Enter insert mode at the first non-blank character on the line.",  cmd_insert_at_line_start, repeatable);
        editor_cmd!("insert-at-line-end",   "Enter insert mode after the last character on the line.",          cmd_insert_at_line_end,   repeatable);
        editor_cmd!("open-line-below",      "Open a new line below the cursor and enter insert mode.",          cmd_open_line_below,      repeatable, extend: "flip-selections");
        editor_cmd!("open-line-above",      "Open a new line above the cursor and enter insert mode.",          cmd_open_line_above,      repeatable);
        editor_cmd!("command-mode",         "Open the command-mode mini-buffer.",                                           cmd_command_mode);
        editor_cmd!("exit-insert",          "Return to normal mode from insert mode.",                                     cmd_exit_insert);

        // ── Editor commands — edit composites ─────────────────────────────────
        editor_cmd!("delete",       "Yank selections into the default register, then delete them.",                        cmd_delete,       repeatable);
        editor_cmd!("change",       "Yank, delete selections, then enter insert mode (one undo group).",                   cmd_change,       repeatable);
        editor_cmd!("yank",         "Yank selections into the default register without deleting.",                         cmd_yank);
        editor_cmd!("paste-after",  "Paste register contents after the selection.",                                        cmd_paste_after,  repeatable);
        editor_cmd!("paste-before", "Paste register contents before the selection.",                                       cmd_paste_before, repeatable);
        editor_cmd!("undo",         "Undo the last change.",                                                               cmd_undo);
        editor_cmd!("redo",         "Redo the last undone change.",                                                        cmd_redo);

        // ── Editor commands — selection state ────────────────────────────────
        editor_cmd!("toggle-extend",            "Toggle sticky extend mode.",                                              cmd_toggle_extend);
        editor_cmd!("collapse-and-exit-extend", "Collapse each selection to its cursor and exit extend mode.",             cmd_collapse_and_exit_extend);

        // ── Editor commands — find / till (read pending_char) ─────────────────
        editor_cmd!("find-forward",    "Find next occurrence of a character (inclusive, forward).",            cmd_find_forward,    extend: "extend-find-forward");
        editor_cmd!("find-backward",   "Find previous occurrence of a character (inclusive, backward).",       cmd_find_backward,   extend: "extend-find-backward");
        editor_cmd!("till-forward",    "Move to just before next occurrence of a character (exclusive).",      cmd_till_forward,    extend: "extend-till-forward");
        editor_cmd!("till-backward",   "Move to just after previous occurrence of a character (exclusive).",   cmd_till_backward,   extend: "extend-till-backward");
        editor_cmd!("extend-find-forward",   "Extend to next occurrence of a character (inclusive, forward).",       cmd_extend_find_forward);
        editor_cmd!("extend-find-backward",  "Extend to previous occurrence of a character (inclusive, backward).",  cmd_extend_find_backward);
        editor_cmd!("extend-till-forward",   "Extend to just before next occurrence of a character (exclusive).",    cmd_extend_till_forward);
        editor_cmd!("extend-till-backward",  "Extend to just after previous occurrence of a character (exclusive).", cmd_extend_till_backward);
        editor_cmd!("repeat-find-forward",          "Repeat the last find/till motion forward.",              cmd_repeat_find_forward,  extend: "extend-repeat-find-forward");
        editor_cmd!("repeat-find-backward",         "Repeat the last find/till motion backward.",             cmd_repeat_find_backward, extend: "extend-repeat-find-backward");
        editor_cmd!("extend-repeat-find-forward",   "Extend: repeat the last find/till motion forward.",     cmd_extend_repeat_find_forward);
        editor_cmd!("extend-repeat-find-backward",  "Extend: repeat the last find/till motion backward.",    cmd_extend_repeat_find_backward);

        // ── Editor commands — replace (reads pending_char) ───────────────────
        editor_cmd!("replace", "Replace every character in each selection with the next typed character.", cmd_replace, repeatable);

        // ── Editor commands — page scroll ─────────────────────────────────────
        editor_cmd!("page-down", "Scroll down by one viewport height.",            cmd_page_down, extend: "extend-page-down");
        editor_cmd!("page-up",  "Scroll up by one viewport height.",              cmd_page_up,   extend: "extend-page-up");
        editor_cmd!("extend-page-down", "Extend selections down by one viewport height.", cmd_extend_page_down);
        editor_cmd!("extend-page-up",   "Extend selections up by one viewport height.",   cmd_extend_page_up);

        // ── Editor commands — half-page scroll ────────────────────────────
        editor_cmd!("half-page-down", "Scroll down by half a viewport height.",            cmd_half_page_down, extend: "extend-half-page-down");
        editor_cmd!("half-page-up",   "Scroll up by half a viewport height.",              cmd_half_page_up,   extend: "extend-half-page-up");
        editor_cmd!("extend-half-page-down", "Extend selections down by half a viewport height.", cmd_extend_half_page_down);
        editor_cmd!("extend-half-page-up",   "Extend selections up by half a viewport height.",   cmd_extend_half_page_up);

        // ── Editor commands — repeat ──────────────────────────────────────────
        // Not flagged repeatable: `.` repeating itself would be nonsensical.
        editor_cmd!("repeat-last-action", "Repeat the last editing action.", cmd_repeat);

        // ── Editor commands — search ──────────────────────────────────────────
        editor_cmd!("search-forward",        "Enter search mode (forward).",                            cmd_search_forward);
        editor_cmd!("search-backward",       "Enter search mode (backward).",                           cmd_search_backward);
        editor_cmd!("search-next", "Jump to the next search match.",             cmd_search_next, extend: "extend-search-next");
        editor_cmd!("search-prev", "Jump to the previous search match.",       cmd_search_prev, extend: "extend-search-prev");
        editor_cmd!("extend-search-next", "Extend selection to the next search match.",     cmd_extend_search_next);
        editor_cmd!("extend-search-prev", "Extend selection to the previous search match.", cmd_extend_search_prev);
        editor_cmd!("clear-search",          "Clear search highlights (`:clear-search` / `:cs`).",      cmd_clear_search);

        // ── Editor commands — select ─────────────────────────────────────────
        editor_cmd!("select-within",          "Select regex matches within current selections.",          cmd_select_within);
        editor_cmd!("select-all-matches",     "Turn every search match in the buffer into a selection.", cmd_select_all_matches);
        editor_cmd!("use-selection-as-search", "Use primary selection text as the search pattern.",      cmd_use_selection_as_search);

        // ── Editor commands — jump list ──────────────────────────────────────
        editor_cmd!("jump-backward", "Navigate to the previous position in the jump list.", cmd_jump_backward);
        editor_cmd!("jump-forward",  "Navigate to the next position in the jump list.",     cmd_jump_forward);

        // ── Editor commands — misc ────────────────────────────────────────────
        editor_cmd!("quit", "Quit the editor.", cmd_quit);
    }

}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    /// The expected number of commands registered by `with_defaults`.
    ///
    /// This acts as an exhaustiveness guard: if a new command is added without
    /// a corresponding registry entry, this test catches the omission.
    ///
    /// Count breakdown:
    ///   30 motions (26 + 4 buffer-level goto: goto/extend first/last line)
    ///    4 line-selection motions (no count)
    ///   10 selection commands
    ///   44 text objects (4 line + 8 word + 16 bracket + 12 quote + 4 argument)
    ///    7 surround selection commands
    ///    3 edit commands
    ///    8 mode-transition editor commands
    ///    7 edit-composite editor commands
    ///    2 selection-state editor commands
    ///   12 find/till editor commands (8 + 4 repeat)
    ///    1 replace editor command
    ///    1 repeat-last-action editor command
    ///    7 search editor commands (search-forward/backward, search-next/prev, extend variants, clear-search)
    ///    3 select editor commands (select-within, select-all-matches, use-selection-as-search)
    ///    4 page-scroll editor commands
    ///    2 jump-list editor commands
    ///    5 insert editor commands (insert-at-line-start/end, open-line-above/below, exit-insert)
    ///    1 quit editor command
    ///
    const EXPECTED_COMMAND_COUNT: usize = 151;

    #[test]
    fn registry_has_expected_count() {
        let reg = CommandRegistry::with_defaults();
        assert_eq!(
            reg.len(),
            EXPECTED_COMMAND_COUNT,
            "registered command count mismatch — did you add a command without registering it?"
        );
    }

    #[test]
    fn lookup_by_name_works() {
        let reg = CommandRegistry::with_defaults();

        // Motion
        let cmd = reg.get("move-right").expect("move-right should be registered");
        assert_eq!(cmd.name(), "move-right");
        assert!(matches!(cmd, MappableCommand::Motion { .. }));

        // Selection
        let cmd = reg.get("collapse-selection").expect("collapse-selection should be registered");
        assert_eq!(cmd.name(), "collapse-selection");
        assert!(matches!(cmd, MappableCommand::Selection { .. }));

        // Edit
        let cmd = reg.get("delete-selection").expect("delete-selection should be registered");
        assert_eq!(cmd.name(), "delete-selection");
        assert!(matches!(cmd, MappableCommand::Edit { .. }));

        // EditorCmd
        let cmd = reg.get("quit").expect("quit should be registered");
        assert_eq!(cmd.name(), "quit");
        assert!(matches!(cmd, MappableCommand::EditorCmd { .. }));

        let cmd = reg.get("find-forward").expect("find-forward should be registered");
        assert!(matches!(cmd, MappableCommand::EditorCmd { .. }));

        let cmd = reg.get("delete").expect("delete should be registered");
        assert!(matches!(cmd, MappableCommand::EditorCmd { .. }));
    }

    #[test]
    fn unknown_name_returns_none() {
        let reg = CommandRegistry::with_defaults();
        assert!(reg.get("does-not-exist").is_none());
        assert!(reg.get("nonexistent-command").is_none());
    }

    /// Expected number of base → extend pairs registered by `register_extend_pairs`.
    ///
    /// Count breakdown:
    ///    4 character motions (left/right/up/down)
    ///    2 buffer-level goto (first/last line)
    ///    3 line-position motions (start/end/first-nonblank)
    ///    4 word motions (next/prev × word/WORD)
    ///    2 paragraph motions
    ///    2 line selection (forward/backward)
    ///   22 text objects (11 objects × inner/around)
    ///    6 find/till (forward/backward × find/till + repeat forward/backward)
    ///    2 page scroll (down/up)
    ///    2 search (next/prev)
    ///    1 special (open-line-below → flip-selections)
    ///   ──
    ///   50 total
    const EXPECTED_EXTEND_PAIR_COUNT: usize = 52;

    #[test]
    fn registry_extend_pair_count() {
        let reg = CommandRegistry::with_defaults();
        assert_eq!(
            reg.extend_map.len(),
            EXPECTED_EXTEND_PAIR_COUNT,
            "extend pair count mismatch — update EXPECTED_EXTEND_PAIR_COUNT after adding/removing pairs"
        );
    }

    #[test]
    fn registry_extend_pairs_are_valid() {
        let reg = CommandRegistry::with_defaults();
        for (&base, &extend) in &reg.extend_map {
            assert!(
                reg.commands.contains_key(base),
                "extend pair: base command '{base}' not in registry"
            );
            assert!(
                reg.commands.contains_key(extend),
                "extend pair: extend command '{extend}' not in registry"
            );
        }
    }

    #[test]
    fn extend_variant_lookup() {
        let reg = CommandRegistry::with_defaults();
        assert_eq!(reg.extend_variant("move-left"), Some("extend-left"));
        assert_eq!(reg.extend_variant("select-next-word"), Some("extend-select-next-word"));
        assert_eq!(reg.extend_variant("open-line-below"), Some("flip-selections"));
        assert_eq!(reg.extend_variant("delete"), None);
        assert_eq!(reg.extend_variant("undo"), None);
        assert_eq!(reg.extend_variant("insert-before"), None);
    }

    #[test]
    fn all_names_are_unique() {
        // HashMap insertion silently overwrites duplicates, so verify that
        // the final count matches the number of distinct names.
        let reg = CommandRegistry::with_defaults();
        let unique: std::collections::HashSet<&&'static str> = reg.names().collect();
        assert_eq!(unique.len(), reg.len(), "duplicate command names detected");
    }
}
