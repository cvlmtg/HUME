use crate::core::text::Text;
use crate::core::grapheme::{next_grapheme_boundary, prev_grapheme_boundary};
use crate::helpers::{classify_char, is_word_boundary, is_WORD_boundary, line_content_end, line_end_exclusive, CharClass};
use crate::core::selection::{Selection, SelectionSet};
use super::pair::{find_bracket_pair, find_quote_pair};
use super::MotionMode;

// ── Text object framework ──────────────────────────────────────────────────────

/// Apply a text object to every selection in the set.
///
/// Unlike motions, which map a single cursor position to a new position, a
/// text object maps a cursor position to a *range* — the region to select.
/// `text_object` returns `Some((start, end))` as an inclusive char-offset pair,
/// or `None` if no match exists (e.g., cursor not inside any bracket pair).
///
/// On `None`, the existing selection is preserved — `mi(` when not inside parens
/// is a no-op. On `Some`, the selection is replaced with a
/// forward selection anchored at `start` and with head at `end`.
///
/// Uses `map_and_merge` so that multiple cursors landing on the same range
/// (e.g., both cursors inside the same bracket pair) are automatically merged.
pub(crate) fn apply_text_object(
    buf: &Text,
    sels: SelectionSet,
    text_object: impl Fn(&Text, usize) -> Option<(usize, usize)>,
) -> SelectionSet {
    let result = sels.map_and_merge(|sel| match text_object(buf, sel.head) {
        Some((start, end)) => Selection::new(start, end),
        None => sel,
    });
    result.debug_assert_valid(buf);
    result
}

/// Apply a text object in extend mode: union the matched range with the current selection.
///
/// On match, the result spans `min(sel.start(), start)` to `max(sel.end(), end)`,
/// preserving the direction of the original selection. On no-match, the selection
/// is unchanged.
///
/// Two-pass strategy for outward growth:
/// 1. Try `text_object(buf, sel.head)`. If the result is *larger* than the current
///    selection, use it — this handles the initial extend-from-cursor case.
/// 2. If the result is a subset (union doesn't grow), retry from the position just
///    past `sel.end()`. For bracket/quote text objects this escapes the current pair
///    and causes the search to find the next enclosing pair instead.
pub(crate) fn apply_text_object_extend(
    buf: &Text,
    sels: SelectionSet,
    text_object: impl Fn(&Text, usize) -> Option<(usize, usize)>,
) -> SelectionSet {
    let result = sels.map_and_merge(|sel| {
        let forward = sel.anchor <= sel.head;

        // First try from head (correct for initial extend from a cursor).
        if let Some((start, end)) = text_object(buf, sel.head) {
            let new_start = sel.start().min(start);
            let new_end = sel.end().max(end);
            if new_start != sel.start() || new_end != sel.end() {
                return Selection::directed(new_start, new_end, forward);
            }
        }

        // Result was a subset (no growth). Retry from one past the selection end so
        // bracket/quote searches find the enclosing pair rather than the current one.
        let past_end = next_grapheme_boundary(buf, sel.end());
        if past_end < buf.len_chars()
            && let Some((start, end)) = text_object(buf, past_end)
        {
            let new_start = sel.start().min(start);
            let new_end = sel.end().max(end);
            return Selection::directed(new_start, new_end, forward);
        }

        sel
    });
    result.debug_assert_valid(buf);
    result
}

/// Dispatch to [`apply_text_object`] or [`apply_text_object_extend`] based on `mode`.
#[inline]
fn apply_text_object_by_mode(
    buf: &Text,
    sels: SelectionSet,
    mode: MotionMode,
    f: impl Fn(&Text, usize) -> Option<(usize, usize)>,
) -> SelectionSet {
    match mode {
        MotionMode::Move   => apply_text_object(buf, sels, f),
        MotionMode::Extend => apply_text_object_extend(buf, sels, f),
    }
}

// ── Line ───────────────────────────────────────────────────────────────────────

/// Inner line: the line content excluding the trailing newline.
/// Returns `None` for lines that contain only a newline (no content to select).
fn inner_line(buf: &Text, pos: usize) -> Option<(usize, usize)> {
    let line = buf.char_to_line(pos);
    let line_start = buf.line_to_char(line);
    // line_content_end returns the grapheme cluster *start* of the last
    // non-newline grapheme (uses prev_grapheme_boundary internally, so
    // combining clusters are handled correctly). For empty lines it returns
    // line_start (the '\n' itself).
    let content_start = line_content_end(buf, line);
    if content_start == line_start && buf.char_at(line_start) == Some('\n') {
        return None; // empty line — no selectable content
    }
    // Convert grapheme start → last codepoint of that cluster, so the
    // selection includes all combining marks (same convention as inner_word).
    let end_inclusive = next_grapheme_boundary(buf, content_start).saturating_sub(1);
    Some((line_start, end_inclusive))
}

