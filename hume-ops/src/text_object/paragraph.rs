//! Inner/around paragraph text objects.

use hume_editing::state::EditState;

use super::apply_text_object_by_mode;
use crate::MotionMode;
use crate::motion::paragraph_at;

/// Inner paragraph: the paragraph's own lines, excluding any blank gap.
pub fn cmd_inner_paragraph(state: EditState, _count: usize, mode: MotionMode) -> EditState {
    apply_text_object_by_mode(state, mode, |t, p| paragraph_at(t, p, false))
}

/// Around paragraph: the paragraph plus its trailing blank gap, if any.
pub fn cmd_around_paragraph(state: EditState, _count: usize, mode: MotionMode) -> EditState {
    apply_text_object_by_mode(state, mode, |t, p| paragraph_at(t, p, true))
}
