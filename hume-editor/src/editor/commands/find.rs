use hume_core::Count;
use hume_editing::state::EditState;
use hume_engine::pipeline::EngineView;
use hume_ops::MotionMode;
use hume_ops::motion::{
    find_char_backward, find_char_forward, repeat_find_char_backward, repeat_find_char_forward,
};

use super::super::EditorState;
use crate::editor::error::CommandError;
use hume_ops::motion::FindKind;

use super::{CommandPane, apply_pane_motion};

// ── Find/till state ───────────────────────────────────────────────────────────

/// The character and kind stored by the last find/till motion.
///
/// Direction is NOT stored: `repeat-find-forward` and `repeat-find-backward`
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
    t: CommandPane,
    count: usize,
    mode: MotionMode,
    find: FindChar,
    find_fn: fn(EditState, usize, MotionMode, char, FindKind) -> EditState,
) {
    let FindChar { ch, kind } = find;
    apply_pane_motion(state, view, t, |st| find_fn(st, count, mode, ch, kind));
    state.last_find = Some(find);
}

pub(in crate::editor) fn cmd_find_forward(
    state: &mut EditorState,
    view: &mut EngineView,
    t: CommandPane,
    count: Count,
    mode: MotionMode,
    ch: char,
) -> Result<(), CommandError> {
    find_char(
        state,
        view,
        t,
        count.get(),
        mode,
        FindChar {
            ch,
            kind: FindKind::Inclusive,
        },
        find_char_forward,
    );
    Ok(())
}
pub(in crate::editor) fn cmd_find_backward(
    state: &mut EditorState,
    view: &mut EngineView,
    t: CommandPane,
    count: Count,
    mode: MotionMode,
    ch: char,
) -> Result<(), CommandError> {
    find_char(
        state,
        view,
        t,
        count.get(),
        mode,
        FindChar {
            ch,
            kind: FindKind::Inclusive,
        },
        find_char_backward,
    );
    Ok(())
}
pub(in crate::editor) fn cmd_till_forward(
    state: &mut EditorState,
    view: &mut EngineView,
    t: CommandPane,
    count: Count,
    mode: MotionMode,
    ch: char,
) -> Result<(), CommandError> {
    find_char(
        state,
        view,
        t,
        count.get(),
        mode,
        FindChar {
            ch,
            kind: FindKind::Exclusive,
        },
        find_char_forward,
    );
    Ok(())
}
pub(in crate::editor) fn cmd_till_backward(
    state: &mut EditorState,
    view: &mut EngineView,
    t: CommandPane,
    count: Count,
    mode: MotionMode,
    ch: char,
) -> Result<(), CommandError> {
    find_char(
        state,
        view,
        t,
        count.get(),
        mode,
        FindChar {
            ch,
            kind: FindKind::Exclusive,
        },
        find_char_backward,
    );
    Ok(())
}

// ── Repeat find ───────────────────────────────────────────────────────────────

fn repeat_find(
    state: &mut EditorState,
    view: &mut EngineView,
    t: CommandPane,
    count: usize,
    mode: MotionMode,
    find_fn: fn(EditState, usize, MotionMode, char, FindKind) -> EditState,
) {
    if let Some(FindChar { ch, kind }) = state.last_find {
        apply_pane_motion(state, view, t, |st| find_fn(st, count, mode, ch, kind));
    }
}

pub(in crate::editor) fn cmd_repeat_find_forward(
    state: &mut EditorState,
    view: &mut EngineView,
    t: CommandPane,
    count: Count,
    mode: MotionMode,
) -> Result<(), CommandError> {
    repeat_find(state, view, t, count.get(), mode, repeat_find_char_forward);
    Ok(())
}
pub(in crate::editor) fn cmd_repeat_find_backward(
    state: &mut EditorState,
    view: &mut EngineView,
    t: CommandPane,
    count: Count,
    mode: MotionMode,
) -> Result<(), CommandError> {
    repeat_find(state, view, t, count.get(), mode, repeat_find_char_backward);
    Ok(())
}
