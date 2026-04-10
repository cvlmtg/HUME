//! Command registry — the single namespace for all user-facing commands.
//!
//! Two kinds of commands share this registry:
//!
//! - [`MappableCommand`] — bindable to keys. The keymap trie stores command
//!   *names*; the registry resolves them to `MappableCommand` values at
//!   dispatch time inside `execute_keymap_command` (`editor/mappings.rs`).
//! - [`TypedCommand`] — invocable from the `:` command line. The dispatcher
//!   in `execute_command` (`editor/mappings.rs`) calls
//!   [`CommandRegistry::get_typed`] to resolve name or alias to a
//!   `TypedCommand`.
//!
//! The shared namespace prevents name collisions between the two kinds and
//! provides a single source for `:help` and command-palette display.
//!
//! # Extend mode
//!
//! Extend mode is handled at dispatch time via a `MotionMode` parameter, not
//! via separate extend-variant commands. All Motion and Selection commands
//! accept `MotionMode` and branch on `Move` vs `Extend`. EditorCmds that
//! support extend carry `extendable: true`; the dispatcher passes the correct
//! `MotionMode` based on the current mode or Ctrl+letter state.
//!
//! # Mappable command variants
//!
//! 1. **Motion** — pure `fn(&Buffer, SelectionSet, usize, MotionMode) -> SelectionSet`
//! 2. **Selection** — pure `fn(&Buffer, SelectionSet, MotionMode) -> SelectionSet`
//! 3. **Edit** — pure `fn(Buffer, SelectionSet) -> (Buffer, SelectionSet, ChangeSet)`
//! 4. **EditorCmd** — `fn(&mut Editor, usize, MotionMode)` for composite/side-effectful
//!    operations (mode changes, registers, undo groups, parameterized motions).
//!    Implemented in `editor/commands.rs`; stored and dispatched as a
//!    function pointer exactly like the other variants.

use std::borrow::Cow;
use std::collections::HashMap;

use crate::core::buffer::Buffer;
use crate::core::error::CommandError;
use crate::core::changeset::ChangeSet;
use crate::ops::MotionMode;
use crate::ops::edit::{delete_char_backward, delete_char_forward, delete_selection};
use crate::ops::motion::{
    cmd_goto_first_line, cmd_goto_first_nonblank,
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
    cmd_around_single_quote, cmd_around_word, cmd_around_WORD, cmd_inner_angle,
    cmd_inner_argument, cmd_inner_backtick, cmd_inner_brace, cmd_inner_bracket,
    cmd_inner_double_quote, cmd_inner_line, cmd_inner_paren, cmd_inner_single_quote,
    cmd_inner_word, cmd_inner_WORD,
};

// ── MappableCommand ───────────────────────────────────────────────────────────

/// A command that can be bound to a key in a keymap.
///
/// The keymap trie stores command *names*; the registry resolves names to
/// `MappableCommand` values at dispatch time.
#[derive(Clone)]
pub(crate) enum MappableCommand {
    /// Motion that repeats `count` times.
    ///
    /// Signature: `fn(&Buffer, SelectionSet, usize, MotionMode) -> SelectionSet`
    ///
    /// Motions are always extendable. The `mode` parameter selects Move or Extend
    /// semantics at dispatch time — no separate extend-variant functions needed.
    Motion {
        name: Cow<'static, str>,
        doc: Cow<'static, str>,
        fun: fn(&Buffer, SelectionSet, usize, MotionMode) -> SelectionSet,
        /// Whether this motion always records a jump list entry before executing,
        /// regardless of how far the cursor moves. Used for goto commands.
        jump: bool,
    },
    /// Selection or text-object operation (no count).
    ///
    /// Signature: `fn(&Buffer, SelectionSet, MotionMode) -> SelectionSet`
    ///
    /// All selection commands receive `MotionMode`. Non-extendable ones accept
    /// `_mode` and ignore it; extendable text objects branch on it.
    Selection {
        name: Cow<'static, str>,
        doc: Cow<'static, str>,
        fun: fn(&Buffer, SelectionSet, MotionMode) -> SelectionSet,
    },
    /// Buffer-modifying edit with no extra arguments.
    ///
    /// Signature: `fn(Buffer, SelectionSet) -> (Buffer, SelectionSet, ChangeSet)`
    ///
    /// Edits are never extendable — they don't carry `MotionMode`.
    Edit {
        name: Cow<'static, str>,
        doc: Cow<'static, str>,
        fun: fn(Buffer, SelectionSet) -> (Buffer, SelectionSet, ChangeSet),
        /// Whether `.` should replay this command. Set to `true` for edits that
        /// are meaningful to repeat (e.g. user-facing deletions). Set to `false`
        /// for internal primitives like `delete-char-backward`.
        repeatable: bool,
    },
    /// Editor-level command requiring `&mut Editor` context.
    ///
    /// Signature: `fn(&mut Editor, usize, MotionMode) -> Result<(), CommandError>`
    ///
    /// Covers composite operations: mode changes, register access, undo group
    /// management, and parameterized motions (find/till/replace). Returns `Err`
    /// only for true user-facing failures (e.g. "no match", I/O errors).
    /// Silent no-ops (boundary conditions) return `Ok(())`. Stored and
    /// dispatched as a function pointer exactly like the other variants.
    EditorCmd {
        name: Cow<'static, str>,
        doc: Cow<'static, str>,
        fun: fn(&mut super::Editor, usize, MotionMode) -> Result<(), CommandError>,
        /// Whether `.` should replay this command.
        repeatable: bool,
        /// Whether this command always records a jump list entry before executing.
        /// Used for search jumps and explicit page-scroll commands.
        jump: bool,
        /// Whether this command is a visual-line motion (move-down/up, extend-down/up).
        /// The preferred display column is preserved across consecutive visual-line moves.
        visual_move: bool,
        /// Whether this EditorCmd has extend semantics (used by the Ctrl+key guard
        /// to decide if Ctrl+key should trigger extend dispatch).
        ///
        /// Motion and Selection are always extendable (implicit). Edit is never
        /// extendable (implicit). Only EditorCmd needs an explicit flag.
        extendable: bool,
    },
}

