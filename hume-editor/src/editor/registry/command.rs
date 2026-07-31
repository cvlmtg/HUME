use std::borrow::Cow;

use crate::editor::error::CommandError;
use hume_editing::changeset::ChangeSet;
use hume_editing::selection::SelectionSet;
use hume_editing::text::Text;
use hume_engine::pipeline::EngineView;
use hume_ops::MotionMode;

// ── Command metadata for dispatch bookkeeping ────────────────────────────────

/// Declarative metadata extracted from a MappableCommand variant.
///
/// Drives the dispatch pipeline — the pipeline reads this instead of matching
/// on variant type or checking string sets.
///
/// Each field is one independent behavioral aspect read by exactly one dispatch
/// stage. This is a flat set of orthogonal flags; there is no category enum.
/// Adding a new behavior = add one field here + one `step_*` function that reads
/// it — no existing fields or call sites are affected.
///
/// `Copy` and name-free on purpose: the variant→property mapping lives in
/// `MappableCommand::meta()` and nowhere else, so `meta()` must be cheap enough
/// that no caller is ever tempted to re-`match` the variant for a single bit
/// (which would fork the SSOT). The command name is owned data — it is read
/// separately via [`MappableCommand::name`] and cloned once per dispatch by the
/// pipeline, not carried in here.
#[derive(Debug, Clone, Copy)]
pub(crate) struct CmdMeta {
    /// Whether this command updates the selection recipe after it runs.
    ///
    /// `true` for Motion and Selection variants. The recipe accumulates the
    /// sequence of selection-building steps so dot-repeat can re-establish the
    /// selection before replaying an edit. All other commands clear the recipe.
    pub tracks_selection: bool,
    /// Whether this command is a cursor motion (as opposed to a selection
    /// builder, edit, or editor action).
    ///
    /// Feeds `step_capture_pre_jump`: motions, jump-flagged commands, and
    /// visual-line commands all snapshot their pre-body cursor position so the
    /// jump list can record a threshold-exceeding move. Selection commands are
    /// excluded — staging a text-object is not deliberate navigation.
    pub is_motion: bool,
    /// Whether this command is a paste-family command (p, P, [, ]).
    ///
    /// Read by `commands/edit.rs` to detect a paste-after pattern (p → p appends
    /// from `last_paste` instead of the clipboard). Does not affect the
    /// paste-session commit — that is driven solely by `defers_paste_commit`.
    pub is_paste: bool,
    /// Whether this command defers the paste-session commit.
    ///
    /// `true` only for ring-cycle commands (`[` / `]`). Ring cycles must NOT
    /// commit the paste session — they fold into one undo step with the original
    /// paste. Always implies `is_paste`.
    pub defers_paste_commit: bool,
    /// Whether this command always records a jump-list entry before executing,
    /// regardless of how far the cursor moves (goto / search / page-scroll /
    /// `select-all`).
    ///
    /// Single source of truth for jump-command classification — there is no
    /// parallel `JUMP_COMMANDS` list, and the dispatch pipeline reads this rather
    /// than matching on the command variant.
    pub is_jump: bool,
    /// Whether this motion's Move-mode result is a reaching selection (i.e. it
    /// navigates away from the cursor to anchor on a new region). `true` for the
    /// word motions (`select-next-word` / `-prev-word` / uppercase-word variants). `false`
    /// for everything else.
    ///
    /// `step_update_recipe` uses this to suppress the establish step for reaching
    /// Move results — replaying such a step would advance past the intended word.
    pub reaching: bool,
    /// Whether this command is a visual-line motion (`move-down`/`move-up`).
    /// The preferred display column is preserved across consecutive visual-line
    /// moves and cleared for any other command.
    pub is_visual_move: bool,
    /// Whether `.` should replay this command.
    ///
    /// Orthogonal to all other aspects: `paste-after` is paste + repeatable,
    /// `surround-add` is an editor action + repeatable, `delete` is an edit +
    /// repeatable. One bit here covers every combination without requiring an
    /// enum variant per combination.
    pub repeatable: bool,
    /// Whether dispatching this command overwrites `last_command`.
    ///
    /// `false` for `exit-insert` only — it closes an insert session that a kill
    /// (`c`) may have opened, so stamping it would clobber the `"change"` marker
    /// and break `c <text> Esc p` → ring. Set at registration via
    /// `.transparent_to_last_command()` on the `EditorCmdBuilder`. All other
    /// commands are `true`.
    pub stamps_last_command: bool,
    /// Whether this command exits sticky Extend mode after it runs.
    ///
    /// `true` for buffer-modifying selection acts: `delete`, `paste-*`, `replace`,
    /// `surround-add`. `false` for `yank` (non-destructive; preserves the
    /// selection), `change` (already enters Insert, which is its own mode exit),
    /// `undo`/`redo` (history navigation, not selection acts), motions, and
    /// selection commands. Mirrors Vim visual-mode: any operator on a visual
    /// selection returns to normal. Read by the dispatch pipeline's AFTER step.
    pub clears_extend: bool,
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

/// Signature shared by `Motion`/`Selection`'s `fun` and `around_fun` fields.
/// Named only to keep `Option<fn(...)>` under clippy's type-complexity
/// threshold — `fun` itself stays inline since the bare (non-`Option`) form
/// doesn't trip it.
type SelectionFn = fn(&Text, SelectionSet, usize, MotionMode) -> SelectionSet;

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
        /// Alternate body used in place of `fun` when the focused buffer
        /// resolves `word-selects-whitespace` to true (see
        /// `run_native_body`'s dispatch swap). `None` for every motion except
        /// the word motions (`select-next-word` et al.), which swap in their
        /// `_around` twin — same signature, covers the destination word's
        /// whitespace bookend in both `Move` and `Extend` modes.
        around_fun: Option<SelectionFn>,
        /// Whether this motion always records a jump list entry before executing,
        /// regardless of how far the cursor moves. Used for goto commands.
        jump: bool,
        /// Whether this motion's Move-mode result anchors the selection on a
        /// region reached by navigating *away* from the cursor (`select-next-word`
        /// et al.). Reaching motions in Move mode do NOT push an establish step
        /// onto the selection recipe — replaying such a step would advance past
        /// the word under the cursor, causing dot-repeat to act on the wrong region.
        ///
        /// Extend-mode reaching steps (e.g. `Ctrl+w`) are still recorded; an
        /// extend grows an existing selection by a relative amount and is safe.
        reaching: bool,
    },
    /// Selection or text-object operation (accepts count).
    ///
    /// Signature: `fn(&Text, SelectionSet, usize, MotionMode) -> SelectionSet`
    ///
    /// All selection commands receive `MotionMode`. Non-extendable ones accept
    /// `_mode` and ignore it; extendable text objects branch on it. The `usize`
    /// is a count argument; commands that don't use it accept `_count`.
    Selection {
        name: Cow<'static, str>,
        // Pending command-palette / :help integration.
        #[allow(dead_code)]
        doc: Cow<'static, str>,
        fun: fn(&Text, SelectionSet, usize, MotionMode) -> SelectionSet,
        /// Alternate body used in place of `fun` when the focused buffer
        /// resolves `word-selects-whitespace` to true. `None` for every
        /// selection command except `select-word`/`select-uppercase-word`
        /// (`mm`/`MM`), which swap to their around-word body. See
        /// `Motion::around_fun`.
        around_fun: Option<SelectionFn>,
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
        /// Whether this command is a paste-family command (p, P, [, ]).
        /// See [`CmdMeta::is_paste`] for the full rationale.
        is_paste: bool,
        /// Whether this command defers the paste-session commit.
        /// `true` only for ring-cycle commands (`[` / `]`). Always implies `is_paste`.
        /// See [`CmdMeta::defers_paste_commit`] for the full rationale.
        defers_paste_commit: bool,
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
        /// Whether this command exits sticky Extend mode after it runs.
        /// See [`CmdMeta::clears_extend`] for the full rationale.
        clears_extend: bool,
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
    /// Registered by `CommandHost::register_lazy_command` as `declare-plugin`
    /// processes its `#:commands` entries. When dispatched, the owning
    /// plugin's body is evaluated, the stub is replaced by the real
    /// `SteelBacked` command, and dispatch re-runs.
    Lazy {
        name: Cow<'static, str>,
        plugin: hume_scripting::attribution::PluginId,
    },
}

