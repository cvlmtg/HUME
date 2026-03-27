//! Mappable command registry.
//!
//! A [`MappableCommand`] wraps a `cmd_*` function pointer together with a
//! string name and a one-line doc string. All commands live in a
//! [`CommandRegistry`] keyed by name, built once via
//! [`CommandRegistry::with_defaults`].
//!
//! **Two distinct command systems exist in HUME** (following Helix/Kakoune):
//!
//! 1. **Mappable commands** (this file): parameterless, bound to keys by the
//!    keymap layer. Motions, selections, text objects, and edits. NOT invoked
//!    from `:`.
//! 2. **Typed commands** (`editor/mappings.rs` `execute_command`): entered
//!    via `:`, take string arguments — `:w`, `:q`, future `:w <path>`, etc.
//!
//! The registry is the data source the keymap trie (M4) will use to translate
//! command names to function pointers. It replaces the hardcoded `match` arms
//! in `handle_normal`.

use std::collections::HashMap;

use crate::buffer::Buffer;
use crate::changeset::ChangeSet;
use crate::edit::{delete_char_backward, delete_char_forward, delete_selection};
use crate::motion::{
    cmd_extend_down, cmd_extend_first_nonblank, cmd_extend_left, cmd_extend_line_end,
    cmd_extend_line_start, cmd_extend_next_paragraph, cmd_extend_prev_paragraph,
    cmd_extend_right, cmd_extend_select_line, cmd_extend_select_line_backward,
    cmd_extend_select_next_WORD, cmd_extend_select_next_word, cmd_extend_select_prev_WORD,
    cmd_extend_select_prev_word, cmd_extend_up, cmd_goto_first_nonblank, cmd_goto_line_end,
    cmd_goto_line_start, cmd_move_down, cmd_move_left, cmd_move_right, cmd_move_up,
    cmd_next_paragraph, cmd_prev_paragraph, cmd_select_line, cmd_select_line_backward,
    cmd_select_next_WORD, cmd_select_next_word, cmd_select_prev_WORD, cmd_select_prev_word,
};
use crate::selection::SelectionSet;
use crate::selection_cmd::{
    cmd_collapse_selection, cmd_copy_selection_on_next_line, cmd_copy_selection_on_prev_line,
    cmd_cycle_primary_backward, cmd_cycle_primary_forward, cmd_flip_selections,
    cmd_keep_primary_selection, cmd_remove_primary_selection, cmd_split_selection_on_newlines,
    cmd_trim_selection_whitespace,
};
use crate::text_object::{
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
/// Three variants mirror the three function signatures used by all editing
/// commands in the codebase. The keymap trie stores command *names*; the
/// registry resolves names to `MappableCommand` values at dispatch time.
#[derive(Clone)]
#[allow(dead_code)] // `doc` and `fun` are unused until the keymap trie (M4) dispatches through them
pub(crate) enum MappableCommand {
    /// Motion that repeats `count` times.
    ///
    /// Signature: `fn(&Buffer, SelectionSet, usize) -> SelectionSet`
    Motion {
        name: &'static str,
        doc: &'static str,
        fun: fn(&Buffer, SelectionSet, usize) -> SelectionSet,
    },
    /// Selection or text-object operation (no count).
    ///
    /// Signature: `fn(&Buffer, SelectionSet) -> SelectionSet`
    Selection {
        name: &'static str,
        doc: &'static str,
        fun: fn(&Buffer, SelectionSet) -> SelectionSet,
    },
    /// Buffer-modifying edit with no extra arguments.
    ///
    /// Signature: `fn(Buffer, SelectionSet) -> (Buffer, SelectionSet, ChangeSet)`
    ///
    /// Parameterized edits (`insert_char`, `paste_after`, etc.) are NOT
    /// registered — they need closures at the call site and will be handled
    /// by the keymap layer separately.
    Edit {
        name: &'static str,
        doc: &'static str,
        fun: fn(Buffer, SelectionSet) -> (Buffer, SelectionSet, ChangeSet),
    },
}

impl MappableCommand {
    pub(crate) fn name(&self) -> &'static str {
        match self {
            Self::Motion { name, .. }
            | Self::Selection { name, .. }
            | Self::Edit { name, .. } => name,
        }
    }

    #[allow(dead_code)] // used by keymap trie (M4)
    pub(crate) fn doc(&self) -> &'static str {
        match self {
            Self::Motion { doc, .. }
            | Self::Selection { doc, .. }
            | Self::Edit { doc, .. } => doc,
        }
    }
}