impl MappableCommand {
    #[allow(dead_code)]
    pub(crate) fn name(&self) -> &str {
        match self {
            Self::Motion { name, .. }
            | Self::Selection { name, .. }
            | Self::Edit { name, .. }
            | Self::EditorCmd { name, .. } => name.as_ref(),
        }
    }

    /// One-line description of the command, for `:help` and command-palette display.
    #[allow(dead_code)]
    pub(crate) fn doc(&self) -> &str {
        match self {
            Self::Motion { doc, .. }
            | Self::Selection { doc, .. }
            | Self::Edit { doc, .. }
            | Self::EditorCmd { doc, .. } => doc.as_ref(),
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

    /// Returns `true` if this command always records a jump list entry before
    /// executing, regardless of how far the cursor moves.
    ///
    /// This is the single source of truth for jump-command classification —
    /// there is no parallel `JUMP_COMMANDS` list.
    pub(crate) fn is_jump(&self) -> bool {
        match self {
            Self::Motion { jump, .. } | Self::EditorCmd { jump, .. } => *jump,
            Self::Selection { .. } | Self::Edit { .. } => false,
        }
    }

    /// Returns `true` if this command is a visual-line motion.
    ///
    /// The editor preserves the preferred display column across consecutive
    /// visual-line moves and clears it for any other command.
    pub(crate) fn is_visual_move(&self) -> bool {
        match self {
            Self::EditorCmd { visual_move, .. } => *visual_move,
            _ => false,
        }
    }

    /// Returns `true` if this command has extend semantics and can be triggered
    /// as a one-shot extend via Ctrl+key.
    ///
    /// Motion and Selection are always extendable. Edit is never extendable.
    /// EditorCmd has an explicit `extendable` flag set at registration time.
    pub(crate) fn is_extendable(&self) -> bool {
        match self {
            Self::Motion { .. } | Self::Selection { .. } => true,
            Self::Edit { .. } => false,
            Self::EditorCmd { extendable, .. } => *extendable,
        }
    }
}

// ── TypedCommand ──────────────────────────────────────────────────────────────

/// A command invocable from the `:` command line.
///
/// Typed commands have a canonical name and optional short aliases. They are
/// stored in [`CommandRegistry`] alongside [`MappableCommand`] entries in a
/// single `HashMap`, sharing the same namespace.
///
/// The function signature differs from mappable commands: it receives an
/// optional string argument (e.g. the path for `:w foo.txt`) and a force flag
/// (whether `!` was appended), rather than a numeric count.
pub(crate) struct TypedCommand {
    /// Canonical name, e.g. `"write"`. Used as the registry key.
    pub name: Cow<'static, str>,
    /// One-line description for `:help` and command-palette display.
    #[allow(dead_code)]
    pub doc: Cow<'static, str>,
    /// Short aliases, e.g. `&["w"]`. Each alias is also registered in the
    /// alias index for O(1) lookup. Empty for commands with no alias.
    ///
    /// `&'static [&'static str]` covers all built-in commands. Steel-registered
    /// typed commands pass `&[]` and register aliases separately if needed.
    pub aliases: &'static [&'static str],
    /// The function to execute. Receives the editor, an optional argument
    /// (e.g. a file path), and whether `!` was appended.
    pub fun: fn(&mut super::Editor, Option<&str>, bool) -> Result<(), CommandError>,
}

// ── CommandRegistry ───────────────────────────────────────────────────────────

/// Registry of all commands — the single namespace for mappable and typed commands.
///
/// Built once via [`CommandRegistry::with_defaults`] and stored on the editor.
///
/// - **Mappable commands** are bound to keys. The keymap dispatcher
///   (`execute_keymap_command` in `editor/mappings.rs`) resolves them via
///   [`Self::get_mappable`].
/// - **Typed commands** are invoked from the `:` command line. The dispatcher
///   (`execute_command` in `editor/mappings.rs`) resolves them via
///   [`Self::get_typed`]. Aliases are supported via [`Self::alias_map`].
/// - The `:` command line also falls back to **mappable commands** when no
///   typed command matches — any mappable command can be invoked by name
///   from the command line with an implicit `count = 1`.
///
/// The single `commands` map prevents name collisions between the two kinds.
pub(crate) struct CommandRegistry {
    /// All commands keyed by canonical name.
    commands: HashMap<Cow<'static, str>, Command>,
    /// Maps typed-command alias → canonical name, for O(1) alias lookup.
    alias_map: HashMap<Cow<'static, str>, Cow<'static, str>>,
}

/// A command stored in [`CommandRegistry`].
///
/// Mappable and typed commands share the same namespace but have different
/// signatures and dispatch paths.
pub(crate) enum Command {
    Mappable(MappableCommand),
    Typed(TypedCommand),
}

impl CommandRegistry {
    /// Build a registry pre-populated with every default command.
    pub(crate) fn with_defaults() -> Self {
        let mut reg = Self {
            commands: HashMap::new(),
            alias_map: HashMap::new(),
        };
        reg.register_defaults();
        reg
    }

