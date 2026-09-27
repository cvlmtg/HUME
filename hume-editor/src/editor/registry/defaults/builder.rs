use std::borrow::Cow;

use crate::editor::commands::NativeBody;
use crate::editor::registry::{
    CommandRegistry, EditorCmdBody, FocusedCmdFn, MappableCommand, PaneCmdFn, SelectionTracking,
};

// Builder for EditorCmd registration. Each method sets one field (a bool,
// except the two `selection_tracking` setters below); .reg(registry)
// terminates the chain. Adding a new flag costs one method; existing call
// sites are unaffected.
pub(super) struct EditorCmdBuilder {
    name: &'static str,
    doc: &'static str,
    fun: EditorCmdBody,
    defers_paste_commit: bool,
    repeatable: bool,
    jump: bool,
    visual_move: bool,
    extendable: bool,
    clears_extend: bool,
    selection_tracking: SelectionTracking,
    manages_own_session: bool,
}
impl EditorCmdBuilder {
    pub(super) fn repeatable(mut self) -> Self {
        self.repeatable = true;
        self
    }
    pub(super) fn jump(mut self) -> Self {
        self.jump = true;
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
    /// next. Two distinct callers need this: the ring-cycle commands (`[`/`]`)
    /// use it so a chain of cycles folds into one undo step with the
    /// original paste; `repeat-last-action` uses it so a replayed ring-cycle
    /// command (`.` after `[`/`]`) still finds the session open: its own
    /// (non-deferring) dispatch would otherwise close it before the replay
    /// even runs. `Editor::replay_dot` makes the real commit/defer decision
    /// itself, from the REPLAYED command's own meta, once that command is
    /// known.
    pub(super) fn defers_paste_commit(mut self) -> Self {
        self.defers_paste_commit = true;
        self
    }
    /// Mark this command as one that opens or continues
    /// `EditorState::active_session` itself. See
    /// [`crate::editor::registry::CmdMeta::manages_own_session`] for the
    /// full rationale. Every paste-family registration in this file needs
    /// it; nothing else does.
    pub(super) fn manages_own_session(mut self) -> Self {
        self.manages_own_session = true;
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
    /// [`crate::editor::registry::CmdMeta::selection_tracking`].
    pub(super) fn establishes_selection(mut self) -> Self {
        self.selection_tracking = SelectionTracking::Establishes;
        self
    }
    /// Opt this command into the dot-repeat selection recipe as a composing
    /// step. See [`SelectionTracking::Composes`].
    pub(super) fn composes_selection(mut self) -> Self {
        self.selection_tracking = SelectionTracking::Composes;
        self
    }
    pub(super) fn reg(self, r: &mut CommandRegistry) {
        r.register(MappableCommand::EditorCmd {
            name: Cow::Borrowed(self.name),
            doc: Cow::Borrowed(self.doc),
            fun: NativeBody::new(self.fun),
            defers_paste_commit: self.defers_paste_commit,
            repeatable: self.repeatable,
            jump: self.jump,
            visual_move: self.visual_move,
            extendable: self.extendable,
            clears_extend: self.clears_extend,
            selection_tracking: self.selection_tracking,
            manages_own_session: self.manages_own_session,
        });
    }
}

fn ecmd_builder(name: &'static str, doc: &'static str, fun: EditorCmdBody) -> EditorCmdBuilder {
    EditorCmdBuilder {
        name,
        doc,
        fun,
        defers_paste_commit: false,
        repeatable: false,
        jump: false,
        visual_move: false,
        extendable: false,
        clears_extend: false,
        selection_tracking: SelectionTracking::Untracked,
        manages_own_session: false,
    }
}

// Two constructors, one per `EditorCmdBody` variant: a call site names its
// command's shape by which one it calls, and the compiler rejects a
// function pointer of the wrong shape (see `EditorCmdBody`'s own doc). No
// bare `ecmd` that takes a pre-built `EditorCmdBody`: that would let a
// registration build the enum value without ever naming its shape at the
// call site, the one thing this two-way split exists to force.

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
