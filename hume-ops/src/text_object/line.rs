//! Inner/around line text objects.

use hume_editing::grapheme::next_grapheme_boundary;
use hume_editing::lines::{is_empty_line, line_content_end, line_end_exclusive};
use hume_editing::selection::SelectionSet;
use hume_editing::text::BufferText;

use super::apply_text_object_by_mode;
use crate::MotionMode;

/// Inner line: the line content excluding the trailing newline.
/// Returns `None` for lines that contain only a newline (no content to select).
fn inner_line(buf: &BufferText, pos: usize) -> Option<(usize, usize)> {
    let line = buf.char_to_line(pos);
    if is_empty_line(buf, line) {
        return None; // empty line — no selectable content
    }
    let line_start = buf.line_to_char(line);
    // line_content_end returns the grapheme cluster *start* of the last
    // non-newline grapheme (uses prev_grapheme_boundary internally, so
    // combining clusters are handled correctly).
    let content_start = line_content_end(buf, line);
    // Convert grapheme start → last codepoint of that cluster, so the
    // selection includes all combining marks (same convention as inner_word).
    let end_inclusive = next_grapheme_boundary(buf, content_start).saturating_sub(1);
    Some((line_start, end_inclusive))
}

/// Around line: the full line including the trailing newline.
fn around_line(buf: &BufferText, pos: usize) -> Option<(usize, usize)> {
    let line = buf.char_to_line(pos);
    let start = buf.line_to_char(line);
    let end_excl = line_end_exclusive(buf, line);
    if end_excl == start {
        return None;
    }
    Some((start, end_excl - 1))
}

pub fn cmd_inner_line(
    buf: &BufferText,
    sels: SelectionSet,
    _count: usize,
    mode: MotionMode,
) -> SelectionSet {
    apply_text_object_by_mode(buf, sels, mode, inner_line)
}

pub fn cmd_around_line(
    buf: &BufferText,
    sels: SelectionSet,
    _count: usize,
    mode: MotionMode,
) -> SelectionSet {
    apply_text_object_by_mode(buf, sels, mode, around_line)
}
