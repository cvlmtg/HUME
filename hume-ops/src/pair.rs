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
///
/// `pub(crate)`, not just the tables it's built from, so a caller that needs
/// "is this char a bracket, and which side" — [`super::text_object::argument`]'s
/// comma-depth counter is the current one — can ask the crate's one answer
/// instead of hardcoding its own copy of `BRACKET_PAIRS`'s contents.
pub(crate) fn bracket_role(ch: char) -> Option<(usize, bool)> {
    let byte = u32::from(ch);
    if byte >= BRACKET_ROLE_TABLE.len() as u32 {
        return None;
    }
    let (k, is_open) = BRACKET_ROLE_TABLE[byte as usize]?;
    Some((k as usize, is_open))
}

/// Feeds one scanned char into a bracket type's depth counter, resolving
/// `slots[k]` — `opens` when `seek_open` (scanning left for an unmatched
/// open), `closes` when scanning right for an unmatched close — the first
/// time depth returns to zero on the side being sought. Returns the type
/// index just resolved, if any.
///
/// Shared by both scan directions in [`find_tightest_bracket_pair`]: the two
/// differ only in which side they're seeking and which array they fill, so
/// one function serves both, called with `depths`/`slots` swapped.
fn step_bracket(
    (i, c): (usize, char),
    seek_open: bool,
    depths: &mut [usize; BRACKET_PAIRS.len()],
    slots: &mut [Option<usize>; BRACKET_PAIRS.len()],
) -> Option<usize> {
    let (k, is_open) = bracket_role(c)?;
    if slots[k].is_some() {
        return None;
    }
    if is_open != seek_open {
        depths[k] += 1;
        return None;
    }
    if depths[k] > 0 {
        depths[k] -= 1;
        return None;
    }
    slots[k] = Some(i);
    Some(k)
}

