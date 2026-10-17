use std::borrow::Cow;

use crate::editor::commands::{CommandPane, FocusedPane, NativeBody};
use crate::editor::error::CommandError;
use hume_editing::edit::Edited;
use hume_editing::state::EditState;
use hume_engine::pipeline::EngineView;
use hume_ops::search::SearchDirection;
use hume_ops::{LineEnd, MotionMode, WordCtx};
use hume_treesitter::textobjects::{Direction, ObjectKind, ObjectSpan};

// ── Native command count ─────────────────────────────────────────────────────

/// A native command body's repeat count: one the user typed (or a script
/// passed), or the default of one. Vertical moves read the difference: `9j`
/// moves nine buffer lines, a bare `j` one visual line.
#[derive(Clone, Copy)]
pub(in crate::editor) struct Count(Option<usize>);

impl Count {
    /// `typed` is [`CmdCtx::count`](crate::editor::dispatch::CmdCtx::count):
    /// `None` when no count was given.
    pub(in crate::editor) fn new(typed: Option<usize>) -> Self {
        Self(typed)
    }

    /// The repeat count, at least one.
    pub(in crate::editor) fn get(self) -> usize {
        self.0.unwrap_or(1).max(1)
    }

    pub(in crate::editor) fn is_typed(self) -> bool {
        self.0.is_some()
    }
}

// ── Native command target shape ──────────────────────────────────────────────

/// Function pointer for an [`EditorCmdBody::Pane`] handler: acts through `t`,
/// a pane proven live and showing its buffer, not necessarily the focused
/// pane.
pub(in crate::editor) type PaneCmdFn = fn(
    &mut super::super::EditorState,
    &mut EngineView,
    CommandPane,
    Count,
    MotionMode,
) -> Result<(), CommandError>;

/// Function pointer for an [`EditorCmdBody::FocusedPane`] handler: acts on
/// `bid` through the *focused* pane, which the [`FocusedPane`] argument
/// proves; see its doc for what that licenses.
pub(in crate::editor) type FocusedCmdFn = fn(
    &mut super::super::EditorState,
    &mut EngineView,
    FocusedPane,
    Count,
    MotionMode,
) -> Result<(), CommandError>;

/// Function pointer for an [`EditorCmdBody::PaneChar`] handler: a
/// [`PaneCmdFn`] that also takes the character its operand names (a key read
/// after the command was dispatched, or a `call!` argument). The character is
/// required: a key dispatched without one reads it from the next key first,
/// and a `call!` without one raises.
pub(in crate::editor) type PaneCharCmdFn = fn(
    &mut super::super::EditorState,
    &mut EngineView,
    CommandPane,
    Count,
    MotionMode,
    char,
) -> Result<(), CommandError>;

/// Function pointer for an [`EditorCmdBody::PaneText`] handler: a
/// [`PaneCmdFn`] that also takes the pattern its operand names (typed at the
/// command's prompt, or a `call!` argument). The pattern is required: a key
/// dispatched without one opens the prompt first, and a `call!` without one
/// raises.
pub(in crate::editor) type PaneTextCmdFn = fn(
    &mut super::super::EditorState,
    &mut EngineView,
    CommandPane,
    Count,
    MotionMode,
    &str,
) -> Result<(), CommandError>;

/// The prompt a [`EditorCmdBody::PaneText`] command opens to read its
/// pattern when dispatched without one.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::editor) enum PatternPrompt {
    Search(SearchDirection),
    Sift,
}

/// Body shape for [`NativeCommand::EditorCmd`]'s `fun` field: decides at
/// compile time whether `(call! "cmd" pane)` may act on a non-focused pane,
/// and which operand, if any, the command takes.
/// Motion/Selection/Edit need no such field, since a pure body can only reach
/// the buffer it is handed.
///
/// A command that moves focus, changes the focused pane's buffer, or touches
/// focus-bound state (a mode layer, sticky Extend, an Insert/paste session)
/// is `FocusedPane`, so an async caller must prove its handle is the focus
/// instead of acting on whatever is focused when it runs. Everything else is
/// `Pane`.
///
/// Wrapped in [`NativeBody`]. `registry/defaults/builder.rs` has one `ecmd_*`
/// constructor per variant, so a function pointer of the wrong signature
/// fails to compile.
#[derive(Clone, Copy)]
pub(in crate::editor) enum EditorCmdBody {
    Pane(PaneCmdFn),
    FocusedPane(FocusedCmdFn),
    PaneChar(PaneCharCmdFn),
    PaneText(PaneTextCmdFn, PatternPrompt),
}

