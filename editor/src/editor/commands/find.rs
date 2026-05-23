use crate::core::selection::SelectionSet;
use crate::core::text::Text;
use crate::ops::MotionMode;
use crate::ops::motion::{find_char_backward, find_char_forward};

use super::super::{doc_ops, FindChar};
use super::super::Editor;
use crate::core::error::CommandError;
use crate::ops::motion::FindKind;

// ── Find / till character ─────────────────────────────────────────────────────
//
// All eight find/till commands read the character argument from
// `ed.pending_char`, which was stored by the WaitChar consumption path.

/// Shared implementation for the eight find/till commands.
fn find_char(
    ed: &mut Editor,
    count: usize,
    mode: MotionMode,
    kind: FindKind,
    find_fn: fn(&Text, SelectionSet, MotionMode, usize, char, FindKind) -> SelectionSet,
) {
    if let Some(ch) = ed.pending_char.take() {
        let focused = ed.focused_pane_id;
        let buf = ed.focused_buffer_id();
        doc_ops::apply_doc_motion(&ed.buffers, &mut ed.pane_state, focused, buf, |b, s| {
            find_fn(b, s, mode, count, ch, kind)
        });
        ed.last_find = Some(FindChar { ch, kind });
    }
}

pub fn cmd_find_forward(
    ed: &mut Editor,
    count: usize,
    mode: MotionMode,
) -> Result<(), CommandError> {
    find_char(ed, count, mode, FindKind::Inclusive, find_char_forward);
    Ok(())
}
pub fn cmd_find_backward(
    ed: &mut Editor,
    count: usize,
    mode: MotionMode,
) -> Result<(), CommandError> {
    find_char(ed, count, mode, FindKind::Inclusive, find_char_backward);
    Ok(())
}
pub fn cmd_till_forward(
    ed: &mut Editor,
    count: usize,
    mode: MotionMode,
) -> Result<(), CommandError> {
    find_char(ed, count, mode, FindKind::Exclusive, find_char_forward);
    Ok(())
}
pub fn cmd_till_backward(
    ed: &mut Editor,
    count: usize,
    mode: MotionMode,
) -> Result<(), CommandError> {
    find_char(ed, count, mode, FindKind::Exclusive, find_char_backward);
    Ok(())
}

// ── Repeat find ───────────────────────────────────────────────────────────────

/// Shared implementation for the four repeat-find commands.
fn repeat_find(
    ed: &mut Editor,
    count: usize,
    mode: MotionMode,
    find_fn: fn(&Text, SelectionSet, MotionMode, usize, char, FindKind) -> SelectionSet,
) {
    if let Some(FindChar { ch, kind }) = ed.last_find {
        let focused = ed.focused_pane_id;
        let buf = ed.focused_buffer_id();
        doc_ops::apply_doc_motion(&ed.buffers, &mut ed.pane_state, focused, buf, |b, s| {
            find_fn(b, s, mode, count, ch, kind)
        });
    }
}

pub fn cmd_repeat_find_forward(
    ed: &mut Editor,
    count: usize,
    mode: MotionMode,
) -> Result<(), CommandError> {
    repeat_find(ed, count, mode, find_char_forward);
    Ok(())
}
pub fn cmd_repeat_find_backward(
    ed: &mut Editor,
    count: usize,
    mode: MotionMode,
) -> Result<(), CommandError> {
    repeat_find(ed, count, mode, find_char_backward);
    Ok(())
}

