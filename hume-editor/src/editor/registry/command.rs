use std::borrow::Cow;

use hume_editing::changeset::ChangeSet;
use crate::editor::error::CommandError;
use hume_editing::selection::SelectionSet;
use hume_editing::text::Text;
use hume_engine::pipeline::EngineView;
use crate::ops::MotionMode;

// ── Command metadata for dispatch bookkeeping ────────────────────────────────

/// Semantic category of a command — drives which bookkeeping stages run.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CmdCategory {
    /// Cursor movement. Tracks the selection recipe.
    Motion,
    /// Selection builder. Tracks the selection recipe.
    Selection,
    /// Buffer-modifying edit. Snapshots dot-repeat when `CmdMeta.repeatable` is true.
    Edit,
    /// Paste-family command (p, P, [, ]).
    Paste { family: PasteFamily },
    /// Editor action (undo, redo, mode changes, …). Clears selection recipe.
    EditorAction,
    /// Steel-backed or Lazy command. Dot-repeat opt-in via `CmdMeta.repeatable`.
    Lazy,
}

/// Distinguishes normal paste from ring-cycle commands.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PasteFamily {
    /// p / P — commit paste session before next command.
    Normal,
    /// [ / ] — must NOT commit paste session (cycles fold into one undo step).
    RingCycle,
}

impl CmdCategory {
    /// Whether this command type should update the selection recipe.
    pub(crate) fn tracks_selection(&self) -> bool {
        matches!(self, CmdCategory::Motion | CmdCategory::Selection)
    }
}

/// Declarative metadata extracted from a MappableCommand variant.
///
/// Drives the dispatch pipeline — the pipeline reads this instead of matching
/// on variant type or checking string sets.
#[derive(Debug, Clone)]
pub(crate) struct CmdMeta {
    /// Command name (for `last_command` stamping, dot-repeat recording).
    pub name: Cow<'static, str>,
    /// Semantic category — drives bookkeeping stages.
    pub category: CmdCategory,
    /// Whether this command always records a jump-list entry before executing,
    /// regardless of how far the cursor moves (goto / search / page-scroll /
    /// `select-all`).
    ///
    /// Single source of truth for jump-command classification — there is no
    /// parallel `JUMP_COMMANDS` list, and the dispatch pipeline reads this rather
    /// than matching on the command variant.
    pub is_jump: bool,
    /// Whether this command is a visual-line motion (`move-down`/`move-up`).
    /// The preferred display column is preserved across consecutive visual-line
    /// moves and cleared for any other command.
    pub is_visual_move: bool,
    /// Whether `.` should replay this command.
    pub repeatable: bool,
    /// Whether dispatching this command overwrites `last_command`.
    ///
    /// `false` for `exit-insert` only — it closes an insert session that a kill
    /// (`c`) may have opened, so stamping it would clobber the `"change"` marker
    /// and break `c <text> Esc p` → ring. Set at registration via
    /// `.transparent_to_last_command()` on the `EditorCmdBuilder`. All other
    /// commands are `true`.
    pub stamps_last_command: bool,
}

