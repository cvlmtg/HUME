//! Inner/around paragraph text objects.

use hume_editing::selection::SelectionSet;
use hume_editing::text::BufferText;

use super::apply_text_object_by_mode;
use crate::MotionMode;
use crate::motion::{current_paragraph_start, paragraph_span};

/// Inner paragraph: the paragraph's own lines, excluding any blank gap.
pub fn cmd_inner_paragraph(
    text: &BufferText,
    sels: SelectionSet,
    _count: usize,
    mode: MotionMode,
) -> SelectionSet {
    apply_text_object_by_mode(text, sels, mode, |t, p| {
        Some(paragraph_span(t, current_paragraph_start(t, p)?, false))
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
        Some(paragraph_span(t, current_paragraph_start(t, p)?, true))
    })
}