// ── Command metadata for dispatch bookkeeping ────────────────────────────────

/// How a command's post-dispatch selection interacts with the dot-repeat
/// selection recipe (`EditorState::selection_recipe`). See
/// [`CmdMeta::selection_tracking`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::editor) enum SelectionTracking {
    /// Not a selection builder: clears the recipe.
    Untracked,
    /// A motion's Move-mode result is a bare cursor, with nothing to replay, so
    /// the recipe clears. Extend-mode steps still append: extending grows an
    /// existing selection by a relative amount and is safe to replay. Every
    /// `Motion` variant carries this, including the word motions
    /// (`select-next-word` et al.), whose Move-mode result *looks*
    /// replayable (it lands on a selected word) but isn't: replaying it
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
/// Drives the dispatch pipeline: the pipeline reads this instead of matching
/// on variant type or checking string sets.
///
/// Each field is one independent behavioral aspect read by exactly one dispatch
/// stage. This is a flat set of orthogonal flags; there is no category enum.
/// Adding a new behavior = add one field here + one `step_*` function that reads
/// it. No existing fields or call sites are affected.
///
/// The one derived reading is [`CmdMeta::moves_cursor`], a disjunction of the
/// three motion flags that two dispatch stages both need. It adds no state and
/// no category: each flag is still set independently at its own `meta()` arm.
///
/// `Copy` and name-free on purpose: the variant→property mapping lives in
/// `NativeCommand::meta()` and `steel_meta` and nowhere else, so `meta()`
/// must be cheap enough that no caller is ever tempted to re-`match` the
/// variant for a single bit (which would fork the SSOT). The command name is owned data: it is read
/// separately via [`MappableCommand::name`] and cloned once per dispatch by the
/// pipeline, not carried in here.
#[derive(Debug, Clone, Copy)]
pub(in crate::editor) struct CmdMeta {
    /// How this command updates the selection recipe after it runs.
    ///
    /// Always `Extends` for Motion variants. `Selection` and `EditorCmd`
    /// each carry a `selection_tracking` field of their own (per-command
    /// opt-in); see [`NativeCommand::Selection`]/[`NativeCommand::EditorCmd`].
    /// `EditorCmd` additionally covers the rare case where a command needs
    /// `EditorState`/`EngineView` access to build a replayable selection
    /// extent: `select-all-matches` (`m/`) reads the buffer's search
    /// pattern, which the pure `Selection` body signature has no channel
    /// for. The recipe accumulates the sequence of selection-building steps
    /// so dot-repeat can re-establish the selection before replaying an
    /// edit. All other commands clear the recipe (`Untracked`).
    pub selection_tracking: SelectionTracking,
    /// Whether this command is a cursor motion (as opposed to a selection
    /// builder, edit, or editor action).
    ///
    /// Feeds `with_jump`: motions, jump-flagged commands, and
    /// visual-line commands all snapshot their pre-body cursor position so the
    /// jump list can record a threshold-exceeding move. Selection commands are
    /// excluded: staging a text-object is not deliberate navigation.
    pub is_motion: bool,
    /// Whether this command defers the paste-session commit.
    ///
    /// `true` only for ring-cycle commands (`[` / `]`). Ring cycles must NOT
    /// commit the paste session; they fold into one undo step with the original
    /// paste.
    pub defers_paste_commit: bool,
    /// Whether this command always records a jump-list entry before executing,
    /// regardless of how far the cursor moves (goto / search / page-scroll /
    /// `select-all`).
    ///
    /// Single source of truth for jump-command classification: there is no
    /// parallel `JUMP_COMMANDS` list, and the dispatch pipeline reads this rather
    /// than matching on the command variant.
    pub is_jump: bool,
    /// Whether this command is a visual-line motion (`move-down`/`move-up`).
    /// Read only by `moves_cursor`, alongside `is_jump`/`is_motion`,
    /// to decide whether to snapshot the pre-move selection for the jump
    /// list. It does not gate the sticky display column, which a
    /// `Selection` carries and clears itself regardless of this
    /// flag (see `Selection::sticky_display_col`).
    pub is_visual_move: bool,
    /// Whether `.` should replay this command, and so whether the
    /// interactive inputs its code opens are recorded rather than ending the
    /// change in flight (`EditorState::waiting_input`).
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
    /// Whether this command's landing spot should be re-aligned in the
    /// viewport per `EditorSettings::object_jump_align`, after it runs.
    /// `true` only for `Motion`'s forward object-jump family (`}`,
    /// `goto-next-<kind>`).
    pub aligns_view: bool,
    /// Whether this command opens or continues `EditorState::active_session`
    /// itself, instead of relying on an external wrapper to group its edit.
    ///
    /// `true` only for the paste family (`paste-after`/`-before`,
    /// `smart-paste-after`/`-before`, `paste-ring-older`/`-newer`; see
    /// `commands::paste`'s `do_paste`/`do_paste_cycle`). `Editor::replay_dot`
    /// reads this to decide whether it may safely pre-open a session before
    /// dispatching the replayed command: pre-opening one for a command in
    /// this set would collide with the session that command is about to
    /// open or continue itself: `paste-ring-older`/`-newer` in particular
    /// need an *already-open* Paste session to find, not a fresh Replay
    /// placeholder.
    /// Every other command relies on `replay_dot`'s wrapper to fold a
    /// multi-step recipe replay plus the main edit into one undo revision.
    ///
    /// Governs only the pre-open decision, not whether `replay_dot`
    /// commits afterward. A Steel-backed command can never set this flag
    /// (see the `SteelBacked`/`Lazy` constructors below, always `false`),
    /// yet its body can still dispatch native paste, which retargets a
    /// pre-opened session to `Paste` in place
    /// (`edit_session::open_or_retarget`). `Editor::finish_replay_session`
    /// reads the *resulting* session's own kind to decide whether to leave
    /// it open for a following `[`/`]`, so that decision stays correct
    /// whether the command declaring it statically is native or was
    /// discovered dynamically inside a Steel body.
    pub manages_own_session: bool,
    /// Whether only a key may run this command, so a script's `call!`
    /// refuses it. `true` only for `repeat-last-action`: replaying a change
    /// needs the Steel engine, which a `call!` from Steel runs inside, so
    /// the replay waits for the end of the keypress and would run after the
    /// script's own later calls.
    pub key_only: bool,
}

