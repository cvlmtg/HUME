//! Scanning primitives for paired delimiters (brackets and quotes).
//!
//! These functions are used by both [`super::text_object`] (to implement `mi(`,
//! `a"`, etc.) and [`super::surround`] (to find the delimiter pair that wraps
//! the cursor before replacing or deleting it).

use hume_editing::grapheme::next_grapheme_boundary;
use hume_editing::lines::line_end_exclusive;
use hume_editing::selection::Selection;
use hume_editing::text::BufferText;

// ---------------------------------------------------------------------------
// Bracket pairs
// ---------------------------------------------------------------------------

/// The bracket pairs `%`-style matching and the argument text object both
/// scan for. `<>` is deliberately absent — in real code it's a comparison
/// operator (`a < b`) far more often than a delimiter, which is why vim's own
/// `matchpairs` default excludes it too; `<div>`/`</div>` tag matching is a
/// separate scan ([`crate::tag`]).
pub(crate) const BRACKET_PAIRS: [(char, char); 3] = [('(', ')'), ('[', ']'), ('{', '}')];

/// Scan left from `pos` (exclusive) to find an unmatched `open` bracket.
pub(crate) fn scan_left_for_open(
    text: &BufferText,
    pos: usize,
    open: char,
    close: char,
) -> Option<usize> {
    let mut depth = 0usize;
    let mut cursor = text.chars_at(pos);
    while let Some((i, ch)) = cursor.prev() {
        if ch == close {
            depth += 1;
        } else if ch == open {
            if depth == 0 {
                return Some(i);
            }
            depth -= 1;
        }
    }
    None
}

/// Scan right from `pos` (exclusive) to find an unmatched `close` bracket.
pub(crate) fn scan_right_for_close(
    text: &BufferText,
    pos: usize,
    open: char,
    close: char,
) -> Option<usize> {
    let mut depth = 0usize;
    for (i, ch) in text.chars_at(pos) {
        if ch == open {
            depth += 1;
        } else if ch == close {
            if depth == 0 {
                return Some(i);
            }
            depth -= 1;
        }
    }
    None
}

/// Find the innermost bracket pair `(open, close)` that encloses `pos`.
///
/// If the cursor is ON an open bracket, that bracket itself is the start.
/// If ON a close bracket, that bracket is the end.
/// Otherwise, scans both directions for the enclosing pair.
pub(crate) fn find_bracket_pair(
    text: &BufferText,
    pos: usize,
    open: char,
    close: char,
) -> Option<(usize, usize)> {
    match text.char_at(pos)? {
        ch if ch == open => {
            // Cursor is on an open bracket — scan right for the matching close.
            let close_pos = scan_right_for_close(text, pos + 1, open, close)?;
            Some((pos, close_pos))
        }
        ch if ch == close => {
            // Cursor is on a close bracket — scan left for the matching open.
            let open_pos = scan_left_for_open(text, pos, open, close)?;
            Some((open_pos, pos))
        }
        _ => {
            // Cursor is inside — scan both directions.
            let open_pos = scan_left_for_open(text, pos, open, close)?;
            let close_pos = scan_right_for_close(text, pos, open, close)?;
            Some((open_pos, close_pos))
        }
    }
}

/// The bracket pair `ch` belongs to, if any.
fn bracket_pair_for(ch: char) -> Option<(char, char)> {
    BRACKET_PAIRS
        .iter()
        .copied()
        .find(|&(o, c)| ch == o || ch == c)
}

/// The first bracket char `indices` yields, with the pair it belongs to.
/// Direction (and thus which bracket counts as "nearest") is entirely up to
/// the caller's iterator — a plain `Range` scans left to right, `.rev()`'d
/// scans right to left.
fn first_bracket_in(
    text: &BufferText,
    mut indices: impl Iterator<Item = usize>,
) -> Option<(usize, char, char)> {
    indices.find_map(|i| {
        let ch = text.char_at(i)?;
        let (o, c) = bracket_pair_for(ch)?;
        Some((i, o, c))
    })
}

/// Find the bracket nearest `sel`'s head, considering the whole selection —
/// not just the head's own grapheme cluster.
///
/// `sel`'s head is always one extremity of the selection (`start()` or
/// `end()`; for a collapsed selection the two coincide) — never interior —
/// so "nearest to head, within the selection" only ever means "closest to
/// head, walking inward toward the anchor". That collapses the search to a
/// single direction: the head's own cluster first (exactly what the old
/// head-only resolver checked, preserved unchanged so a collapsed selection
/// behaves identically to before), then one scan from there toward the
/// anchor, stopping at the first hit — which is, by construction, the
/// nearest one.
fn nearest_bracket(text: &BufferText, sel: &Selection) -> Option<(usize, char, char)> {
    let head = sel.head();
    let cluster_end = next_grapheme_boundary(text, head);
    if let Some(found) = first_bracket_in(text, head..cluster_end) {
        return Some(found);
    }
    if head == sel.start() {
        first_bracket_in(text, cluster_end..sel.end_inclusive(text) + 1)
    } else {
        first_bracket_in(text, (sel.start()..head).rev())
    }
}

/// Find the partner of the bracket nearest `sel`'s head, within `sel`.
///
/// Resolution prefers the head's own grapheme cluster — a bracket char is
/// always ASCII and never itself combines forward, but a `GC_Prepend`
/// codepoint immediately before one (e.g. U+0600 ARABIC NUMBER SIGN) joins
/// *into* it, so a head landing on that leading codepoint — exactly where
/// [`hume_editing::grapheme::snap_to_cluster_start`] leaves a motion after
/// matching such a bracket — must still resolve, or a second `%`-style press
/// (an involution) finds nothing and the cursor-match highlight goes dark on
/// a bracket the cursor is visibly beside. Failing that, it falls back to the
/// nearest bracket anywhere else in the selection — e.g. a `w`-motion
/// selection like `") "` with the head on the trailing space rather than the
/// bracket itself.
///
/// This is the resolver `%`-style matching needs ("which pair is this
/// delimiter part of, and where's the other end") that [`find_bracket_pair`]
/// doesn't provide on its own, since that function is only ever called with
/// one already-known pair. Also the single resolver for the bracket-match
/// cursor highlight (`hume-editor`'s `decoration_providers`) — both need the
/// same answer to "what does this character pair with", so there is exactly
/// one place `BRACKET_PAIRS` gets consulted for it.
pub fn matching_bracket(text: &BufferText, sel: &Selection) -> Option<usize> {
    let (bracket_pos, open, close) = nearest_bracket(text, sel)?;
    let (open_pos, close_pos) = find_bracket_pair(text, bracket_pos, open, close)?;
    Some(if bracket_pos == open_pos {
        close_pos
    } else {
        open_pos
    })
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
pub(crate) fn find_quote_pair(
    text: &BufferText,
    pos: usize,
    quote: char,
) -> Option<(usize, usize)> {
    let line = text.char_to_line(pos);
    let line_start = text.line_to_char(line);
    let line_end = line_end_exclusive(text, line);

    // Single pass: track the opening quote position; on every second hit we
    // have a complete pair and can test whether `pos` falls inside it.
    let mut open: Option<usize> = None;
    for (i, ch) in text.chars_at(line_start).take(line_end - line_start) {
        if ch == quote {
            match open {
                None => open = Some(i), // odd occurrence → opening quote
                Some(open_pos) => {
                    // even occurrence → closing quote
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
