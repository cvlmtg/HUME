use crate::core::grapheme::next_grapheme_boundary;
use crate::core::selection::Selection;
use crate::ops::MotionMode;
use crate::ops::edit::insert_char;
use crate::ops::motion::{
    cmd_goto_first_nonblank, cmd_goto_line_newline, cmd_goto_line_start, cmd_goto_line_end,
    cmd_move_left, cmd_move_right,
};

use super::super::{doc_ops, MiniBuffer, Mode};
use super::super::Editor;
use crate::core::error::CommandError;

// ── Mode transitions ──────────────────────────────────────────────────────────

pub fn cmd_insert_before(
    ed: &mut Editor,
    _count: usize,
    _mode: MotionMode,
) -> Result<(), CommandError> {
    let focused = ed.focused_pane_id;
    let buf = ed.focused_buffer_id();
    doc_ops::apply_doc_motion(&ed.buffers, &mut ed.pane_state, focused, buf, |_b, sels| {
        sels.map(|s| Selection::collapsed(s.start()))
    });
    ed.begin_insert_session();
    Ok(())
}

pub fn cmd_insert_after(
    ed: &mut Editor,
    _count: usize,
    _mode: MotionMode,
) -> Result<(), CommandError> {
    let focused = ed.focused_pane_id;
    let buf = ed.focused_buffer_id();
    doc_ops::apply_doc_motion(&ed.buffers, &mut ed.pane_state, focused, buf, |b, s| {
        cmd_move_right(b, s, 1, MotionMode::Move)
    });
    ed.begin_insert_session();
    Ok(())
}

pub fn cmd_insert_at_line_start(
    ed: &mut Editor,
    _count: usize,
    _mode: MotionMode,
) -> Result<(), CommandError> {
    let focused = ed.focused_pane_id;
    let buf = ed.focused_buffer_id();
    doc_ops::apply_doc_motion(&ed.buffers, &mut ed.pane_state, focused, buf, |b, s| {
        cmd_goto_first_nonblank(b, s, 1, MotionMode::Move)
    });
    ed.begin_insert_session();
    Ok(())
}

pub fn cmd_insert_at_line_end(
    ed: &mut Editor,
    _count: usize,
    _mode: MotionMode,
) -> Result<(), CommandError> {
    let focused = ed.focused_pane_id;
    let buf = ed.focused_buffer_id();
    doc_ops::apply_doc_motion(&ed.buffers, &mut ed.pane_state, focused, buf, |b, s| {
        cmd_goto_line_end(b, s, 1, MotionMode::Move)
    });
    doc_ops::apply_doc_motion(&ed.buffers, &mut ed.pane_state, focused, buf, |b, s| {
        cmd_move_right(b, s, 1, MotionMode::Move)
    });
    ed.begin_insert_session();
    Ok(())
}

/// Enter insert mode at the start of each selection (min of anchor and head).
/// For a collapsed cursor this is identical to `i`.
pub fn cmd_insert_at_selection_start(
    ed: &mut Editor,
    _count: usize,
    _mode: MotionMode,
) -> Result<(), CommandError> {
    let focused = ed.focused_pane_id;
    let buf = ed.focused_buffer_id();
    doc_ops::apply_doc_motion(&ed.buffers, &mut ed.pane_state, focused, buf, |_b, sels| {
        sels.map(|sel| Selection::collapsed(sel.start()))
    });
    ed.begin_insert_session();
    Ok(())
}

/// Enter insert mode after the end of each selection (one past max of anchor and head).
/// For a collapsed cursor this is identical to `a`.
///
/// Clamps to `len_chars() - 1` so pressing `a` on the structural trailing `\n`
/// (the last char in the buffer) does not place the cursor out of bounds.
pub fn cmd_insert_at_selection_end(
    ed: &mut Editor,
    _count: usize,
    _mode: MotionMode,
) -> Result<(), CommandError> {
    let focused = ed.focused_pane_id;
    let buf = ed.focused_buffer_id();
    doc_ops::apply_doc_motion(&ed.buffers, &mut ed.pane_state, focused, buf, |b, sels| {
        // len_chars() - 1 is safe: the buffer invariant guarantees at least one char.
        let max = b.len_chars() - 1;
        sels.map(|sel| Selection::collapsed(next_grapheme_boundary(b, sel.end()).min(max)))
    });
    ed.begin_insert_session();
    Ok(())
}

/// Open a new line below the cursor and enter insert mode.
///
/// `begin_insert_session` opens the edit group so the structural `\n` and
/// everything typed before Esc form one undo step — the same pattern as
/// `cmd_change`.
pub fn cmd_open_line_below(
    ed: &mut Editor,
    _count: usize,
    _mode: MotionMode,
) -> Result<(), CommandError> {
    let focused = ed.focused_pane_id;
    let buf = ed.focused_buffer_id();
    ed.begin_insert_session();
    doc_ops::apply_doc_motion(&ed.buffers, &mut ed.pane_state, focused, buf, |b, s| {
        cmd_goto_line_newline(b, s, 1, MotionMode::Move)
    });
    doc_ops::apply_doc_edit_grouped(&mut ed.buffers, &mut ed.pane_state, focused, buf, |b, s| {
        insert_char(b, s, '\n')
    });
    Ok(())
}

/// Open a new line above the cursor and enter insert mode.
pub fn cmd_open_line_above(
    ed: &mut Editor,
    _count: usize,
    _mode: MotionMode,
) -> Result<(), CommandError> {
    let focused = ed.focused_pane_id;
    let buf = ed.focused_buffer_id();
    ed.begin_insert_session();
    doc_ops::apply_doc_motion(&ed.buffers, &mut ed.pane_state, focused, buf, |b, s| {
        cmd_goto_line_start(b, s, 1, MotionMode::Move)
    });
    doc_ops::apply_doc_edit_grouped(&mut ed.buffers, &mut ed.pane_state, focused, buf, |b, s| {
        insert_char(b, s, '\n')
    });
    doc_ops::apply_doc_motion(&ed.buffers, &mut ed.pane_state, focused, buf, |b, s| {
        cmd_move_left(b, s, 1, MotionMode::Move)
    });
    Ok(())
}

pub fn cmd_command_mode(
    ed: &mut Editor,
    _count: usize,
    _mode: MotionMode,
) -> Result<(), CommandError> {
    ed.history.begin_session_all();
    ed.set_mode(Mode::Command);
    ed.minibuf = Some(MiniBuffer {
        prompt: ':',
        input: String::new(),
        cursor: 0,
    });
    Ok(())
}

pub fn cmd_exit_insert(
    ed: &mut Editor,
    _count: usize,
    _mode: MotionMode,
) -> Result<(), CommandError> {
    ed.end_insert_session();
    Ok(())
}