impl CmdMeta {
    /// Returns `true` if this command moved the cursor rather than editing:
    /// the disjunction of `is_motion`, `is_jump`, and `is_visual_move`.
    ///
    /// The pipeline runs these commands, and only these, inside `with_jump`, which
    /// snapshots the pre-body position for the jump list and invalidates a
    /// still-open Insert session's pinned typed run.
    ///
    /// **Blind spot**: [`SteelCommand::meta`] hardcodes all three flags
    /// `false`, as does a `Lazy` stub's, since a Steel command has no way to
    /// declare its own motion semantics. A user-bound Steel motion therefore
    /// answers `false` here: the jump list won't record it, and a pinned typed
    /// run survives it. Both callers accept that rather than guess.
    pub(in crate::editor) fn moves_cursor(&self) -> bool {
        self.is_motion || self.is_jump || self.is_visual_move
    }
}

/// Function pointer for an [`Edit`](NativeCommand::Edit) handler.
pub(in crate::editor) type EditFn = fn(EditState) -> Edited;

// ── MappableCommand ───────────────────────────────────────────────────────────

/// Body shape for `Motion`/`Selection`'s `fun` field.
///
/// Most native motion/selection commands take `(EditState, usize,
/// MotionMode)`. The word family (`w`/`W`/`b`/`B`, `mm`/`MM`,
/// `miw`/`maw`) additionally needs this buffer's configured
/// `word-chars` and effective `word-selects-whitespace`, resolved from
/// settings the same way `tab_width`/`TabStyle` are for `align_selections`/
/// `insert_tab`. Rather than widening every command's signature for the large
/// majority of registrations that would ignore the extra data, the word
/// family (the four `w`/`W`/`b`/`B` motions and six word-tagged selections)
/// gets a body shape of its own, `Word`. The four motions
/// specifically stay `Motion` rather than joining `select-word-nearest-on-line`
/// as an `EditorCmd` (which also resolves settings, via a `DisplayLineMap`) because
/// `NativeCommand::meta` only ever sets `CmdMeta::is_motion` true for the
/// `Motion` variant, and Move mode's jump-list/dot-repeat handling depends on
/// that flag.
#[derive(Clone, Copy)]
pub(in crate::editor) enum SelectionBody {
    Plain(fn(EditState, usize, MotionMode) -> EditState),
    Word(fn(EditState, usize, WordCtx<'_>) -> EditState),
    /// `goto-line-end`, whose landing the pipeline resolves per pane through
    /// `line_end_for`.
    LineEnd(fn(EditState, usize, MotionMode, LineEnd) -> EditState),
    Structural(StructuralBody),
}

/// Data, not a fn pointer: kind × span (`Select`) and kind × direction
/// (`Goto`) are enumerable, so one interpreter in the dispatch funnel
/// (`commands/structural.rs::StructuralBody::apply`) replaces what would
/// otherwise be 22 near-identical thin functions, one per structural
/// command name.
#[derive(Clone, Copy)]
pub(in crate::editor) enum StructuralBody {
    /// `m i <k>` / `m a <k>`: the smallest captured `<kind>.<span>` object at
    /// each cursor. Extend mode grows outward through
    /// `apply_text_object_extend`'s past-end retry, so an object that shares
    /// its end with its parent (a nested `def` closing where its class
    /// closes) cannot grow to that parent, the same limit the bracket
    /// objects have, accepted rather than special-cased.
    Select { kind: ObjectKind, span: ObjectSpan },
    /// `goto-next-<k>` / `goto-prev-<k>`.
    Goto { kind: ObjectKind, dir: Direction },
    /// `m i a` / `m a a`: `parameter.inside` with the lexical scan as
    /// fallback: one structure-aware argument family rather than a
    /// separate `parameter` object.
    Argument { around: bool },
}

/// A command that can be bound to a key in a keymap.
///
/// The keymap trie stores command *names*; the registry resolves names to
/// `MappableCommand` values at dispatch time.
///
/// `pub(in crate::editor)`, not `pub(crate)`: ordinary minimum-reach
/// visibility hygiene, since every usage site already lives under `crate::editor`,
/// same as [`StructuralBody::apply`]. It is not what stops a second dispatch
/// path: enum variants inherit their enum's visibility and cannot be narrowed
/// individually, so this alone would not stop any file under `crate::editor`
/// from destructuring a native variant's `fun` and calling it directly,
/// skipping `commands::run`'s bookkeeping (jump list, dot-repeat,
/// paste session). Every [`NativeCommand`] variant's `fun` is wrapped in
/// [`NativeBody`] instead: its private
/// field, readable only from `commands::pipeline::run_body`, is what
/// actually closes that off.
#[derive(Clone)]
pub(in crate::editor) enum MappableCommand {
    /// A command implemented in Rust.
    Native(NativeCommand),
    /// A command implemented as a Steel (Scheme) lambda.
    SteelBacked(SteelCommand),
    /// A placeholder for a lazy plugin command that has not yet been loaded.
    ///
    /// Registered by `CommandHost::register_lazy_command` as `declare-plugin!`
    /// processes its `#:commands` entries. When dispatched, the owning
    /// plugin's body is evaluated, the stub is replaced by the real
    /// `SteelBacked` command, and dispatch re-runs.
    Lazy {
        name: Cow<'static, str>,
        plugin: hume_scripting::attribution::EntryId,
    },
}

impl MappableCommand {
    /// The command's registered name.
    ///
    /// Returns the stored `Cow` (not `&str`) so the dispatch pipeline can clone
    /// it preserving `Cow::Borrowed`: a `&'static str` name (every built-in)
    /// clones with no heap allocation; only `Cow::Owned` Steel names allocate.
    /// Pure field extraction, distinct from [`NativeCommand::meta`], which is
    /// the single source of truth for derived bookkeeping properties.
    pub(in crate::editor) fn name(&self) -> &Cow<'static, str> {
        match self {
            Self::Native(cmd) => cmd.name(),
            Self::SteelBacked(cmd) => &cmd.name,
            Self::Lazy { name, .. } => name,
        }
    }

    /// One-line description of the command, for `:help` and command-palette display.
    #[cfg(test)]
    pub(in crate::editor) fn doc(&self) -> &str {
        match self {
            Self::Native(cmd) => cmd.doc(),
            Self::SteelBacked(cmd) => cmd.doc.as_ref(),
            Self::Lazy { .. } => "",
        }
    }

    /// Declarative metadata for the dispatch pipeline. See
    /// [`NativeCommand::meta`].
    #[cfg(test)]
    pub(in crate::editor) fn meta(&self) -> CmdMeta {
        match self {
            Self::Native(cmd) => cmd.meta(),
            Self::SteelBacked(cmd) => cmd.meta(),
            Self::Lazy { .. } => steel_meta(false),
        }
    }

    /// Returns `true` if this command executes synchronously in Rust
    /// ([`NativeCommand`]) rather than through the Steel dispatch queue
    /// (`SteelBacked`/`Lazy`).
    ///
    /// Single source of truth for native-vs-scripted classification: the
    /// `%call-native!` sync-dispatch gate and bare-binding registration both
    /// derive from this.
    pub(in crate::editor) fn is_native(&self) -> bool {
        match self {
            Self::Native(_) => true,
            Self::SteelBacked(_) | Self::Lazy { .. } => false,
        }
    }

    /// Returns `true` if this command has extend semantics and can be triggered
    /// as a one-shot extend via Ctrl-key.
    ///
    /// Steel commands (SteelBacked and Lazy stubs) are always extendable:
    /// the resolved lambda receives `extend` as its second arg. See
    /// [`NativeCommand::is_extendable`] for native ones.
    pub(in crate::editor) fn is_extendable(&self) -> bool {
        match self {
            Self::Native(cmd) => cmd.is_extendable(),
            Self::SteelBacked(_) | Self::Lazy { .. } => true,
        }
    }
}

/// A command implemented in Rust, run synchronously by `commands::run`.
#[derive(Clone)]
pub(in crate::editor) enum NativeCommand {
    /// Motion that repeats `count` times.
    ///
    /// `fun` is a [`SelectionBody`] wrapped in
    /// [`NativeBody`]: `Plain(fn(EditState,
    /// usize, MotionMode) -> EditState)` for almost every
    /// motion, `Word` for the word family.
    ///
    /// Motions are always extendable. The `mode` parameter selects Move or Extend
    /// semantics at dispatch time, with no separate extend-variant functions needed.
    Motion {
        name: Cow<'static, str>,
        // Pending command-palette / :help integration.
        #[allow(dead_code)]
        doc: Cow<'static, str>,
        fun: NativeBody<SelectionBody>,
        /// Whether this motion always records a jump list entry before executing,
        /// regardless of how far the cursor moves. Used for goto commands.
        jump: bool,
        /// Whether this motion's landing spot is re-aligned after the body
        /// runs. See [`CmdMeta::aligns_view`] for the full rationale.
        aligns_view: bool,
    },
    /// Selection or text-object operation (accepts count).
    ///
    /// `fun` is a [`SelectionBody`]; see [`NativeCommand::Motion`]'s doc.
    ///
    /// All selection commands receive `MotionMode`. Non-extendable ones accept
    /// `_mode` and ignore it; extendable text objects branch on it. The `usize`
    /// is a count argument; commands that don't use it accept `_count`.
    Selection {
        name: Cow<'static, str>,
        // Pending command-palette / :help integration.
        #[allow(dead_code)]
        doc: Cow<'static, str>,
        fun: NativeBody<SelectionBody>,
        /// Whether this command always records a jump list entry before executing,
        /// regardless of how far the cursor moves. Used for `select-all` (`%`).
        jump: bool,
        /// How this command opts into the dot-repeat selection recipe.
        /// See [`CmdMeta::selection_tracking`]. `Establishes` for most
        /// selection commands (`select-line`, `ms(`, `select-all`: each
        /// replayable on its own from a fresh cursor); `Composes` for the
        /// handful that transform or reduce whatever is already staged
        /// instead; see `registry/defaults/selections.rs` for the full list.
        selection_tracking: SelectionTracking,
    },
    /// BufferText-modifying edit with no extra arguments.
    ///
    /// Signature: `fn(EditState) -> Edited`
    ///
    /// Edits are never extendable: they don't carry `MotionMode`.
    Edit {
        name: Cow<'static, str>,
        // Pending command-palette / :help integration.
        #[allow(dead_code)]
        doc: Cow<'static, str>,
        fun: NativeBody<EditFn>,
        /// Whether `.` should replay this command. Set to `true` for edits that
        /// are meaningful to repeat (e.g. user-facing deletions). Set to `false`
        /// for internal primitives like `delete-char-backward`.
        repeatable: bool,
    },
    /// Editor-level command operating on `EditorState` + `EngineView`, plus
    /// whatever target its own [`EditorCmdBody`] variant demands.
    ///
    /// Covers composite operations: mode changes, register access, undo group
    /// management, and parameterized motions (find/till/replace). Returns
    /// `Err` for a user-facing failure, reported at that `CommandError`'s own
    /// severity: `CommandError::transient` for a boundary condition (e.g.
    /// "no match"), `CommandError::new` for a real failure (I/O errors).
    /// Silent no-ops return `Ok(())`. Stored and dispatched as a function
    /// pointer exactly like the other variants.
    EditorCmd {
        name: Cow<'static, str>,
        // Pending command-palette / :help integration.
        #[allow(dead_code)]
        doc: Cow<'static, str>,
        fun: NativeBody<EditorCmdBody>,
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
        /// Whether this EditorCmd has extend semantics (used by the Ctrl-key guard
        /// to decide if Ctrl-key should trigger extend dispatch).
        ///
        /// Motion and Selection are always extendable (implicit). Edit is never
        /// extendable (implicit). Only EditorCmd needs an explicit flag.
        extendable: bool,
        /// Whether this command exits sticky Extend mode after it runs.
        /// See [`CmdMeta::clears_extend`] for the full rationale.
        clears_extend: bool,
        /// How this command opts into the dot-repeat selection recipe.
        /// `Untracked` unless a specific registration opts in; see
        /// [`CmdMeta::selection_tracking`] and each opt-in site's own comment
        /// in `registry/defaults/`.
        selection_tracking: SelectionTracking,
        /// Whether this command manages `EditorState::active_session` itself.
        /// See [`CmdMeta::manages_own_session`] for the full rationale.
        manages_own_session: bool,
        /// Whether only a key may run this command. See
        /// [`CmdMeta::key_only`].
        key_only: bool,
    },
}

impl NativeCommand {
    /// The command's registered name. See [`MappableCommand::name`].
    pub(in crate::editor) fn name(&self) -> &Cow<'static, str> {
        match self {
            Self::Motion { name, .. }
            | Self::Selection { name, .. }
            | Self::Edit { name, .. }
            | Self::EditorCmd { name, .. } => name,
        }
    }

