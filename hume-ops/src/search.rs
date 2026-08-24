//! Incremental search over a rope buffer using `regex-cursor`.
//!
//! All functions here are pure: they read `BufferText` and a compiled `Regex`,
//! return char-offset ranges, and never modify editor state. The regex match
//! byte offsets from `regex-cursor` are converted to HUME's char offsets via
//! `BufferText::byte_to_char`.
//!
//! # Coordinate system
//!
//! `regex-cursor` operates on byte offsets; HUME's selection model uses char
//! (Unicode scalar value) offsets. Conversion is done here at the boundary so
//! callers work exclusively in char offsets.

use regex_cursor::{Input, RopeyCursor, engines::meta::Regex};

use hume_editing::text::BufferText;

/// Direction for `search-forward` / `search-backward` and `search-next` / `search-prev`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SearchDirection {
    Forward,
    Backward,
}

// ── compile_search_regex ──────────────────────────────────────────────────────

/// Compile a search pattern with **smart case**: all-lowercase patterns become
/// case-insensitive; patterns containing any uppercase character stay
/// case-sensitive.
///
/// Explicit `(?i)`/`(?-i)` in the pattern wins: smart-case only prepends `(?i)`,
/// so a later flag group in the pattern overrides it.
pub fn compile_search_regex(pattern: &str) -> Option<Regex> {
    let effective;
    let pat = if pattern.chars().any(|c| c.is_uppercase()) {
        pattern
    } else {
        // Prepend (?i) — an explicit (?-i) later in the pattern will override.
        effective = format!("(?i){pattern}");
        &effective
    };
    Regex::new(pat).ok()
}

// ── find_next_match ───────────────────────────────────────────────────────────

/// Find the next regex match in `text`, starting from char offset `from_char`.
///
/// # Direction
///
/// - **Forward**: finds the first match whose start is ≥ `from_char` (in byte
///   terms). Wraps to the start of the buffer if no match is found forward.
/// - **Backward**: finds the last match whose start is < `from_char`. Wraps to
///   the end of the buffer if no match is found backward.
///
/// # Return value
///
/// `Some((start_char, end_char_inclusive, wrapped))` on success, where:
/// - `start_char` is the char offset of the first character of the match
/// - `end_char_inclusive` is the char offset of the last character (HUME's
///   inclusive selection model — `anchor == head` is a 1-char selection)
/// - `wrapped` is `true` when the match was found after wrapping around the
///   buffer boundary
///
/// Returns `None` when no match exists anywhere in the buffer, or when the
/// match is zero-width (which would cause the cursor to appear stuck).
pub fn find_next_match(
    text: &BufferText,
    regex: &Regex,
    from_char: usize,
    direction: SearchDirection,
) -> Option<(usize, usize, bool)> {
    let from_byte = text.char_to_byte(from_char);
    let total_bytes = text.len_bytes();

    match direction {
        SearchDirection::Forward => {
            // Primary: search from_byte..end
            if let Some((s, e)) = search_match_in(text, regex, from_byte..total_bytes, false) {
                return Some((s, e, false));
            }
            // Wrap: search 0..from_byte
            if let Some((s, e)) = search_match_in(text, regex, 0..from_byte, false) {
                return Some((s, e, true));
            }
        }
        SearchDirection::Backward => {
            // Primary: search 0..from_byte, take the last match
            if let Some((s, e)) = search_match_in(text, regex, 0..from_byte, true) {
                return Some((s, e, false));
            }
            // Wrap: search from_byte..end, take the last match
            if let Some((s, e)) = search_match_in(text, regex, from_byte..total_bytes, true) {
                return Some((s, e, true));
            }
        }
    }

    None
}

// ── find_all_matches ──────────────────────────────────────────────────────────

/// Return all non-overlapping regex matches in `text` as char-offset ranges.
///
/// Results are `(start_char, end_char_inclusive)` pairs in document order.
/// Zero-width matches are skipped.
///
/// Used by `SearchMatchHighlighter` to convert matches to line-relative byte
/// ranges for the engine's highlight provider system.
pub fn find_all_matches(text: &BufferText, regex: &Regex) -> Vec<(usize, usize)> {
    find_matches_in_range(text, regex, 0, text.len_chars() - 1)
}

// ── find_matches_in_range ─────────────────────────────────────────────────────

/// Return all non-overlapping regex matches within a char range of `text`.
///
/// Only matches that fall entirely within `[start_char, end_char]` (inclusive)
/// are returned. Results are `(start_char, end_char_inclusive)` pairs in
/// document order. Zero-width matches are skipped.
pub fn find_matches_in_range(
    text: &BufferText,
    regex: &Regex,
    start_char: usize,
    end_char: usize, // inclusive
) -> Vec<(usize, usize)> {
    let start_byte = text.char_to_byte(start_char);
    // end_char is inclusive — we need the byte *after* the last char in range.
    let end_byte = text.char_to_byte(end_char + 1);

    let cursor = RopeyCursor::new(text.full_slice());
    let mut input = Input::new(cursor);
    input.set_range(start_byte..end_byte);

    regex
        .find_iter(input)
        .filter(|m| m.start() < m.end()) // skip zero-width matches
        .map(|m| {
            let s = text.byte_to_char(m.start());
            let e = text.byte_to_char(m.end()) - 1;
            (s, e)
        })
        .collect()
}

