use std::borrow::Cow;
use std::rc::Rc;

use crate::editor::commands::{CommandPane, FocusedPane, NativeBody};
use crate::editor::error::CommandError;
use crate::editor::jump_list::JumpRule;
use hume_core::Count;
use hume_editing::edit::Edited;
use hume_editing::state::EditState;
use hume_engine::pipeline::EngineView;
use hume_ops::search::SearchDirection;
use hume_ops::{LineEnd, MotionMode, WordCtx};
use hume_treesitter::textobjects::{Direction, ObjectKind, ObjectSpan};

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

/// Function pointer for an [`EditorCmdBody::FocusedPane`] handler: acts
/// through the focused pane, which the [`FocusedPane`] argument proves.
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
/// [`PaneCmdFn`] that also takes the text its operand names (typed at the
/// command's prompt, or a `call!` argument). The text is required: a key
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

/// The prompt a [`EditorCmdBody::PaneText`] command opens to read its text
/// operand when dispatched without one.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::editor) enum TextPrompt {
    Search(SearchDirection),
    Sift,
    /// The `|` line `pipe` reads its shell command from.
    Shell,
}

impl TextPrompt {
    /// What the operand this prompt reads is, for a message about it.
    pub(in crate::editor) fn operand_noun(self) -> &'static str {
        match self {
            Self::Search(_) | Self::Sift => "a pattern",
            Self::Shell => "a shell command",
        }
    }
}

/// Body shape for [`NativeCommand::EditorCmd`]: decides at compile time
/// whether `(call! "cmd" pane)` may act on a non-focused pane, and which
/// operand, if any, the command takes.
///
/// A command that moves focus, changes the focused pane's buffer, or touches
/// focus-bound state (a mode layer, sticky Extend, an Insert/paste session)
/// is `FocusedPane`, so an async caller must prove its handle is the focus
/// instead of acting on whatever is focused when it runs. Everything else is
/// `Pane`.
#[derive(Clone, Copy)]
pub(in crate::editor) enum EditorCmdBody {
    Pane(PaneCmdFn),
    FocusedPane(FocusedCmdFn),
    PaneChar(PaneCharCmdFn),
    PaneText(PaneTextCmdFn, TextPrompt),
}

// ── Command metadata for dispatch bookkeeping ────────────────────────────────

/// What a selection command does to the selections. The dot-repeat
/// selection recipe (`EditorState::update_recipe`, for a
/// [`SelectionRole::recipe_step`] command) and the selection history
/// (`selection_history::Record::of`) both read it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::editor) enum SelectionEffect {
    /// Moves the selections in Move mode and grows them in Extend mode.
    /// Every `Motion` carries this.
    Extends,
    /// Establishes an extent of its own (`select-line`, `ms(`, `m/`).
    Establishes,
    /// Transforms the extent already staged
    /// (`copy-selection-on-next-line`/`-prev-line`, `keep-primary-selection`).
    /// Replayed alone from a fresh cursor it would rebuild nothing, so as a
    /// recipe step it always appends.
    Composes,
}

/// What a command that builds selections does to them, and whether
/// dot-repeat replays it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::editor) struct SelectionRole {
    pub effect: SelectionEffect,
    /// Whether dot-repeat records this command as a selection-recipe step,
    /// to replay before the edit. `true` for every `Motion` and
    /// `Selection`; an `EditorCmd` opts in. A command that takes an operand
    /// or cannot be replayed from a fresh cursor (`find-forward`,
    /// `sift-within`) is no step, and clears the recipe.
    pub recipe_step: bool,
}