    /// One-line description of the command. See [`MappableCommand::doc`].
    #[cfg(test)]
    fn doc(&self) -> &str {
        match self {
            Self::Motion { doc, .. }
            | Self::Selection { doc, .. }
            | Self::Edit { doc, .. }
            | Self::EditorCmd { doc, .. } => doc.as_ref(),
        }
    }

    /// Declarative metadata for the dispatch pipeline.
    ///
    /// This is the single source of truth for bookkeeping properties, with
    /// `steel_meta` for Steel commands. The pipeline reads `CmdMeta`; it
    /// never matches on variant or checks string sets to decide what
    /// bookkeeping to run.
    pub(in crate::editor) fn meta(&self) -> CmdMeta {
        match self {
            Self::Motion {
                jump, aligns_view, ..
            } => CmdMeta {
                selection_tracking: SelectionTracking::Extends,
                is_motion: true,
                defers_paste_commit: false,
                is_jump: *jump,
                is_visual_move: false,
                repeatable: false,
                clears_extend: false,
                aligns_view: *aligns_view,
                manages_own_session: false,
                key_only: false,
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
                aligns_view: false,
                manages_own_session: false,
                key_only: false,
            },
            Self::Edit { repeatable, .. } => CmdMeta {
                selection_tracking: SelectionTracking::Untracked,
                is_motion: false,
                defers_paste_commit: false,
                is_jump: false,
                is_visual_move: false,
                repeatable: *repeatable,
                clears_extend: false,
                aligns_view: false,
                manages_own_session: false,
                key_only: false,
            },
            Self::EditorCmd {
                defers_paste_commit,
                repeatable,
                jump,
                visual_move,
                clears_extend,
                selection_tracking,
                manages_own_session,
                key_only,
                ..
            } => CmdMeta {
                selection_tracking: *selection_tracking,
                is_motion: false,
                defers_paste_commit: *defers_paste_commit,
                is_jump: *jump,
                is_visual_move: *visual_move,
                repeatable: *repeatable,
                clears_extend: *clears_extend,
                aligns_view: false,
                manages_own_session: *manages_own_session,
                key_only: *key_only,
            },
        }
    }

