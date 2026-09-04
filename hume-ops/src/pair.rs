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
const BRACKET_PAIRS: [(char, char); 3] = [('(', ')'), ('[', ']'), ('{', '}')];

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

/// `bracket_role` for every ASCII byte, built from `BRACKET_PAIRS` so the
/// table can never drift out of sync with the pairs it indexes. Every
/// `BRACKET_PAIRS` char is ASCII, so a 128-entry table covers all of them;
/// `bracket_role` is the hot per-char dispatch in
/// [`find_tightest_bracket_pair`]'s scanning loops, where a linear scan over
/// `BRACKET_PAIRS` would otherwise run on every character those loops visit,
/// most of which aren't brackets at all.
const BRACKET_ROLE_TABLE: [Option<(u8, bool)>; 128] = {
    let mut table = [None; 128];
    let mut k = 0;
    while k < BRACKET_PAIRS.len() {
        let (open, close) = BRACKET_PAIRS[k];
        table[open as usize] = Some((k as u8, true));
        table[close as usize] = Some((k as u8, false));
        k += 1;
    }
    table
};

/// Which `BRACKET_PAIRS` entry `ch` belongs to, and whether it's the open
/// side (`true`) or the close side (`false`). `None` for any other char.
fn bracket_role(ch: char) -> Option<(usize, bool)> {
    let byte = u32::from(ch);
    if byte >= BRACKET_ROLE_TABLE.len() as u32 {
        return None;
    }
    let (k, is_open) = BRACKET_ROLE_TABLE[byte as usize]?;
    Some((k as usize, is_open))
}

/// Find the tightest (innermost, smallest-span) of the three `BRACKET_PAIRS`
/// pairs that encloses `pos`.
///
/// The three bracket types are resolved as **independent** candidates, never
/// as "nearest unmatched open, of any type": on `{(abc}    )` with the
/// cursor on `b`, `(` at index 1 is the nearest unmatched open, but its
/// partner `)` at 10 gives `()` a span of 9, while `{}` resolves to `(0, 5)`
/// — span 5 — and wins. A single depth counter shared across bracket types
/// would return the `()` span instead; three counters, tracked in lockstep
/// and combined only at the end, are load-bearing.
///
/// Each type still gets `find_bracket_pair`'s own on-open/on-close shortcut:
/// if `pos` sits on that type's open or close char, that side costs no scan.
/// A type that doesn't own the char at `pos` can't have its depth changed by
/// it, so every type's rightward scan starts at `pos + 1` regardless of
/// whether that type took the shortcut — which is what lets one pass serve
/// all three.
///
/// The two passes each stop once every type still needing an answer has
/// one — not once *any* type resolves, which the `{(abc}    )` example above
/// would break, since `()` resolves before `{}` but loses. Ties (crossed
/// nesting can produce genuine equal spans, e.g. `({a)}`) break in
/// `BRACKET_PAIRS` order — `min_by_key` keeps the first of equal minima.
pub(crate) fn find_tightest_bracket_pair(text: &BufferText, pos: usize) -> Option<(usize, usize)> {
    let ch = text.char_at(pos)?;
    let role = bracket_role(ch);

    // Leftward pass: each type's own first unmatched open, or the cursor
    // itself if `pos` is on that type's open char. `ch` can be at most one
    // type's open char, so at most one slot is seeded here — `pending`
    // always starts at 2 or 3, never 0.
    let mut opens: [Option<usize>; BRACKET_PAIRS.len()] = [None; BRACKET_PAIRS.len()];
    let mut pending = BRACKET_PAIRS.len();
    if let Some((k, true)) = role {
        opens[k] = Some(pos);
        pending -= 1;
    }
    {
        let mut depths = [0usize; BRACKET_PAIRS.len()];
        let mut cursor = text.chars_at(pos);
        while let Some((i, c)) = cursor.prev() {
            let Some((k, is_open)) = bracket_role(c) else {
                continue;
            };
            if opens[k].is_some() {
                continue;
            }
            if !is_open {
                depths[k] += 1;
            } else if depths[k] == 0 {
                opens[k] = Some(i);
                pending -= 1;
                if pending == 0 {
                    break;
                }
            } else {
                depths[k] -= 1;
            }
        }
    }

    // Rightward pass: only for types that resolved an open above — a type
    // with no unmatched open leftward is already out of the candidate set.
    // Unlike the leftward pass, `pending` can legitimately start at 0 here
    // (no type resolved leftward, or the sole survivor took the on-close
    // shortcut below), so this guard — unlike the leftward one — stays.
    let mut closes: [Option<usize>; BRACKET_PAIRS.len()] = [None; BRACKET_PAIRS.len()];
    let mut pending = opens.iter().flatten().count();
    if let Some((k, false)) = role
        && opens[k].is_some()
    {
        closes[k] = Some(pos);
        pending -= 1;
    }
    if pending > 0 {
        let mut depths = [0usize; BRACKET_PAIRS.len()];
        // `pos + 1` cannot panic: `text.char_at(pos)` above already proved
        // `pos < text.len_chars()`.
        for (i, c) in text.chars_at(pos + 1) {
            let Some((k, is_open)) = bracket_role(c) else {
                continue;
            };
            if opens[k].is_none() || closes[k].is_some() {
                continue;
            }
            if is_open {
                depths[k] += 1;
            } else if depths[k] == 0 {
                closes[k] = Some(i);
                pending -= 1;
                if pending == 0 {
                    break;
                }
            } else {
                depths[k] -= 1;
            }
        }
    }

    opens
        .iter()
        .zip(&closes)
        .filter_map(|(&open_pos, &close_pos)| open_pos.zip(close_pos))
        .min_by_key(|&(open_pos, close_pos)| close_pos - open_pos)
}

