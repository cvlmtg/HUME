//! Incremental search over a rope buffer using `regex-cursor`.
//!
//! All functions here are pure: they read `Buffer` and a compiled `Regex`,
//! return char-offset ranges, and never modify editor state. The regex match
//! byte offsets from `regex-cursor` are converted to HUME's char offsets via
//! `Buffer::byte_to_char`.
//!
//! # Coordinate system
//!
//! `regex-cursor` operates on byte offsets; HUME's selection model uses char
//! (Unicode scalar value) offsets. Conversion is done here at the boundary so
//! callers work exclusively in char offsets.

use regex_cursor::{engines::meta::Regex, Input, RopeyCursor};

use crate::core::buffer::Buffer;
use crate::editor::SearchDirection;

// ── find_next_match ───────────────────────────────────────────────────────────

/// Find the next regex match in `buf`, starting from char offset `from_char`.
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
pub(crate) fn find_next_match(
    buf: &Buffer,
    regex: &Regex,
    from_char: usize,
    direction: SearchDirection,
) -> Option<(usize, usize, bool)> {
    let from_byte = buf.char_to_byte(from_char);
    let total_bytes = buf.len_bytes();

    match direction {
        SearchDirection::Forward => {
            // Primary: search from_byte..end
            if let Some(m) = search_first_in(buf, regex, from_byte..total_bytes) {
                return Some(m);
            }
            // Wrap: search 0..from_byte
            if let Some((s, e, _)) = search_first_in(buf, regex, 0..from_byte) {
                return Some((s, e, true));
            }
        }
        SearchDirection::Backward => {
            // Primary: search 0..from_byte, take the last match
            if let Some(m) = search_last_in(buf, regex, 0..from_byte) {
                return Some(m);
            }
            // Wrap: search from_byte..end, take the last match
            if let Some((s, e, _)) = search_last_in(buf, regex, from_byte..total_bytes) {
                return Some((s, e, true));
            }
        }
    }

    None
}

// ── find_all_matches ──────────────────────────────────────────────────────────

/// Return all non-overlapping regex matches in `buf` as char-offset ranges.
///
/// Results are `(start_char, end_char_inclusive)` pairs in document order.
/// Zero-width matches are skipped.
///
/// Used by the renderer to build the `HighlightSet` for search-match
/// highlighting every frame.
pub(crate) fn find_all_matches(buf: &Buffer, regex: &Regex) -> Vec<(usize, usize)> {
    let cursor = RopeyCursor::new(buf.full_slice());
    let input = Input::new(cursor);
    regex
        .find_iter(input)
        .filter(|m| m.start() < m.end()) // skip zero-width matches
        .map(|m| {
            let start = buf.byte_to_char(m.start());
            let end_excl = buf.byte_to_char(m.end());
            (start, end_excl - 1)
        })
        .collect()
}

// ── Helpers ───────────────────────────────────────────────────────────────────

/// Find the first non-zero-width match in `byte_range`, returning
/// `Some((start_char, end_char_inclusive, false))` or `None`.
fn search_first_in(
    buf: &Buffer,
    regex: &Regex,
    byte_range: std::ops::Range<usize>,
) -> Option<(usize, usize, bool)> {
    if byte_range.is_empty() {
        return None;
    }
    let cursor = RopeyCursor::new(buf.full_slice());
    let mut input = Input::new(cursor);
    input.set_range(byte_range);
    let m = regex.find(input).filter(|m| m.start() < m.end())?;
    let start = buf.byte_to_char(m.start());
    let end_incl = buf.byte_to_char(m.end()) - 1;
    Some((start, end_incl, false))
}

