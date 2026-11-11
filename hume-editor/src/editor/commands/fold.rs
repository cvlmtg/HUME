//! Folding: closing the regions a buffer's grammar names, and opening
//! closed folds. The fold commands and a press on a fold caret all act
//! through [`apply_fold`].

use hume_core::{Count, PaneId};
use hume_engine::pipeline::EngineView;
use hume_ops::MotionMode;
use hume_rope::line::ContentLine;

use super::CommandPane;
use crate::editor::EditorState;
use crate::editor::error::CommandError;
use crate::editor::syntax::{SyntaxUse, ensure_syntax_current};

const NO_TREE: &str = "No syntax tree to fold with";

/// What [`apply_fold`] does at a line.
#[derive(Copy, Clone, PartialEq, Eq)]
pub(in crate::editor) enum FoldOp {
    /// Open the closed fold starting there.
    Open,
    /// Close the innermost region holding it that is not closed already.
    Close,
    /// Open the closed fold starting there, or else close as [`FoldOp::Close`].
    Toggle,
}

/// Apply `op` at each of `lines` in pane `pid`. A hidden line acts as the
/// line shown in its place.
///
/// # Errors
/// When a region must close and the buffer has no syntax tree for its
/// current text. Nothing changes then.
pub(in crate::editor) fn apply_fold(
    state: &mut EditorState,
    view: &mut EngineView,
    pid: PaneId,
    lines: impl IntoIterator<Item = ContentLine>,
    op: FoldOp,
) -> Result<(), CommandError> {
    let hidden = state.hidden_lines(view, pid);
    let (open, close): (Vec<ContentLine>, Vec<ContentLine>) = lines
        .into_iter()
        .map(|line| hidden.shown_at_or_before(line))
        .partition(|&line| match op {
            FoldOp::Open => true,
            FoldOp::Close => false,
            FoldOp::Toggle => hidden.closed_at(line),
        });
    let bid = view.panes[pid].buffer_id();
    let mut folds = Vec::new();
    if !close.is_empty() {
        ensure_syntax_current(state, &mut view.registry, bid, SyntaxUse::Folds);
        let buf = state.buffers.get(bid);
        let layers = buf
            .current_layers()
            .ok_or_else(|| CommandError::transient(NO_TREE))?;
        let mut spanning = Vec::new();
        for line in close {
            layers.folds_spanning(buf.text().rope(), line, &mut spanning);
            folds.extend(spanning.iter().find(|fold| !hidden.closed_at(fold.first())));
        }
    }
    let text = state.buffers.get(bid).text();
    let pane = state.panes.entries.shown_mut(&view.panes, pid);
    for line in open {
        pane.open_fold_at(line);
    }
    pane.close_folds(folds, text);
    Ok(())
}

/// The line of each selection head of pane `t`.
fn head_lines(state: &EditorState, view: &EngineView, t: CommandPane) -> Vec<ContentLine> {
    let text = state.buffers.get(t.bid(view)).text();
    state
        .panes
        .entries
        .shown(&view.panes, t.pid())
        .view(text)
        .iter()
        .map(|sel| sel.head_line())
        .collect()
}

fn fold_at_heads(
    state: &mut EditorState,
    view: &mut EngineView,
    t: CommandPane,
    op: FoldOp,
) -> Result<(), CommandError> {
    let lines = head_lines(state, view, t);
    apply_fold(state, view, t.pid(), lines, op)
}

pub(in crate::editor) fn cmd_toggle_fold(
    state: &mut EditorState,
    view: &mut EngineView,
    t: CommandPane,
    _count: Count,
    _mode: MotionMode,
) -> Result<(), CommandError> {
    fold_at_heads(state, view, t, FoldOp::Toggle)
}

pub(in crate::editor) fn cmd_open_fold(
    state: &mut EditorState,
    view: &mut EngineView,
    t: CommandPane,
    _count: Count,
    _mode: MotionMode,
) -> Result<(), CommandError> {
    fold_at_heads(state, view, t, FoldOp::Open)
}

pub(in crate::editor) fn cmd_close_fold(
    state: &mut EditorState,
    view: &mut EngineView,
    t: CommandPane,
    _count: Count,
    _mode: MotionMode,
) -> Result<(), CommandError> {
    fold_at_heads(state, view, t, FoldOp::Close)
}

pub(in crate::editor) fn cmd_open_all_folds(
    state: &mut EditorState,
    view: &mut EngineView,
    t: CommandPane,
    _count: Count,
    _mode: MotionMode,
) -> Result<(), CommandError> {
    state
        .panes
        .entries
        .shown_mut(&view.panes, t.pid())
        .open_all_folds();
    Ok(())
}

/// Close every region the buffer's grammar names.
///
/// # Errors
/// When the buffer has no syntax tree for its current text.
pub(in crate::editor) fn cmd_close_all_folds(
    state: &mut EditorState,
    view: &mut EngineView,
    t: CommandPane,
    _count: Count,
    _mode: MotionMode,
) -> Result<(), CommandError> {
    let bid = t.bid(view);
    ensure_syntax_current(state, &mut view.registry, bid, SyntaxUse::Folds);
    let buf = state.buffers.get(bid);
    let layers = buf
        .current_layers()
        .ok_or_else(|| CommandError::transient(NO_TREE))?;
    let mut folds = Vec::new();
    layers.all_folds(buf.text().rope(), &mut folds);
    let text = buf.text();
    state
        .panes
        .entries
        .shown_mut(&view.panes, t.pid())
        .close_folds(folds, text);
    Ok(())
}
