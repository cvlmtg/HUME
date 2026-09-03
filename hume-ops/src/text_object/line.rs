//! Inner/around line text objects.

use hume_editing::lines::{is_empty_line, line_end_exclusive, line_last_char};
use hume_editing::selection::SelectionSet;
use hume_editing::text::BufferText;

use super::apply_text_object_by_mode;
use crate::MotionMode;

/// Inner line: the line content excluding the trailing newline.
/// Returns `None` for lines that contain only a newline (no content to select).
fn inner_line(text: &BufferText, pos: usize) -> Option<(usize, usize)> {
    let line = text.char_to_line(pos);
    if is_empty_line(text, line) {
        return None; // empty line — no selectable content
    }
    let line_start = text.line_to_char(line);
    Some((line_start, line_last_char(text, line)))
}

/// Around line: the full line including the trailing newline.
fn around_line(text: &BufferText, pos: usize) -> Option<(usize, usize)> {
    let line = text.char_to_line(pos);
    let start = text.line_to_char(line);
    let end_excl = line_end_exclusive(text, line);
    if end_excl == start {
        return None;
    }
    Some((start, end_excl - 1))
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
