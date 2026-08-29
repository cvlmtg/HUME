use std::borrow::Cow;

use crate::editor::registry::{CommandRegistry, EditorCmdFn, MappableCommand};

// Builder for EditorCmd registration. Each flag method sets one bool;
// .reg(registry) terminates the chain. Adding a new flag costs one
// method — existing call sites are unaffected.
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
    tracks_selection: bool,
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
    /// Mark as a ring-cycle command ([ / ]). Suppresses paste-session
    /// commit so ring cycles fold into one undo step with the original paste.
    pub(super) fn paste_cycle(mut self) -> Self {
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
    /// Opt this command into the dot-repeat selection recipe. Use only for a
    /// command that builds a replayable selection extent but can't be a pure
    /// `Selection` variant (needs `EditorState`/`EngineView` access) — see
    /// [`crate::editor::registry::CmdMeta::tracks_selection`].
    pub(super) fn tracks_selection(mut self) -> Self {
        self.tracks_selection = true;
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
            tracks_selection: self.tracks_selection,
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
        tracks_selection: false,
    }
}