    /// Returns `true` if this command has extend semantics and can be
    /// triggered as a one-shot extend via Ctrl-key.
    ///
    /// Motion and Selection are always extendable. Edit is never extendable.
    /// EditorCmd has an explicit flag set at registration time.
    pub(in crate::editor) fn is_extendable(&self) -> bool {
        match self {
            Self::Motion { .. } | Self::Selection { .. } => true,
            Self::Edit { .. } => false,
            Self::EditorCmd { extendable, .. } => *extendable,
        }
    }
}

/// A command implemented as a Steel (Scheme) lambda.
///
/// Dispatched by [`hume_scripting::ScriptingHost::call_steel_cmd`], which
/// applies `body.proc` directly; `call!` reaches it through
/// `%dispatch-command!`, which applies it inside the VM.
///
/// All Steel commands are extendable (Ctrl-key delivers `extend = #t` to the
/// lambda body). Dot-repeat is opt-in via `#:repeatable #t` in `(define-command! …)`.
#[derive(Clone)]
pub(in crate::editor) struct SteelCommand {
    pub(in crate::editor) name: Cow<'static, str>,
    // Pending command-palette / :help integration.
    #[allow(dead_code)]
    pub(in crate::editor) doc: Cow<'static, str>,
    pub(in crate::editor) body: hume_scripting::SteelCmdBody,
}

