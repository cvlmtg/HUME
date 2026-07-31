//! Inner/around argument (comma-separated item) text objects — function
//! arguments, array items, object fields, or any comma list inside brackets.

use hume_editing::grapheme::{next_grapheme_boundary, prev_grapheme_boundary};
use hume_editing::selection::SelectionSet;
use hume_editing::text::Text;

use super::apply_text_object_by_mode;
use crate::MotionMode;
use crate::pair::find_bracket_pair;

/// Find the tightest bracket pair among `()`, `[]`, `{}` that encloses `pos`.
///
/// Tries all three bracket types and returns the pair with the smallest span.
/// Tightest means innermost — for nested structures, we want the closest pair.
fn find_tightest_bracket_pair(buf: &Text, pos: usize) -> Option<(usize, usize)> {
    const PAIRS: [(char, char); 3] = [('(', ')'), ('[', ']'), ('{', '}')];
    PAIRS
        .iter()
        .filter_map(|&(open, close)| find_bracket_pair(buf, pos, open, close))
        .min_by_key(|&(o, c)| c - o)
}

/// Collect all comma-separated segments at depth 0 between `open_pos` and `close_pos`.
///
/// Returns a vec of `(start, end)` inclusive char-index pairs, one per segment,
/// including leading/trailing whitespace. Commas inside nested `()`, `[]`, or `{}`
/// are skipped. Returns an empty vec for adjacent brackets (`()`).
fn find_comma_segments(buf: &Text, open_pos: usize, close_pos: usize) -> Vec<(usize, usize)> {
    // Content zone: open_pos+1 ..= close_pos-1. Empty when brackets are adjacent.
    if close_pos <= open_pos + 1 {
        return Vec::new();
    }
    let content_start = open_pos + 1;
    let content_end = close_pos - 1; // inclusive

    let mut segments = Vec::new();
    let mut seg_start = content_start;
    let mut depth = 0usize;

    for (i, ch) in buf
        .chars_at(content_start)
        .take(content_end - content_start + 1)
    {
        match ch {
            '(' | '[' | '{' => depth += 1,
            ')' | ']' | '}' => depth = depth.saturating_sub(1),
            ',' if depth == 0 => {
                // i - 1 >= seg_start - 1; safe since seg_start >= content_start >= 1.
                segments.push((seg_start, i - 1));
                seg_start = i + 1;
            }
            _ => {}
        }
    }

    // Final segment: everything after the last comma, or the whole content if no commas.
    segments.push((seg_start, content_end));
    segments
}

/// Find which segment in `segments` contains `pos`.
///
/// If `pos` falls in a gap (e.g., on a comma between two segments), associate
/// it with the following segment — matching Helix/Kakoune behaviour.
fn which_segment(segments: &[(usize, usize)], pos: usize) -> Option<usize> {
    // Direct containment.
    for (idx, &(start, end)) in segments.iter().enumerate() {
        if pos >= start && pos <= end {
            return Some(idx);
        }
    }
    // pos is in a gap (on a comma). Return the next segment.
    for idx in 0..segments.len().saturating_sub(1) {
        let (_, prev_end) = segments[idx];
        let (next_start, _) = segments[idx + 1];
        if pos > prev_end && pos < next_start {
            return Some(idx + 1);
        }
    }
    None
}

