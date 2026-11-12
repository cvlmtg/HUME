use std::borrow::Cow;

use crate::editor::commands::NativeBody;
use crate::editor::jump_list::JumpRule;
use crate::editor::registry::{
    ArgExpansion, CommandRegistry, EditorCmdBody, FocusedCmdFn, MappableCommand, NativeCommand,
    NativeTyped, PaneCharCmdFn, PaneCmdFn, PaneTextCmdFn, SelectionEffect, SelectionRole,
    TextPrompt, TypedBody, TypedCommand,
};

// Builder for EditorCmd registration. Each method sets one field, except the
// three selection-role setters below, of which a command calls at most one;
// .reg(registry) terminates the chain. Adding a new flag costs one method;
// existing call sites are unaffected.
pub(super) struct EditorCmdBuilder {
    name: &'static str,
    doc: &'static str,
    fun: EditorCmdBody,
    defers_paste_commit: bool,
    repeatable: bool,
    jump: Option<JumpRule>,
    visual_move: bool,
    extendable: bool,
    clears_extend: bool,
    selection: Option<SelectionRole>,
    manages_own_session: bool,
    key_only: bool,
}
impl EditorCmdBuilder {
    pub(super) fn repeatable(mut self) -> Self {
        self.repeatable = true;
        self
    }
    pub(super) fn jump(mut self, rule: JumpRule) -> Self {
        self.jump = Some(rule);
        self
    }
    pub(super) fn visual_move(mut self) -> Self {
        self.visual_move = true;
        self
    }
    pub(super) fn extendable(mut self) -> Self {
        self.extendable = true;
        self
    }
    /// Suppress the automatic paste-session commit that normally runs before
    /// this command's own dispatch, leaving that decision to whatever runs
    /// next. The ring-cycle commands (`[`/`]`) use it so a chain of cycles
    /// folds into one undo step with the original paste.
    pub(super) fn defers_paste_commit(mut self) -> Self {
        self.defers_paste_commit = true;
        self
    }
    /// Mark this command as one that opens `EditorState::active_session`
    /// itself. See [`crate::editor::registry::CmdMeta::manages_own_session`]
    /// for the full rationale. The four paste commands need it; nothing else
    /// does.
    pub(super) fn manages_own_session(mut self) -> Self {
        self.manages_own_session = true;
        self
    }
    /// Refuse this command from a script's `call!`. See
    /// [`crate::editor::registry::CmdMeta::key_only`].
    pub(super) fn key_only(mut self) -> Self {
        self.key_only = true;
        self
    }
    /// Mark this as a selection-consuming edit that exits sticky Extend mode.
    /// Use for buffer-modifying acts: delete, paste, replace, surround-add.
    /// Do NOT use for yank, change, undo, redo, or mode-entry commands.
    pub(super) fn clears_extend(mut self) -> Self {
        self.clears_extend = true;
        self
    }
    /// Opt this command into the dot-repeat selection recipe as an
    /// establishing step. Use only for a command that builds a replayable
    /// selection extent on its own but can't be a pure `Selection` variant
    /// (needs `EditorState`/`EngineView` access); see
    /// [`crate::editor::registry::CmdMeta::selection`].
    pub(super) fn establishes_selection(self) -> Self {
        self.selection_role(SelectionRole {
            effect: SelectionEffect::Establishes,
            recipe_step: true,
        })
    }
    /// Opt this command into the dot-repeat selection recipe as a composing
    /// step. See [`SelectionEffect::Composes`].
    pub(super) fn composes_selection(self) -> Self {
        self.selection_role(SelectionRole {
            effect: SelectionEffect::Composes,
            recipe_step: true,
        })
    }
    /// Declare what this command does to the selections without making it a
    /// dot-repeat recipe step: it takes an operand or cannot be replayed
    /// from a fresh cursor. See [`SelectionRole::recipe_step`].
    pub(super) fn selection_effect(self, effect: SelectionEffect) -> Self {
        self.selection_role(SelectionRole {
            effect,
            recipe_step: false,
        })
    }
    fn selection_role(mut self, role: SelectionRole) -> Self {
        assert!(
            self.selection.is_none(),
            "{:?} declares a selection role twice",
            self.name
        );
        self.selection = Some(role);
        self
    }
    pub(super) fn reg(self, r: &mut CommandRegistry) {
        assert!(
            !(self.extendable
                && self
                    .selection
                    .is_some_and(|role| role.effect == SelectionEffect::Extends)),
            "{:?} is extendable through its Extends effect",
            self.name
        );
        r.register_builtin(MappableCommand::Native(NativeCommand::EditorCmd {
            name: self.name.into(),
            doc: Cow::Borrowed(self.doc),
            fun: NativeBody::new(self.fun),
            defers_paste_commit: self.defers_paste_commit,
            repeatable: self.repeatable,
            jump: self.jump,
            visual_move: self.visual_move,
            extendable: self.extendable,
            clears_extend: self.clears_extend,
            selection: self.selection,
            manages_own_session: self.manages_own_session,
            key_only: self.key_only,
        }));
    }
}