impl SteelCommand {
    /// Declarative metadata for the dispatch pipeline. See
    /// [`NativeCommand::meta`].
    pub(in crate::editor) fn meta(&self) -> CmdMeta {
        steel_meta(self.body.repeatable)
    }
}

/// The metadata of a Steel command or `Lazy` stub: only `repeatable` varies.
fn steel_meta(repeatable: bool) -> CmdMeta {
    CmdMeta {
        selection_tracking: SelectionTracking::Untracked,
        is_motion: false,
        defers_paste_commit: false,
        is_jump: false,
        is_visual_move: false,
        repeatable,
        clears_extend: false,
        aligns_view: false,
        manages_own_session: false,
        key_only: false,
    }
}

/// A registry command ready to run, as `Editor::resolve_mappable` returns
/// it: a `Lazy` stub is activated into its Steel command first, so
/// `Editor::dispatch` never sees one.
pub(in crate::editor) enum Dispatchable {
    Native(NativeCommand),
    Steel(SteelCommand),
}

impl Dispatchable {
    /// Declarative metadata for the dispatch pipeline. See
    /// [`NativeCommand::meta`].
    pub(in crate::editor) fn meta(&self) -> CmdMeta {
        match self {
            Self::Native(cmd) => cmd.meta(),
            Self::Steel(cmd) => cmd.meta(),
        }
    }
}