/// Around line: the full line including the trailing newline.
fn around_line(buf: &Text, pos: usize) -> Option<(usize, usize)> {
    let line = buf.char_to_line(pos);
    let start = buf.line_to_char(line);
    let end_excl = line_end_exclusive(buf, line);
    if end_excl == start {
        return None;
    }
    Some((start, end_excl - 1))
}

pub(crate) fn cmd_inner_line(buf: &Text, sels: SelectionSet, mode: MotionMode) -> SelectionSet {
    apply_text_object_by_mode(buf, sels, mode, inner_line)
}

pub(crate) fn cmd_around_line(buf: &Text, sels: SelectionSet, mode: MotionMode) -> SelectionSet {
    apply_text_object_by_mode(buf, sels, mode, around_line)
}


// ── Word / WORD ────────────────────────────────────────────────────────────────

/// Inner word parameterised by boundary predicate.
///
/// Scans left and right from `pos` while adjacent chars share the same
/// "class" (no boundary crossing). Whatever class the char at `pos` belongs
/// to defines the selected run — including whitespace runs and EOL.
pub(crate) fn inner_word_impl(
    buf: &Text,
    pos: usize,
    is_boundary: impl Fn(CharClass, CharClass) -> bool,
) -> Option<(usize, usize)> {
    let class = classify_char(buf.char_at(pos)?);

    // Scan left: walk back by grapheme cluster boundaries while the preceding
    // grapheme belongs to the same class. Using prev_grapheme_boundary ensures
    // we always inspect the *base* codepoint of each grapheme (not a combining
    // codepoint like U+0301 that would be misclassified as Punctuation).
    let mut start = pos;
    while start > 0 {
        let prev_pos = prev_grapheme_boundary(buf, start);
        let prev = classify_char(buf.char_at(prev_pos)?);
        if is_boundary(prev, class) {
            break;
        }
        start = prev_pos;
    }

    // Scan right: walk forward by grapheme cluster boundaries while the next
    // grapheme belongs to the same class. We track the grapheme-*start* position
    // and convert to an inclusive char-level end at the very end, so that the
    // returned range covers the full grapheme (including combining codepoints).
    let mut end_grapheme_start = pos;
    loop {
        let next_pos = next_grapheme_boundary(buf, end_grapheme_start);
        if next_pos >= buf.len_chars() {
            break;
        }
        let next = classify_char(buf.char_at(next_pos)?);
        if is_boundary(class, next) {
            break;
        }
        end_grapheme_start = next_pos;
    }
    // Convert grapheme start → inclusive char-level end. For a 1-codepoint
    // grapheme this equals end_grapheme_start. For e + U+0301 (2 codepoints),
    // next_grapheme_boundary returns start+2, so end = start+1 (the combining
    // codepoint), ensuring the selection includes the full grapheme cluster.
    // Subtracting 1 is safe: the buffer always has a trailing '\n', so
    // next_grapheme_boundary is always > 0.
    let end = next_grapheme_boundary(buf, end_grapheme_start) - 1; // grapheme-safe: result of next_grapheme_boundary is a cluster boundary; -1 is the last codepoint of the current cluster

    Some((start, end))
}