impl MappableCommand {
    /// The command's registered name.
    ///
    /// Returns the stored `Cow` (not `&str`) so the dispatch pipeline can clone
    /// it preserving `Cow::Borrowed` — a `&'static str` name (every built-in)
    /// clones with no heap allocation; only `Cow::Owned` Steel names allocate.
    /// Pure field extraction — distinct from [`MappableCommand::meta`], which is
    /// the single source of truth for derived bookkeeping properties.
    pub(crate) fn name(&self) -> &Cow<'static, str> {
        match self {
            Self::Motion { name, .. }
            | Self::Selection { name, .. }
            | Self::Edit { name, .. }
            | Self::EditorCmd { name, .. }
            | Self::SteelBacked { name, .. }
            | Self::Lazy { name, .. } => name,
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
            Self::Motion { jump, reaching, .. } => CmdMeta {
                tracks_selection: true,
                is_motion: true,
                is_paste: false,
                defers_paste_commit: false,
                is_jump: *jump,
                is_visual_move: false,
                reaching: *reaching,
                repeatable: false,
                stamps_last_command: true,
                clears_extend: false,
            },
            Self::Selection { jump, .. } => CmdMeta {
                tracks_selection: true,
                is_motion: false,
                is_paste: false,
                defers_paste_commit: false,
                is_jump: *jump,
                is_visual_move: false,
                reaching: false,
                repeatable: false,
                stamps_last_command: true,
                clears_extend: false,
            },
            Self::Edit { repeatable, .. } => CmdMeta {
                tracks_selection: false,
                is_motion: false,
                is_paste: false,
                defers_paste_commit: false,
                is_jump: false,
                is_visual_move: false,
                reaching: false,
                repeatable: *repeatable,
                stamps_last_command: true,
                clears_extend: false,
            },
            Self::EditorCmd {
                is_paste,
                defers_paste_commit,
                repeatable,
                jump,
                visual_move,
                stamps_last_command,
                clears_extend,
                ..
            } => CmdMeta {
                tracks_selection: false,
                is_motion: false,
                is_paste: *is_paste,
                defers_paste_commit: *defers_paste_commit,
                is_jump: *jump,
                is_visual_move: *visual_move,
                reaching: false,
                repeatable: *repeatable,
                stamps_last_command: *stamps_last_command,
                clears_extend: *clears_extend,
            },
            Self::SteelBacked { repeatable, .. } => CmdMeta {
                tracks_selection: false,
                is_motion: false,
                is_paste: false,
                defers_paste_commit: false,
                is_jump: false,
                is_visual_move: false,
                reaching: false,
                repeatable: *repeatable,
                stamps_last_command: true,
                clears_extend: false,
            },
            Self::Lazy { .. } => CmdMeta {
                tracks_selection: false,
                is_motion: false,
                is_paste: false,
                defers_paste_commit: false,
                is_jump: false,
                is_visual_move: false,
                reaching: false,
                repeatable: false,
                stamps_last_command: true,
                clears_extend: false,
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