    /// Register a mappable command.
    ///
    /// The name is extracted from the command and used as the `HashMap` key.
    /// For static built-ins the clone is a pointer copy (zero allocation).
    pub(crate) fn register(&mut self, cmd: MappableCommand) {
        let key = match &cmd {
            MappableCommand::Motion { name, .. }
            | MappableCommand::Selection { name, .. }
            | MappableCommand::Edit { name, .. }
            | MappableCommand::EditorCmd { name, .. } => name.clone(),
        };
        self.commands.insert(key, Command::Mappable(cmd));
    }

    /// Register a typed command.
    ///
    /// Inserts the canonical name into `commands` and each alias into
    /// `alias_map`. This is the future `define-typed-command!` entry point
    /// for the Steel scripting layer.
    pub(crate) fn register_typed(&mut self, cmd: TypedCommand) {
        let canonical = cmd.name.clone();
        for &alias in cmd.aliases {
            self.alias_map.insert(Cow::Borrowed(alias), canonical.clone());
        }
        self.commands.insert(canonical, Command::Typed(cmd));
    }

    /// Look up a mappable command by name.
    ///
    /// Returns `None` if the name is unknown or resolves to a typed command.
    /// Used by `execute_keymap_command` in `editor/mappings.rs`.
    pub(crate) fn get_mappable(&self, name: &str) -> Option<&MappableCommand> {
        match self.commands.get(name)? {
            Command::Mappable(cmd) => Some(cmd),
            Command::Typed(_) => None,
        }
    }

    /// Look up a typed command by canonical name or alias.
    ///
    /// Returns `None` if the name is unknown or resolves to a mappable command.
    /// The `:` command dispatcher falls back to [`Self::get_mappable`] when
    /// this returns `None` — see `execute_command` in `editor/mappings.rs`.
    pub(crate) fn get_typed(&self, name: &str) -> Option<&TypedCommand> {
        let canonical = self.alias_map.get(name).map_or(name, |c| c.as_ref());
        match self.commands.get(canonical)? {
            Command::Typed(cmd) => Some(cmd),
            Command::Mappable(_) => None,
        }
    }

    /// Iterate over all registered canonical command names (not aliases).
    #[allow(dead_code)]
    pub(crate) fn names(&self) -> impl Iterator<Item = &str> {
        self.commands.keys().map(|k| k.as_ref())
    }

    /// Total number of registered commands (mappable + typed, not counting aliases).
    #[allow(dead_code)]
    pub(crate) fn len(&self) -> usize {
        self.commands.len()
    }