/// Around word parameterised by boundary predicate.
///
/// Computes the inner word range, then extends to include surrounding
/// whitespace. The rule (matching Vim/Helix):
/// - If the word is a real word (non-whitespace): prefer trailing whitespace,
///   fall back to leading whitespace if no trailing whitespace exists.
/// - If the word IS whitespace: extend to include the adjacent non-whitespace
///   word that follows (or precedes if at end of line).
fn around_word_impl(
    buf: &Text,
    pos: usize,
    is_boundary: impl Fn(CharClass, CharClass) -> bool + Copy,
) -> Option<(usize, usize)> {
    let (mut start, mut end) = inner_word_impl(buf, pos, is_boundary)?;
    let class = classify_char(buf.char_at(pos)?);

    // `next_pos` is the start of the grapheme cluster immediately after the
    // inner word. Since `end` is the last *char* of the last grapheme (as
    // returned by inner_word_impl), next_grapheme_boundary(end) gives the
    // first char of the following grapheme — equivalent to end + 1 for ASCII
    // but correct for multi-codepoint graphemes (e.g. e + combining accent).
    //
    // Similarly, `prev_start` is the start of the grapheme cluster immediately
    // before the inner word. Since `start` is always a grapheme-start position,
    // prev_grapheme_boundary(start) gives the start of the preceding grapheme —
    // equivalent to start - 1 for ASCII but safe for combining sequences.

    if class == CharClass::Space || class == CharClass::Eol {
        // The cursor is on whitespace. Extend to include the following word.
        // If there's no following word (e.g., at line end), include the
        // preceding word instead.
        let next_pos = next_grapheme_boundary(buf, end);
        if next_pos < buf.len_chars() {
            let next_class = classify_char(buf.char_at(next_pos)?);
            if next_class != CharClass::Space && next_class != CharClass::Eol {
                // Walk right to end of that word.
                let (_, word_end) = inner_word_impl(buf, next_pos, is_boundary)?;
                end = word_end;
            } else if start > 0 {
                // Try preceding word.
                let prev_start = prev_grapheme_boundary(buf, start);
                let prev_class = classify_char(buf.char_at(prev_start)?);
                if prev_class != CharClass::Space && prev_class != CharClass::Eol {
                    let (word_start, _) = inner_word_impl(buf, prev_start, is_boundary)?;
                    start = word_start;
                }
            }
        } else if start > 0 {
            let prev_start = prev_grapheme_boundary(buf, start);
            let prev_class = classify_char(buf.char_at(prev_start)?);
            if prev_class != CharClass::Space && prev_class != CharClass::Eol {
                let (word_start, _) = inner_word_impl(buf, prev_start, is_boundary)?;
                start = word_start;
            }
        }
    } else {
        // Real word. Try to include trailing whitespace first.
        let next_pos = next_grapheme_boundary(buf, end);
        if next_pos < buf.len_chars() {
            let next_class = classify_char(buf.char_at(next_pos)?);
            if next_class == CharClass::Space {
                // Walk right while whitespace.
                let (_, space_end) = inner_word_impl(buf, next_pos, is_boundary)?;
                end = space_end;
            } else {
                // No trailing space (next is Eol or another word) —
                // include leading whitespace instead.
                if start > 0 {
                    let prev_start = prev_grapheme_boundary(buf, start);
                    let prev_class = classify_char(buf.char_at(prev_start)?);
                    if prev_class == CharClass::Space {
                        let (space_start, _) = inner_word_impl(buf, prev_start, is_boundary)?;
                        start = space_start;
                    }
                }
            }
        } else if start > 0 {
            let prev_start = prev_grapheme_boundary(buf, start);
            let prev_class = classify_char(buf.char_at(prev_start)?);
            if prev_class == CharClass::Space {
                let (space_start, _) = inner_word_impl(buf, prev_start, is_boundary)?;
                start = space_start;
            }
        }
    }

    Some((start, end))
}

pub(crate) fn cmd_inner_word(buf: &Text, sels: SelectionSet, mode: MotionMode) -> SelectionSet {
    apply_text_object_by_mode(buf, sels, mode, |b, pos| inner_word_impl(b, pos, is_word_boundary))
}

pub(crate) fn cmd_around_word(buf: &Text, sels: SelectionSet, mode: MotionMode) -> SelectionSet {
    apply_text_object_by_mode(buf, sels, mode, |b, pos| around_word_impl(b, pos, is_word_boundary))
}


#[allow(non_snake_case)]
pub(crate) fn cmd_inner_WORD(buf: &Text, sels: SelectionSet, mode: MotionMode) -> SelectionSet {
    apply_text_object_by_mode(buf, sels, mode, |b, pos| inner_word_impl(b, pos, is_WORD_boundary))
}

#[allow(non_snake_case)]
pub(crate) fn cmd_around_WORD(buf: &Text, sels: SelectionSet, mode: MotionMode) -> SelectionSet {
    apply_text_object_by_mode(buf, sels, mode, |b, pos| around_word_impl(b, pos, is_WORD_boundary))
}


// ── Brackets ───────────────────────────────────────────────────────────────────

fn inner_bracket(buf: &Text, pos: usize, open: char, close: char) -> Option<(usize, usize)> {
    let (open_pos, close_pos) = find_bracket_pair(buf, pos, open, close)?;
    // Empty brackets: no valid inner range in the inclusive selection model.
    if open_pos + 1 > close_pos - 1 || close_pos == 0 {
        return None;
    }
    Some((open_pos + 1, close_pos - 1))
}