/// A command invocable from the `:` command line.
///
/// Typed commands have a canonical name and optional short aliases. They are
/// stored in [`super::CommandRegistry`] alongside [`MappableCommand`] entries in a
/// single `HashMap`, sharing the same namespace.
///
/// The signature differs from mappable commands: it receives an optional
/// string argument (e.g. the path for `:w foo.txt`) and a force flag (whether
/// `!` was appended), rather than a numeric count; see [`TypedBody`].
pub(in crate::editor) struct TypedCommand {
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
    /// How this command executes: see [`TypedBody`].
    pub body: TypedBody,
    /// Names this command's `:` argument-completion source in
    /// `completion::SourceRegistry`, if it declares one. A name, not a
    /// closed enum: `completion/orchestrate.rs`'s `resolve_minibuf_source`
    /// looks it up at completion time instead of matching on it, which is
    /// what lets a Steel typed command's `#:complete` name a source (native
    /// or Steel-registered) the same way. `Cow`, like `name`: `Borrowed`
    /// for a built-in's `&'static str` constant, `Owned` for the runtime
    /// string `define-typed-command!` hands over.
    ///
    /// The name is not checked at compile time: a typo resolves to nothing
    /// at completion time and only logs a `Severity::Trace` line. Built-in
    /// commands still get a compile-time check by referencing the
    /// `&'static str` constants declared beside each source's registration
    /// (`completion::COMMAND_SOURCE` and siblings).
    pub completer: Option<Cow<'static, str>>,
    /// What `%` and `#` in this command's `:` argument mean.
    pub arg_expansion: ArgExpansion,
}

