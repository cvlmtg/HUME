//! Inner/around paragraph text objects.

use hume_editing::selection::SelectionSet;
use hume_editing::text::BufferText;

use super::apply_text_object_by_mode;
use crate::MotionMode;
use crate::motion::{current_paragraph_start, paragraph_span};

/// Inner paragraph: the paragraph's own lines, excluding any blank gap.
fn inner_paragraph(text: &BufferText, pos: usize) -> Option<(usize, usize)> {
    Some(paragraph_span(
        text,
        current_paragraph_start(text, pos)?,
        false,
    ))
}

/// Around paragraph: the paragraph plus its trailing blank gap, if any.
fn around_paragraph(text: &BufferText, pos: usize) -> Option<(usize, usize)> {
    Some(paragraph_span(
        text,
        current_paragraph_start(text, pos)?,
        true,
    ))
}

pub fn cmd_inner_paragraph(
    text: &BufferText,
    sels: SelectionSet,
    _count: usize,
    mode: MotionMode,
) -> SelectionSet {
    apply_text_object_by_mode(text, sels, mode, inner_paragraph)
}

pub fn cmd_around_paragraph(
    text: &BufferText,
    sels: SelectionSet,
    _count: usize,
    mode: MotionMode,
) -> SelectionSet {
    apply_text_object_by_mode(text, sels, mode, around_paragraph)
}