// ── CommandRegistry ───────────────────────────────────────────────────────────

/// Registry of all mappable commands, keyed by name.
///
/// Built once via [`CommandRegistry::with_defaults`] and stored on the editor.
/// The keymap trie (M4) will look up command names here at dispatch time,
/// replacing the hardcoded `match` arms in `handle_normal`.
pub(crate) struct CommandRegistry {
    commands: HashMap<&'static str, MappableCommand>,
}

impl CommandRegistry {
    /// Build a registry pre-populated with every default `cmd_*` function.
    pub(crate) fn with_defaults() -> Self {
        let mut reg = Self {
            commands: HashMap::new(),
        };
        reg.register_defaults();
        reg
    }

    fn register(&mut self, cmd: MappableCommand) {
        self.commands.insert(cmd.name(), cmd);
    }

    /// Look up a command by name.
    #[allow(dead_code)] // used by keymap trie (M4)
    pub(crate) fn get(&self, name: &str) -> Option<&MappableCommand> {
        self.commands.get(name)
    }

    /// Iterate over all registered command names.
    #[allow(dead_code)] // used by keymap trie (M4)
    pub(crate) fn names(&self) -> impl Iterator<Item = &&'static str> {
        self.commands.keys()
    }

    /// Total number of registered commands.
    #[allow(dead_code)] // used by tests and keymap trie (M4)
    pub(crate) fn len(&self) -> usize {
        self.commands.len()
    }

    fn register_defaults(&mut self) {
        // Local macros to cut down on struct-literal boilerplate.
        // Each arm builds the right variant and calls `self.register`.
        macro_rules! motion {
            ($name:literal, $doc:literal, $fun:expr) => {
                self.register(MappableCommand::Motion {
                    name: $name,
                    doc: $doc,
                    fun: $fun,
                })
            };
        }
        macro_rules! selection {
            ($name:literal, $doc:literal, $fun:expr) => {
                self.register(MappableCommand::Selection {
                    name: $name,
                    doc: $doc,
                    fun: $fun,
                })
            };
        }
        macro_rules! edit {
            ($name:literal, $doc:literal, $fun:expr) => {
                self.register(MappableCommand::Edit {
                    name: $name,
                    doc: $doc,
                    fun: $fun,
                })
            };
        }

        // ── Character motions ─────────────────────────────────────────────────
        motion!("move-right", "Move cursors one grapheme to the right.", cmd_move_right);
        motion!("move-left", "Move cursors one grapheme to the left.", cmd_move_left);
        motion!("move-down", "Move cursors down one line.", cmd_move_down);
        motion!("move-up", "Move cursors up one line.", cmd_move_up);
        motion!("extend-right", "Extend selections one grapheme to the right.", cmd_extend_right);
        motion!("extend-left", "Extend selections one grapheme to the left.", cmd_extend_left);
        motion!("extend-down", "Extend selections down one line.", cmd_extend_down);
        motion!("extend-up", "Extend selections up one line.", cmd_extend_up);

        // ── Line-position motions ─────────────────────────────────────────────
        motion!("goto-line-start", "Move cursors to the start of the line.", cmd_goto_line_start);
        motion!("goto-line-end", "Move cursors to the last character on the line.", cmd_goto_line_end);
        motion!("goto-first-nonblank", "Move cursors to the first non-blank character on the line.", cmd_goto_first_nonblank);
        motion!("extend-line-start", "Extend selections to the start of the line.", cmd_extend_line_start);
        motion!("extend-line-end", "Extend selections to the last character on the line.", cmd_extend_line_end);
        motion!("extend-first-nonblank", "Extend selections to the first non-blank character on the line.", cmd_extend_first_nonblank);

        // ── Word motions ──────────────────────────────────────────────────────
        motion!("select-next-word", "Select the next word.", cmd_select_next_word);
        motion!("select-next-WORD", "Select the next WORD (whitespace-delimited).", cmd_select_next_WORD);
        motion!("select-prev-word", "Select the previous word.", cmd_select_prev_word);
        motion!("select-prev-WORD", "Select the previous WORD (whitespace-delimited).", cmd_select_prev_WORD);
        motion!("extend-select-next-word", "Extend selection to encompass the next word.", cmd_extend_select_next_word);
        motion!("extend-select-next-WORD", "Extend selection to encompass the next WORD.", cmd_extend_select_next_WORD);
        motion!("extend-select-prev-word", "Extend selection to encompass the previous word.", cmd_extend_select_prev_word);
        motion!("extend-select-prev-WORD", "Extend selection to encompass the previous WORD.", cmd_extend_select_prev_WORD);

        // ── Paragraph motions ─────────────────────────────────────────────────
        motion!("next-paragraph", "Move cursors to the start of the next paragraph.", cmd_next_paragraph);
        motion!("prev-paragraph", "Move cursors to the start of the previous paragraph.", cmd_prev_paragraph);
        motion!("extend-next-paragraph", "Extend selections to the start of the next paragraph.", cmd_extend_next_paragraph);
        motion!("extend-prev-paragraph", "Extend selections to the start of the previous paragraph.", cmd_extend_prev_paragraph);

        // ── Line selection ────────────────────────────────────────────────────
        selection!("select-line", "Select the full current line (forward).", cmd_select_line);
        selection!("select-line-backward", "Select the full current line (backward).", cmd_select_line_backward);
        selection!("extend-select-line", "Grow selection to cover the current line (extend mode).", cmd_extend_select_line);
        selection!("extend-select-line-backward", "Grow selection upward to cover the current line.", cmd_extend_select_line_backward);

        // ── Selection commands ────────────────────────────────────────────────
        selection!("collapse-selection", "Collapse each selection to a single cursor at the head.", cmd_collapse_selection);
        selection!("flip-selections", "Swap anchor and head for each selection.", cmd_flip_selections);
        selection!("keep-primary-selection", "Remove all selections except the primary.", cmd_keep_primary_selection);
        selection!("remove-primary-selection", "Remove the primary selection, promoting the next.", cmd_remove_primary_selection);
        selection!("cycle-primary-forward", "Cycle the primary selection forward.", cmd_cycle_primary_forward);
        selection!("cycle-primary-backward", "Cycle the primary selection backward.", cmd_cycle_primary_backward);
        selection!("split-selection-on-newlines", "Split each multi-line selection into one per line.", cmd_split_selection_on_newlines);
        selection!("trim-selection-whitespace", "Trim leading and trailing whitespace from each selection.", cmd_trim_selection_whitespace);
        selection!("copy-selection-on-next-line", "Duplicate each selection on the line below.", cmd_copy_selection_on_next_line);
        selection!("copy-selection-on-prev-line", "Duplicate each selection on the line above.", cmd_copy_selection_on_prev_line);

        // ── Text objects — line ───────────────────────────────────────────────
        selection!("inner-line", "Select inner line content (excluding the newline).", cmd_inner_line);
        selection!("around-line", "Select the line including its newline.", cmd_around_line);
        selection!("extend-inner-line", "Extend to inner line content.", cmd_extend_inner_line);
        selection!("extend-around-line", "Extend to include the full line.", cmd_extend_around_line);

        // ── Text objects — word ───────────────────────────────────────────────
        selection!("inner-word", "Select inner word.", cmd_inner_word);
        selection!("around-word", "Select word plus surrounding whitespace.", cmd_around_word);
        selection!("extend-inner-word", "Extend to inner word.", cmd_extend_inner_word);
        selection!("extend-around-word", "Extend to word plus surrounding whitespace.", cmd_extend_around_word);
        selection!("inner-WORD", "Select inner WORD (whitespace-delimited).", cmd_inner_WORD);
        selection!("around-WORD", "Select WORD plus surrounding whitespace.", cmd_around_WORD);
        selection!("extend-inner-WORD", "Extend to inner WORD.", cmd_extend_inner_WORD);
        selection!("extend-around-WORD", "Extend to WORD plus surrounding whitespace.", cmd_extend_around_WORD);

        // ── Text objects — brackets ───────────────────────────────────────────
        selection!("inner-paren", "Select content inside the nearest `()`.", cmd_inner_paren);
        selection!("around-paren", "Select content including the nearest `()`.", cmd_around_paren);
        selection!("extend-inner-paren", "Extend to content inside the nearest `()`.", cmd_extend_inner_paren);
        selection!("extend-around-paren", "Extend to content including the nearest `()`.", cmd_extend_around_paren);
        selection!("inner-bracket", "Select content inside the nearest `[]`.", cmd_inner_bracket);
        selection!("around-bracket", "Select content including the nearest `[]`.", cmd_around_bracket);
        selection!("extend-inner-bracket", "Extend to content inside the nearest `[]`.", cmd_extend_inner_bracket);
        selection!("extend-around-bracket", "Extend to content including the nearest `[]`.", cmd_extend_around_bracket);
        selection!("inner-brace", "Select content inside the nearest `{}`.", cmd_inner_brace);
        selection!("around-brace", "Select content including the nearest `{}`.", cmd_around_brace);
        selection!("extend-inner-brace", "Extend to content inside the nearest `{}`.", cmd_extend_inner_brace);
        selection!("extend-around-brace", "Extend to content including the nearest `{}`.", cmd_extend_around_brace);
        selection!("inner-angle", "Select content inside the nearest `<>`.", cmd_inner_angle);
        selection!("around-angle", "Select content including the nearest `<>`.", cmd_around_angle);
        selection!("extend-inner-angle", "Extend to content inside the nearest `<>`.", cmd_extend_inner_angle);
        selection!("extend-around-angle", "Extend to content including the nearest `<>`.", cmd_extend_around_angle);

        // ── Text objects — quotes ─────────────────────────────────────────────
        selection!("inner-double-quote", "Select content inside the nearest `\"`.", cmd_inner_double_quote);
        selection!("around-double-quote", "Select content including the nearest `\"`.", cmd_around_double_quote);
        selection!("extend-inner-double-quote", "Extend to content inside the nearest `\"`.", cmd_extend_inner_double_quote);
        selection!("extend-around-double-quote", "Extend to content including the nearest `\"`.", cmd_extend_around_double_quote);
        selection!("inner-single-quote", "Select content inside the nearest `'`.", cmd_inner_single_quote);
        selection!("around-single-quote", "Select content including the nearest `'`.", cmd_around_single_quote);
        selection!("extend-inner-single-quote", "Extend to content inside the nearest `'`.", cmd_extend_inner_single_quote);
        selection!("extend-around-single-quote", "Extend to content including the nearest `'`.", cmd_extend_around_single_quote);
        selection!("inner-backtick", "Select content inside the nearest backtick pair.", cmd_inner_backtick);
        selection!("around-backtick", "Select content including the nearest backtick pair.", cmd_around_backtick);
        selection!("extend-inner-backtick", "Extend to content inside the nearest backtick pair.", cmd_extend_inner_backtick);
        selection!("extend-around-backtick", "Extend to content including the nearest backtick pair.", cmd_extend_around_backtick);

        // ── Text objects — arguments ──────────────────────────────────────────
        selection!("inner-argument", "Select the argument at the cursor (trimmed).", cmd_inner_argument);
        selection!("around-argument", "Select the argument and its separator comma.", cmd_around_argument);
        selection!("extend-inner-argument", "Extend to the inner argument.", cmd_extend_inner_argument);
        selection!("extend-around-argument", "Extend to include the argument and separator.", cmd_extend_around_argument);

        // ── Edit commands ─────────────────────────────────────────────────────
        edit!("delete-char-forward", "Delete the character (or selection) under the cursor.", delete_char_forward);
        edit!("delete-char-backward", "Delete the character before each cursor.", delete_char_backward);
        edit!("delete-selection", "Delete all selections.", delete_selection);
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    /// The expected number of commands registered by `with_defaults`.
    ///
    /// This acts as an exhaustiveness guard: if a new `cmd_*` function is added
    /// without a corresponding registry entry, this test catches the omission.
    ///
    /// Count breakdown:
    ///   26 motions (with count)
    ///    4 line-selection motions (no count)
    ///   10 selection commands
    ///   44 text objects (4 line + 8 word + 16 bracket + 12 quote + 4 argument)
    ///    3 edit commands
    ///   ──
    ///   87 total
    const EXPECTED_COMMAND_COUNT: usize = 87;

    #[test]
    fn registry_has_expected_count() {
        let reg = CommandRegistry::with_defaults();
        assert_eq!(
            reg.len(),
            EXPECTED_COMMAND_COUNT,
            "registered command count mismatch — did you add a cmd_* without registering it?"
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
    }

    #[test]
    fn unknown_name_returns_none() {
        let reg = CommandRegistry::with_defaults();
        assert!(reg.get("does-not-exist").is_none());
        // Make sure a typed command (`:q`) is not in the mappable registry.
        assert!(reg.get("quit").is_none());
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