/// Function pointer for an [`EditorCmd`] handler.
///
/// All handlers share one shape: `(&mut EditorState, &mut EngineView, usize, MotionMode)`.
/// Handlers that need no viewport access bind the view parameter as `_view`.
/// This single shape is synchronous, Steel-eval-safe (no `&mut Editor` needed),
/// and reachable from both the keypress path and `run_command_sync`.
///
/// [`EditorCmd`]: MappableCommand::EditorCmd
pub(crate) type EditorCmdFn = fn(
    &mut super::super::EditorState,
    &mut EngineView,
    usize,
    MotionMode,
) -> Result<(), CommandError>;

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
        /// Whether this command always records a jump list entry before executing,
        /// regardless of how far the cursor moves. Used for `select-all` (`%`).
        jump: bool,
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
    /// Editor-level command operating on `EditorState` + `EngineView`.
    ///
    /// Signature: `fn(&mut EditorState, &mut EngineView, usize, MotionMode) -> Result<(), CommandError>`
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
        fun: EditorCmdFn,
        /// Semantic category used by the dispatch pipeline instead of
        /// string-matching on the command name.
        category: CmdCategory,
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
        /// Whether dispatching this command overwrites `last_command` for smart-p.
        /// `false` only for `exit-insert`; all other commands are `true`.
        stamps_last_command: bool,
    },
    /// A command implemented as a Steel (Scheme) lambda.
    ///
    /// Dispatched by [`hume_scripting::ScriptingHost::call_steel_cmd`], which
    /// routes through `%dispatch-command` → `command_table` → `(apply proc args)`.
    ///
    /// All Steel commands are extendable (Ctrl+key delivers `extend = #t` to the
    /// lambda body). Dot-repeat is opt-in via `#:repeatable #t` in `(define-command! …)`.
    SteelBacked {
        name: Cow<'static, str>,
        // Pending command-palette / :help integration.
        #[allow(dead_code)]
        doc: Cow<'static, str>,
        /// Number of required positional parameters.
        ///
        /// Used at dispatch time to inject the right number of `[count, extend]`
        /// leading args (0, 1, or 2) when the keymap calls the command with no
        /// explicit arguments. Introspected from the closure arity at define time.
        arity: u16,
        /// `true` if the lambda accepts a rest parameter.
        is_variadic: bool,
        /// `true` if dispatch should bracket the call with an alt-screen exit
        /// so subprocess output streams live to the terminal.
        inline_output: bool,
        /// `true` if pressing `.` should replay this command.
        ///
        /// Opt in via `#:repeatable #t` in `(define-command! …)`.
        /// Never `true` when `inline_output` is `true` — enforced at definition time.
        repeatable: bool,
    },
    /// A placeholder for a lazy plugin command that has not yet been loaded.
    ///
    /// Registered by `register_lazy_command_stubs` after `init_scripting`.
    /// When dispatched, the owning plugin's body is evaluated, the stub is
    /// replaced by the real `SteelBacked` command, and dispatch re-runs.
    Lazy {
        name: Cow<'static, str>,
        plugin: hume_scripting::attribution::PluginId,
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

    /// Declarative metadata for the dispatch pipeline.
    ///
    /// This is the single source of truth for bookkeeping properties. The
    /// pipeline reads `CmdMeta` — it never matches on variant or checks
    /// string sets to decide what bookkeeping to run.
    pub(crate) fn meta(&self) -> CmdMeta {
        match self {
            Self::Motion { name, jump, .. } => CmdMeta {
                name: name.clone(),
                category: CmdCategory::Motion,
                is_jump: *jump,
                is_visual_move: false,
                repeatable: false,
                stamps_last_command: true,
            },
            Self::Selection { name, jump, .. } => CmdMeta {
                name: name.clone(),
                category: CmdCategory::Selection,
                is_jump: *jump,
                is_visual_move: false,
                repeatable: false,
                stamps_last_command: true,
            },
            Self::Edit { name, repeatable, .. } => CmdMeta {
                name: name.clone(),
                category: CmdCategory::Edit,
                is_jump: false,
                is_visual_move: false,
                repeatable: *repeatable,
                stamps_last_command: true,
            },
            Self::EditorCmd { name, category, repeatable, jump, visual_move, stamps_last_command, .. } => CmdMeta {
                name: name.clone(),
                category: *category,
                is_jump: *jump,
                is_visual_move: *visual_move,
                repeatable: *repeatable,
                stamps_last_command: *stamps_last_command,
            },
            Self::SteelBacked { name, repeatable, .. } => CmdMeta {
                name: name.clone(),
                category: CmdCategory::Lazy,
                is_jump: false,
                is_visual_move: false,
                repeatable: *repeatable,
                stamps_last_command: true,
            },
            Self::Lazy { name, .. } => CmdMeta {
                name: name.clone(),
                category: CmdCategory::Lazy,
                is_jump: false,
                is_visual_move: false,
                repeatable: false,
                stamps_last_command: true,
            },
        }
    }

    /// Returns `true` if this command executes synchronously in Rust
    /// (`Motion`/`Selection`/`Edit`/`EditorCmd`) rather than through the Steel
    /// dispatch queue (`SteelBacked`/`Lazy`).
    ///
    /// Single source of truth for native-vs-scripted classification: the `%call-native!`
    /// sync-dispatch gate, `run_command_sync`, and bare-binding registration all
    /// derive from this. The match is intentionally exhaustive (no `_`) so a new
    /// variant forces a decision here at compile time.
    pub(crate) fn is_native(&self) -> bool {
        match self {
            Self::Motion { .. }
            | Self::Selection { .. }
            | Self::Edit { .. }
            | Self::EditorCmd { .. } => true,
            Self::SteelBacked { .. } | Self::Lazy { .. } => false,
        }
    }

    /// Returns `true` if this command has extend semantics and can be triggered
    /// as a one-shot extend via Ctrl+key.
    ///
    /// Motion and Selection are always extendable. Edit is never extendable.
    /// EditorCmd has an explicit flag set at registration time.
    /// Steel commands (SteelBacked and Lazy stubs) are always extendable —
    /// the resolved lambda receives `extend` as its second arg.
    pub(crate) fn is_extendable(&self) -> bool {
        match self {
            Self::Motion { .. }
            | Self::Selection { .. }
            | Self::SteelBacked { .. }
            | Self::Lazy { .. } => true,
            Self::Edit { .. } => false,
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
