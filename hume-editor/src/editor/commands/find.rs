use hume_editing::selection::SelectionSet;
use hume_editing::text::Text;
use hume_engine::pipeline::EngineView;
use crate::ops::MotionMode;
use crate::ops::motion::{find_char_backward, find_char_forward};

use super::super::{doc_ops, FindChar, EditorState};
use crate::editor::error::CommandError;
use crate::ops::motion::FindKind;

use super::focused_buffer_id;

// ── Find / till character ─────────────────────────────────────────────────────

fn find_char(
    state: &mut EditorState,
    view: &mut EngineView,
    count: usize,
    mode: MotionMode,
    kind: FindKind,
    find_fn: fn(&Text, SelectionSet, MotionMode, usize, char, FindKind) -> SelectionSet,
) {
    if let Some(ch) = state.pending_char.take() {
        let focused = state.focused_pane_id;
        let buf = focused_buffer_id(state, view);
        doc_ops::apply_doc_motion(&state.buffers, &mut state.panes.state, focused, buf, |b, s| {
            find_fn(b, s, mode, count, ch, kind)
        });
        state.last_find = Some(FindChar { ch, kind });
    }
}

pub fn cmd_find_forward(
    state: &mut EditorState,
    view: &mut EngineView,
    count: usize,
    mode: MotionMode,
) -> Result<(), CommandError> {
    find_char(state, view, count, mode, FindKind::Inclusive, find_char_forward);
    Ok(())
}
pub fn cmd_find_backward(
    state: &mut EditorState,
    view: &mut EngineView,
    count: usize,
    mode: MotionMode,
) -> Result<(), CommandError> {
    find_char(state, view, count, mode, FindKind::Inclusive, find_char_backward);
    Ok(())
}
pub fn cmd_till_forward(
    state: &mut EditorState,
    view: &mut EngineView,
    count: usize,
    mode: MotionMode,
) -> Result<(), CommandError> {
    find_char(state, view, count, mode, FindKind::Exclusive, find_char_forward);
    Ok(())
}
pub fn cmd_till_backward(
    state: &mut EditorState,
    view: &mut EngineView,
    count: usize,
    mode: MotionMode,
) -> Result<(), CommandError> {
    find_char(state, view, count, mode, FindKind::Exclusive, find_char_backward);
    Ok(())
}

// ── Repeat find ───────────────────────────────────────────────────────────────

fn repeat_find(
    state: &mut EditorState,
    view: &mut EngineView,
    count: usize,
    mode: MotionMode,
    find_fn: fn(&Text, SelectionSet, MotionMode, usize, char, FindKind) -> SelectionSet,
) {
    if let Some(FindChar { ch, kind }) = state.last_find {
        let focused = state.focused_pane_id;
        let buf = focused_buffer_id(state, view);
        doc_ops::apply_doc_motion(&state.buffers, &mut state.panes.state, focused, buf, |b, s| {
            find_fn(b, s, mode, count, ch, kind)
        });
    }
}

pub fn cmd_repeat_find_forward(
    state: &mut EditorState,
    view: &mut EngineView,
    count: usize,
    mode: MotionMode,
) -> Result<(), CommandError> {
    repeat_find(state, view, count, mode, find_char_forward);
    Ok(())
}
pub fn cmd_repeat_find_backward(
    state: &mut EditorState,
    view: &mut EngineView,
    count: usize,
    mode: MotionMode,
) -> Result<(), CommandError> {
    repeat_find(state, view, count, mode, find_char_backward);
    Ok(())
}