/// Find the last non-zero-width match in `byte_range`, returning
/// `Some((start_char, end_char_inclusive, false))` or `None`.
///
/// Implemented by collecting all matches and taking the last one — correct and
/// simple, acceptable for typical buffer sizes. A reverse-DFA approach could be
/// added later for very large files.
fn search_last_in(
    buf: &Buffer,
    regex: &Regex,
    byte_range: std::ops::Range<usize>,
) -> Option<(usize, usize, bool)> {
    if byte_range.is_empty() {
        return None;
    }
    let cursor = RopeyCursor::new(buf.full_slice());
    let mut input = Input::new(cursor);
    input.set_range(byte_range);
    let m = regex
        .find_iter(input)
        .filter(|m| m.start() < m.end())
        .last()?;
    let start = buf.byte_to_char(m.start());
    let end_incl = buf.byte_to_char(m.end()) - 1;
    Some((start, end_incl, false))
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::buffer::Buffer;

    fn re(pattern: &str) -> Regex {
        Regex::new(pattern).expect("test regex should be valid")
    }

    fn buf(text: &str) -> Buffer {
        Buffer::from(text)
    }

    // ── find_all_matches ──────────────────────────────────────────────────────

    #[test]
    fn all_matches_empty_buffer() {
        // Empty buffer is just "\n" — no "foo" match.
        let b = buf("\n");
        assert_eq!(find_all_matches(&b, &re("foo")), vec![]);
    }

    #[test]
    fn all_matches_single_hit() {
        let b = buf("hello world\n");
        // "world" starts at char 6, ends at 10 (inclusive).
        assert_eq!(find_all_matches(&b, &re("world")), vec![(6, 10)]);
    }

    #[test]
    fn all_matches_multiple_hits() {
        let b = buf("aababab\n");
        // "ab" at chars 1..2, 3..4, 5..6
        assert_eq!(find_all_matches(&b, &re("ab")), vec![(1, 2), (3, 4), (5, 6)]);
    }

    #[test]
    fn all_matches_skips_zero_width() {
        // Pattern "a*" matches zero-width at every position. Only the "a" at
        // positions with actual 'a' chars should survive the zero-width filter.
        // In practice "a*" also matches 'a' (length 1) before zero-width gaps,
        // but this test ensures zero-width matches are suppressed.
        let b = buf("ab\n");
        let matches = find_all_matches(&b, &re("a*"));
        // All matches must be non-zero-width
        for (start, end) in &matches {
            assert!(end >= start, "zero-width match found at {start}");
        }
    }

    // ── find_next_match (forward) ─────────────────────────────────────────────

    #[test]
    fn forward_basic() {
        let b = buf("hello world\n");
        let (s, e, wrapped) = find_next_match(&b, &re("world"), 0, SearchDirection::Forward).unwrap();
        assert_eq!(s, 6);
        assert_eq!(e, 10);
        assert!(!wrapped);
    }

    #[test]
    fn forward_from_match_start() {
        // Searching from the start of the existing match should find the same match.
        let b = buf("hello world\n");
        let (s, e, _) = find_next_match(&b, &re("world"), 6, SearchDirection::Forward).unwrap();
        assert_eq!((s, e), (6, 10));
    }

    #[test]
    fn forward_wraps() {
        let b = buf("hello world\n");
        // Searching from after "world" (char 11 = '\n') should wrap and find "world".
        let (s, e, wrapped) = find_next_match(&b, &re("world"), 11, SearchDirection::Forward).unwrap();
        assert_eq!((s, e), (6, 10));
        assert!(wrapped);
    }

    #[test]
    fn forward_no_match() {
        let b = buf("hello\n");
        assert!(find_next_match(&b, &re("xyz"), 0, SearchDirection::Forward).is_none());
    }

    #[test]
    fn forward_multiple_matches_picks_first_after_from() {
        let b = buf("aababab\n");
        // Two "ab" matches at (1,2) and (3,4) and (5,6). Searching from char 3.
        let (s, e, _) = find_next_match(&b, &re("ab"), 3, SearchDirection::Forward).unwrap();
        assert_eq!((s, e), (3, 4));
    }

    // ── find_next_match (backward) ────────────────────────────────────────────

    #[test]
    fn backward_basic() {
        let b = buf("hello world\n");
        // Search backward from position 11 ('\n') — should find "world" at (6,10).
        let (s, e, wrapped) = find_next_match(&b, &re("world"), 11, SearchDirection::Backward).unwrap();
        assert_eq!((s, e), (6, 10));
        assert!(!wrapped);
    }

    #[test]
    fn backward_wraps() {
        // Searching backward from before the only match should wrap.
        let b = buf("hello world\n");
        let (s, e, wrapped) = find_next_match(&b, &re("world"), 3, SearchDirection::Backward).unwrap();
        assert_eq!((s, e), (6, 10));
        assert!(wrapped);
    }

    #[test]
    fn backward_multiple_matches_picks_last_before_from() {
        let b = buf("aababab\n");
        // Matches: (1,2), (3,4), (5,6). Searching backward from char 5.
        let (s, e, _) = find_next_match(&b, &re("ab"), 5, SearchDirection::Backward).unwrap();
        assert_eq!((s, e), (3, 4));
    }

    // ── Unicode / grapheme cluster ────────────────────────────────────────────

    #[test]
    fn unicode_multibyte_char() {
        // "é" in NFC is a single codepoint (U+00E9, 2 bytes in UTF-8).
        // Buffer chars: [é, space, b, o, n, \n]
        let b = buf("é bon\n");
        let (s, e, _) = find_next_match(&b, &re("bon"), 0, SearchDirection::Forward).unwrap();
        assert_eq!((s, e), (2, 4));
    }

    #[test]
    fn unicode_combining_sequence() {
        // "é" as combining sequence: e (U+0065) + combining acute (U+0301) = 2 chars.
        // Buffer chars: [e, \u{0301}, space, b, o, n, \n]  (7 chars total)
        let b = buf("e\u{0301} bon\n");
        let (s, e, _) = find_next_match(&b, &re("bon"), 0, SearchDirection::Forward).unwrap();
        // "b" is at char 3, "bon" spans chars 3..5 inclusive
        assert_eq!((s, e), (3, 5));
    }
}
