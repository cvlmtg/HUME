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
//! # Extend-mode pairing (mappable commands only)
//!
//! Each mappable command can declare an extend variant inline at registration
//! time via an `extend: "variant-name"` argument. When extend mode is active,
//! the dispatcher calls [`CommandRegistry::extend_variant`] to swap in the
//! extend variant — the keymap stores only base command names.
//!
//! # Mappable command variants
//!
//! 1. **Motion** — pure `fn(&Buffer, SelectionSet, usize) -> SelectionSet`
//! 2. **Selection** — pure `fn(&Buffer, SelectionSet) -> SelectionSet`
//! 3. **Edit** — pure `fn(Buffer, SelectionSet) -> (Buffer, SelectionSet, ChangeSet)`
//! 4. **EditorCmd** — `fn(&mut Editor, usize)` for composite/side-effectful
//!    operations (mode changes, registers, undo groups, parameterized motions).
//!    Implemented in `editor/commands.rs`; stored and dispatched as a
//!    function pointer exactly like the other variants.

use std::borrow::Cow;
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
#[derive(Clone)]
pub(crate) enum MappableCommand {
    /// Motion that repeats `count` times.
    ///
    /// Signature: `fn(&Buffer, SelectionSet, usize) -> SelectionSet`
    ///
    /// Motions never mutate the buffer, so `repeatable` is always `false`.
    Motion {
        name: Cow<'static, str>,
        doc: Cow<'static, str>,
        fun: fn(&Buffer, SelectionSet, usize) -> SelectionSet,
        /// Whether this motion always records a jump list entry before executing,
        /// regardless of how far the cursor moves. Used for goto commands.
        jump: bool,
    },
    /// Selection or text-object operation (no count).
    ///
    /// Signature: `fn(&Buffer, SelectionSet) -> SelectionSet`
    ///
    /// Pure selection ops never mutate the buffer, so `repeatable` is always `false`.
    Selection {
        name: Cow<'static, str>,
        doc: Cow<'static, str>,
        fun: fn(&Buffer, SelectionSet) -> SelectionSet,
    },
    /// Buffer-modifying edit with no extra arguments.
    ///
    /// Signature: `fn(Buffer, SelectionSet) -> (Buffer, SelectionSet, ChangeSet)`
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
    /// Signature: `fn(&mut Editor, usize)`
    ///
    /// Covers composite operations: mode changes, register access, undo group
    /// management, and parameterized motions (find/till/replace). Stored and
    /// dispatched as a function pointer exactly like the other variants —
    /// `fn(&mut Editor, usize)` is a thin pointer so there is no self-referential
    /// sizing issue despite `Editor` owning the registry.
    EditorCmd {
        name: Cow<'static, str>,
        doc: Cow<'static, str>,
        fun: fn(&mut super::Editor, usize),
        /// Whether `.` should replay this command.
        repeatable: bool,
        /// Whether this command always records a jump list entry before executing.
        /// Used for search jumps and explicit page-scroll commands.
        jump: bool,
        /// Whether this command is a visual-line motion (move-down/up, extend-down/up).
        /// The preferred display column is preserved across consecutive visual-line moves.
        visual_move: bool,
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
    pub doc: Cow<'static, str>,
    /// Short aliases, e.g. `&["w"]`. Each alias is also registered in the
    /// alias index for O(1) lookup. Empty for commands with no alias.
    ///
    /// `&'static [&'static str]` covers all built-in commands. Steel-registered
    /// typed commands pass `&[]` and register aliases separately if needed.
    pub aliases: &'static [&'static str],
    /// The function to execute. Receives the editor, an optional argument
    /// (e.g. a file path), and whether `!` was appended.
    pub fun: fn(&mut super::Editor, Option<&str>, bool),
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
    /// Maps a base mappable-command name to its extend variant.
    ///
    /// When extend mode is active, the dispatcher looks up the base command
    /// here and dispatches the extend variant instead. This is the single
    /// source of truth for extend pairing — the keymap stores only base names.
    extend_map: HashMap<Cow<'static, str>, Cow<'static, str>>,
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
            extend_map: HashMap::new(),
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

