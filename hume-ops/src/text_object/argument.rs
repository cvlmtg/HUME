//! Inner/around argument (comma-separated item) text objects — function
//! arguments, array items, object fields, or any comma list inside brackets.

use hume_editing::grapheme::{next_grapheme_boundary, prev_grapheme_boundary};
use hume_editing::selection::SelectionSet;
use hume_editing::text::BufferText;

use super::apply_text_object_by_mode;
use crate::MotionMode;
use crate::pair::{BRACKET_PAIRS, find_bracket_pair};

/// One comma segment's inclusive `(start, end)` char range, leading and
/// trailing whitespace included.
type Segment = (usize, usize);

/// Find the tightest bracket pair among `()`, `[]`, `{}` that encloses `pos`.
///
/// Tries all three bracket types and returns the pair with the smallest span.
/// Tightest means innermost — for nested structures, we want the closest pair.
fn find_tightest_bracket_pair(text: &BufferText, pos: usize) -> Option<(usize, usize)> {
    BRACKET_PAIRS
        .iter()
        .filter_map(|&(open, close)| find_bracket_pair(text, pos, open, close))
        .min_by_key(|&(o, c)| c - o)
}

/// Collect all comma-separated segments at depth 0 between `open_pos` and `close_pos`.
///
/// Returns a vec of `(start, end)` inclusive char-index pairs, one per segment,
/// including leading/trailing whitespace. Commas inside nested `()`, `[]`, or `{}`
/// are skipped. Returns an empty vec for adjacent brackets (`()`).
fn find_comma_segments(text: &BufferText, open_pos: usize, close_pos: usize) -> Vec<Segment> {
    // Content zone: open_pos+1 ..= close_pos-1. Empty when brackets are adjacent.
    if close_pos <= open_pos + 1 {
        return Vec::new();
    }
    let content_start = open_pos + 1;
    let content_end = close_pos - 1; // inclusive

    let mut segments = Vec::new();
    let mut seg_start = content_start;
    let mut depth = 0usize;

    for (i, ch) in text
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
fn which_segment(segments: &[Segment], pos: usize) -> Option<usize> {
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

/// Resolve `pos` to its enclosing bracket pair's comma segments and the
/// index of the segment containing (or, on a comma gap, following) it.
///
/// Shared prelude for [`inner_argument`] and [`around_argument`]: locate the
/// tightest bracket pair, nudge `pos` off the bracket itself when it sits on
/// one, split the content into comma segments, and resolve which segment
/// `pos` falls in. Returns the nudged `pos` too — `around_argument`'s
/// only-argument case re-enters [`inner_argument`] with it, which lets that
/// case descend into a nested bracket pair instead of trimming the segment
/// already resolved against the outer one.
fn locate_argument(text: &BufferText, pos: usize) -> Option<(Vec<Segment>, usize, usize)> {
    let (open_pos, close_pos) = find_tightest_bracket_pair(text, pos)?;

    // Nudge: if the cursor is on a bracket itself, step into the content zone.
    let pos = if pos == open_pos {
        open_pos + 1
    } else if pos == close_pos {
        close_pos.saturating_sub(1)
    } else {
        pos
    };

    let segments = find_comma_segments(text, open_pos, close_pos);
    if segments.is_empty() {
        return None;
    }

    let idx = which_segment(&segments, pos)?;
    Some((segments, idx, pos))
}

/// Whitespace HUME's argument separator rule treats as blank: space, tab, or
/// newline. Shared by [`trim_segment`] (leading/trailing trim) and
/// [`around_from_inner`] (searching either side of an inner span for its
/// separator comma).
fn is_blank(text: &BufferText, pos: usize) -> bool {
    matches!(text.char_at(pos), Some(' ' | '\t' | '\n'))
}

/// Narrower than [`is_blank`]: space or tab only, no newline. Used only for
/// the run trailing a separator comma in [`around_from_inner`] — a line
/// break there belongs to the *next* argument's indentation, not to this
/// one's trailing whitespace, so `foo(\n    a,\n    b\n)` around `a` eats
/// `a,` and leaves the newline.
fn is_inline_blank(text: &BufferText, pos: usize) -> bool {
    matches!(text.char_at(pos), Some(' ' | '\t'))
}

/// Extends `pos` forward while the char immediately after it is blank,
/// returning the last position still covered by the run (`pos` itself if
/// the very next char isn't blank).
fn extend_forward_while(
    text: &BufferText,
    mut pos: usize,
    blank: impl Fn(&BufferText, usize) -> bool,
) -> usize {
    loop {
        let next = next_grapheme_boundary(text, pos);
        if next == pos || !blank(text, next) {
            return pos;
        }
        pos = next;
    }
}

/// Extends `pos` backward while the char immediately before it is blank,
/// returning the position at the start of the run (`pos` itself if the
/// preceding char isn't blank).
fn extend_backward_while(
    text: &BufferText,
    mut pos: usize,
    blank: impl Fn(&BufferText, usize) -> bool,
) -> usize {
    while pos > 0 {
        let prev = prev_grapheme_boundary(text, pos);
        if !blank(text, prev) {
            break;
        }
        pos = prev;
    }
    pos
}

/// Trim leading and trailing whitespace from a raw segment span. Returns
/// `None` if the segment is entirely whitespace.
fn trim_segment(text: &BufferText, (raw_start, raw_end): Segment) -> Option<(usize, usize)> {
    let mut start = raw_start;
    while start <= raw_end && is_blank(text, start) {
        start = next_grapheme_boundary(text, start);
    }
    let mut end = raw_end;
    while end > start && is_blank(text, end) {
        end = prev_grapheme_boundary(text, end);
    }
    // Segment is entirely whitespace — nothing to select.
    if start > raw_end {
        return None;
    }

    Some((start, end))
}

/// Inner argument: the text of the comma-separated item at `pos`, with leading
/// and trailing whitespace trimmed.
///
/// Works for function arguments `foo(a, b)`, array items `[1, 2]`, object
/// fields `{x: 1, y: 2}`, and any comma-separated list inside brackets.
pub fn inner_argument(text: &BufferText, pos: usize) -> Option<(usize, usize)> {
    let (segments, idx, _) = locate_argument(text, pos)?;
    trim_segment(text, segments[idx])
}

/// Derives an argument's "around" span from its "inner" span by locating its
/// separator comma — HUME's own rule, independent of how the inner span was
/// found (the lexical scan below, or a tree-sitter `parameter.inside`
/// capture), so `m i a`/`m a a` stay one structure-aware family rather than
/// two separate objects.
///
/// **Preceding separator first**: if the blank run immediately before
/// `start` is bounded by a comma, this argument is not first — the comma
/// and everything back to it becomes the new start, and `end` extends
/// forward over its own trailing blanks (a no-op for every argument but the
/// last, which has none to eat). Otherwise, if the blank run immediately
/// after `end` is bounded by a comma, this argument is first: `start`
/// extends backward over blanks — reaching the opening delimiter, never a
/// comma, since the first rule would have fired otherwise — and `end`
/// extends through the comma plus its inline blank run (space/tab only, see
/// [`is_inline_blank`]). An only argument matches neither rule and is
/// returned unchanged.
pub fn around_from_inner(text: &BufferText, (start, end): (usize, usize)) -> (usize, usize) {
    let before = extend_backward_while(text, start, is_blank);
    if before > 0 {
        let comma = prev_grapheme_boundary(text, before);
        if text.char_at(comma) == Some(',') {
            let new_end = extend_forward_while(text, end, is_blank);
            return (comma, new_end);
        }
    }

    let after = extend_forward_while(text, end, is_blank);
    let comma = next_grapheme_boundary(text, after);
    if text.char_at(comma) == Some(',') {
        let new_start = extend_backward_while(text, start, is_blank);
        let new_end = extend_forward_while(text, comma, is_inline_blank);
        return (new_start, new_end);
    }

    (start, end)
}

/// Around argument: the item plus its separator comma, so that deleting
/// around leaves a clean, properly-spaced list. See [`around_from_inner`]
/// for the separator rule itself.
pub fn around_argument(text: &BufferText, pos: usize) -> Option<(usize, usize)> {
    let (segments, idx, nudged_pos) = locate_argument(text, pos)?;

    if segments.len() == 1 {
        // Only argument — no separator to eat; same as inner. Re-enter
        // inner_argument (rather than trimming `segments[idx]` directly) so
        // a cursor on the outer bracket of `foo((a))` still resolves to the
        // nested pair's argument, not the whole `(a)` outer segment.
        return inner_argument(text, nudged_pos);
    }

    let inner = trim_segment(text, segments[idx])?;
    Some(around_from_inner(text, inner))
}

pub fn cmd_inner_argument(
    text: &BufferText,
    sels: SelectionSet,
    _count: usize,
    mode: MotionMode,
) -> SelectionSet {
    apply_text_object_by_mode(text, sels, mode, inner_argument)
}

pub fn cmd_around_argument(
    text: &BufferText,
    sels: SelectionSet,
    _count: usize,
    mode: MotionMode,
) -> SelectionSet {
    apply_text_object_by_mode(text, sels, mode, around_argument)
}
