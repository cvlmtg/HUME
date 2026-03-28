use crate::core::buffer::Buffer;
use crate::core::grapheme::{next_grapheme_boundary, prev_grapheme_boundary};
use crate::helpers::{classify_char, is_word_boundary, is_WORD_boundary, line_content_end, line_end_exclusive, CharClass};
use crate::core::selection::{Selection, SelectionSet};

// ── Text object framework ──────────────────────────────────────────────────────

/// Apply a text object to every selection in the set.
///
/// Unlike motions, which map a single cursor position to a new position, a
/// text object maps a cursor position to a *range* — the region to select.
/// `text_object` returns `Some((start, end))` as an inclusive char-offset pair,
/// or `None` if no match exists (e.g., cursor not inside any bracket pair).
///
/// On `None`, the existing selection is preserved (Helix behaviour: `mi(` when
/// not inside parens is a no-op). On `Some`, the selection is replaced with a
/// forward selection anchored at `start` and with head at `end`.
///
/// Uses `map_and_merge` so that multiple cursors landing on the same range
/// (e.g., both cursors inside the same bracket pair) are automatically merged.
pub(crate) fn apply_text_object(
    buf: &Buffer,
    sels: SelectionSet,
    text_object: impl Fn(&Buffer, usize) -> Option<(usize, usize)>,
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
    buf: &Buffer,
    sels: SelectionSet,
    text_object: impl Fn(&Buffer, usize) -> Option<(usize, usize)>,
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

// ── Line ───────────────────────────────────────────────────────────────────────

/// Inner line: the line content excluding the trailing newline.
/// Returns `None` for lines that contain only a newline (no content to select).
fn inner_line(buf: &Buffer, pos: usize) -> Option<(usize, usize)> {
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
fn around_line(buf: &Buffer, pos: usize) -> Option<(usize, usize)> {
    let line = buf.char_to_line(pos);
    let start = buf.line_to_char(line);
    let end_excl = line_end_exclusive(buf, line);
    if end_excl == start {
        return None;
    }
    Some((start, end_excl - 1))
}

pub(crate) fn cmd_inner_line(buf: &Buffer, sels: SelectionSet) -> SelectionSet {
    apply_text_object(buf, sels, inner_line)
}

pub(crate) fn cmd_around_line(buf: &Buffer, sels: SelectionSet) -> SelectionSet {
    apply_text_object(buf, sels, around_line)
}

pub(crate) fn cmd_extend_inner_line(buf: &Buffer, sels: SelectionSet) -> SelectionSet {
    apply_text_object_extend(buf, sels, inner_line)
}

pub(crate) fn cmd_extend_around_line(buf: &Buffer, sels: SelectionSet) -> SelectionSet {
    apply_text_object_extend(buf, sels, around_line)
}

// ── Word / WORD ────────────────────────────────────────────────────────────────

/// Inner word parameterised by boundary predicate.
///
/// Scans left and right from `pos` while adjacent chars share the same
/// "class" (no boundary crossing). Whatever class the char at `pos` belongs
/// to defines the selected run — including whitespace runs and EOL.
fn inner_word_impl(
    buf: &Buffer,
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
    buf: &Buffer,
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

pub(crate) fn cmd_inner_word(buf: &Buffer, sels: SelectionSet) -> SelectionSet {
    apply_text_object(buf, sels, |b, pos| inner_word_impl(b, pos, is_word_boundary))
}

pub(crate) fn cmd_around_word(buf: &Buffer, sels: SelectionSet) -> SelectionSet {
    apply_text_object(buf, sels, |b, pos| around_word_impl(b, pos, is_word_boundary))
}

pub(crate) fn cmd_extend_inner_word(buf: &Buffer, sels: SelectionSet) -> SelectionSet {
    apply_text_object_extend(buf, sels, |b, pos| inner_word_impl(b, pos, is_word_boundary))
}

pub(crate) fn cmd_extend_around_word(buf: &Buffer, sels: SelectionSet) -> SelectionSet {
    apply_text_object_extend(buf, sels, |b, pos| around_word_impl(b, pos, is_word_boundary))
}

#[allow(non_snake_case)]
pub(crate) fn cmd_inner_WORD(buf: &Buffer, sels: SelectionSet) -> SelectionSet {
    apply_text_object(buf, sels, |b, pos| inner_word_impl(b, pos, is_WORD_boundary))
}

#[allow(non_snake_case)]
pub(crate) fn cmd_around_WORD(buf: &Buffer, sels: SelectionSet) -> SelectionSet {
    apply_text_object(buf, sels, |b, pos| around_word_impl(b, pos, is_WORD_boundary))
}

#[allow(non_snake_case)]
pub(crate) fn cmd_extend_inner_WORD(buf: &Buffer, sels: SelectionSet) -> SelectionSet {
    apply_text_object_extend(buf, sels, |b, pos| inner_word_impl(b, pos, is_WORD_boundary))
}

#[allow(non_snake_case)]
pub(crate) fn cmd_extend_around_WORD(buf: &Buffer, sels: SelectionSet) -> SelectionSet {
    apply_text_object_extend(buf, sels, |b, pos| around_word_impl(b, pos, is_WORD_boundary))
}

// ── Brackets ───────────────────────────────────────────────────────────────────

/// Scan left from `pos` (exclusive) to find an unmatched `open` bracket.
/// `depth` is the pre-loaded nesting depth (pass 0 when starting fresh).
fn scan_left_for_open(buf: &Buffer, pos: usize, open: char, close: char) -> Option<usize> {
    let mut depth = 0usize;
    let mut i = pos;
    loop {
        if i == 0 {
            return None;
        }
        i -= 1;
        match buf.char_at(i) {
            Some(ch) if ch == close => depth += 1,
            Some(ch) if ch == open => {
                if depth == 0 {
                    return Some(i);
                }
                depth -= 1;
            }
            _ => {}
        }
    }
}

/// Scan right from `pos` (exclusive) to find an unmatched `close` bracket.
fn scan_right_for_close(buf: &Buffer, pos: usize, open: char, close: char) -> Option<usize> {
    let mut depth = 0usize;
    let len = buf.len_chars();
    let mut i = pos;
    while i < len {
        match buf.char_at(i) {
            Some(ch) if ch == open => depth += 1,
            Some(ch) if ch == close => {
                if depth == 0 {
                    return Some(i);
                }
                depth -= 1;
            }
            _ => {}
        }
        i += 1;
    }
    None
}

/// Find the innermost bracket pair `(open, close)` that encloses `pos`.
///
/// If the cursor is ON an open bracket, that bracket itself is the start.
/// If ON a close bracket, that bracket is the end.
/// Otherwise, scans both directions for the enclosing pair.
pub(crate) fn find_bracket_pair(buf: &Buffer, pos: usize, open: char, close: char) -> Option<(usize, usize)> {
    match buf.char_at(pos)? {
        ch if ch == open => {
            // Cursor is on an open bracket — scan right for the matching close.
            let close_pos = scan_right_for_close(buf, pos + 1, open, close)?;
            Some((pos, close_pos))
        }
        ch if ch == close => {
            // Cursor is on a close bracket — scan left for the matching open.
            let open_pos = scan_left_for_open(buf, pos, open, close)?;
            Some((open_pos, pos))
        }
        _ => {
            // Cursor is inside — scan both directions.
            let open_pos = scan_left_for_open(buf, pos, open, close)?;
            let close_pos = scan_right_for_close(buf, pos, open, close)?;
            Some((open_pos, close_pos))
        }
    }
}

fn inner_bracket(buf: &Buffer, pos: usize, open: char, close: char) -> Option<(usize, usize)> {
    let (open_pos, close_pos) = find_bracket_pair(buf, pos, open, close)?;
    // Empty brackets: no valid inner range in the inclusive selection model.
    if open_pos + 1 > close_pos - 1 || close_pos == 0 {
        return None;
    }
    Some((open_pos + 1, close_pos - 1))
}

fn around_bracket(buf: &Buffer, pos: usize, open: char, close: char) -> Option<(usize, usize)> {
    find_bracket_pair(buf, pos, open, close)
}

macro_rules! bracket_cmds {
    ($inner_name:ident, $around_name:ident, $ext_inner_name:ident, $ext_around_name:ident, $open:literal, $close:literal) => {
        pub(crate) fn $inner_name(buf: &Buffer, sels: SelectionSet) -> SelectionSet {
            apply_text_object(buf, sels, |b, pos| inner_bracket(b, pos, $open, $close))
        }
        pub(crate) fn $around_name(buf: &Buffer, sels: SelectionSet) -> SelectionSet {
            apply_text_object(buf, sels, |b, pos| around_bracket(b, pos, $open, $close))
        }
        pub(crate) fn $ext_inner_name(buf: &Buffer, sels: SelectionSet) -> SelectionSet {
            apply_text_object_extend(buf, sels, |b, pos| inner_bracket(b, pos, $open, $close))
        }
        pub(crate) fn $ext_around_name(buf: &Buffer, sels: SelectionSet) -> SelectionSet {
            apply_text_object_extend(buf, sels, |b, pos| around_bracket(b, pos, $open, $close))
        }
    };
}

bracket_cmds!(cmd_inner_paren, cmd_around_paren, cmd_extend_inner_paren, cmd_extend_around_paren, '(', ')');
bracket_cmds!(cmd_inner_bracket, cmd_around_bracket, cmd_extend_inner_bracket, cmd_extend_around_bracket, '[', ']');
bracket_cmds!(cmd_inner_brace, cmd_around_brace, cmd_extend_inner_brace, cmd_extend_around_brace, '{', '}');
bracket_cmds!(cmd_inner_angle, cmd_around_angle, cmd_extend_inner_angle, cmd_extend_around_angle, '<', '>');

// ── Quotes ─────────────────────────────────────────────────────────────────────

/// Find the quote pair on the current line that encloses or is nearest to `pos`.
///
/// Quotes don't span lines (current limitation). Strategy: scan the current line
/// tracking parity — odd occurrences are opening quotes, even occurrences are
/// closing quotes. Returns the pair that contains `pos`.
///
/// If `pos` is ON a quote char, parity resolves whether it is open or close.
fn find_quote_pair(buf: &Buffer, pos: usize, quote: char) -> Option<(usize, usize)> {
    let line = buf.char_to_line(pos);
    let line_start = buf.line_to_char(line);
    let line_end = line_end_exclusive(buf, line);

    // Single pass: track the opening quote position; on every second hit we
    // have a complete pair and can test whether `pos` falls inside it.
    let mut open: Option<usize> = None;
    for i in line_start..line_end {
        if buf.char_at(i) == Some(quote) {
            match open {
                None => open = Some(i), // odd occurrence → opening quote
                Some(open_pos) => {     // even occurrence → closing quote
                    if open_pos <= pos && pos <= i {
                        return Some((open_pos, i));
                    }
                    open = None; // reset for next pair
                }
            }
        }
    }
    None
}

fn inner_quote(buf: &Buffer, pos: usize, quote: char) -> Option<(usize, usize)> {
    let (open, close) = find_quote_pair(buf, pos, quote)?;
    // Empty quotes: no inner range.
    if open + 1 > close - 1 || close == 0 {
        return None;
    }
    Some((open + 1, close - 1))
}

fn around_quote(buf: &Buffer, pos: usize, quote: char) -> Option<(usize, usize)> {
    find_quote_pair(buf, pos, quote)
}

macro_rules! quote_cmds {
    ($inner_name:ident, $around_name:ident, $ext_inner_name:ident, $ext_around_name:ident, $quote:literal) => {
        pub(crate) fn $inner_name(buf: &Buffer, sels: SelectionSet) -> SelectionSet {
            apply_text_object(buf, sels, |b, pos| inner_quote(b, pos, $quote))
        }
        pub(crate) fn $around_name(buf: &Buffer, sels: SelectionSet) -> SelectionSet {
            apply_text_object(buf, sels, |b, pos| around_quote(b, pos, $quote))
        }
        pub(crate) fn $ext_inner_name(buf: &Buffer, sels: SelectionSet) -> SelectionSet {
            apply_text_object_extend(buf, sels, |b, pos| inner_quote(b, pos, $quote))
        }
        pub(crate) fn $ext_around_name(buf: &Buffer, sels: SelectionSet) -> SelectionSet {
            apply_text_object_extend(buf, sels, |b, pos| around_quote(b, pos, $quote))
        }
    };
}

quote_cmds!(cmd_inner_double_quote, cmd_around_double_quote, cmd_extend_inner_double_quote, cmd_extend_around_double_quote, '"');
quote_cmds!(cmd_inner_single_quote, cmd_around_single_quote, cmd_extend_inner_single_quote, cmd_extend_around_single_quote, '\'');
quote_cmds!(cmd_inner_backtick, cmd_around_backtick, cmd_extend_inner_backtick, cmd_extend_around_backtick, '`');

// ── Arguments (comma-separated items) ──────────────────────────────────────────

/// Find the tightest bracket pair among `()`, `[]`, `{}` that encloses `pos`.
///
/// Tries all three bracket types and returns the pair with the smallest span.
/// Tightest means innermost — for nested structures, we want the closest pair.
fn find_tightest_bracket_pair(buf: &Buffer, pos: usize) -> Option<(usize, usize)> {
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
fn find_comma_segments(buf: &Buffer, open_pos: usize, close_pos: usize) -> Vec<(usize, usize)> {
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
fn inner_argument(buf: &Buffer, pos: usize) -> Option<(usize, usize)> {
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

/// Around argument: the item plus its separator comma, following the Helix convention
/// that deleting around leaves a clean, properly-spaced list.
///
/// - **Only arg**: same as inner (no separator to consume).
/// - **First arg**: extend end through the trailing comma and any whitespace
///   leading into the next argument, so `delete(around aaa)` in `foo(aaa, bbb)`
///   yields `foo(bbb)` with no leading space.
/// - **Non-first arg**: extend start back to include the preceding comma,
///   so `delete(around bbb)` in `foo(aaa, bbb)` yields `foo(aaa)`.
fn around_argument(buf: &Buffer, pos: usize) -> Option<(usize, usize)> {
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

pub(crate) fn cmd_inner_argument(buf: &Buffer, sels: SelectionSet) -> SelectionSet {
    apply_text_object(buf, sels, inner_argument)
}

pub(crate) fn cmd_around_argument(buf: &Buffer, sels: SelectionSet) -> SelectionSet {
    apply_text_object(buf, sels, around_argument)
}

pub(crate) fn cmd_extend_inner_argument(buf: &Buffer, sels: SelectionSet) -> SelectionSet {
    apply_text_object_extend(buf, sels, inner_argument)
}

pub(crate) fn cmd_extend_around_argument(buf: &Buffer, sels: SelectionSet) -> SelectionSet {
    apply_text_object_extend(buf, sels, around_argument)
}

// ── Tests ──────────────────────────────────────────────────────────────────────

#[cfg(test)]
#[allow(non_snake_case)] // WORD (uppercase) is an intentional Vim concept, distinct from word (lowercase)
mod tests {
    use super::*;
    use crate::assert_state;

    // ── Line ──────────────────────────────────────────────────────────────────

    #[test]
    fn inner_line_middle() {
        // Selection covers `world`, head=d (last char before \n).
        assert_state!(
            "hello\n-[w]>orld\nfoo\n",
            |(buf, sels)| cmd_inner_line(&buf, sels),
            "hello\n-[world]>\nfoo\n"
        );
    }

    #[test]
    fn inner_line_start_of_line() {
        assert_state!(
            "-[h]>ello world\n",
            |(buf, sels)| cmd_inner_line(&buf, sels),
            "-[hello world]>\n"
        );
    }

    #[test]
    fn inner_line_end_of_content() {
        assert_state!(
            "hello worl-[d]>\n",
            |(buf, sels)| cmd_inner_line(&buf, sels),
            "-[hello world]>\n"
        );
    }

    #[test]
    fn inner_line_empty_line_is_noop() {
        // An empty line is just "\n" — no content, so inner_line returns None
        // and the selection is preserved.
        assert_state!(
            "hello\n-[\n]>world\n",
            |(buf, sels)| cmd_inner_line(&buf, sels),
            "hello\n-[\n]>world\n"
        );
    }

    #[test]
    fn inner_line_combining_grapheme_before_newline() {
        // "cafe\u{0301}" = c(0) a(1) f(2) e(3) combining_acute(4) \n(5).
        // inner_line must include the full last grapheme cluster, so the
        // selection end must be 4 (the combining mark) not 3 (the 'e' alone).
        // The old `last - 1` arithmetic would have produced a broken
        // mid-cluster end position.
        assert_state!(
            "-[c]>afe\u{0301}\n",
            |(buf, sels)| cmd_inner_line(&buf, sels),
            "-[cafe\u{0301}]>\n"
        );
    }

    #[test]
    fn around_line_includes_newline() {
        // Selection covers `world\n`; head is the newline char.
        assert_state!(
            "hello\n-[w]>orld\nfoo\n",
            |(buf, sels)| cmd_around_line(&buf, sels),
            "hello\n-[world\n]>foo\n"
        );
    }

    #[test]
    fn around_line_empty_line() {
        // An empty line is just "\n"; around_line selects that single char.
        // anchor == head, so serialises as a cursor (|).
        assert_state!(
            "hello\n-[\n]>world\n",
            |(buf, sels)| cmd_around_line(&buf, sels),
            "hello\n-[\n]>world\n"
        );
    }

    // ── Word ──────────────────────────────────────────────────────────────────

    #[test]
    fn inner_word_middle() {
        // head=o (last char of `hello`).
        assert_state!(
            "-[h]>ello world\n",
            |(buf, sels)| cmd_inner_word(&buf, sels),
            "-[hello]> world\n"
        );
    }

    #[test]
    fn inner_word_cursor_at_end_of_word() {
        assert_state!(
            "hell-[o]> world\n",
            |(buf, sels)| cmd_inner_word(&buf, sels),
            "-[hello]> world\n"
        );
    }

    #[test]
    fn inner_word_cursor_on_whitespace() {
        // Two spaces between `foo` and `bar`; cursor on the first space.
        // inner_word selects the entire whitespace run (both spaces).
        // head = second space, serialised as `#[ | ]#`.
        assert_state!(
            "foo-[ ]> bar\n",
            |(buf, sels)| cmd_inner_word(&buf, sels),
            "foo-[  ]>bar\n"
        );
    }

    #[test]
    fn inner_word_cursor_on_punctuation() {
        // Both `!!` are Punctuation — selected as one run.
        assert_state!(
            "foo-[!]>!\n",
            |(buf, sels)| cmd_inner_word(&buf, sels),
            "foo-[!!]>\n"
        );
    }

    #[test]
    fn around_word_includes_trailing_space() {
        // Trailing space is included; head = the space char.
        assert_state!(
            "-[h]>ello world\n",
            |(buf, sels)| cmd_around_word(&buf, sels),
            "-[hello ]>world\n"
        );
    }

    #[test]
    fn around_word_no_trailing_space_uses_leading() {
        // "world" at end of line has no trailing space, so leading space included.
        assert_state!(
            "hello -[w]>orld\n",
            |(buf, sels)| cmd_around_word(&buf, sels),
            "hello-[ world]>\n"
        );
    }

    #[test]
    fn inner_word_includes_combining_grapheme() {
        // Buffer: "cafe\u{0301} world\n"
        // char offsets: c(0) a(1) f(2) e(3) ◌́(4) ' '(5) w(6) ...
        // Grapheme clusters: {c}{a}{f}{e◌́}{ }{w}...
        //
        // Old code (end += 1) stops at offset 3 because the combining codepoint
        // at offset 4 is classified as Punctuation — a false word/punct boundary
        // inside the grapheme. New code steps by grapheme boundary: the next
        // cluster after offset 3 starts at offset 5 (space), so the word ends
        // at offset 4 (last codepoint of the {e◌́} grapheme) — the full cluster
        // is included.
        assert_state!(
            "-[c]>afe\u{0301} world\n",
            |(buf, sels)| cmd_inner_word(&buf, sels),
            "-[cafe\u{0301}]> world\n"
        );
    }

    // ── WORD ──────────────────────────────────────────────────────────────────

    #[test]
    fn inner_WORD_spans_punctuation() {
        // `hello.world` is one WORD (no whitespace boundary within it).
        assert_state!(
            "-[h]>ello.world foo\n",
            |(buf, sels)| cmd_inner_WORD(&buf, sels),
            "-[hello.world]> foo\n"
        );
    }

    // ── Brackets ──────────────────────────────────────────────────────────────

    #[test]
    fn inner_paren_cursor_inside() {
        assert_state!(
            "(-[h]>ello)\n",
            |(buf, sels)| cmd_inner_paren(&buf, sels),
            "(-[hello]>)\n"
        );
    }

    #[test]
    fn around_paren_cursor_inside() {
        // around includes the parens themselves; head = `)`.
        assert_state!(
            "(-[h]>ello)\n",
            |(buf, sels)| cmd_around_paren(&buf, sels),
            "-[(hello)]>\n"
        );
    }

    #[test]
    fn inner_paren_cursor_on_open() {
        // Cursor ON `(` — treated as if inside; same result as cursor inside.
        assert_state!(
            "-[(]>hello)\n",
            |(buf, sels)| cmd_inner_paren(&buf, sels),
            "(-[hello]>)\n"
        );
    }

    #[test]
    fn inner_paren_cursor_on_close() {
        assert_state!(
            "(hello-[)]>\n",
            |(buf, sels)| cmd_inner_paren(&buf, sels),
            "(-[hello]>)\n"
        );
    }

    #[test]
    fn inner_paren_empty_is_noop() {
        assert_state!(
            "-[(]>)\n",
            |(buf, sels)| cmd_inner_paren(&buf, sels),
            "-[(]>)\n"
        );
    }

    #[test]
    fn inner_paren_nested_cursor_on_inner() {
        // Cursor inside inner `(b)` — selects `b`, which is a single char.
        // anchor == head, so serialises as a cursor.
        assert_state!(
            "(a(-[b]>)c)\n",
            |(buf, sels)| cmd_inner_paren(&buf, sels),
            "(a(-[b]>)c)\n"
        );
    }

    #[test]
    fn inner_paren_nested_cursor_on_outer_content() {
        // Cursor on `a` (outside inner parens) — innermost enclosing pair
        // is the outer `(...)`, selects `a(b)c`.
        assert_state!(
            "(-[a]>(b)c)\n",
            |(buf, sels)| cmd_inner_paren(&buf, sels),
            "(-[a(b)c]>)\n"
        );
    }

    #[test]
    fn inner_brace_basic() {
        assert_state!(
            "{-[h]>ello}\n",
            |(buf, sels)| cmd_inner_brace(&buf, sels),
            "{-[hello]>}\n"
        );
    }

    #[test]
    fn inner_bracket_basic() {
        assert_state!(
            "[-[h]>ello]\n",
            |(buf, sels)| cmd_inner_bracket(&buf, sels),
            "[-[hello]>]\n"
        );
    }

    #[test]
    fn inner_angle_basic() {
        assert_state!(
            "<-[h]>ello>\n",
            |(buf, sels)| cmd_inner_angle(&buf, sels),
            "<-[hello]>>\n"
        );
    }

    #[test]
    fn inner_paren_no_match_is_noop() {
        assert_state!(
            "hel-[l]>o\n",
            |(buf, sels)| cmd_inner_paren(&buf, sels),
            "hel-[l]>o\n"
        );
    }

    #[test]
    fn inner_paren_multiline() {
        // Bracket pair spans two lines; inner content is `\nhello\n`.
        // anchor = `\n` after `(`, head = `\n` before `)`.
        assert_state!(
            "(\n-[h]>ello\n)\n",
            |(buf, sels)| cmd_inner_paren(&buf, sels),
            "(-[\nhello\n]>)\n"
        );
    }

    // ── Quotes ────────────────────────────────────────────────────────────────

    #[test]
    fn inner_double_quote_cursor_inside() {
        assert_state!(
            "\"hel-[l]>o\"\n",
            |(buf, sels)| cmd_inner_double_quote(&buf, sels),
            "\"-[hello]>\"\n"
        );
    }

    #[test]
    fn around_double_quote_cursor_inside() {
        // around includes both quote chars; head = closing `"`.
        assert_state!(
            "\"hel-[l]>o\"\n",
            |(buf, sels)| cmd_around_double_quote(&buf, sels),
            "-[\"hello\"]>\n"
        );
    }

    #[test]
    fn inner_double_quote_cursor_on_open() {
        assert_state!(
            "-[\"]>hello\"\n",
            |(buf, sels)| cmd_inner_double_quote(&buf, sels),
            "\"-[hello]>\"\n"
        );
    }

    #[test]
    fn inner_double_quote_cursor_on_close() {
        assert_state!(
            "\"hello-[\"]>\n",
            |(buf, sels)| cmd_inner_double_quote(&buf, sels),
            "\"-[hello]>\"\n"
        );
    }

    #[test]
    fn inner_double_quote_empty_is_noop() {
        assert_state!(
            "-[\"]>\"foo\n",
            |(buf, sels)| cmd_inner_double_quote(&buf, sels),
            "-[\"]>\"foo\n"
        );
    }

    #[test]
    fn inner_double_quote_second_pair() {
        // Two pairs on the same line — cursor in second pair selects second.
        assert_state!(
            "\"a\" \"b-[c]>\"\n",
            |(buf, sels)| cmd_inner_double_quote(&buf, sels),
            "\"a\" \"-[bc]>\"\n"
        );
    }

    #[test]
    fn inner_single_quote_basic() {
        assert_state!(
            "'hel-[l]>o'\n",
            |(buf, sels)| cmd_inner_single_quote(&buf, sels),
            "'-[hello]>'\n"
        );
    }

    #[test]
    fn inner_backtick_basic() {
        assert_state!(
            "`hel-[l]>o`\n",
            |(buf, sels)| cmd_inner_backtick(&buf, sels),
            "`-[hello]>`\n"
        );
    }

    #[test]
    fn inner_double_quote_not_inside_is_noop() {
        assert_state!(
            "hel-[l]>o\n",
            |(buf, sels)| cmd_inner_double_quote(&buf, sels),
            "hel-[l]>o\n"
        );
    }

    // ── Multi-cursor ──────────────────────────────────────────────────────────

    #[test]
    fn inner_word_multi_cursor_different_words() {
        assert_state!(
            "-[h]>ello -[w]>orld\n",
            |(buf, sels)| cmd_inner_word(&buf, sels),
            "-[hello]> -[world]>\n"
        );
    }

    #[test]
    fn inner_word_multi_cursor_same_word_merges() {
        // Two cursors in the same word — both select "hello", merge to one selection.
        assert_state!(
            "-[h]>el-[l]>o world\n",
            |(buf, sels)| cmd_inner_word(&buf, sels),
            "-[hello]> world\n"
        );
    }

    #[test]
    fn around_word_multi_cursor() {
        // "hello world foo\n": cursor 0 on 'h'(0) → "hello "(0..5); cursor 1 on 'f'(12) → " foo"(11..14).
        assert_state!(
            "-[h]>ello world-[ ]>foo\n",
            |(buf, sels)| cmd_around_word(&buf, sels),
            "-[hello ]>world-[ foo]>\n"
        );
    }

    #[test]
    fn inner_line_multi_cursor_same_line_merges() {
        // Two cursors on the same line both select that line's content, then merge.
        assert_state!(
            "-[h]>el-[l]>o\n",
            |(buf, sels)| cmd_inner_line(&buf, sels),
            "-[hello]>\n"
        );
    }

    #[test]
    fn inner_line_multi_cursor_different_lines() {
        assert_state!(
            "-[h]>ello\n-[w]>orld\n",
            |(buf, sels)| cmd_inner_line(&buf, sels),
            "-[hello]>\n-[world]>\n"
        );
    }

    #[test]
    fn around_line_multi_cursor_different_lines() {
        assert_state!(
            "-[h]>ello\n-[w]>orld\n",
            |(buf, sels)| cmd_around_line(&buf, sels),
            "-[hello\n]>-[world\n]>"
        );
    }

    #[test]
    fn inner_WORD_multi_cursor() {
        assert_state!(
            "-[h]>ello.world -[f]>oo\n",
            |(buf, sels)| cmd_inner_WORD(&buf, sels),
            "-[hello.world]> -[foo]>\n"
        );
    }

    #[test]
    fn inner_paren_two_cursors_same_pair_merge() {
        // Both cursors inside the same parens — both map to the same range → merge.
        assert_state!(
            "(-[h]>el-[l]>o)\n",
            |(buf, sels)| cmd_inner_paren(&buf, sels),
            "(-[hello]>)\n"
        );
    }

    // ── around_WORD ───────────────────────────────────────────────────────────

    #[test]
    fn around_WORD_includes_trailing_space() {
        assert_state!(
            "-[h]>ello.world foo\n",
            |(buf, sels)| cmd_around_WORD(&buf, sels),
            "-[hello.world ]>foo\n"
        );
    }

    #[test]
    fn around_WORD_no_trailing_space_uses_leading() {
        // Last WORD has no trailing space — grabs leading space instead.
        assert_state!(
            "hello.world -[f]>oo\n",
            |(buf, sels)| cmd_around_WORD(&buf, sels),
            "hello.world-[ foo]>\n"
        );
    }

    #[test]
    #[allow(non_snake_case)]
    fn around_WORD_end_of_buffer_with_leading_space_uses_WORD_boundary() {
        // B1 regression: the fallback path for "WORD at end of buffer with no
        // trailing space" was calling inner_word_impl with the wrong predicate
        // (is_word_boundary instead of is_WORD_boundary). This test catches
        // that by using a WORD that contains punctuation — `is_word_boundary`
        // would split "foo.bar" into two words while `is_WORD_boundary` keeps
        // it as one WORD, so the leading-space extent would differ.
        assert_state!(
            "  -[f]>oo.bar\n",
            |(buf, sels)| cmd_around_WORD(&buf, sels),
            "-[  foo.bar]>\n"
        );
    }

    #[test]
    fn around_WORD_cursor_on_whitespace_extends_to_next_WORD() {
        assert_state!(
            "foo-[ ]>bar\n",
            |(buf, sels)| cmd_around_WORD(&buf, sels),
            "foo-[ bar]>\n"
        );
    }

    #[test]
    fn around_WORD_multi_cursor() {
        // "hello world foo\n": cursor on 'h'(0) → "hello "(0..5); cursor on 'f'(12) → " foo"(11..14).
        assert_state!(
            "-[h]>ello world-[ ]>foo\n",
            |(buf, sels)| cmd_around_WORD(&buf, sels),
            "-[hello ]>world-[ foo]>\n"
        );
    }

    #[test]
    fn around_WORD_treats_punctuation_as_part_of_word() {
        // WORD includes adjacent punctuation; `around_word` (lower-case) would stop at '.'.
        // "foo.bar baz\n" — cursor on 'f': around_WORD selects "foo.bar " (whole WORD + space).
        // around_word would only select "foo " (stopping at '.').
        assert_state!(
            "-[f]>oo.bar baz\n",
            |(buf, sels)| cmd_around_WORD(&buf, sels),
            "-[foo.bar ]>baz\n"
        );
    }

    #[test]
    fn around_word_stops_at_punctuation() {
        // Contrast: around_word (lower-case) on "foo.bar baz\n", cursor on 'f'.
        // Inner word = "foo" (0..2). Next char = '.' (Punctuation, not Space) →
        // no trailing space. No leading space (cursor at col 0) → no expansion.
        // Result: just "foo".
        assert_state!(
            "-[f]>oo.bar baz\n",
            |(buf, sels)| cmd_around_word(&buf, sels),
            "-[foo]>.bar baz\n"
        );
    }

    // ── around_bracket variants ───────────────────────────────────────────────

    #[test]
    fn around_brace_basic() {
        assert_state!(
            "{-[h]>ello}\n",
            |(buf, sels)| cmd_around_brace(&buf, sels),
            "-[{hello}]>\n"
        );
    }

    #[test]
    fn around_bracket_basic() {
        assert_state!(
            "[-[h]>ello]\n",
            |(buf, sels)| cmd_around_bracket(&buf, sels),
            "-[[hello]]>\n"
        );
    }

    #[test]
    fn around_angle_basic() {
        assert_state!(
            "<-[h]>ello>\n",
            |(buf, sels)| cmd_around_angle(&buf, sels),
            "-[<hello>]>\n"
        );
    }

    // ── around_quote variants ─────────────────────────────────────────────────

    #[test]
    fn around_single_quote_basic() {
        assert_state!(
            "'hel-[l]>o'\n",
            |(buf, sels)| cmd_around_single_quote(&buf, sels),
            "-['hello']>\n"
        );
    }

    #[test]
    fn around_backtick_basic() {
        assert_state!(
            "`hel-[l]>o`\n",
            |(buf, sels)| cmd_around_backtick(&buf, sels),
            "-[`hello`]>\n"
        );
    }

    // ── multi-line bracket for non-paren types ────────────────────────────────

    #[test]
    fn inner_brace_multiline() {
        assert_state!(
            "{\n-[h]>ello\n}\n",
            |(buf, sels)| cmd_inner_brace(&buf, sels),
            "{-[\nhello\n]>}\n"
        );
    }

    // ── edge cases ────────────────────────────────────────────────────────────

    #[test]
    fn inner_word_on_structural_newline() {
        // Empty buffer: cursor on structural '\n'. inner_word selects the '\n'
        // (Eol class), which equals the original cursor — no visible change.
        assert_state!(
            "-[\n]>",
            |(buf, sels)| cmd_inner_word(&buf, sels),
            "-[\n]>"
        );
    }

    #[test]
    fn inner_WORD_on_structural_newline() {
        assert_state!(
            "-[\n]>",
            |(buf, sels)| cmd_inner_WORD(&buf, sels),
            "-[\n]>"
        );
    }

    // ── apply_text_object_extend (union semantics) ────────────────────────────

    #[test]
    fn extend_inner_paren_grows_selection() {
        // "hello (world) foo\n": '('=6, ')'=12. Forward sel from 'h'(0) to 'w'(7).
        // extend_inner_paren at head=7 ('w' inside parens):
        //   inner_bracket(7) → inner = (7, 11) = "world".
        //   Union: min(0,7)=0, max(7,11)=11. head=11 ('d').
        // Serialized: ]> at position 12 (before ')') → "-[hello (world]>) foo\n".
        assert_state!(
            "-[hello (w]>orld) foo\n",
            |(buf, sels)| cmd_extend_inner_paren(&buf, sels),
            "-[hello (world]>) foo\n"
        );
    }

    #[test]
    fn extend_text_object_noop_on_no_match() {
        // When extend text-object has no match, selection is unchanged.
        // inner_paren on "hello\n" finds no parens → returns None → sel unchanged.
        assert_state!(
            "-[h]>ello\n",
            |(buf, sels)| {
                let s1 = cmd_inner_word(&buf, sels);  // selects "hello" (0,4)
                cmd_extend_inner_paren(&buf, s1)      // no parens → no-op → "hello" unchanged
            },
            "-[hello]>\n"
        );
    }

    #[test]
    fn extend_around_paren_grows_selection() {
        // "hello (world) foo\n": forward selection from 'h'(0) to 'w'(7).
        // extend_around_paren at head=7 ('w' inside parens):
        //   around_bracket(7) finds "(world)" (6,13).
        //   Union: min(0,6)=0, max(7,13)=13 → (0,13) = "hello (world)".
        assert_state!(
            "-[hello (w]>orld) foo\n",
            |(buf, sels)| cmd_extend_around_paren(&buf, sels),
            "-[hello (world)]> foo\n"
        );
    }

    #[test]
    fn extend_text_object_preserves_backward_direction() {
        // Backward selection "<[he]-llo world\n": head=0 ('h'), anchor=1 ('e').
        // extend_inner_word at head=0 → inner_word "hello" (0,4).
        // Union: sel.start()=0, sel.end()=1, word=(0,4).
        //   new_start=min(0,0)=0, new_end=max(1,4)=4, forward=false.
        // Result: Selection::directed(0,4,false) = {anchor=4, head=0}.
        // Serialized: `]-` placed at (anchor+1)=5 → "<[hello]- world\n".
        assert_state!(
            "<[he]-llo world\n",
            |(buf, sels)| cmd_extend_inner_word(&buf, sels),
            "<[hello]- world\n"
        );
    }

    // ── apply_text_object_extend: outward growth from already-matched pair ─────

    #[test]
    fn extend_around_paren_from_matched_pair_grows_outward() {
        // Regression: selection is already "(b)" via a prior `ma(`; pressing
        // extend-`ma(` again should grow to the enclosing "(a (b) a)".
        //
        // "(a (b) a)\n": (=0,a=1,' '=2,(=3,b=4,)=5,' '=6,a=7,)=8,\n=9
        // Selection: anchor=3, head=5 (covers "(b)").
        //
        // First try: around_bracket(head=5) finds ')' at 5 → same pair (3,5).
        // Union is a no-op. Retry from next_grapheme_boundary(end()=5)=6 (' ').
        // around_bracket(6): scan_left finds '(' at 0 (skipping the inner pair),
        // scan_right finds ')' at 8 → (0,8). Union: (0,8). Grows.
        assert_state!(
            "(a -[(b)]> a)\n",
            |(buf, sels)| cmd_extend_around_paren(&buf, sels),
            "-[(a (b) a)]>\n"
        );
    }

    #[test]
    fn extend_inner_paren_from_matched_pair_grows_outward() {
        // Same setup: selection "(b)" in "(a (b) a)\n".
        // First try: inner_bracket(head=5) → (4,4) = "b". Union no-op (subset).
        // Retry from pos 6: inner_bracket(6) → inner of outer pair = (1,7) = "a (b) a".
        // Union: (1,7). anchor=1, head=7 → "(-[a (b) a]>)\n".
        assert_state!(
            "(a -[(b)]> a)\n",
            |(buf, sels)| cmd_extend_inner_paren(&buf, sels),
            "(-[a (b) a]>)\n"
        );
    }

    #[test]
    fn extend_around_paren_no_outer_pair_is_noop() {
        // When the selection already covers the outermost pair, there is no
        // enclosing pair to grow into — the command is a no-op.
        //
        // "(a b)\n": (=0,a=1,' '=2,b=3,)=4,\n=5. Selection anchor=0, head=4.
        // First try: around_bracket(head=4=')') → (0,4). Union no-op.
        // Retry from pos 5 ('\n'): scan_left hits ')' at 4 (depth=1), then
        // '(' at 0 (depth=0→continues), exits at i=0 → None. No-op.
        assert_state!(
            "-[(a b)]>\n",
            |(buf, sels)| cmd_extend_around_paren(&buf, sels),
            "-[(a b)]>\n"
        );
    }

    // ── Arguments ─────────────────────────────────────────────────────────────

    // ── inner_argument ────────────────────────────────────────────────────────

    #[test]
    fn inner_argument_first() {
        assert_state!(
            "foo(-[a]>aa, bbb, ccc)\n",
            |(buf, sels)| cmd_inner_argument(&buf, sels),
            "foo(-[aaa]>, bbb, ccc)\n"
        );
    }

    #[test]
    fn inner_argument_middle() {
        assert_state!(
            "foo(aaa, -[b]>bb, ccc)\n",
            |(buf, sels)| cmd_inner_argument(&buf, sels),
            "foo(aaa, -[bbb]>, ccc)\n"
        );
    }

    #[test]
    fn inner_argument_last() {
        assert_state!(
            "foo(aaa, bbb, -[c]>cc)\n",
            |(buf, sels)| cmd_inner_argument(&buf, sels),
            "foo(aaa, bbb, -[ccc]>)\n"
        );
    }

    #[test]
    fn inner_argument_single() {
        assert_state!(
            "foo(-[a]>aa)\n",
            |(buf, sels)| cmd_inner_argument(&buf, sels),
            "foo(-[aaa]>)\n"
        );
    }

    #[test]
    fn inner_argument_trims_whitespace() {
        // Leading/trailing spaces inside the segment are excluded.
        assert_state!(
            "foo(  -[a]>aa  , bbb)\n",
            |(buf, sels)| cmd_inner_argument(&buf, sels),
            "foo(  -[aaa]>  , bbb)\n"
        );
    }

    #[test]
    fn inner_argument_nested_parens_skips_inner_comma() {
        // The comma inside bar(x, y) is at depth 1 — not a segment boundary.
        assert_state!(
            "foo(-[b]>ar(x, y), z)\n",
            |(buf, sels)| cmd_inner_argument(&buf, sels),
            "foo(-[bar(x, y)]>, z)\n"
        );
    }

    #[test]
    fn inner_argument_nested_brackets_skips_inner_comma() {
        assert_state!(
            "foo(-[b]>ar[x, y], z)\n",
            |(buf, sels)| cmd_inner_argument(&buf, sels),
            "foo(-[bar[x, y]]>, z)\n"
        );
    }

    #[test]
    fn inner_argument_nested_braces_skips_inner_comma() {
        // The comma inside {a: 1, b: 2} is at depth 1 — not a segment boundary.
        // Cursor in the second argument selects "ccc", not something split by the inner comma.
        assert_state!(
            "foo({a: 1, b: 2}, cc-[c]>)\n",
            |(buf, sels)| cmd_inner_argument(&buf, sels),
            "foo({a: 1, b: 2}, -[ccc]>)\n"
        );
    }

    #[test]
    fn inner_argument_picks_tightest_bracket_pair() {
        // The cursor is inside (aaa, bbb) which is itself inside [...].
        // The tightest enclosing pair is (), not [].
        assert_state!(
            "[(aaa, -[b]>bb), ccc]\n",
            |(buf, sels)| cmd_inner_argument(&buf, sels),
            "[(aaa, -[bbb]>), ccc]\n"
        );
    }

    #[test]
    fn inner_argument_cursor_on_comma_associates_with_next() {
        // Cursor on the comma — treated as belonging to the following segment.
        assert_state!(
            "foo(aaa-[,]> bbb)\n",
            |(buf, sels)| cmd_inner_argument(&buf, sels),
            "foo(aaa, -[bbb]>)\n"
        );
    }

    #[test]
    fn inner_argument_cursor_on_open_bracket() {
        assert_state!(
            "foo-[(]>aaa, bbb)\n",
            |(buf, sels)| cmd_inner_argument(&buf, sels),
            "foo(-[aaa]>, bbb)\n"
        );
    }

    #[test]
    fn inner_argument_cursor_on_close_bracket() {
        assert_state!(
            "foo(aaa, bbb-[)]>\n",
            |(buf, sels)| cmd_inner_argument(&buf, sels),
            "foo(aaa, -[bbb]>)\n"
        );
    }

    #[test]
    fn inner_argument_empty_brackets_is_noop() {
        assert_state!(
            "foo-[(]>)\n",
            |(buf, sels)| cmd_inner_argument(&buf, sels),
            "foo-[(]>)\n"
        );
    }

    #[test]
    fn inner_argument_no_enclosing_bracket_is_noop() {
        assert_state!(
            "foo-[,]>bar\n",
            |(buf, sels)| cmd_inner_argument(&buf, sels),
            "foo-[,]>bar\n"
        );
    }

    #[test]
    fn inner_argument_array_items() {
        assert_state!(
            "[-[1]>11, 222, 333]\n",
            |(buf, sels)| cmd_inner_argument(&buf, sels),
            "[-[111]>, 222, 333]\n"
        );
    }

    #[test]
    fn inner_argument_object_fields() {
        assert_state!(
            "{-[f]>oo, a: b}\n",
            |(buf, sels)| cmd_inner_argument(&buf, sels),
            "{-[foo]>, a: b}\n"
        );
    }

    #[test]
    fn inner_argument_multi_cursor() {
        assert_state!(
            "foo(-[a]>aa, bbb, -[c]>cc)\n",
            |(buf, sels)| cmd_inner_argument(&buf, sels),
            "foo(-[aaa]>, bbb, -[ccc]>)\n"
        );
    }

    // ── around_argument ───────────────────────────────────────────────────────

    #[test]
    fn around_argument_first() {
        // Deletes "aaa, " — no orphan space before bbb.
        assert_state!(
            "foo(-[a]>aa, bbb, ccc)\n",
            |(buf, sels)| cmd_around_argument(&buf, sels),
            "foo(-[aaa, ]>bbb, ccc)\n"
        );
    }

    #[test]
    fn around_argument_middle() {
        // Deletes ", bbb" — eats the preceding comma.
        assert_state!(
            "foo(aaa, -[b]>bb, ccc)\n",
            |(buf, sels)| cmd_around_argument(&buf, sels),
            "foo(aaa-[, bbb]>, ccc)\n"
        );
    }

    #[test]
    fn around_argument_last() {
        // Deletes ", ccc" — eats the preceding comma.
        assert_state!(
            "foo(aaa, bbb, -[c]>cc)\n",
            |(buf, sels)| cmd_around_argument(&buf, sels),
            "foo(aaa, bbb-[, ccc]>)\n"
        );
    }

    #[test]
    fn around_argument_single_equals_inner() {
        // No comma to eat — same as inner.
        assert_state!(
            "foo(-[a]>aa)\n",
            |(buf, sels)| cmd_around_argument(&buf, sels),
            "foo(-[aaa]>)\n"
        );
    }

    #[test]
    fn around_argument_nested() {
        // First arg is a nested call — around eats trailing ", ".
        assert_state!(
            "foo(-[b]>ar(x, y), z)\n",
            |(buf, sels)| cmd_around_argument(&buf, sels),
            "foo(-[bar(x, y), ]>z)\n"
        );
    }

    // ── extend mode ───────────────────────────────────────────────────────────

    #[test]
    fn extend_inner_argument_basic() {
        assert_state!(
            "foo(aaa, -[b]>bb, ccc)\n",
            |(buf, sels)| cmd_extend_inner_argument(&buf, sels),
            "foo(aaa, -[bbb]>, ccc)\n"
        );
    }
}