/// A command's bookkeeping properties, which the dispatch pipeline reads
/// instead of matching on the command variant.
///
/// Each field is one independent behaviour read by one dispatch stage. The
/// variant-to-property mapping lives only in `NativeCommand::meta()` and
/// `plain_meta`; the struct is `Copy` and carries no name so `meta()` stays
/// cheap enough that no caller re-`match`es the variant for a single bit.
#[derive(Debug, Clone, Copy)]
pub(in crate::editor) struct CmdMeta {
    /// What this command does to the selections, `None` for a command that
    /// builds none. `Extends` for every motion; `Selection` and `EditorCmd`
    /// set it per command. An `EditorCmd` can build a replayable extent from
    /// editor state, as `select-all-matches` (`m/`) does from the search
    /// pattern.
    pub selection: Option<SelectionRole>,
    /// Whether this command is a cursor motion rather than a selection
    /// builder, edit, or editor action. Selection commands are not: staging
    /// a text object is not navigation the jump list records.
    pub is_motion: bool,
    /// Whether this command defers the paste-session commit: `true` only for
    /// the ring-cycle commands (`[`/`]`), which fold into one undo step with
    /// the original paste.
    pub defers_paste_commit: bool,
    /// The jump this command declares, recorded however far the cursor
    /// moves (goto, search, page scroll, `select-all`, buffer switches).
    pub jump: Option<JumpRule>,
    /// Whether this command is a visual-line motion (`move-down`/`move-up`).
    /// Read only by [`Self::jump_rule`]. It does not gate the sticky
    /// display column, which a `Selection` keeps and clears itself.
    pub is_visual_move: bool,
    /// Whether `.` replays this command, and so whether the interactive
    /// inputs its code opens are recorded rather than ending the change in
    /// flight (`EditorState::waiting_input`).
    pub repeatable: bool,
    /// Whether this command exits sticky Extend mode after it runs: `true`
    /// for the buffer-modifying selection acts (`delete`, `paste-*`,
    /// `replace`, `surround-add`). `yank` keeps the selection, and `change`
    /// leaves through Insert.
    pub clears_extend: bool,
    /// Whether this command's landing spot should be re-aligned in the
    /// viewport per `EditorSettings::object_jump_align`, after it runs.
    /// `true` only for `Motion`'s forward object-jump family (`}`,
    /// `goto-next-<kind>`).
    pub aligns_view: bool,
    /// Whether this command opens or continues `EditorState::active_session`
    /// itself: `true` only for the paste family. `Editor::replay_dot` does
    /// not pre-open a session for such a command, since
    /// `paste-ring-older`/`-newer` need the already-open Paste session, not a
    /// fresh Replay one. Every other command relies on that pre-opened
    /// session to fold the recipe replay and the edit into one undo revision.
    ///
    /// A Steel command never sets this, yet its body can dispatch native
    /// paste, which retargets a pre-opened session to `Paste` in place.
    /// `Editor::finish_replay_session` reads the resulting session's kind to
    /// decide whether to leave it open for a following `[`/`]`.
    pub manages_own_session: bool,
    /// Whether only a key may run this command, so a script's `call!`
    /// refuses it. `true` only for `repeat-last-action`: replaying a change
    /// needs the Steel engine, which a `call!` from Steel runs inside, so
    /// the replay waits for the end of the keypress and would run after the
    /// script's own later calls.
    pub key_only: bool,
}

impl CmdMeta {
    /// The rule the pipeline records this command's jump under: its
    /// declared [`Self::jump`], else [`JumpRule::PrimaryPastThreshold`] for a
    /// motion. `None` for a command that does not move the cursor, which
    /// the pipeline runs outside `with_jump`: no jump-list snapshot, and a
    /// still-open Insert session keeps its pinned typed run.
    ///
    /// A Steel command or `Lazy` stub cannot declare motion semantics, so it
    /// answers `None`: the jump list does not record a Steel motion, and a
    /// pinned typed run survives it.
    pub(in crate::editor) fn jump_rule(&self) -> Option<JumpRule> {
        self.jump.or_else(|| {
            (self.is_motion || self.is_visual_move).then_some(JumpRule::PrimaryPastThreshold)
        })
    }
}

/// Function pointer for an [`Edit`](NativeCommand::Edit) handler.
pub(in crate::editor) type EditFn = fn(EditState) -> Edited;

// ── MappableCommand ───────────────────────────────────────────────────────────

