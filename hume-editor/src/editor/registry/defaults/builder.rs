use std::borrow::Cow;

use crate::editor::registry::{CommandRegistry, EditorCmdFn, MappableCommand, SelectionTracking};

// Builder for EditorCmd registration. Each method sets one field (a bool,
// except the two `selection_tracking` setters below); .reg(registry)
// terminates the chain. Adding a new flag costs one method — existing call
// sites are unaffected.
pub(super) struct EditorCmdBuilder {
    name: &'static str,
    doc: &'static str,
    fun: EditorCmdFn,
    defers_paste_commit: bool,
    repeatable: bool,
    jump: bool,
    visual_move: bool,
    extendable: bool,
    clears_extend: bool,
    selection_tracking: SelectionTracking,
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
    /// command (`.` after `[`/`]`) still finds the session open — its own
    /// (non-deferring) dispatch would otherwise close it before the replay
    /// even runs. `Editor::replay_dot` makes the real commit/defer decision
    /// itself, from the REPLAYED command's own meta, once that command is
    /// known.
    pub(super) fn defers_paste_commit(mut self) -> Self {
        self.defers_paste_commit = true;
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
    /// (needs `EditorState`/`EngineView` access) — see
    /// [`crate::editor::registry::CmdMeta::selection_tracking`].
    pub(super) fn establishes_selection(mut self) -> Self {
        self.selection_tracking = SelectionTracking::Establishes;
        self
    }
    /// Opt this command into the dot-repeat selection recipe as a composing
    /// step — see [`SelectionTracking::Composes`].
    pub(super) fn composes_selection(mut self) -> Self {
        self.selection_tracking = SelectionTracking::Composes;
        self
    }
    pub(super) fn reg(self, r: &mut CommandRegistry) {
        r.register(MappableCommand::EditorCmd {
            name: Cow::Borrowed(self.name),
            doc: Cow::Borrowed(self.doc),
            fun: self.fun,
            defers_paste_commit: self.defers_paste_commit,
            repeatable: self.repeatable,
            jump: self.jump,
            visual_move: self.visual_move,
            extendable: self.extendable,
            clears_extend: self.clears_extend,
            selection_tracking: self.selection_tracking,
        });
    }
}
// Construct a builder for an EditorCmd. All handlers share one shape:
// fn(&mut EditorState, &mut EngineView, usize, MotionMode) -> Result<(), CommandError>.
pub(super) fn ecmd(name: &'static str, doc: &'static str, fun: EditorCmdFn) -> EditorCmdBuilder {
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
    }
}