fn ecmd_builder(name: &'static str, doc: &'static str, fun: EditorCmdBody) -> EditorCmdBuilder {
    EditorCmdBuilder {
        name,
        doc,
        fun,
        defers_paste_commit: false,
        repeatable: false,
        jump: None,
        visual_move: false,
        extendable: false,
        clears_extend: false,
        selection: None,
        manages_own_session: false,
        key_only: false,
    }
}

// One constructor per `EditorCmdBody` variant: a call site names its
// command's shape by which one it calls, and the compiler rejects a
// function pointer of the wrong shape (see `EditorCmdBody`'s own doc). No
// bare `ecmd` that takes a pre-built `EditorCmdBody`: that would let a
// registration build the enum value without ever naming its shape at the
// call site, the one thing this split exists to force.

/// A command needing any pane showing the target buffer, not necessarily
/// the focused one. See [`crate::editor::registry::EditorCmdBody::Pane`].
pub(super) fn ecmd_pane(name: &'static str, doc: &'static str, fun: PaneCmdFn) -> EditorCmdBuilder {
    ecmd_builder(name, doc, EditorCmdBody::Pane(fun))
}

/// A command needing the *focused* pane to show the target buffer. See
/// [`crate::editor::registry::EditorCmdBody::FocusedPane`].
pub(super) fn ecmd_focused(
    name: &'static str,
    doc: &'static str,
    fun: FocusedCmdFn,
) -> EditorCmdBuilder {
    ecmd_builder(name, doc, EditorCmdBody::FocusedPane(fun))
}

/// An [`ecmd_pane`] command that takes a character operand. See
/// [`crate::editor::registry::EditorCmdBody::PaneChar`].
pub(super) fn ecmd_pane_char(
    name: &'static str,
    doc: &'static str,
    fun: PaneCharCmdFn,
) -> EditorCmdBuilder {
    ecmd_builder(name, doc, EditorCmdBody::PaneChar(fun))
}

/// An [`ecmd_pane`] command that takes a pattern, read at `prompt` when
/// dispatched without one. See
/// [`crate::editor::registry::EditorCmdBody::PaneText`].
pub(super) fn ecmd_pane_text(
    name: &'static str,
    doc: &'static str,
    fun: PaneTextCmdFn,
    prompt: TextPrompt,
) -> EditorCmdBuilder {
    ecmd_builder(name, doc, EditorCmdBody::PaneText(fun, prompt))
}

// Builder for a built-in typed command's registration, the `:` counterpart
// of `EditorCmdBuilder`: an argument completer, verbatim arguments and a jump
// are each one method, and .reg(registry) terminates the chain.
pub(super) struct TypedCmdBuilder {
    name: &'static str,
    doc: &'static str,
    aliases: &'static [&'static str],
    native: NativeTyped,
    completer: Option<&'static str>,
    arg_expansion: ArgExpansion,
    jump: Option<JumpRule>,
}
impl TypedCmdBuilder {
    /// Complete this command's argument from the source named `source`.
    pub(super) fn completer(mut self, source: &'static str) -> Self {
        self.completer = Some(source);
        self
    }
    /// Pass the argument as typed, without `%`/`#` path expansion.
    pub(super) fn verbatim(mut self) -> Self {
        self.arg_expansion = ArgExpansion::Verbatim;
        self
    }
    /// Record `rule`'s jump in the focused pane's jump list when the
    /// command moves it.
    pub(super) fn jump(mut self, rule: JumpRule) -> Self {
        self.jump = Some(rule);
        self
    }
    pub(super) fn reg(self, r: &mut CommandRegistry) {
        r.register_builtin_typed(TypedCommand {
            name: Cow::Borrowed(self.name),
            doc: Cow::Borrowed(self.doc),
            aliases: self.aliases,
            body: TypedBody::Native {
                native: self.native,
                jump: self.jump,
            },
            completer: self.completer.map(Cow::Borrowed),
            arg_expansion: self.arg_expansion,
        });
    }
}

/// A built-in typed command run as `native`.
pub(super) fn typed(
    name: &'static str,
    doc: &'static str,
    aliases: &'static [&'static str],
    native: NativeTyped,
) -> TypedCmdBuilder {
    TypedCmdBuilder {
        name,
        doc,
        aliases,
        native,
        completer: None,
        arg_expansion: ArgExpansion::Paths,
        jump: None,
    }
}