// ── escape_regex ─────────────────────────────────────────────────────────────

/// Escape regex metacharacters so the string matches literally.
///
/// Used by `*` (search-word-under-cursor) and Ctrl+/ (search-selection) to
/// turn arbitrary text into a pattern that matches exactly that text.
pub fn escape_regex(s: &str) -> String {
    let mut escaped = String::with_capacity(s.len() * 2);
    for c in s.chars() {
        if matches!(
            c,
            '\\' | '.' | '+' | '*' | '?' | '(' | ')' | '|' | '[' | ']' | '{' | '}' | '^' | '$'
        ) {
            escaped.push('\\');
        }
        escaped.push(c);
    }
    escaped
}

// ── search_match_info ─────────────────────────────────────────────────────────

/// Return `(current_1based, total)` for a pre-computed match list.
///
/// `total` is the number of matches in `matches`.
/// `current_1based` is the 1-based index of the match whose range contains
/// `cursor_head`, or `0` when the cursor is not on any match (e.g. during
/// live search before a hit is found).
///
/// `matches` must be in document order (sorted by start position, non-overlapping),
/// as produced by [`find_all_matches`].
pub fn search_match_info(matches: &[(usize, usize)], cursor_head: usize) -> (usize, usize) {
    let total = matches.len();
    // partition_point gives the first index where start > cursor_head, so
    // idx-1 is the last match that could contain cursor_head. If cursor_head
    // also falls within its end, the cursor is on that match.
    let idx = matches.partition_point(|&(start, _)| start <= cursor_head);
    let current = idx
        .checked_sub(1)
        .filter(|&i| cursor_head <= matches[i].1)
        .map(|i| i + 1) // convert to 1-based
        .unwrap_or(0);
    (current, total)
}

// ── find_match_from_cache ─────────────────────────────────────────────────────

/// Find the next match relative to `from_char` by binary-searching a
/// pre-computed, sorted match list rather than re-scanning the buffer.
///
/// This is O(log M) where M is the number of matches, vs O(buffer_size) for
/// the regex-scan path. Use this on the `n`/`N` hot path when the cache is
/// populated; fall back to [`find_next_match`] during live search when the
/// cache may not yet reflect the current regex.
///
/// # Direction
///
/// - **Forward**: first match whose `start ≥ from_char`. Wraps to `matches[0]`
///   if none is found at or after `from_char`.
/// - **Backward**: last match whose `start < from_char`. Wraps to
///   `matches.last()` if none is found before `from_char`.
///
/// Returns `None` only when `matches` is empty.
/// Returns `Some((start_char, end_char_inclusive, wrapped))` otherwise.
pub fn find_match_from_cache(
    matches: &[(usize, usize)],
    from_char: usize,
    direction: SearchDirection,
) -> Option<(usize, usize, bool)> {
    if matches.is_empty() {
        return None;
    }
    match direction {
        SearchDirection::Forward => {
            // First match with start >= from_char.
            let idx = matches.partition_point(|&(s, _)| s < from_char);
            if let Some(&(s, e)) = matches.get(idx) {
                Some((s, e, false))
            } else {
                // Wrap: take the very first match in the buffer.
                let &(s, e) = &matches[0]; // non-empty guard above
                Some((s, e, true))
            }
        }
        SearchDirection::Backward => {
            // Last match with start < from_char.
            let idx = matches.partition_point(|&(s, _)| s < from_char);
            if let Some(&(s, e)) = idx.checked_sub(1).and_then(|i| matches.get(i)) {
                Some((s, e, false))
            } else {
                // Wrap: take the very last match in the buffer.
                let &(s, e) = &matches[matches.len() - 1]; // non-empty guard above
                Some((s, e, true))
            }
        }
    }
}

// ── Helpers ───────────────────────────────────────────────────────────────────

/// Find a non-zero-width match in `byte_range`, returning
/// `Some((start_char, end_char_inclusive))` or `None`.
///
/// `take_last`: `false` takes the first match found (forward search);
/// `true` scans every match in the range and takes the last one —
/// implemented by collecting all matches, which is correct and simple,
/// acceptable for typical buffer sizes. A reverse-DFA approach could be
/// added later for very large files.
fn search_match_in(
    text: &BufferText,
    regex: &Regex,
    byte_range: std::ops::Range<usize>,
    take_last: bool,
) -> Option<(usize, usize)> {
    if byte_range.is_empty() {
        return None;
    }
    let cursor = RopeyCursor::new(text.full_slice());
    let mut input = Input::new(cursor);
    input.set_range(byte_range);
    let m = if take_last {
        regex
            .find_iter(input)
            .filter(|m| m.start() < m.end())
            .last()?
    } else {
        regex.find(input).filter(|m| m.start() < m.end())?
    };
    let start = text.byte_to_char(m.start());
    let end_incl = text.byte_to_char(m.end()) - 1;
    Some((start, end_incl))
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests;