    fn register_defaults(&mut self) {
        // Local macros to cut down on struct-literal boilerplate.
        macro_rules! motion {
            ($name:literal, $doc:literal, $fun:expr, jump) => {
                self.register(MappableCommand::Motion { name: Cow::Borrowed($name), doc: Cow::Borrowed($doc), fun: $fun, jump: true })
            };
            ($name:literal, $doc:literal, $fun:expr) => {
                self.register(MappableCommand::Motion { name: Cow::Borrowed($name), doc: Cow::Borrowed($doc), fun: $fun, jump: false })
            };
        }
        macro_rules! selection {
            ($name:literal, $doc:literal, $fun:expr) => {
                self.register(MappableCommand::Selection { name: Cow::Borrowed($name), doc: Cow::Borrowed($doc), fun: $fun })
            };
        }
        macro_rules! edit {
            ($name:literal, $doc:literal, $fun:expr) => {
                self.register(MappableCommand::Edit { name: Cow::Borrowed($name), doc: Cow::Borrowed($doc), fun: $fun, repeatable: false })
            };
            ($name:literal, $doc:literal, $fun:expr, repeatable) => {
                self.register(MappableCommand::Edit { name: Cow::Borrowed($name), doc: Cow::Borrowed($doc), fun: $fun, repeatable: true })
            };
        }

        // Builder for EditorCmd registration. Each flag method sets one bool;
        // .reg(registry) terminates the chain. Adding a new flag costs one
        // method — existing call sites are unaffected.
        struct EditorCmdBuilder {
            name: &'static str,
            doc:  &'static str,
            fun:  fn(&mut super::Editor, usize, MotionMode) -> Result<(), CommandError>,
            repeatable:  bool,
            jump:        bool,
            visual_move: bool,
            extendable:  bool,
        }
        impl EditorCmdBuilder {
            fn repeatable(mut self)  -> Self { self.repeatable  = true; self }
            fn jump(mut self)        -> Self { self.jump        = true; self }
            fn visual_move(mut self) -> Self { self.visual_move = true; self }
            fn extendable(mut self)  -> Self { self.extendable  = true; self }
            fn reg(self, r: &mut CommandRegistry) {
                r.register(MappableCommand::EditorCmd {
                    name: Cow::Borrowed(self.name),
                    doc:  Cow::Borrowed(self.doc),
                    fun:  self.fun,
                    repeatable:  self.repeatable,
                    jump:        self.jump,
                    visual_move: self.visual_move,
                    extendable:  self.extendable,
                });
            }
        }
        // Construct a builder with all flags false.
        let ecmd = |name: &'static str, doc: &'static str, fun: fn(&mut super::Editor, usize, MotionMode) -> Result<(), CommandError>| {
            EditorCmdBuilder { name, doc, fun, repeatable: false, jump: false, visual_move: false, extendable: false }
        };

        // ── Character motions ─────────────────────────────────────────────────
        motion!("move-right", "Move cursors one grapheme to the right.", cmd_move_right);
        motion!("move-left",  "Move cursors one grapheme to the left.",  cmd_move_left);
        ecmd("move-down", "Move cursors down one visual line.", cmd_visual_move_down).extendable().visual_move().reg(self);
        ecmd("move-up",   "Move cursors up one visual line.",   cmd_visual_move_up  ).extendable().visual_move().reg(self);

        // ── Buffer-level goto motions ─────────────────────────────────────────
        motion!("goto-first-line", "Move cursors to the first character of the buffer.",    cmd_goto_first_line, jump);
        motion!("goto-last-line",  "Move cursors to the first character of the last line.", cmd_goto_last_line,  jump);

        // ── Line-position motions ─────────────────────────────────────────────
        motion!("goto-line-start",    "Move cursors to the start of the line.",                     cmd_goto_line_start);
        motion!("goto-line-end",      "Move cursors to the last character on the line.",            cmd_goto_line_end);
        motion!("goto-first-nonblank","Move cursors to the first non-blank character on the line.", cmd_goto_first_nonblank);

        // ── Word motions ──────────────────────────────────────────────────────
        motion!("select-next-word", "Select the next word.",                           cmd_select_next_word);
        motion!("select-next-WORD", "Select the next WORD (whitespace-delimited).",    cmd_select_next_WORD);
        motion!("select-prev-word", "Select the previous word.",                       cmd_select_prev_word);
        motion!("select-prev-WORD", "Select the previous WORD (whitespace-delimited).",cmd_select_prev_WORD);

        // ── Paragraph motions ─────────────────────────────────────────────────
        motion!("next-paragraph", "Move cursors to the start of the next paragraph.",     cmd_next_paragraph);
        motion!("prev-paragraph", "Move cursors to the start of the previous paragraph.", cmd_prev_paragraph);

        // ── Line selection ────────────────────────────────────────────────────
        selection!("select-line",          "Select the full current line (forward).",   cmd_select_line);
        selection!("select-line-backward", "Select the full current line (backward).", cmd_select_line_backward);

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
        selection!("inner-line",  "Select inner line content (excluding the newline).", cmd_inner_line);
        selection!("around-line", "Select the line including its newline.",             cmd_around_line);

        // ── Text objects — word ───────────────────────────────────────────────
        selection!("inner-word",  "Select inner word.",                          cmd_inner_word);
        selection!("around-word", "Select word plus surrounding whitespace.",    cmd_around_word);
        selection!("inner-WORD",  "Select inner WORD (whitespace-delimited).",  cmd_inner_WORD);
        selection!("around-WORD", "Select WORD plus surrounding whitespace.",   cmd_around_WORD);

        // ── Text objects — brackets ───────────────────────────────────────────
        selection!("inner-paren",   "Select content inside the nearest `()`.",    cmd_inner_paren);
        selection!("around-paren",  "Select content including the nearest `()`.", cmd_around_paren);
        selection!("inner-bracket",   "Select content inside the nearest `[]`.",    cmd_inner_bracket);
        selection!("around-bracket",  "Select content including the nearest `[]`.", cmd_around_bracket);
        selection!("inner-brace",   "Select content inside the nearest `{}`.",    cmd_inner_brace);
        selection!("around-brace",  "Select content including the nearest `{}`.", cmd_around_brace);
        selection!("inner-angle",   "Select content inside the nearest `<>`.",    cmd_inner_angle);
        selection!("around-angle",  "Select content including the nearest `<>`.", cmd_around_angle);

        // ── Text objects — quotes ─────────────────────────────────────────────
        selection!("inner-double-quote",  "Select content inside the nearest `\"`.",    cmd_inner_double_quote);
        selection!("around-double-quote", "Select content including the nearest `\"`.", cmd_around_double_quote);
        selection!("inner-single-quote",  "Select content inside the nearest `'`.",    cmd_inner_single_quote);
        selection!("around-single-quote", "Select content including the nearest `'`.", cmd_around_single_quote);
        selection!("inner-backtick",  "Select content inside the nearest backtick pair.",    cmd_inner_backtick);
        selection!("around-backtick", "Select content including the nearest backtick pair.", cmd_around_backtick);

        // ── Text objects — arguments ──────────────────────────────────────────
        selection!("inner-argument",  "Select the argument at the cursor (trimmed).",    cmd_inner_argument);
        selection!("around-argument", "Select the argument and its separator comma.",     cmd_around_argument);

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
        ecmd("insert-before",        "Enter insert mode; collapse each selection to its start.",         cmd_insert_before       ).repeatable().reg(self);
        ecmd("insert-after",         "Enter insert mode after the cursor (move one grapheme right).",    cmd_insert_after        ).repeatable().reg(self);
        ecmd("insert-at-line-start",      "Enter insert mode at the first non-blank character on the line.", cmd_insert_at_line_start     ).repeatable().reg(self);
        ecmd("insert-at-line-end",        "Enter insert mode after the last character on the line.",         cmd_insert_at_line_end       ).repeatable().reg(self);
        ecmd("insert-at-selection-start", "Enter insert mode at the start of the selection.",                cmd_insert_at_selection_start).repeatable().reg(self);
        ecmd("insert-at-selection-end",   "Enter insert mode after the end of the selection.",               cmd_insert_at_selection_end  ).repeatable().reg(self);
        ecmd("open-line-below",      "Open a new line below the cursor and enter insert mode.",         cmd_open_line_below     ).repeatable().reg(self);
        ecmd("open-line-above",      "Open a new line above the cursor and enter insert mode.",         cmd_open_line_above     ).repeatable().reg(self);
        ecmd("command-mode",         "Open the command-mode mini-buffer.",                              cmd_command_mode        ).reg(self);
        ecmd("exit-insert",          "Return to normal mode from insert mode.",                         cmd_exit_insert         ).reg(self);

        // ── Editor commands — edit composites ─────────────────────────────────
        ecmd("delete",       "Yank selections into the default register, then delete them.",              cmd_delete      ).repeatable().reg(self);
        ecmd("change",       "Yank, delete selections, then enter insert mode (one undo group).",         cmd_change      ).repeatable().reg(self);
        ecmd("yank",         "Yank selections into the default register without deleting.",               cmd_yank        ).reg(self);
        ecmd("paste-after",  "Paste register contents after the selection.",                              cmd_paste_after ).repeatable().reg(self);
        ecmd("paste-before", "Paste register contents before the selection.",                             cmd_paste_before).repeatable().reg(self);
        ecmd("undo",         "Undo the last change.",                                                     cmd_undo        ).reg(self);
        ecmd("redo",         "Redo the last undone change.",                                              cmd_redo        ).reg(self);

        // ── Editor commands — selection state ────────────────────────────────
        ecmd("toggle-extend",            "Toggle sticky extend mode.",                                              cmd_toggle_extend           ).reg(self);
        ecmd("collapse-and-exit-extend", "Collapse each selection to its cursor and exit extend mode.",             cmd_collapse_and_exit_extend).reg(self);

        // ── Editor commands — find / till (read pending_char) ─────────────────
        ecmd("find-forward",         "Find next occurrence of a character (inclusive, forward).",          cmd_find_forward        ).extendable().reg(self);
        ecmd("find-backward",        "Find previous occurrence of a character (inclusive, backward).",     cmd_find_backward       ).extendable().reg(self);
        ecmd("till-forward",         "Move to just before next occurrence of a character (exclusive).",    cmd_till_forward        ).extendable().reg(self);
        ecmd("till-backward",        "Move to just after previous occurrence of a character (exclusive).", cmd_till_backward       ).extendable().reg(self);
        ecmd("repeat-find-forward",  "Repeat the last find/till motion forward.",                         cmd_repeat_find_forward ).extendable().reg(self);
        ecmd("repeat-find-backward", "Repeat the last find/till motion backward.",                        cmd_repeat_find_backward).extendable().reg(self);

        // ── Editor commands — replace (reads pending_char) ───────────────────
        ecmd("replace", "Replace every character in each selection with the next typed character.", cmd_replace).repeatable().reg(self);

        // ── Editor commands — page scroll ─────────────────────────────────────
        ecmd("page-down", "Scroll down by one viewport height.", cmd_page_down).extendable().jump().reg(self);
        ecmd("page-up",   "Scroll up by one viewport height.",   cmd_page_up  ).extendable().jump().reg(self);

        // ── Editor commands — half-page scroll ────────────────────────────────
        ecmd("half-page-down", "Scroll down by half a viewport height.", cmd_half_page_down).extendable().reg(self);
        ecmd("half-page-up",   "Scroll up by half a viewport height.",   cmd_half_page_up  ).extendable().reg(self);

        // ── Editor commands — repeat ──────────────────────────────────────────
        // Not flagged repeatable: `.` repeating itself would be nonsensical.
        ecmd("repeat-last-action", "Repeat the last editing action.", cmd_repeat).reg(self);

        // ── Editor commands — search ──────────────────────────────────────────
        ecmd("search-forward",          "Enter search mode (forward).",                          cmd_search_forward         ).reg(self);
        ecmd("search-backward",         "Enter search mode (backward).",                         cmd_search_backward        ).reg(self);
        ecmd("search-next",             "Jump to the next search match.",                        cmd_search_next            ).extendable().jump().reg(self);
        ecmd("search-prev",             "Jump to the previous search match.",                    cmd_search_prev            ).extendable().jump().reg(self);
        ecmd("clear-search",            "Clear search highlights (`:clear-search` / `:cs`).",    cmd_clear_search           ).reg(self);

        // ── Editor commands — select ─────────────────────────────────────────
        ecmd("select-within",           "Select regex matches within current selections.",          cmd_select_within          ).reg(self);
        ecmd("select-all-matches",      "Turn every search match in the buffer into a selection.", cmd_select_all_matches     ).reg(self);
        ecmd("use-selection-as-search", "Use primary selection text as the search pattern.",       cmd_use_selection_as_search).reg(self);

        // ── Editor commands — jump list ──────────────────────────────────────
        ecmd("jump-backward", "Navigate to the previous position in the jump list.", cmd_jump_backward).reg(self);
        ecmd("jump-forward",  "Navigate to the next position in the jump list.",     cmd_jump_forward ).reg(self);

        // ── Editor commands — misc ────────────────────────────────────────────
        ecmd("force-quit", "Quit without checking for unsaved changes.", cmd_quit).reg(self);

        // ── Typed commands (`:` command line) ─────────────────────────────────
        macro_rules! typed_cmd {
            ($name:literal, $doc:literal, $aliases:expr, $fun:expr) => {
                self.register_typed(TypedCommand {
                    name: Cow::Borrowed($name),
                    doc: Cow::Borrowed($doc),
                    aliases: $aliases,
                    fun: $fun,
                })
            };
        }

        typed_cmd!("quit",             "Close the editor.",                                        &["q"],    typed_quit);
        typed_cmd!("write",            "Write changes to disk.",                                  &["w"],    typed_write);
        typed_cmd!("write-quit",       "Write changes and quit.",                                 &["wq"],   typed_write_quit);
        typed_cmd!("toggle-soft-wrap", "Toggle soft line wrapping.",                              &["wrap"], typed_toggle_soft_wrap);
        typed_cmd!("set",              "Set a configuration value: :set global|buffer key=value.", &[],     typed_set);
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
    /// Count breakdown (extend-variant commands removed; MotionMode is now a runtime param):
    ///   13 motions
    ///    2 line-selection motions (no count)
    ///   10 selection commands
    ///   22 text objects (2 line + 4 word + 8 bracket + 6 quote + 2 argument)
    ///    7 surround selection commands
    ///    3 edit commands
    ///    8 mode-transition editor commands
    ///    7 edit-composite editor commands
    ///    2 selection-state editor commands
    ///    6 find/till editor commands (4 + 2 repeat)
    ///    1 replace editor command
    ///    1 repeat-last-action editor command
    ///    5 search editor commands (search-forward/backward, search-next/prev, clear-search)
    ///    3 select editor commands (select-within, select-all-matches, use-selection-as-search)
    ///    2 page-scroll editor commands
    ///    2 half-page-scroll editor commands
    ///    2 jump-list editor commands
    ///    7 insert editor commands (insert-at-line-start/end, insert-at-selection-start/end, open-line-above/below, exit-insert)
    ///    1 force-quit editor command
    ///    5 typed commands (quit, write, write-quit, toggle-soft-wrap, set)
    ///  ──
    ///  107 total
    const EXPECTED_COMMAND_COUNT: usize = 107;

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
    fn mappable_lookup_by_name_works() {
        let reg = CommandRegistry::with_defaults();

        // Motion
        let cmd = reg.get_mappable("move-right").expect("move-right should be registered");
        assert_eq!(cmd.name(), "move-right");
        assert!(matches!(cmd, MappableCommand::Motion { .. }));

        // Selection
        let cmd = reg.get_mappable("collapse-selection").expect("collapse-selection should be registered");
        assert_eq!(cmd.name(), "collapse-selection");
        assert!(matches!(cmd, MappableCommand::Selection { .. }));

        // Edit
        let cmd = reg.get_mappable("delete-selection").expect("delete-selection should be registered");
        assert_eq!(cmd.name(), "delete-selection");
        assert!(matches!(cmd, MappableCommand::Edit { .. }));

        // EditorCmd
        let cmd = reg.get_mappable("force-quit").expect("force-quit should be registered");
        assert_eq!(cmd.name(), "force-quit");
        assert!(matches!(cmd, MappableCommand::EditorCmd { .. }));

        let cmd = reg.get_mappable("find-forward").expect("find-forward should be registered");
        assert!(matches!(cmd, MappableCommand::EditorCmd { .. }));

        let cmd = reg.get_mappable("delete").expect("delete should be registered");
        assert!(matches!(cmd, MappableCommand::EditorCmd { .. }));
    }

    #[test]
    fn typed_lookup_by_canonical_name() {
        let reg = CommandRegistry::with_defaults();
        let tc = reg.get_typed("write").expect("write should be a typed command");
        assert_eq!(tc.name, "write");
        assert!(!tc.doc.is_empty());
    }

    #[test]
    fn typed_lookup_by_alias() {
        let reg = CommandRegistry::with_defaults();
        assert_eq!(reg.get_typed("w").expect("w alias").name, "write");
        assert_eq!(reg.get_typed("q").expect("q alias").name, "quit");
        assert_eq!(reg.get_typed("wq").expect("wq alias").name, "write-quit");
        assert_eq!(reg.get_typed("wrap").expect("wrap alias").name, "toggle-soft-wrap");
    }

    #[test]
    fn typed_lookup_does_not_return_mappable() {
        let reg = CommandRegistry::with_defaults();
        // Mappable commands are not accessible via get_typed.
        assert!(reg.get_typed("move-right").is_none());
        assert!(reg.get_typed("force-quit").is_none());
        assert!(reg.get_typed("clear-search").is_none());
        assert!(reg.get_typed("select-all-matches").is_none());
    }

    #[test]
    fn mappable_lookup_does_not_return_typed() {
        let reg = CommandRegistry::with_defaults();
        // Typed commands are not accessible via get_mappable.
        assert!(reg.get_mappable("write").is_none());
        assert!(reg.get_mappable("quit").is_none());
        assert!(reg.get_mappable("write-quit").is_none());
    }

    #[test]
    fn unknown_name_returns_none() {
        let reg = CommandRegistry::with_defaults();
        assert!(reg.get_mappable("does-not-exist").is_none());
        assert!(reg.get_typed("does-not-exist").is_none());
    }

    #[test]
    fn doc_strings_are_stored_and_accessible() {
        let reg = CommandRegistry::with_defaults();
        let cmd = reg.get_mappable("move-right").unwrap();
        assert!(!cmd.doc().is_empty(), "move-right should have a non-empty doc string");
        let cmd = reg.get_mappable("delete-selection").unwrap();
        assert!(!cmd.doc().is_empty(), "delete-selection should have a non-empty doc string");
        let tc = reg.get_typed("write").unwrap();
        assert!(!tc.doc.is_empty(), "write should have a non-empty doc string");
    }

    #[test]
    fn is_extendable_motion_and_selection_always_true() {
        let reg = CommandRegistry::with_defaults();
        // All Motion commands are extendable.
        for name in ["move-right", "move-left", "move-down", "move-up",
                     "goto-first-line", "goto-last-line",
                     "goto-line-start", "goto-line-end", "goto-first-nonblank",
                     "select-next-word", "select-prev-word",
                     "next-paragraph", "prev-paragraph"] {
            let cmd = reg.get_mappable(name).unwrap_or_else(|| panic!("{name} not found"));
            assert!(cmd.is_extendable(), "Motion '{name}' should be extendable");
        }
        // All Selection commands are extendable.
        for name in ["select-line", "select-line-backward",
                     "collapse-selection", "flip-selections",
                     "inner-word", "around-word",
                     "inner-paren", "around-paren"] {
            let cmd = reg.get_mappable(name).unwrap_or_else(|| panic!("{name} not found"));
            assert!(cmd.is_extendable(), "Selection '{name}' should be extendable");
        }
    }

    #[test]
    fn is_extendable_editor_cmd_true_for_extendable() {
        let reg = CommandRegistry::with_defaults();
        // EditorCmds marked extendable: true.
        for name in ["find-forward", "find-backward",
                     "till-forward", "till-backward",
                     "repeat-find-forward", "repeat-find-backward",
                     "page-down", "page-up",
                     "half-page-down", "half-page-up",
                     "search-next", "search-prev",
                     "move-down", "move-up"] {
            let cmd = reg.get_mappable(name).unwrap_or_else(|| panic!("{name} not found"));
            assert!(cmd.is_extendable(), "EditorCmd '{name}' should be extendable");
        }
    }

    #[test]
    fn is_extendable_false_for_edits_and_non_extendable_editor_cmds() {
        let reg = CommandRegistry::with_defaults();
        // Edit commands are never extendable.
        for name in ["delete-selection", "delete-char-forward", "delete-char-backward"] {
            let cmd = reg.get_mappable(name).unwrap_or_else(|| panic!("{name} not found"));
            assert!(!cmd.is_extendable(), "Edit '{name}' should not be extendable");
        }
        // Non-extendable EditorCmds.
        for name in ["undo", "redo", "insert-before", "insert-after",
                     "open-line-below", "open-line-above",
                     "force-quit", "exit-insert"] {
            let cmd = reg.get_mappable(name).unwrap_or_else(|| panic!("{name} not found"));
            assert!(!cmd.is_extendable(), "EditorCmd '{name}' should not be extendable");
        }
    }

    #[test]
    fn all_names_are_unique() {
        // HashMap insertion silently overwrites duplicates — verify the final
        // count matches the number of distinct registered names.
        let reg = CommandRegistry::with_defaults();
        let unique: std::collections::HashSet<&str> = reg.names().collect();
        assert_eq!(unique.len(), reg.len(), "duplicate command names detected");
    }

    #[test]
    fn runtime_register_and_lookup() {
        use crate::editor::Editor;
        let mut reg = CommandRegistry::with_defaults();
        let before = reg.len();

        fn dummy_fn(_ed: &mut Editor, _count: usize, _mode: crate::ops::MotionMode) -> Result<(), crate::core::error::CommandError> { Ok(()) }
        let cmd = MappableCommand::EditorCmd {
            name: Cow::Owned("steel-test-cmd".to_string()),
            doc: Cow::Borrowed("A dummy Steel command for testing."),
            fun: dummy_fn,
            repeatable: false,
            jump: false,
            visual_move: false,
            extendable: false,
        };
        reg.register(cmd);

        assert_eq!(reg.len(), before + 1);
        assert!(reg.get_mappable("steel-test-cmd").is_some());
        assert_eq!(reg.get_mappable("steel-test-cmd").unwrap().name(), "steel-test-cmd");
    }


    #[test]
    fn mappable_commands_not_shadowed_by_typed() {
        // Mappable commands like clear-search and select-all-matches must remain
        // accessible as mappable so keybinds continue to work. The command line
        // reaches them via the fallback in execute_command, not via get_typed.
        let reg = CommandRegistry::with_defaults();
        assert!(reg.get_mappable("clear-search").is_some());
        assert!(reg.get_mappable("select-all-matches").is_some());
    }

    #[test]
    fn runtime_register_typed_and_lookup() {
        use crate::editor::Editor;
        let mut reg = CommandRegistry::with_defaults();
        let before = reg.len();

        fn dummy_typed(_ed: &mut Editor, _arg: Option<&str>, _force: bool) -> Result<(), crate::core::error::CommandError> { Ok(()) }
        reg.register_typed(TypedCommand {
            name: Cow::Owned("steel-typed-cmd".to_string()),
            doc: Cow::Borrowed("A dummy Steel typed command for testing."),
            aliases: &["stc"],
            fun: dummy_typed,
        });

        assert_eq!(reg.len(), before + 1);
        // Reachable by canonical name.
        assert_eq!(reg.get_typed("steel-typed-cmd").unwrap().name, "steel-typed-cmd");
        // Reachable by alias.
        assert_eq!(reg.get_typed("stc").unwrap().name, "steel-typed-cmd");
        // Not reachable as a mappable command.
        assert!(reg.get_mappable("steel-typed-cmd").is_none());
    }
}