    /// Look up the extend variant for a mappable command, if one exists.
    ///
    /// Returns an owned `Cow<'static, str>` so the caller is not bound by
    /// the registry's borrow lifetime. For static entries the clone is a
    /// pointer copy (zero allocation).
    pub(crate) fn extend_variant(&self, name: &str) -> Option<Cow<'static, str>> {
        self.extend_map.get(name).cloned()
    }

    /// Register a base → extend-variant pair at runtime.
    ///
    /// Used by Steel's `define-mappable-command!` + extend-mode support.
    #[allow(dead_code)]
    pub(crate) fn register_extend_pair(&mut self, base: Cow<'static, str>, variant: Cow<'static, str>) {
        self.extend_map.insert(base, variant);
    }

    fn register_defaults(&mut self) {
        // Local macros to cut down on struct-literal boilerplate.
        // The optional `extend: "name"` trailing argument links this command to
        // its extend variant in the extend_map (single source of truth).
        macro_rules! motion {
            ($name:literal, $doc:literal, $fun:expr, extend: $ext:literal, jump) => {{
                self.register(MappableCommand::Motion { name: Cow::Borrowed($name), doc: Cow::Borrowed($doc), fun: $fun, jump: true });
                self.extend_map.insert(Cow::Borrowed($name), Cow::Borrowed($ext));
            }};
            ($name:literal, $doc:literal, $fun:expr, extend: $ext:literal) => {{
                self.register(MappableCommand::Motion { name: Cow::Borrowed($name), doc: Cow::Borrowed($doc), fun: $fun, jump: false });
                self.extend_map.insert(Cow::Borrowed($name), Cow::Borrowed($ext));
            }};
            ($name:literal, $doc:literal, $fun:expr) => {
                self.register(MappableCommand::Motion { name: Cow::Borrowed($name), doc: Cow::Borrowed($doc), fun: $fun, jump: false })
            };
        }
        macro_rules! selection {
            ($name:literal, $doc:literal, $fun:expr, extend: $ext:literal) => {{
                self.register(MappableCommand::Selection { name: Cow::Borrowed($name), doc: Cow::Borrowed($doc), fun: $fun });
                self.extend_map.insert(Cow::Borrowed($name), Cow::Borrowed($ext));
            }};
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
        macro_rules! editor_cmd {
            ($name:literal, $doc:literal, $fun:expr, repeatable, extend: $ext:literal) => {{
                self.register(MappableCommand::EditorCmd { name: Cow::Borrowed($name), doc: Cow::Borrowed($doc), fun: $fun, repeatable: true, jump: false, visual_move: false });
                self.extend_map.insert(Cow::Borrowed($name), Cow::Borrowed($ext));
            }};
            ($name:literal, $doc:literal, $fun:expr, extend: $ext:literal, jump) => {{
                self.register(MappableCommand::EditorCmd { name: Cow::Borrowed($name), doc: Cow::Borrowed($doc), fun: $fun, repeatable: false, jump: true, visual_move: false });
                self.extend_map.insert(Cow::Borrowed($name), Cow::Borrowed($ext));
            }};
            ($name:literal, $doc:literal, $fun:expr, jump) => {
                self.register(MappableCommand::EditorCmd { name: Cow::Borrowed($name), doc: Cow::Borrowed($doc), fun: $fun, repeatable: false, jump: true, visual_move: false })
            };
            ($name:literal, $doc:literal, $fun:expr, extend: $ext:literal, visual_move) => {{
                self.register(MappableCommand::EditorCmd { name: Cow::Borrowed($name), doc: Cow::Borrowed($doc), fun: $fun, repeatable: false, jump: false, visual_move: true });
                self.extend_map.insert(Cow::Borrowed($name), Cow::Borrowed($ext));
            }};
            ($name:literal, $doc:literal, $fun:expr, visual_move) => {
                self.register(MappableCommand::EditorCmd { name: Cow::Borrowed($name), doc: Cow::Borrowed($doc), fun: $fun, repeatable: false, jump: false, visual_move: true })
            };
            ($name:literal, $doc:literal, $fun:expr, extend: $ext:literal) => {{
                self.register(MappableCommand::EditorCmd { name: Cow::Borrowed($name), doc: Cow::Borrowed($doc), fun: $fun, repeatable: false, jump: false, visual_move: false });
                self.extend_map.insert(Cow::Borrowed($name), Cow::Borrowed($ext));
            }};
            ($name:literal, $doc:literal, $fun:expr, repeatable) => {
                self.register(MappableCommand::EditorCmd { name: Cow::Borrowed($name), doc: Cow::Borrowed($doc), fun: $fun, repeatable: true, jump: false, visual_move: false })
            };
            ($name:literal, $doc:literal, $fun:expr) => {
                self.register(MappableCommand::EditorCmd { name: Cow::Borrowed($name), doc: Cow::Borrowed($doc), fun: $fun, repeatable: false, jump: false, visual_move: false })
            };
        }

        // ── Character motions ─────────────────────────────────────────────────
        motion!("move-right", "Move cursors one grapheme to the right.", cmd_move_right, extend: "extend-right");
        motion!("move-left",  "Move cursors one grapheme to the left.",  cmd_move_left,  extend: "extend-left");
        editor_cmd!("move-down",  "Move cursors down one visual line.",   cmd_visual_move_down,   extend: "extend-down", visual_move);
        editor_cmd!("move-up",    "Move cursors up one visual line.",     cmd_visual_move_up,     extend: "extend-up",   visual_move);
        motion!("extend-right", "Extend selections one grapheme to the right.", cmd_extend_right);
        motion!("extend-left",  "Extend selections one grapheme to the left.",  cmd_extend_left);
        editor_cmd!("extend-down",  "Extend selections down one visual line.", cmd_visual_extend_down, visual_move);
        editor_cmd!("extend-up",    "Extend selections up one visual line.",   cmd_visual_extend_up,   visual_move);

        // ── Buffer-level goto motions ─────────────────────────────────────────
        motion!("goto-first-line", "Move cursors to the first character of the buffer.",     cmd_goto_first_line, extend: "extend-first-line", jump);
        motion!("goto-last-line",  "Move cursors to the first character of the last line.",  cmd_goto_last_line,  extend: "extend-last-line",  jump);
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
        editor_cmd!("page-down", "Scroll down by one viewport height.",            cmd_page_down, extend: "extend-page-down", jump);
        editor_cmd!("page-up",  "Scroll up by one viewport height.",              cmd_page_up,   extend: "extend-page-up",   jump);
        editor_cmd!("extend-page-down", "Extend selections down by one viewport height.", cmd_extend_page_down, jump);
        editor_cmd!("extend-page-up",   "Extend selections up by one viewport height.",   cmd_extend_page_up,   jump);

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
        editor_cmd!("search-next", "Jump to the next search match.",             cmd_search_next, extend: "extend-search-next", jump);
        editor_cmd!("search-prev", "Jump to the previous search match.",       cmd_search_prev, extend: "extend-search-prev", jump);
        editor_cmd!("extend-search-next", "Extend selection to the next search match.",     cmd_extend_search_next, jump);
        editor_cmd!("extend-search-prev", "Extend selection to the previous search match.", cmd_extend_search_prev, jump);
        editor_cmd!("clear-search",          "Clear search highlights (`:clear-search` / `:cs`).",      cmd_clear_search);

        // ── Editor commands — select ─────────────────────────────────────────
        editor_cmd!("select-within",          "Select regex matches within current selections.",          cmd_select_within);
        editor_cmd!("select-all-matches",     "Turn every search match in the buffer into a selection.", cmd_select_all_matches);
        editor_cmd!("use-selection-as-search", "Use primary selection text as the search pattern.",      cmd_use_selection_as_search);

        // ── Editor commands — jump list ──────────────────────────────────────
        editor_cmd!("jump-backward", "Navigate to the previous position in the jump list.", cmd_jump_backward);
        editor_cmd!("jump-forward",  "Navigate to the next position in the jump list.",     cmd_jump_forward);

        // ── Editor commands — misc ────────────────────────────────────────────
        editor_cmd!("force-quit", "Quit without checking for unsaved changes.", cmd_quit);

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
    ///    4 half-page-scroll editor commands
    ///    2 jump-list editor commands
    ///    5 insert editor commands (insert-at-line-start/end, open-line-above/below, exit-insert)
    ///    1 force-quit editor command
    ///    5 typed commands (quit, write, write-quit, toggle-soft-wrap, set)
    ///
    const EXPECTED_COMMAND_COUNT: usize = 156;

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
    ///    2 half-page scroll (down/up)
    ///    2 search (next/prev)
    ///    1 special (open-line-below → flip-selections)
    ///   ──
    ///   52 total
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
        for (base, extend) in &reg.extend_map {
            assert!(
                reg.get_mappable(base.as_ref()).is_some(),
                "extend pair: base command '{base}' not in registry"
            );
            assert!(
                reg.get_mappable(extend.as_ref()).is_some(),
                "extend pair: extend command '{extend}' not in registry"
            );
        }
    }

    #[test]
    fn extend_variant_lookup() {
        let reg = CommandRegistry::with_defaults();
        assert_eq!(reg.extend_variant("move-left").as_deref(), Some("extend-left"));
        assert_eq!(reg.extend_variant("select-next-word").as_deref(), Some("extend-select-next-word"));
        assert_eq!(reg.extend_variant("open-line-below").as_deref(), Some("flip-selections"));
        assert_eq!(reg.extend_variant("delete"), None);
        assert_eq!(reg.extend_variant("undo"), None);
        assert_eq!(reg.extend_variant("insert-before"), None);
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

        fn dummy_fn(_ed: &mut Editor, _count: usize) {}
        let cmd = MappableCommand::EditorCmd {
            name: Cow::Owned("steel-test-cmd".to_string()),
            doc: Cow::Borrowed("A dummy Steel command for testing."),
            fun: dummy_fn,
            repeatable: false,
            jump: false,
            visual_move: false,
        };
        reg.register(cmd);

        assert_eq!(reg.len(), before + 1);
        assert!(reg.get_mappable("steel-test-cmd").is_some());
        assert_eq!(reg.get_mappable("steel-test-cmd").unwrap().name(), "steel-test-cmd");
    }

    #[test]
    fn register_extend_pair_dynamic() {
        let mut reg = CommandRegistry::with_defaults();

        reg.register_extend_pair(
            Cow::Borrowed("move-left"),
            Cow::Owned("custom-extend-left".to_string()),
        );
        // Overwrites the built-in extend variant for move-left.
        assert_eq!(
            reg.extend_variant("move-left").as_deref(),
            Some("custom-extend-left"),
        );

        // A brand-new pair.
        reg.register_extend_pair(
            Cow::Owned("steel-cmd".to_string()),
            Cow::Owned("steel-cmd-extend".to_string()),
        );
        assert_eq!(
            reg.extend_variant("steel-cmd").as_deref(),
            Some("steel-cmd-extend"),
        );
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

        fn dummy_typed(_ed: &mut Editor, _arg: Option<&str>, _force: bool) {}
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