/// Inner argument: the text of the comma-separated item at `pos`, with leading
/// and trailing whitespace trimmed.
///
/// Works for function arguments `foo(a, b)`, array items `[1, 2]`, object
/// fields `{x: 1, y: 2}`, and any comma-separated list inside brackets.
fn inner_argument(buf: &Text, pos: usize) -> Option<(usize, usize)> {
    let (open_pos, close_pos) = find_tightest_bracket_pair(buf, pos)?;

    // Nudge: if the cursor is on a bracket itself, step into the content zone.
    let pos = if pos == open_pos {
        open_pos + 1
    } else if pos == close_pos {
        close_pos.saturating_sub(1)
    } else {
        pos
    };

    let segments = find_comma_segments(buf, open_pos, close_pos);
    if segments.is_empty() {
        return None;
    }

    let idx = which_segment(&segments, pos)?;
    let (raw_start, raw_end) = segments[idx];

    // Trim leading whitespace. next_grapheme_boundary is required here because
    // `start` is a text position — raw `+= 1` would mis-step on multi-byte clusters.
    let mut start = raw_start;
    while start <= raw_end && matches!(buf.char_at(start), Some(' ' | '\t' | '\n' | '\r')) {
        start = next_grapheme_boundary(buf, start);
    }
    // Trim trailing whitespace.
    let mut end = raw_end;
    while end > start && matches!(buf.char_at(end), Some(' ' | '\t' | '\n' | '\r')) {
        end = prev_grapheme_boundary(buf, end);
    }
    // Segment is entirely whitespace — nothing to select.
    if start > raw_end {
        return None;
    }

    Some((start, end))
}

/// Around argument: the item plus its separator comma, so that deleting around
/// leaves a clean, properly-spaced list.
///
/// - **Only arg**: same as inner (no separator to consume).
/// - **First arg**: extend end through the trailing comma and any whitespace
///   leading into the next argument, so `delete(around aaa)` in `foo(aaa, bbb)`
///   yields `foo(bbb)` with no leading space.
/// - **Non-first arg**: extend start back to include the preceding comma,
///   so `delete(around bbb)` in `foo(aaa, bbb)` yields `foo(aaa)`.
fn around_argument(buf: &Text, pos: usize) -> Option<(usize, usize)> {
    let (open_pos, close_pos) = find_tightest_bracket_pair(buf, pos)?;

    // Nudge cursor off the bracket itself.
    let pos = if pos == open_pos {
        open_pos + 1
    } else if pos == close_pos {
        close_pos.saturating_sub(1)
    } else {
        pos
    };

    let segments = find_comma_segments(buf, open_pos, close_pos);
    if segments.is_empty() {
        return None;
    }

    let idx = which_segment(&segments, pos)?;
    let (raw_start, raw_end) = segments[idx];

    if segments.len() == 1 {
        // Only argument — no separator to eat; same as inner.
        return inner_argument(buf, pos);
    }

    if idx == 0 {
        // First arg: eat the trailing comma and skip whitespace to the start
        // of the next argument's content, so no orphan space is left behind.
        let (next_raw_start, next_raw_end) = segments[1];
        let mut end = next_raw_start;
        while end <= next_raw_end && matches!(buf.char_at(end), Some(' ' | '\t')) {
            end = next_grapheme_boundary(buf, end);
        }
        // `end` is now the first content char of the next segment.
        // Our range is raw_start ..= (end - 1), eating "aaa, ".
        Some((raw_start, end - 1)) // grapheme-safe: end was advanced by next_grapheme_boundary; -1 is the last codepoint of the preceding (whitespace) cluster
    } else {
        // Non-first arg: eat the preceding comma (it sits at prev_raw_end + 1).
        // The raw segment already includes any leading space after the comma,
        // so this range covers ", bbb" naturally.
        let prev_raw_end = segments[idx - 1].1;
        Some((prev_raw_end + 1, raw_end)) // grapheme-safe: comma is single-codepoint ASCII
    }
}

pub fn cmd_inner_argument(
    buf: &Text,
    sels: SelectionSet,
    _count: usize,
    mode: MotionMode,
) -> SelectionSet {
    apply_text_object_by_mode(buf, sels, mode, inner_argument)
}

pub fn cmd_around_argument(
    buf: &Text,
    sels: SelectionSet,
    _count: usize,
    mode: MotionMode,
) -> SelectionSet {
    apply_text_object_by_mode(buf, sels, mode, around_argument)
}