/// Find the tightest (innermost, smallest-span) of the three `BRACKET_PAIRS`
/// pairs that encloses `pos`.
///
/// The three bracket types are resolved as **independent** candidates, never
/// as "nearest unmatched open, of any type": on `{(abc}    )` with the
/// cursor on `b`, `(` at index 1 is the nearest unmatched open, but its
/// partner `)` at 10 gives `()` a span of 9, while `{}` resolves to `(0, 5)`
/// — span 5 — and wins. A single depth counter shared across bracket types
/// would return the `()` span instead; three counters per direction, tracked
/// in lockstep, are load-bearing.
///
/// Each type still gets `find_bracket_pair`'s own on-open/on-close shortcut:
/// if `pos` sits on that type's open or close char, that side costs no scan.
///
/// **Bounded by the best complete span found so far.** The leftward and
/// rightward scans run interleaved, one char at a time, tracking how far
/// each has walked from `pos` (`dl`, `dr`). Once some type fully resolves
/// (both its open and close known), no type still missing its open can beat
/// that span once `dl` reaches it: a still-missing open lies more than `dl`
/// chars back, or it would already be found, and its close (never `pos`
/// itself — only the one type `role` names as a close can have that, and it
/// resolves on the spot the moment its open does, see below) lies at
/// `pos + 1` or later, so the span it could still achieve is `> dl`. The
/// mirror argument bounds the rightward scan by `dr`. This makes the common
/// case — one type (`[]`, almost always) never resolving — cost O(winning
/// span) instead of O(buffer), the unconditional cost every type paid
/// before this bound existed.
///
/// Two things the bound alone doesn't give for free: a side stops as soon as
/// it has nothing left to find, regardless of the bound
/// (`opens_missing`/`right_pending` hitting zero) — otherwise a side whose
/// types all resolved early would keep walking just because the bound
/// hasn't caught up yet. And the rightward scan for a given type only
/// starts once that type's open is known — a type dropped for lack of an
/// open (buffer-edge exhaustion on the left) never gets its close scanned,
/// matching `find_bracket_pair`'s per-type shortcut of never scanning the
/// side its missing half rules out. A type's close *can* still resolve
/// before its own open, though: the rightward scan tracks every type's
/// depth as soon as it's running at all (started by whichever type first
/// needs it), so a slower-to-resolve type's close may already be sitting in
/// `closes` by the time its open turns up on the left — handled inline
/// where each side discovers the other already has an answer.
///
/// Ties (crossed nesting can produce genuine equal spans, e.g. `({a)}`)
/// break in `BRACKET_PAIRS` order — `min_by_key` keeps the first of equal
/// minima.
pub(crate) fn find_tightest_bracket_pair(text: &BufferText, pos: usize) -> Option<(usize, usize)> {
    let ch = text.char_at(pos)?;
    let role = bracket_role(ch);

    let mut opens: [Option<usize>; BRACKET_PAIRS.len()] = [None; BRACKET_PAIRS.len()];
    let mut closes: [Option<usize>; BRACKET_PAIRS.len()] = [None; BRACKET_PAIRS.len()];
    let mut depths_left = [0usize; BRACKET_PAIRS.len()];
    let mut depths_right = [0usize; BRACKET_PAIRS.len()];

    // `ch` can be at most one type's open char, so at most one slot is
    // seeded here — `opens_missing` always starts at 2 or 3, never 0.
    let mut opens_missing = BRACKET_PAIRS.len();
    // Types with a known open and unknown close — what the rightward scan
    // is still live for. Excludes a type whose close is already known (the
    // on-close shortcut just below, or the "resolved out of order" case
    // documented above) the moment its open is found, since neither needs
    // any further scanning.
    let mut right_pending = 0usize;
    if let Some((k, true)) = role {
        opens[k] = Some(pos);
        opens_missing -= 1;
        right_pending += 1;
    }

    let mut best_span: Option<usize> = None;
    let within_bound = |best: Option<usize>, d: usize| best.is_none_or(|s| d < s);

    let mut left_cursor = text.chars_at(pos);
    // `pos + 1` cannot panic: `text.char_at(pos)` above already proved
    // `pos < text.len_chars()`.
    let mut right_cursor = text.chars_at(pos + 1);
    let (mut dl, mut dr) = (0usize, 0usize);
    let (mut left_exhausted, mut right_exhausted) = (false, false);

    loop {
        let left_live = !left_exhausted && opens_missing > 0 && within_bound(best_span, dl);
        let right_live = !right_exhausted && right_pending > 0 && within_bound(best_span, dr);
        if !left_live && !right_live {
            break;
        }

        if left_live {
            match left_cursor.prev() {
                Some(hit) => {
                    dl += 1;
                    if let Some(k) = step_bracket(hit, true, &mut depths_left, &mut opens) {
                        opens_missing -= 1;
                        let open_pos = hit.0;
                        if role == Some((k, false)) {
                            // On-close shortcut: `pos` itself is this
                            // type's close.
                            closes[k] = Some(pos);
                            best_span =
                                Some(best_span.map_or(pos - open_pos, |s| s.min(pos - open_pos)));
                        } else if let Some(close_pos) = closes[k] {
                            // Rightward scan already found this type's
                            // close before its open resolved here.
                            best_span = Some(
                                best_span
                                    .map_or(close_pos - open_pos, |s| s.min(close_pos - open_pos)),
                            );
                        } else {
                            right_pending += 1;
                        }
                    }
                }
                None => left_exhausted = true,
            }
        }

        if right_live {
            match right_cursor.next() {
                Some(hit) => {
                    dr += 1;
                    if let Some(k) = step_bracket(hit, false, &mut depths_right, &mut closes)
                        && let Some(open_pos) = opens[k]
                    {
                        right_pending -= 1;
                        let close_pos = hit.0;
                        best_span = Some(
                            best_span.map_or(close_pos - open_pos, |s| s.min(close_pos - open_pos)),
                        );
                    }
                }
                None => right_exhausted = true,
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

#[cfg(test)]
mod tests;
