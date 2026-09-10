//! Inner/around paragraph text objects.

use hume_editing::selection::SelectionSet;
use hume_editing::text::BufferText;
use hume_rope::offset::InclusiveRange;

use super::apply_text_object_by_mode;
use crate::MotionMode;
use crate::motion::paragraph_at;

/// Inner paragraph: the paragraph's own lines, excluding any blank gap.
pub fn cmd_inner_paragraph(
    text: &BufferText,
    sels: SelectionSet,
    _count: usize,
    mode: MotionMode,
) -> SelectionSet {
    apply_text_object_by_mode(text, sels, mode, |t, p| {
        paragraph_at(t, p, false).map(|(s, e)| InclusiveRange::new(s, e))
    })
}

/// Around paragraph: the paragraph plus its trailing blank gap, if any.
pub fn cmd_around_paragraph(
    text: &BufferText,
    sels: SelectionSet,
    _count: usize,
    mode: MotionMode,
) -> SelectionSet {
    apply_text_object_by_mode(text, sels, mode, |t, p| {
        paragraph_at(t, p, true).map(|(s, e)| InclusiveRange::new(s, e))
    })
}