/// Find the bracket nearest `sel`'s head, scanning the selection span.
///
/// `sel`'s head is always one extremity of the span (`start()` or `end()`;
/// for a collapsed selection the two coincide) — never interior — so
/// "nearest to head, within the selection" is just "scan the span from the
/// head's end inward"; the first hit is, by construction, the nearest one.
/// Uses `chars_at` rather than indexed `char_at` calls so the scan pays
/// ropey's O(log n) tree descent once, not once per char (same reason
/// `scan_left_for_open`/`scan_right_for_close` above use it).
///
/// The span scanned is the selection itself only when it sits on one line —
/// this resolver also runs once per frame for the bracket-match highlight,
/// so scanning a selection of unbounded length (e.g. `%` select-all) would
/// put an unbounded-cost scan on every keystroke. A selection crossing a
/// line boundary falls back to probing just the head's own cluster, the
/// same one-cluster check the resolver made before it went selection-wide.
/// This never loses the motivating `") "` case: `expand_word_unit`'s
/// whitespace bookend stops at a newline, so a `w`/`W`/`maw` selection never
/// crosses one.
fn nearest_bracket(text: &BufferText, sel: Selection) -> Option<(usize, char, char)> {
    let classify = |(i, ch): (usize, char)| {
        let (k, _) = bracket_role(ch)?;
        let (o, c) = BRACKET_PAIRS[k];
        Some((i, o, c))
    };
    let head = sel.head();
    let span = if text.char_to_line(sel.start()) == text.char_to_line(sel.end()) {
        sel.start()..sel.end_inclusive(text) + 1
    } else {
        head..next_grapheme_boundary(text, head)
    };
    if head == span.start {
        text.chars_at(span.start)
            .take(span.len())
            .find_map(classify)
    } else {
        let mut cursor = text.chars_at(span.end);
        while let Some(hit) = cursor.prev() {
            if hit.0 < span.start {
                return None;
            }
            if let Some(found) = classify(hit) {
                return Some(found);
            }
        }
        None
    }
}

/// Find the partner of the bracket nearest `sel`'s head, within `sel`.
///
/// Resolves against the whole selection, not just the head's own grapheme
/// cluster — a `w`-motion selection like `") "` leaves the head on the
/// trailing space rather than the bracket itself (`word-selects-whitespace`
/// is by design), and `%`-style matching should still find the `)`. The
/// head's own cluster is always checked first: a bracket char is always
/// ASCII and never itself combines forward, but a `GC_Prepend` codepoint
/// immediately before one (e.g. U+0600 ARABIC NUMBER SIGN) joins *into* it,
/// so a head landing on that leading codepoint — exactly where
/// [`hume_editing::grapheme::snap_to_cluster_start`] leaves a motion after
/// matching such a bracket — must still resolve, or a second `%`-style press
/// (an involution) finds nothing and the cursor-match highlight goes dark on
/// a bracket the cursor is visibly beside.
///
/// This is the resolver `%`-style matching needs ("which pair is this
/// delimiter part of, and where's the other end") that [`find_bracket_pair`]
/// doesn't provide on its own, since that function is only ever called with
/// one already-known pair. Also the single resolver for the bracket-match
/// cursor highlight (`hume-editor`'s `decoration_providers`) — both need the
/// same answer to "what does this character pair with", so there is exactly
/// one place `BRACKET_PAIRS` gets consulted for it.
pub fn matching_bracket(text: &BufferText, sel: Selection) -> Option<usize> {
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
