//! Inner/around line text objects.

use hume_editing::state::EditState;
use hume_editing::text::BufferText;
use hume_rope::cluster::{ClusterRange, ClusterStart};

use super::apply_text_object_by_mode;
use crate::MotionMode;

/// Inner line: the line content excluding the trailing newline.
/// Returns `None` for lines that contain only a newline (no content to select).
fn inner_line(text: &BufferText, pos: ClusterStart) -> Option<ClusterRange> {
    text.lines().content_range(text.char_to_line(pos.offset()))
}

/// Around line: the full line including the trailing newline.
fn around_line(text: &BufferText, pos: ClusterStart) -> Option<ClusterRange> {
    Some(text.lines().range(text.char_to_line(pos.offset())))
}

pub fn cmd_inner_line(state: EditState, _count: usize, mode: MotionMode) -> EditState {
    apply_text_object_by_mode(state, mode, inner_line)
}

pub fn cmd_around_line(state: EditState, _count: usize, mode: MotionMode) -> EditState {
    apply_text_object_by_mode(state, mode, around_line)
}