/// Body shape for `Motion`/`Selection`'s `fun` field.
///
/// `Word` serves the word family (`w`/`W`/`b`/`B`, `mm`/`MM`, `miw`/`maw`),
/// which also needs the buffer's `word-chars` and effective
/// `word-selects-whitespace`. The word motions stay `Motion` rather than
/// `EditorCmd` because only the `Motion` variant sets
/// [`CmdMeta::is_motion`], which the jump list and dot-repeat read.
#[derive(Clone, Copy)]
pub(in crate::editor) enum SelectionBody {
    Plain(fn(EditState, usize, MotionMode) -> EditState),
    Word(fn(EditState, usize, WordCtx<'_>) -> EditState),
    /// `goto-line-end`, whose landing the pipeline resolves per pane through
    /// `line_end_for`.
    LineEnd(fn(EditState, usize, MotionMode, LineEnd) -> EditState),
    Structural(StructuralBody),
}

/// A structural command as data (kind × span, kind × direction), run by the
/// one interpreter in `commands/structural.rs`.
#[derive(Clone, Copy)]
pub(in crate::editor) enum StructuralBody {
    /// `m i <k>` / `m a <k>`: the smallest captured `<kind>.<span>` object at
    /// each cursor, or the kind's lexical form where none encloses it.
    /// Extend mode cannot grow an object to a parent that shares its end (a
    /// nested `def` closing where its class closes), the same limit the
    /// bracket objects have.
    Select { kind: ObjectKind, span: ObjectSpan },
    /// `goto-next-<k>` / `goto-prev-<k>`. A kind with a lexical form uses it
    /// in a buffer whose grammar does not define the kind.
    Goto { kind: ObjectKind, dir: Direction },
    /// `m i a` / `m a a`: `parameter.inside`, with the lexical scan as
    /// fallback.
    Argument { around: bool },
}

/// A command that can be bound to a key. The keymap trie stores command
/// names; the registry resolves them to `MappableCommand` values at dispatch
/// time.
///
/// Every [`NativeCommand`] variant's `fun` is a [`NativeBody`], whose private
/// field only `commands::pipeline::run_body` reads, so no caller can run a
/// body past `commands::run`'s bookkeeping (jump list, dot-repeat, paste
/// session).
#[derive(Clone)]
pub(in crate::editor) enum MappableCommand {
    /// A command implemented in Rust.
    Native(NativeCommand),
    /// A command implemented as a Steel (Scheme) lambda.
    SteelBacked(SteelCommand),
    /// A placeholder for a lazy plugin command that has not yet been loaded.
    ///
    /// Registered by `CommandHost::register_lazy_commands` as `declare-plugin!`
    /// processes its `#:commands` entries. When dispatched, the owning
    /// plugin's body is evaluated, the stub is replaced by the real
    /// `SteelBacked` command, and dispatch re-runs.
    Lazy {
        name: Rc<str>,
        plugin: hume_scripting::attribution::EntryId,
    },
}

impl MappableCommand {
    /// The command's registered name.
    ///
    /// Returns the stored `Rc` so the dispatch pipeline can keep the name
    /// past the registry borrow without a copy.
    pub(in crate::editor) fn name(&self) -> &Rc<str> {
        match self {
            Self::Native(cmd) => cmd.name(),
            Self::SteelBacked(cmd) => &cmd.name,
            Self::Lazy { name, .. } => name,
        }
    }

    /// One-line description of the command.
    #[cfg(test)]
    pub(in crate::editor) fn doc(&self) -> &str {
        match self {
            Self::Native(cmd) => cmd.doc(),
            Self::SteelBacked(cmd) => cmd.doc.as_ref(),
            Self::Lazy { .. } => "",
        }
    }

    /// Declarative metadata for the dispatch pipeline.
    #[cfg(test)]
    pub(in crate::editor) fn meta(&self) -> CmdMeta {
        match self {
            Self::Native(cmd) => cmd.meta(),
            Self::SteelBacked(cmd) => cmd.meta(),
            Self::Lazy { .. } => plain_meta(false),
        }
    }

    /// Whether a script can reach this command, by `call!` or by its bare
    /// binding. A [`CmdMeta::key_only`] command runs only from a key.
    pub(in crate::editor) fn runs_from_script(&self) -> bool {
        match self {
            Self::Native(cmd) => !cmd.meta().key_only,
            Self::SteelBacked(_) | Self::Lazy { .. } => true,
        }
    }

    /// Whether this command runs synchronously in Rust rather than through
    /// the Steel dispatch queue.
    pub(in crate::editor) fn is_native(&self) -> bool {
        match self {
            Self::Native(_) => true,
            Self::SteelBacked(_) | Self::Lazy { .. } => false,
        }
    }

    /// Whether a Ctrl-key may run this command as a one-shot extend. A Steel
    /// command always may: its lambda receives `extend` as its third
    /// argument, after pane and count.
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
    /// Motion that repeats `count` times. Always extendable: its
    /// `MotionMode` selects Move or Extend at dispatch time.
    Motion {
        name: Rc<str>,
        // Nothing in the editor displays a command's doc; only tests read it.
        #[allow(dead_code)]
        doc: Cow<'static, str>,
        fun: NativeBody<SelectionBody>,
        /// The jump this motion declares, recorded however far the cursor
        /// moves. Used for goto commands.
        jump: Option<JumpRule>,
        /// Whether this motion's landing spot is re-aligned after it runs.
        aligns_view: bool,
    },
    /// Selection or text-object operation, taking a count and a
    /// `MotionMode`; a body that needs neither ignores it.
    Selection {
        name: Rc<str>,
        // Nothing in the editor displays a command's doc; only tests read it.
        #[allow(dead_code)]
        doc: Cow<'static, str>,
        fun: NativeBody<SelectionBody>,
        /// The jump this command declares, recorded however far the cursor
        /// moves. Used for `select-all` (`%`).
        jump: Option<JumpRule>,
        /// `Establishes` for most selection commands, `Composes` for the few
        /// that transform what is already staged.
        selection_effect: SelectionEffect,
    },
    /// Buffer edit with no extra arguments. Never extendable: it takes no
    /// `MotionMode`.
    Edit {
        name: Rc<str>,
        // Nothing in the editor displays a command's doc; only tests read it.
        #[allow(dead_code)]
        doc: Cow<'static, str>,
        fun: NativeBody<EditFn>,
        /// Whether `.` replays this command: `false` for internal primitives
        /// like `delete-char-backward`.
        repeatable: bool,
    },
    /// Editor-level command on `EditorState` + `EngineView`, plus the target
    /// its [`EditorCmdBody`] variant demands: mode changes, register access,
    /// undo groups, parameterized motions (find/till/replace).
    ///
    /// An `Err` is reported at its own severity: `CommandError::transient`
    /// for a boundary condition ("no match"), `CommandError::new` for a real
    /// failure. A no-op returns `Ok(())`.
    EditorCmd {
        name: Rc<str>,
        // Nothing in the editor displays a command's doc; only tests read it.
        #[allow(dead_code)]
        doc: Cow<'static, str>,
        fun: NativeBody<EditorCmdBody>,
        /// `true` only for the ring-cycle commands (`[`/`]`).
        defers_paste_commit: bool,
        repeatable: bool,
        /// The jump this command declares, recorded however far the cursor
        /// moves. Used for search jumps, page scrolls and buffer switches.
        jump: Option<JumpRule>,
        /// Whether this command is a visual-line motion (`move-down`/`-up`).
        visual_move: bool,
        /// Whether a Ctrl-key may run this command as a one-shot extend.
        /// `Motion`/`Selection` always may and `Edit` never may, so only
        /// `EditorCmd` carries the flag. It is the opt-in of a command whose
        /// role is not [`SelectionEffect::Extends`]: an `Extends` command
        /// always may, and its registration leaves the flag unset.
        extendable: bool,
        clears_extend: bool,
        /// `None` unless the registration sets one.
        selection: Option<SelectionRole>,
        manages_own_session: bool,
        key_only: bool,
    },
}

impl NativeCommand {
    /// The command's registered name.
    pub(in crate::editor) fn name(&self) -> &Rc<str> {
        match self {
            Self::Motion { name, .. }
            | Self::Selection { name, .. }
            | Self::Edit { name, .. }
            | Self::EditorCmd { name, .. } => name,
        }
    }

    /// One-line description of the command.
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
    pub(in crate::editor) fn meta(&self) -> CmdMeta {
        match self {
            Self::Motion {
                jump, aligns_view, ..
            } => CmdMeta {
                selection: Some(SelectionRole {
                    effect: SelectionEffect::Extends,
                    recipe_step: true,
                }),
                is_motion: true,
                defers_paste_commit: false,
                jump: *jump,
                is_visual_move: false,
                repeatable: false,
                clears_extend: false,
                aligns_view: *aligns_view,
                manages_own_session: false,
                key_only: false,
            },
            Self::Selection {
                jump,
                selection_effect,
                ..
            } => CmdMeta {
                selection: Some(SelectionRole {
                    effect: *selection_effect,
                    recipe_step: true,
                }),
                is_motion: false,
                defers_paste_commit: false,
                jump: *jump,
                is_visual_move: false,
                repeatable: false,
                clears_extend: false,
                aligns_view: false,
                manages_own_session: false,
                key_only: false,
            },
            Self::Edit { repeatable, .. } => plain_meta(*repeatable),
            Self::EditorCmd {
                defers_paste_commit,
                repeatable,
                jump,
                visual_move,
                clears_extend,
                selection,
                manages_own_session,
                key_only,
                ..
            } => CmdMeta {
                selection: *selection,
                is_motion: false,
                defers_paste_commit: *defers_paste_commit,
                jump: *jump,
                is_visual_move: *visual_move,
                repeatable: *repeatable,
                clears_extend: *clears_extend,
                aligns_view: false,
                manages_own_session: *manages_own_session,
                key_only: *key_only,
            },
        }
    }

    /// Whether a Ctrl-key may run this command as a one-shot extend.
    pub(in crate::editor) fn is_extendable(&self) -> bool {
        match self {
            Self::Motion { .. } | Self::Selection { .. } => true,
            Self::Edit { .. } => false,
            Self::EditorCmd {
                extendable,
                selection,
                ..
            } => {
                *extendable || selection.is_some_and(|role| role.effect == SelectionEffect::Extends)
            }
        }
    }
}

/// A command implemented as a Steel (Scheme) lambda.
///
/// Dispatched by [`hume_scripting::ScriptingHost::call_steel_cmd`], which
/// applies `body.proc` directly; `call!` reaches it through
/// `%dispatch-command!`, which applies it inside the VM.
///
/// Every Steel command is extendable (a Ctrl-key passes `extend = #t`).
/// Dot-repeat is opt-in via `#:repeatable #t` in `(define-command! …)`.
#[derive(Clone)]
pub(in crate::editor) struct SteelCommand {
    pub(in crate::editor) name: Rc<str>,
    // Nothing in the editor displays a command's doc; only tests read it.
    #[allow(dead_code)]
    pub(in crate::editor) doc: Rc<str>,
    pub(in crate::editor) body: hume_scripting::SteelCmdBody,
}

impl SteelCommand {
    /// Declarative metadata for the dispatch pipeline.
    pub(in crate::editor) fn meta(&self) -> CmdMeta {
        plain_meta(self.body.repeatable)
    }
}

/// The metadata of a native edit, a Steel command or a `Lazy` stub: only
/// `repeatable` varies.
fn plain_meta(repeatable: bool) -> CmdMeta {
    CmdMeta {
        selection: None,
        is_motion: false,
        defers_paste_commit: false,
        jump: None,
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
    /// Declarative metadata for the dispatch pipeline.
    pub(in crate::editor) fn meta(&self) -> CmdMeta {
        match self {
            Self::Native(cmd) => cmd.meta(),
            Self::Steel(cmd) => cmd.meta(),
        }
    }
}

/// A command invocable from the `:` command line. It shares
/// [`super::CommandRegistry`]'s namespace with [`MappableCommand`]s, and its
/// body takes an optional argument and a `!` flag rather than a count.
pub(in crate::editor) struct TypedCommand {
    /// Canonical name, e.g. `"write"`; the registry key.
    pub name: Cow<'static, str>,
    /// Nothing in the editor displays a command's doc; only tests read it.
    #[allow(dead_code)]
    pub doc: Cow<'static, str>,
    /// Short aliases, e.g. `&["w"]`, each indexed for lookup. A
    /// Steel-registered typed command has none.
    pub aliases: &'static [&'static str],
    pub body: TypedBody,
    /// The name of this command's `:` argument-completion source in
    /// `completion::SourceRegistry`, looked up at completion time so a Steel
    /// typed command's `#:complete` can name a native or Steel-registered
    /// source. A name with no source logs only a `Severity::Trace` line; a
    /// built-in names its source through the constant beside that source's
    /// registration (`completion::THEME_SOURCE` and siblings).
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

/// How a [`TypedCommand`] executes when dispatched from `:`. A typed command
/// has no bare Steel binding: `Steel`/`Lazy` mean its body is a Steel lambda
/// from `(define-typed-command! …)`, run only from the `:` line.
pub(in crate::editor) enum TypedBody {
    /// A command implemented in Rust. Receives the editor, the pane focused
    /// when the `:` line was confirmed (captured once, so a body never reads
    /// focus after its own work may have moved it), and the inputs its
    /// [`NativeTyped`] shape takes.
    ///
    /// It takes `&mut Editor` rather than an `EditorCmd`'s split borrows: it
    /// never runs while the Steel engine is borrowed, and `:e` and
    /// `:set buffer language=` need shell-level fields (`scripting`,
    /// `parse_worker`).
    Native {
        native: NativeTyped,
        /// The jump this command declares, which the `:` dispatcher records
        /// in the focused pane's jump list.
        jump: Option<JumpRule>,
    },
    /// The lambda receives `(arg force)`, `(arg)`, or `()` by its declared
    /// arity.
    Steel(hume_scripting::SteelTypedCmdBody),
    /// A lazy plugin's typed command that has not been loaded yet, like
    /// [`MappableCommand::Lazy`].
    Lazy(hume_scripting::attribution::EntryId),
}

/// Whether a typed command takes an argument and a `!`. The `:` line
/// refuses either one the command does not take.
#[derive(Debug, Clone, Copy)]
pub(in crate::editor) struct Takes {
    arg: ArgTaken,
    force: ForceTaken,
}

/// Whether a typed command takes an argument.
#[derive(Debug, Clone, Copy)]
enum ArgTaken {
    Never,
    Optional,
    /// It cannot run without one; `usage` says what to type.
    Required {
        usage: &'static str,
    },
}

/// When a typed command takes a `!`.
#[derive(Debug, Clone, Copy)]
enum ForceTaken {
    Never,
    Always,
    /// Only on a line with no argument.
    WithoutArg,
}

impl Takes {
    /// What a Steel typed command's lambda takes: an argument when it has a
    /// second injected parameter, a `!` when it has a third.
    pub(in crate::editor) fn of_steel(body: &hume_scripting::SteelTypedCmdBody) -> Self {
        let params = super::super::dispatch::injected_params(body.arity, body.is_variadic);
        Self {
            arg: if params >= 2 {
                ArgTaken::Optional
            } else {
                ArgTaken::Never
            },
            force: if params >= 3 {
                ForceTaken::Always
            } else {
                ForceTaken::Never
            },
        }
    }

    /// `arg` and `force` for the typed command `name`, refusing either one
    /// it does not take, and a missing argument it requires. `expand` runs
    /// on an accepted argument only, so a refusal is what the user sees, not
    /// an expansion error.
    pub(in crate::editor) fn accept(
        self,
        name: &str,
        arg: Option<&str>,
        force: bool,
        expand: impl FnOnce(&str) -> Result<String, CommandError>,
    ) -> Result<Accepted, CommandError> {
        if arg.is_some() && matches!(self.arg, ArgTaken::Never) {
            return Err(refusal(name, "takes no argument"));
        }
        match (force, self.force) {
            (true, ForceTaken::Never) => return Err(refusal(name, "takes no `!`")),
            (true, ForceTaken::WithoutArg) if arg.is_some() => {
                return Err(refusal(name, "takes no `!` with an argument"));
            }
            _ => {}
        }
        if let (None, ArgTaken::Required { usage }) = (arg, self.arg) {
            return Err(usage_refusal(name, usage));
        }
        Ok(Accepted {
            arg: arg.map(expand).transpose()?,
            force,
        })
    }
}

/// The error for a typed command `name` refusing what was typed: an
/// argument or `!` it does not take is the user's mistake, not noise to
/// drop.
fn refusal(name: &str, reason: &str) -> CommandError {
    CommandError::new(format!("`:{name}` {reason}"))
}

/// The error for a typed command `name` given no argument, or one not in
/// the shape `usage` gives: a typo to correct, shown and not logged.
fn usage_refusal(name: &str, usage: &str) -> CommandError {
    CommandError::transient(format!("Usage: :{name} {usage}"))
}

/// What a typed command was given on the `:` line. Only [`Takes::accept`]
/// builds one, so a body never runs on an input its command refuses.
pub(in crate::editor) struct Accepted {
    arg: Option<String>,
    force: bool,
}

impl Accepted {
    pub(in crate::editor) fn arg(&self) -> Option<&str> {
        self.arg.as_deref()
    }

    pub(in crate::editor) fn force(&self) -> bool {
        self.force
    }
}

/// What a [`NativeTyped::ArgOrForce`] body gets: the argument, or with none,
/// whether `!` was typed.
#[derive(Debug, Clone, Copy)]
pub(in crate::editor) enum ArgOrForceInput<'a> {
    Arg(&'a str),
    Bare { force: bool },
}

type NativeResult = Result<(), CommandError>;

/// Why a [`NativeTyped::ArgRequired`] body did not run to completion.
#[derive(Debug)]
pub(in crate::editor) enum ArgError {
    /// The argument is not in the shape the command's usage gives. The
    /// dispatcher refuses it with that usage, as it refuses a missing one.
    Usage,
    /// The command failed.
    Failed(CommandError),
}

impl From<CommandError> for ArgError {
    fn from(e: CommandError) -> Self {
        Self::Failed(e)
    }
}

/// A native typed command's body, by the inputs it takes. The shape is the
/// declaration [`Takes`] reads, so a body cannot read an input the `:`
/// line would refuse.
#[derive(Clone, Copy)]
pub(in crate::editor) enum NativeTyped {
    Bare(fn(&mut super::super::Editor, FocusedPane) -> NativeResult),
    Arg(fn(&mut super::super::Editor, FocusedPane, Option<&str>) -> NativeResult),
    /// An argument the body cannot run without, and the usage a line
    /// without one, or with one the body finds malformed, is refused with.
    ArgRequired(
        fn(&mut super::super::Editor, FocusedPane, &str) -> Result<(), ArgError>,
        &'static str,
    ),
    Force(fn(&mut super::super::Editor, FocusedPane, bool) -> NativeResult),
    ArgForce(fn(&mut super::super::Editor, FocusedPane, Option<&str>, bool) -> NativeResult),
    /// An argument, or a `!` on a line with none.
    ArgOrForce(fn(&mut super::super::Editor, FocusedPane, ArgOrForceInput<'_>) -> NativeResult),
}

impl NativeTyped {
    pub(in crate::editor) fn takes(self) -> Takes {
        let (arg, force) = match self {
            Self::Bare(_) => (ArgTaken::Never, ForceTaken::Never),
            Self::Arg(_) => (ArgTaken::Optional, ForceTaken::Never),
            Self::ArgRequired(_, usage) => (ArgTaken::Required { usage }, ForceTaken::Never),
            Self::Force(_) => (ArgTaken::Never, ForceTaken::Always),
            Self::ArgForce(_) => (ArgTaken::Optional, ForceTaken::Always),
            Self::ArgOrForce(_) => (ArgTaken::Optional, ForceTaken::WithoutArg),
        };
        Takes { arg, force }
    }

    /// Runs the body of the typed command `name` with the inputs its shape
    /// takes, `input` having been accepted by [`Self::takes`].
    pub(in crate::editor) fn run(
        self,
        name: &str,
        ed: &mut super::super::Editor,
        fp: FocusedPane,
        input: &Accepted,
    ) -> NativeResult {
        let (arg, force) = (input.arg(), input.force());
        match self {
            Self::Bare(f) => f(ed, fp),
            Self::Arg(f) => f(ed, fp, arg),
            Self::ArgRequired(f, usage) => f(ed, fp, arg.expect("a missing argument is refused"))
                .map_err(|e| match e {
                    ArgError::Usage => usage_refusal(name, usage),
                    ArgError::Failed(e) => e,
                }),
            Self::Force(f) => f(ed, fp, force),
            Self::ArgForce(f) => f(ed, fp, arg, force),
            Self::ArgOrForce(f) => {
                debug_assert!(arg.is_none() || !force, "`!` with an argument is refused");
                f(
                    ed,
                    fp,
                    arg.map_or(ArgOrForceInput::Bare { force }, ArgOrForceInput::Arg),
                )
            }
        }
    }
}
