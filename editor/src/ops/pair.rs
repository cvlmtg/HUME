//! Scanning primitives for paired delimiters (brackets and quotes).
//!
//! These functions are used by both [`super::text_object`] (to implement `mi(`,
//! `a"`, etc.) and [`super::surround`] (to find the delimiter pair that wraps
//! the cursor before replacing or deleting it).

use crate::core::text::Text;
use crate::helpers::line_end_exclusive;

// ---------------------------------------------------------------------------
// Bracket pairs
// ---------------------------------------------------------------------------

/// Scan left from `pos` (exclusive) to find an unmatched `open` bracket.
/// `depth` is the pre-loaded nesting depth (pass 0 when starting fresh).
pub(crate) fn scan_left_for_open(buf: &Text, pos: usize, open: char, close: char) -> Option<usize> {
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
pub(crate) fn scan_right_for_close(buf: &Text, pos: usize, open: char, close: char) -> Option<usize> {
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
pub(crate) fn find_bracket_pair(buf: &Text, pos: usize, open: char, close: char) -> Option<(usize, usize)> {
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

// ---------------------------------------------------------------------------
// Quote pairs
// ---------------------------------------------------------------------------

/// Find the quote pair on the current line that encloses or is nearest to `pos`.
///
/// Quotes don't span lines (current limitation). Strategy: scan the current line
/// tracking parity — odd occurrences are opening quotes, even occurrences are
/// closing quotes. Returns the pair that contains `pos`.
///
/// If `pos` is ON a quote char, parity resolves whether it is open or close.
pub(crate) fn find_quote_pair(buf: &Text, pos: usize, quote: char) -> Option<(usize, usize)> {
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