fn around_bracket(buf: &Text, pos: usize, open: char, close: char) -> Option<(usize, usize)> {
    find_bracket_pair(buf, pos, open, close)
}

macro_rules! bracket_cmds {
    ($inner_name:ident, $around_name:ident, $open:literal, $close:literal) => {
        pub(crate) fn $inner_name(buf: &Text, sels: SelectionSet, mode: MotionMode) -> SelectionSet {
            apply_text_object_by_mode(buf, sels, mode, |b, pos| inner_bracket(b, pos, $open, $close))
        }
        pub(crate) fn $around_name(buf: &Text, sels: SelectionSet, mode: MotionMode) -> SelectionSet {
            apply_text_object_by_mode(buf, sels, mode, |b, pos| around_bracket(b, pos, $open, $close))
        }
    };
}

bracket_cmds!(cmd_inner_paren, cmd_around_paren, '(', ')');
bracket_cmds!(cmd_inner_bracket, cmd_around_bracket, '[', ']');
bracket_cmds!(cmd_inner_brace, cmd_around_brace, '{', '}');
bracket_cmds!(cmd_inner_angle, cmd_around_angle, '<', '>');

// ── Quotes ─────────────────────────────────────────────────────────────────────

fn inner_quote(buf: &Text, pos: usize, quote: char) -> Option<(usize, usize)> {
    let (open, close) = find_quote_pair(buf, pos, quote)?;
    // Empty quotes: no inner range.
    if open + 1 > close - 1 || close == 0 {
        return None;
    }
    Some((open + 1, close - 1))
}

fn around_quote(buf: &Text, pos: usize, quote: char) -> Option<(usize, usize)> {
    find_quote_pair(buf, pos, quote)
}

macro_rules! quote_cmds {
    ($inner_name:ident, $around_name:ident, $quote:literal) => {
        pub(crate) fn $inner_name(buf: &Text, sels: SelectionSet, mode: MotionMode) -> SelectionSet {
            apply_text_object_by_mode(buf, sels, mode, |b, pos| inner_quote(b, pos, $quote))
        }
        pub(crate) fn $around_name(buf: &Text, sels: SelectionSet, mode: MotionMode) -> SelectionSet {
            apply_text_object_by_mode(buf, sels, mode, |b, pos| around_quote(b, pos, $quote))
        }
    };
}

quote_cmds!(cmd_inner_double_quote, cmd_around_double_quote, '"');
quote_cmds!(cmd_inner_single_quote, cmd_around_single_quote, '\'');
quote_cmds!(cmd_inner_backtick, cmd_around_backtick, '`');

// ── Arguments (comma-separated items) ──────────────────────────────────────────

/// Find the tightest bracket pair among `()`, `[]`, `{}` that encloses `pos`.
///
/// Tries all three bracket types and returns the pair with the smallest span.
/// Tightest means innermost — for nested structures, we want the closest pair.
fn find_tightest_bracket_pair(buf: &Text, pos: usize) -> Option<(usize, usize)> {
    const PAIRS: [(char, char); 3] = [('(', ')'), ('[', ']'), ('{', '}')];
    PAIRS.iter()
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
    let content_end   = close_pos - 1; // inclusive

    let mut segments  = Vec::new();
    let mut seg_start = content_start;
    let mut depth     = 0usize;
    let mut i         = content_start;

    while i <= content_end {
        match buf.char_at(i) {
            Some('(' | '[' | '{') => depth += 1,
            Some(')' | ']' | '}') => depth = depth.saturating_sub(1),
            Some(',') if depth == 0 => {
                // i - 1 >= seg_start - 1; safe since seg_start >= content_start >= 1.
                segments.push((seg_start, i - 1));
                seg_start = i + 1;
            }
            _ => {}
        }
        i += 1; // ASCII bracket/comma scanning — allowed per CLAUDE.md
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
        let (_, prev_end)    = segments[idx];
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

    let idx             = which_segment(&segments, pos)?;
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

pub(crate) fn cmd_inner_argument(buf: &Text, sels: SelectionSet, mode: MotionMode) -> SelectionSet {
    apply_text_object_by_mode(buf, sels, mode, inner_argument)
}

pub(crate) fn cmd_around_argument(buf: &Text, sels: SelectionSet, mode: MotionMode) -> SelectionSet {
    apply_text_object_by_mode(buf, sels, mode, around_argument)
}



#[cfg(test)]
mod tests;