/// What `%` and `#` mean in a typed command's `:` argument.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::editor) enum ArgExpansion {
    /// A whole-token `%` becomes the current file's path and `#` the
    /// alternate file's.
    Paths,
    /// The argument reaches the command as typed.
    Verbatim,
}

/// How a [`TypedCommand`] executes when dispatched from `:`.
///
/// Three shapes, mirroring [`MappableCommand`]'s native/Steel/Lazy split,
/// but a typed command is never Steel-dispatchable *itself* the way
/// `MappableCommand::SteelBacked` is (there is no `(write)` bare binding);
/// `Steel`/`Lazy` here mean the command's *body* is a Steel lambda invoked
/// only from the `:` line, defined via `(define-typed-command! …)`.
pub(in crate::editor) enum TypedBody {
    /// A command implemented in Rust. Receives the editor, the pane focused
    /// when the `:` line was confirmed (captured once at invocation, the
    /// same rule a mappable command's target follows, so a body never reads
    /// focus again after its own work may have moved it), an optional
    /// argument (e.g. a file path), and whether `!` was appended.
    ///
    /// Kept as `&mut Editor` rather than a native `EditorCmd`'s `(&mut
    /// EditorState, &mut EngineView, …)` shape, for three reasons: the only
    /// callers are the `:` command line and tests, so `&mut Editor` here
    /// never runs while the Steel engine is borrowed; some handlers
    /// need shell-level fields (`:e` and `:set buffer language=`
    /// reach `scripting` for `activate_lazy_language_plugins` and
    /// `parse_worker` for `setup_buffer_syntax`, neither reachable from the
    /// coarse shape); and this is the Editor-orchestration layer, driving
    /// whole-app ops (`:w`, `:e`, `:bd`, `:split`, `:set language`) that
    /// legitimately span state + view + `parse_worker` + Steel together.
    Native(
        fn(&mut super::super::Editor, FocusedPane, Option<&str>, bool) -> Result<(), CommandError>,
    ),
    /// A command implemented as a Steel lambda, registered by
    /// `(define-typed-command! …)`. The lambda receives `(arg force)`,
    /// `(arg)`, or `()` based on its declared arity; see
    /// `Editor::run_typed_steel_command` in `dispatch.rs`.
    Steel(hume_scripting::SteelTypedCmdBody),
    /// A placeholder for a lazy plugin's typed command that has not yet been
    /// loaded. Mirrors [`MappableCommand::Lazy`]; see its doc.
    Lazy(hume_scripting::attribution::EntryId),
}
