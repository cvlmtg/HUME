use hume_editing::selection::SelectionSet;
use hume_editing::text::Text;
use hume_engine::pipeline::EngineView;
use hume_ops::MotionMode;
use hume_ops::motion::{find_char_backward, find_char_forward};

use super::super::EditorState;
use crate::editor::error::CommandError;
use hume_ops::motion::FindKind;

use super::apply_focused_motion;

// ── Find/till state ───────────────────────────────────────────────────────────

/// The character and kind stored by the last find/till motion.
///
/// Direction is NOT stored — `repeat-find-forward` and `repeat-find-backward`
/// use absolute direction, so re-searching always means "next on the right" or
/// "previous on the left" regardless of the original motion's direction.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct FindChar {
    pub ch: char,
    pub kind: FindKind,
}

// ── Find / till character ─────────────────────────────────────────────────────

fn find_char(
    state: &mut EditorState,
    view: &mut EngineView,
    count: usize,
    mode: MotionMode,
    kind: FindKind,
    find_fn: fn(&Text, SelectionSet, usize, MotionMode, char, FindKind) -> SelectionSet,
) {
    if let Some(ch) = state.pending_char.take() {
        apply_focused_motion(state, view, |b, s| find_fn(b, s, count, mode, ch, kind));
        state.last_find = Some(FindChar { ch, kind });
    }
}

pub fn cmd_find_forward(
    state: &mut EditorState,
    view: &mut EngineView,
    count: usize,
    mode: MotionMode,
) -> Result<(), CommandError> {
    find_char(
        state,
        view,
        count,
        mode,
        FindKind::Inclusive,
        find_char_forward,
    );
    Ok(())
}
pub fn cmd_find_backward(
    state: &mut EditorState,
    view: &mut EngineView,
    count: usize,
    mode: MotionMode,
) -> Result<(), CommandError> {
    find_char(
        state,
        view,
        count,
        mode,
        FindKind::Inclusive,
        find_char_backward,
    );
    Ok(())
}
pub fn cmd_till_forward(
    state: &mut EditorState,
    view: &mut EngineView,
    count: usize,
    mode: MotionMode,
) -> Result<(), CommandError> {
    find_char(
        state,
        view,
        count,
        mode,
        FindKind::Exclusive,
        find_char_forward,
    );
    Ok(())
}
pub fn cmd_till_backward(
    state: &mut EditorState,
    view: &mut EngineView,
    count: usize,
    mode: MotionMode,
) -> Result<(), CommandError> {
    find_char(
        state,
        view,
        count,
        mode,
        FindKind::Exclusive,
        find_char_backward,
    );
    Ok(())
}

// ── Repeat find ───────────────────────────────────────────────────────────────

fn repeat_find(
    state: &mut EditorState,
    view: &mut EngineView,
    count: usize,
    mode: MotionMode,
    find_fn: fn(&Text, SelectionSet, usize, MotionMode, char, FindKind) -> SelectionSet,
) {
    if let Some(FindChar { ch, kind }) = state.last_find {
        apply_focused_motion(state, view, |b, s| find_fn(b, s, count, mode, ch, kind));
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
