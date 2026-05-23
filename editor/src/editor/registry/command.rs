use std::borrow::Cow;

use crate::core::changeset::ChangeSet;
use crate::core::error::CommandError;
use crate::core::selection::SelectionSet;
use crate::core::text::Text;
use crate::ops::MotionMode;

// ── MappableCommand ───────────────────────────────────────────────────────────

/// A command that can be bound to a key in a keymap.
///
/// The keymap trie stores command *names*; the registry resolves names to
/// `MappableCommand` values at dispatch time.
#[derive(Clone)]
pub(crate) enum MappableCommand {
    /// Motion that repeats `count` times.
    ///
    /// Signature: `fn(&Text, SelectionSet, usize, MotionMode) -> SelectionSet`
    ///
    /// Motions are always extendable. The `mode` parameter selects Move or Extend
    /// semantics at dispatch time — no separate extend-variant functions needed.
    Motion {
        name: Cow<'static, str>,
        // Pending command-palette / :help integration.
        #[allow(dead_code)]
        doc: Cow<'static, str>,
        fun: fn(&Text, SelectionSet, usize, MotionMode) -> SelectionSet,
        /// Whether this motion always records a jump list entry before executing,
        /// regardless of how far the cursor moves. Used for goto commands.
        jump: bool,
    },
    /// Selection or text-object operation (no count).
    ///
    /// Signature: `fn(&Text, SelectionSet, MotionMode) -> SelectionSet`
    ///
    /// All selection commands receive `MotionMode`. Non-extendable ones accept
    /// `_mode` and ignore it; extendable text objects branch on it.
    Selection {
        name: Cow<'static, str>,
        // Pending command-palette / :help integration.
        #[allow(dead_code)]
        doc: Cow<'static, str>,
        fun: fn(&Text, SelectionSet, MotionMode) -> SelectionSet,
    },
    /// Text-modifying edit with no extra arguments.
    ///
    /// Signature: `fn(Text, SelectionSet) -> (Text, SelectionSet, ChangeSet)`
    ///
    /// Edits are never extendable — they don't carry `MotionMode`.
    Edit {
        name: Cow<'static, str>,
        // Pending command-palette / :help integration.
        #[allow(dead_code)]
        doc: Cow<'static, str>,
        fun: fn(Text, SelectionSet) -> (Text, SelectionSet, ChangeSet),
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
        // Pending command-palette / :help integration.
        #[allow(dead_code)]
        doc: Cow<'static, str>,
        fun: fn(&mut super::super::Editor, usize, MotionMode) -> Result<(), CommandError>,
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
    /// A command implemented as a Steel (Scheme) lambda.
    ///
    /// `steel_proc` is the name under which the lambda is registered in the
    /// Steel engine's global namespace (e.g. `"%hume-cmd-my-command"`).
    /// Dispatched by [`crate::scripting::ScriptingHost::call_steel_cmd`], which
    /// evaluates `(steel_proc)` and drains the resulting `steel_ctx.cmd_queue`.
    ///
    /// Not repeatable, not jump, not visual-line — these can be added as
    /// optional flags once the use-cases emerge.
    SteelBacked {
        name: Cow<'static, str>,
        // Pending command-palette / :help integration.
        #[allow(dead_code)]
        doc: Cow<'static, str>,
        /// Name of the lambda in Steel's global namespace.
        steel_proc: String,
        /// If `true`, this command participates in sticky-Ctrl one-shot extend
        /// (strip-Ctrl fallback).  Set via `(define-command-extend! …)`.
        extendable: bool,
        /// Number of required positional parameters.
        arity: u16,
        /// `true` if the lambda accepts a rest parameter.
        is_variadic: bool,
        /// `true` if dispatch should bracket the call with an alt-screen exit
        /// so subprocess output streams live to the terminal.
        inline_output: bool,
    },
    /// A placeholder for a lazy plugin command that has not yet been loaded.
    ///
    /// Registered by `register_lazy_command_stubs` after `init_scripting`.
    /// When dispatched, the owning plugin's body is evaluated, the stub is
    /// replaced by the real `SteelBacked` command, and dispatch re-runs.
    Lazy {
        name: Cow<'static, str>,
        plugin: crate::scripting::attribution::PluginId,
    },
}

impl MappableCommand {
    #[cfg(test)]
    pub(crate) fn name(&self) -> &str {
        match self {
            Self::Motion { name, .. }
            | Self::Selection { name, .. }
            | Self::Edit { name, .. }
            | Self::EditorCmd { name, .. }
            | Self::SteelBacked { name, .. }
            | Self::Lazy { name, .. } => name.as_ref(),
        }
    }

    /// One-line description of the command, for `:help` and command-palette display.
    #[cfg(test)]
    pub(crate) fn doc(&self) -> &str {
        match self {
            Self::Motion { doc, .. }
            | Self::Selection { doc, .. }
            | Self::Edit { doc, .. }
            | Self::EditorCmd { doc, .. }
            | Self::SteelBacked { doc, .. } => doc.as_ref(),
            Self::Lazy { .. } => "",
        }
    }

    /// Returns `true` if this command should be recorded for `.` repeat.
    ///
    /// Motions and selections are never repeatable — they don't mutate the
    /// buffer. Edit and EditorCmd commands opt in explicitly at registration.
    pub(crate) fn is_repeatable(&self) -> bool {
        match self {
            Self::Motion { .. }
            | Self::Selection { .. }
            | Self::SteelBacked { .. }
            | Self::Lazy { .. } => false,
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
            Self::Selection { .. }
            | Self::Edit { .. }
            | Self::SteelBacked { .. }
            | Self::Lazy { .. } => false,
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
            Self::Edit { .. } | Self::Lazy { .. } => false,
            Self::SteelBacked { extendable, .. } => *extendable,
            Self::EditorCmd { extendable, .. } => *extendable,
        }
    }
}

// ── TypedCommand ──────────────────────────────────────────────────────────────

/// A command invocable from the `:` command line.
///
/// Typed commands have a canonical name and optional short aliases. They are
/// stored in [`super::CommandRegistry`] alongside [`MappableCommand`] entries in a
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
    pub fun: fn(&mut super::super::Editor, Option<&str>, bool) -> Result<(), CommandError>,
}
