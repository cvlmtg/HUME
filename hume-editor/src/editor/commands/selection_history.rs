use hume_core::Count;
use hume_engine::pipeline::EngineView;
use hume_ops::MotionMode;

use super::super::EditorState;
use super::super::selection_history::Walk;
use super::CommandPane;
use crate::editor::doc_ops;
use crate::editor::error::CommandError;

pub(in crate::editor) fn cmd_undo_selection_change(
    state: &mut EditorState,
    view: &mut EngineView,
    t: CommandPane,
    count: Count,
    _mode: MotionMode,
) -> Result<(), CommandError> {
    walk_selection_history(state, view, t, count, Walk::Back)
}

pub(in crate::editor) fn cmd_redo_selection_change(
    state: &mut EditorState,
    view: &mut EngineView,
    t: CommandPane,
    count: Count,
    _mode: MotionMode,
) -> Result<(), CommandError> {
    walk_selection_history(state, view, t, count, Walk::Forward)
}

/// Walk `t`'s selection history. The undo tree's part is read only when an
/// undo would be admitted; a step that needs it otherwise reports why.
fn walk_selection_history(
    state: &mut EditorState,
    view: &EngineView,
    t: CommandPane,
    count: Count,
    walk: Walk,
) -> Result<(), CommandError> {
    let (pid, bid) = (t.pid(), t.bid(view));
    let admitted = {
        let (buffers, mut stores) = state.edit_parts();
        doc_ops::admit_history_walk(buffers, &mut stores, pid, bid)
    };
    let buffer = state.buffers.get(bid);
    let saved = admitted.is_ok().then(|| buffer.saved_selections(pid));
    let walked = t
        .state_mut(&mut state.panes.entries, view)
        .walk_selection_history(count.get(), walk, saved.as_ref(), buffer.text());
    walked.or(admitted)
}
