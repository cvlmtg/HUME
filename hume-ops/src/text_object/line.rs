//! Inner/around line text objects.

use hume_editing::lines::{is_empty_line, line_break_char, line_last_char};
use hume_editing::selection::SelectionSet;
use hume_editing::text::BufferText;
use hume_rope::offset::{CharOffset, InclusiveRange};

use super::apply_text_object_by_mode;
use crate::MotionMode;

/// Inner line: the line content excluding the trailing newline.
/// Returns `None` for lines that contain only a newline (no content to select).
fn inner_line(text: &BufferText, pos: CharOffset) -> Option<InclusiveRange<CharOffset>> {
    let line = text.char_to_line(pos);
    if is_empty_line(text, line.into()) {
        return None; // empty line — no selectable content
    }
    let line_start = text.line_to_char(line.into());
    Some(InclusiveRange::new(line_start, line_last_char(text, line)))
}

/// Around line: the full line including the trailing newline.
///
/// No `None` case: both callers of this closure (`apply_text_object`,
/// `apply_text_object_extend`) only ever pass a position strictly inside the
/// buffer (`sel.head()`, or a retry gated by `< text.end()`), so `line`
/// is always a real content line — exactly `line_break_char`'s precondition.
fn around_line(text: &BufferText, pos: CharOffset) -> Option<InclusiveRange<CharOffset>> {
    let line = text.char_to_line(pos);
    Some(InclusiveRange::new(
        text.line_to_char(line.into()),
        line_break_char(text, line),
    ))
}

pub fn cmd_inner_line(
    text: &BufferText,
    sels: SelectionSet,
    _count: usize,
    mode: MotionMode,
) -> SelectionSet {
    apply_text_object_by_mode(text, sels, mode, inner_line)
}

pub fn cmd_around_line(
    text: &BufferText,
    sels: SelectionSet,
    _count: usize,
    mode: MotionMode,
) -> SelectionSet {
    apply_text_object_by_mode(text, sels, mode, around_line)
}
