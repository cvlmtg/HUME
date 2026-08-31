use std::borrow::Cow;

use crate::editor::error::CommandError;
use hume_editing::changeset::ChangeSet;
use hume_editing::selection::SelectionSet;
use hume_editing::text::BufferText;
use hume_engine::pipeline::EngineView;
use hume_ops::{MotionMode, WordCtx};

// ── Command metadata for dispatch bookkeeping ────────────────────────────────

/// How a command's post-dispatch selection interacts with the dot-repeat
/// selection recipe (`EditorState::selection_recipe`). See
/// [`CmdMeta::selection_tracking`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SelectionTracking {
    /// Not a selection builder — clears the recipe.
    Untracked,
    /// A motion's Move-mode result is a bare cursor — nothing to replay, so
    /// the recipe clears. Extend-mode steps still append: extending grows an
    /// existing selection by a relative amount and is safe to replay. Every
    /// `Motion` variant carries this, including the word motions
    /// (`select-next-word` et al.), whose Move-mode result *looks*
    /// replayable (it lands on a selected word) but isn't — replaying it
    /// would advance past the intended word instead of rebuilding it.
    Extends,
    /// Establishes an extent that is replayable on its own from a fresh
    /// cursor (`select-line`, `ms(`, `m/`): resets the recipe to a single
    /// step in Move mode, appends a step in Extend mode.
    Establishes,
    /// Transforms whatever extent is already staged rather than
    /// establishing one (`copy-selection-on-next-line`/`-prev-line`
    /// duplicate the current selection onto an adjacent line). Always
    /// appends, in both Move and Extend mode: replaying this step alone
    /// from a fresh cursor would rebuild nothing.
    Composes,
}

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
/// The one derived reading is [`CmdMeta::moves_cursor`], a disjunction of the
/// three motion flags that two dispatch stages both need. It adds no state and
/// no category: each flag is still set independently at its own `meta()` arm.
///
/// `Copy` and name-free on purpose: the variant→property mapping lives in
/// `MappableCommand::meta()` and nowhere else, so `meta()` must be cheap enough
/// that no caller is ever tempted to re-`match` the variant for a single bit
/// (which would fork the SSOT). The command name is owned data — it is read
/// separately via [`MappableCommand::name`] and cloned once per dispatch by the
/// pipeline, not carried in here.
#[derive(Debug, Clone, Copy)]
pub(crate) struct CmdMeta {
    /// How this command updates the selection recipe after it runs.
    ///
    /// Always `Extends` for Motion variants. `Selection` and `EditorCmd`
    /// each carry a `selection_tracking` field of their own (per-command
    /// opt-in) — see [`MappableCommand::Selection`]/[`MappableCommand::EditorCmd`].
    /// `EditorCmd` additionally covers the rare case where a command needs
    /// `EditorState`/`EngineView` access to build a replayable selection
    /// extent — `select-all-matches` (`m/`) reads the buffer's search
    /// pattern, which the pure `Selection` body signature has no channel
    /// for. The recipe accumulates the sequence of selection-building steps
    /// so dot-repeat can re-establish the selection before replaying an
    /// edit. All other commands clear the recipe (`Untracked`).
    pub selection_tracking: SelectionTracking,
    /// Whether this command is a cursor motion (as opposed to a selection
    /// builder, edit, or editor action).
    ///
    /// Feeds `step_capture_pre_jump`: motions, jump-flagged commands, and
    /// visual-line commands all snapshot their pre-body cursor position so the
    /// jump list can record a threshold-exceeding move. Selection commands are
    /// excluded — staging a text-object is not deliberate navigation.
    pub is_motion: bool,
    /// Whether this command defers the paste-session commit.
    ///
    /// `true` only for ring-cycle commands (`[` / `]`). Ring cycles must NOT
    /// commit the paste session — they fold into one undo step with the original
    /// paste.
    pub defers_paste_commit: bool,
    /// Whether this command always records a jump-list entry before executing,
    /// regardless of how far the cursor moves (goto / search / page-scroll /
    /// `select-all`).
    ///
    /// Single source of truth for jump-command classification — there is no
    /// parallel `JUMP_COMMANDS` list, and the dispatch pipeline reads this rather
    /// than matching on the command variant.
    pub is_jump: bool,
    /// Whether this command is a visual-line motion (`move-down`/`move-up`).
    /// Read only by `step_capture_pre_jump`, alongside `is_jump`/`is_motion`,
    /// to decide whether to snapshot the pre-move selection for the jump
    /// list — it does not gate the sticky display column, which a
    /// `Selection` carries and clears by construction regardless of this
    /// flag (see `Selection::sticky_display_col`).
    pub is_visual_move: bool,
    /// Whether `.` should replay this command.
    ///
    /// Orthogonal to all other aspects: `paste-after` is paste + repeatable,
    /// `surround-add` is an editor action + repeatable, `delete` is an edit +
    /// repeatable. One bit here covers every combination without requiring an
    /// enum variant per combination.
    pub repeatable: bool,
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

impl CmdMeta {
    /// Returns `true` if this command moved the cursor rather than editing —
    /// the disjunction of `is_motion`, `is_jump`, and `is_visual_move`.
    ///
    /// Two stages want exactly this set and nothing else: `step_capture_pre_jump`
    /// (snapshot the pre-body position for the jump list) and the Insert-mode
    /// trie's pinned-anchor invalidation (`mappings/insert.rs`). Derived in one
    /// place so the two can't drift apart on a future flag.
    ///
    /// **Blind spot**: [`MappableCommand::meta`] hardcodes all three flags
    /// `false` for `SteelBacked` and `Lazy`, since a Steel command has no way to
    /// declare its own motion semantics. A user-bound Steel motion therefore
    /// answers `false` here — the jump list won't record it, and a pinned typed
    /// run survives it. Both callers accept that rather than guess.
    pub(crate) fn moves_cursor(&self) -> bool {
        self.is_motion || self.is_jump || self.is_visual_move
    }
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

/// Body shape for `Motion`/`Selection`'s `fun` field.
///
/// Every native motion/selection command takes `(&BufferText, SelectionSet,
/// usize, MotionMode)` — except the word family (`w`/`W`/`b`/`B`, `mm`/`MM`,
/// `miw`/`maw`), which additionally needs this buffer's configured
/// `word-chars` and effective `word-selects-whitespace`, resolved from
/// settings the same way `tab_width`/`TabStyle` are for `align_selections`/
/// `insert_tab`. Rather than widening every command's signature for the ~28
/// that would ignore the extra data, only the word family gets a second body
/// shape here in the registry — the four `w`/`W`/`b`/`B` motions specifically
/// (rather than joining `select-word-nearest-on-line` as an `EditorCmd`,
/// which also resolves settings, via a `RowMap`) because `MappableCommand::meta`
/// only ever sets `CmdMeta::is_motion` true for the `Motion` variant, and Move
/// mode's jump-list/dot-repeat handling depends on that flag.
#[derive(Clone, Copy)]
pub(crate) enum SelectionBody {
    Plain(fn(&BufferText, SelectionSet, usize, MotionMode) -> SelectionSet),
    Word(fn(&BufferText, SelectionSet, usize, WordCtx<'_>) -> SelectionSet),
}

/// A command that can be bound to a key in a keymap.
///
/// The keymap trie stores command *names*; the registry resolves names to
/// `MappableCommand` values at dispatch time.
#[derive(Clone)]
pub(crate) enum MappableCommand {
    /// Motion that repeats `count` times.
    ///
    /// `fun` is a [`SelectionBody`] — `Plain(fn(&BufferText, SelectionSet,
    /// usize, MotionMode) -> SelectionSet)` for almost every motion, `Word`
    /// for the word family.
    ///
    /// Motions are always extendable. The `mode` parameter selects Move or Extend
    /// semantics at dispatch time — no separate extend-variant functions needed.
    Motion {
        name: Cow<'static, str>,
        // Pending command-palette / :help integration.
        #[allow(dead_code)]
        doc: Cow<'static, str>,
        fun: SelectionBody,
        /// Whether this motion always records a jump list entry before executing,
        /// regardless of how far the cursor moves. Used for goto commands.
        jump: bool,
    },
    /// Selection or text-object operation (accepts count).
    ///
    /// `fun` is a [`SelectionBody`] — see [`MappableCommand::Motion`]'s doc.
    ///
    /// All selection commands receive `MotionMode`. Non-extendable ones accept
    /// `_mode` and ignore it; extendable text objects branch on it. The `usize`
    /// is a count argument; commands that don't use it accept `_count`.
    Selection {
        name: Cow<'static, str>,
        // Pending command-palette / :help integration.
        #[allow(dead_code)]
        doc: Cow<'static, str>,
        fun: SelectionBody,
        /// Whether this command always records a jump list entry before executing,
        /// regardless of how far the cursor moves. Used for `select-all` (`%`).
        jump: bool,
        /// How this command opts into the dot-repeat selection recipe.
        /// See [`CmdMeta::selection_tracking`]. `Establishes` for most
        /// selection commands (`select-line`, `ms(`, `select-all`: each
        /// replayable on its own from a fresh cursor); `Composes` for the
        /// handful that transform or reduce whatever is already staged
        /// instead — see `registry/defaults/selections.rs` for the full list.
        selection_tracking: SelectionTracking,
    },
    /// BufferText-modifying edit with no extra arguments.
    ///
    /// Signature: `fn(BufferText, SelectionSet) -> (BufferText, SelectionSet, ChangeSet)`
    ///
    /// Edits are never extendable — they don't carry `MotionMode`.
    Edit {
        name: Cow<'static, str>,
        // Pending command-palette / :help integration.
        #[allow(dead_code)]
        doc: Cow<'static, str>,
        fun: fn(BufferText, SelectionSet) -> (BufferText, SelectionSet, ChangeSet),
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
        /// Whether this command defers the paste-session commit.
        /// `true` only for ring-cycle commands (`[` / `]`).
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
        /// Whether this command exits sticky Extend mode after it runs.
        /// See [`CmdMeta::clears_extend`] for the full rationale.
        clears_extend: bool,
        /// How this command opts into the dot-repeat selection recipe.
        /// `Untracked` unless a specific registration opts in — see
        /// [`CmdMeta::selection_tracking`] and each opt-in site's own comment
        /// in `registry/defaults/`.
        selection_tracking: SelectionTracking,
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
            Self::Motion { jump, .. } => CmdMeta {
                selection_tracking: SelectionTracking::Extends,
                is_motion: true,
                defers_paste_commit: false,
                is_jump: *jump,
                is_visual_move: false,
                repeatable: false,
                clears_extend: false,
            },
            Self::Selection {
                jump,
                selection_tracking,
                ..
            } => CmdMeta {
                selection_tracking: *selection_tracking,
                is_motion: false,
                defers_paste_commit: false,
                is_jump: *jump,
                is_visual_move: false,
                repeatable: false,
                clears_extend: false,
            },
            Self::Edit { repeatable, .. } => CmdMeta {
                selection_tracking: SelectionTracking::Untracked,
                is_motion: false,
                defers_paste_commit: false,
                is_jump: false,
                is_visual_move: false,
                repeatable: *repeatable,
                clears_extend: false,
            },
            Self::EditorCmd {
                defers_paste_commit,
                repeatable,
                jump,
                visual_move,
                clears_extend,
                selection_tracking,
                ..
            } => CmdMeta {
                selection_tracking: *selection_tracking,
                is_motion: false,
                defers_paste_commit: *defers_paste_commit,
                is_jump: *jump,
                is_visual_move: *visual_move,
                repeatable: *repeatable,
                clears_extend: *clears_extend,
            },
            Self::SteelBacked { repeatable, .. } => CmdMeta {
                selection_tracking: SelectionTracking::Untracked,
                is_motion: false,
                defers_paste_commit: false,
                is_jump: false,
                is_visual_move: false,
                repeatable: *repeatable,
                clears_extend: false,
            },
            Self::Lazy { .. } => CmdMeta {
                selection_tracking: SelectionTracking::Untracked,
                is_motion: false,
                defers_paste_commit: false,
                is_jump: false,
                is_visual_move: false,
                repeatable: false,
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

/// Which argument completer a typed command's `:` command-line argument uses,
/// if any. Declared alongside the command name in `typed_cmd!` so renaming a
/// command can't silently desync it from the completion dispatch in
/// `command_mode.rs`, which reads this instead of re-matching on the name.
pub(crate) enum ArgCompleter {
    /// Path completion. `dirs_only` restricts candidates to directories
    /// (`:change-directory`); `false` covers files too (`:edit`/`:write`).
    Path {
        dirs_only: bool,
    },
    Buffer,
    Theme,
    Set,
}

/// A command invocable from the `:` command line.
///
/// Typed commands have a canonical name and optional short aliases. They are
/// stored in [`super::CommandRegistry`] alongside [`MappableCommand`] entries in a
/// single `HashMap`, sharing the same namespace.
///
/// The function signature differs from mappable commands: it receives an
/// optional string argument (e.g. the path for `:w foo.txt`) and a force flag
/// (whether `!` was appended), rather than a numeric count.
///
/// `fun` deliberately keeps `&mut Editor` rather than [`EditorCmdFn`]'s
/// `(&mut EditorState, &mut EngineView, …)` shape, for three reasons: typed
/// commands are not Steel-dispatchable — only callers are the `:` command
/// line and tests, so `&mut Editor` here never runs while the Steel engine is
/// borrowed; some handlers genuinely need shell-level fields (`:e` and `:set
/// buffer language=` reach `scripting` for `activate_lazy_language_plugins`
/// and `parse_worker` for `setup_buffer_syntax`, neither reachable from the
/// coarse shape); and this is the Editor-orchestration layer, driving
/// whole-app ops (`:w`, `:e`, `:bd`, `:split`, `:set language`) that
/// legitimately span state + view + `parse_worker` + Steel together.
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
    /// Argument completer for this command's `:` command-line argument, if any.
    pub completer: Option<ArgCompleter>,
}
