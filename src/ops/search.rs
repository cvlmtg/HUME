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

// ── compile_search_regex ──────────────────────────────────────────────────────

/// Compile a search pattern with **smart case**: all-lowercase patterns become
/// case-insensitive; patterns containing any uppercase character stay
/// case-sensitive.
///
/// The user can always override with explicit inline flags (`(?i)` to force
/// insensitive, `(?-i)` to force sensitive on a lowercase pattern).
pub(crate) fn compile_search_regex(pattern: &str) -> Option<Regex> {
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
    find_matches_in_range(buf, regex, 0, buf.len_chars() - 1)
}

// ── find_matches_in_range ─────────────────────────────────────────────────────

/// Return all non-overlapping regex matches within a char range of `buf`.
///
/// Only matches that fall entirely within `[start_char, end_char]` (inclusive)
/// are returned. Results are `(start_char, end_char_inclusive)` pairs in
/// document order. Zero-width matches are skipped.
pub(crate) fn find_matches_in_range(
    buf: &Buffer,
    regex: &Regex,
    start_char: usize,
    end_char: usize, // inclusive
) -> Vec<(usize, usize)> {
    let start_byte = buf.char_to_byte(start_char);
    // end_char is inclusive — we need the byte *after* the last char in range.
    let end_byte = buf.char_to_byte(end_char + 1);

    let cursor = RopeyCursor::new(buf.full_slice());
    let mut input = Input::new(cursor);
    input.set_range(start_byte..end_byte);

    regex
        .find_iter(input)
        .filter(|m| m.start() < m.end()) // skip zero-width matches
        .map(|m| {
            let s = buf.byte_to_char(m.start());
            let e = buf.byte_to_char(m.end()) - 1;
            (s, e)
        })
        .collect()
}

// ── escape_regex ─────────────────────────────────────────────────────────────

/// Escape regex metacharacters so the string matches literally.
///
/// Used by `*` (use-selection-as-search) to turn arbitrary selected text
/// into a pattern that matches exactly that text.
pub(crate) fn escape_regex(s: &str) -> String {
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
pub(crate) fn search_match_info(matches: &[(usize, usize)], cursor_head: usize) -> (usize, usize) {
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
pub(crate) fn find_match_from_cache(
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
                let &(s, e) = matches.first().unwrap(); // non-empty guard above
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
                let &(s, e) = matches.last().unwrap(); // non-empty guard above
                Some((s, e, true))
            }
        }
    }
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

    // ── compile_search_regex (smart case) ──────────────────────────────────────

    #[test]
    fn smart_case_lowercase_is_insensitive() {
        let r = compile_search_regex("hello").expect("valid pattern");
        let b = buf("Hello HELLO hello\n");
        let matches = find_all_matches(&b, &r);
        assert_eq!(matches.len(), 3);
    }

    #[test]
    fn smart_case_uppercase_is_sensitive() {
        let r = compile_search_regex("Hello").expect("valid pattern");
        let b = buf("Hello HELLO hello\n");
        let matches = find_all_matches(&b, &r);
        assert_eq!(matches.len(), 1);
        assert_eq!(matches[0], (0, 4));
    }

    #[test]
    fn smart_case_override_force_sensitive() {
        // Explicit (?-i) on a lowercase pattern forces case-sensitive.
        let r = compile_search_regex("(?-i)hello").expect("valid pattern");
        let b = buf("Hello HELLO hello\n");
        let matches = find_all_matches(&b, &r);
        assert_eq!(matches.len(), 1);
        assert_eq!(matches[0], (12, 16));
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
    fn backward_from_position_zero_wraps() {
        // Primary range is 0..0 (empty), so the entire buffer is searched as the
        // wrap range. This exercises the path where the early-return guard in
        // search_last_in fires and the wrap leg does all the work.
        let b = buf("hello world\n");
        let (s, e, wrapped) = find_next_match(&b, &re("world"), 0, SearchDirection::Backward).unwrap();
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

    // ── search_match_info ─────────────────────────────────────────────────────

    #[test]
    fn match_info_no_match_in_buffer() {
        // Empty match list — total=0, current=0.
        assert_eq!(search_match_info(&[], 0), (0, 0));
    }

    #[test]
    fn match_info_cursor_on_only_match() {
        // "world" at chars 6..10; cursor on 'w' (6) → current=1, total=1.
        assert_eq!(search_match_info(&[(6, 10)], 6), (1, 1));
    }

    #[test]
    fn match_info_cursor_on_last_char_of_match() {
        // Cursor on 'd' (10, inclusive end) → still current=1.
        assert_eq!(search_match_info(&[(6, 10)], 10), (1, 1));
    }

    #[test]
    fn match_info_cursor_between_matches() {
        // "ab" at (1,2), (3,4), (5,6). Cursor on pos 0 — not inside any match.
        assert_eq!(search_match_info(&[(1, 2), (3, 4), (5, 6)], 0), (0, 3));
    }

    #[test]
    fn match_info_cursor_on_second_of_three_matches() {
        // Cursor on char 3 (start of second "ab") → current=2, total=3.
        assert_eq!(search_match_info(&[(1, 2), (3, 4), (5, 6)], 3), (2, 3));
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

    // ── find_match_from_cache ─────────────────────────────────────────────────

    // Matches used in the cache tests: three "ab" spans at (1,2), (3,4), (5,6).
    const CACHE: &[(usize, usize)] = &[(1, 2), (3, 4), (5, 6)];

    #[test]
    fn cache_empty_returns_none() {
        assert!(find_match_from_cache(&[], 0, SearchDirection::Forward).is_none());
        assert!(find_match_from_cache(&[], 0, SearchDirection::Backward).is_none());
    }

    #[test]
    fn cache_forward_first_match() {
        // from_char=0 → first match at (1,2), no wrap.
        let (s, e, w) = find_match_from_cache(CACHE, 0, SearchDirection::Forward).unwrap();
        assert_eq!((s, e), (1, 2));
        assert!(!w);
    }

    #[test]
    fn cache_forward_exact_start() {
        // from_char exactly on a match start → that match is returned.
        let (s, e, w) = find_match_from_cache(CACHE, 3, SearchDirection::Forward).unwrap();
        assert_eq!((s, e), (3, 4));
        assert!(!w);
    }

    #[test]
    fn cache_forward_between_matches() {
        // from_char=2 (gap between first and second match) → second match (3,4).
        let (s, e, w) = find_match_from_cache(CACHE, 2, SearchDirection::Forward).unwrap();
        assert_eq!((s, e), (3, 4));
        assert!(!w);
    }

    #[test]
    fn cache_forward_wraps() {
        // from_char past last match start → wrap to first match.
        let (s, e, w) = find_match_from_cache(CACHE, 6, SearchDirection::Forward).unwrap();
        assert_eq!((s, e), (1, 2));
        assert!(w);
    }

    #[test]
    fn cache_backward_last_before_cursor() {
        // from_char=5 → last match with start < 5 is (3,4).
        let (s, e, w) = find_match_from_cache(CACHE, 5, SearchDirection::Backward).unwrap();
        assert_eq!((s, e), (3, 4));
        assert!(!w);
    }

    #[test]
    fn cache_backward_exact_start_excluded() {
        // Backward uses start < from_char (strict), so from_char=3 excludes (3,4)
        // and returns the previous match (1,2).
        let (s, e, w) = find_match_from_cache(CACHE, 3, SearchDirection::Backward).unwrap();
        assert_eq!((s, e), (1, 2));
        assert!(!w);
    }

    #[test]
    fn cache_backward_wraps() {
        // from_char=0 → no match before 0, wrap to last match (5,6).
        let (s, e, w) = find_match_from_cache(CACHE, 0, SearchDirection::Backward).unwrap();
        assert_eq!((s, e), (5, 6));
        assert!(w);
    }

    #[test]
    fn cache_single_match_forward_wrap() {
        let single = &[(4usize, 7usize)];
        // from_char past the only match → wrap to it.
        let (s, e, w) = find_match_from_cache(single, 8, SearchDirection::Forward).unwrap();
        assert_eq!((s, e), (4, 7));
        assert!(w);
    }

    #[test]
    fn cache_single_match_backward_wrap() {
        let single = &[(4usize, 7usize)];
        // from_char before the only match → wrap to it.
        let (s, e, w) = find_match_from_cache(single, 2, SearchDirection::Backward).unwrap();
        assert_eq!((s, e), (4, 7));
        assert!(w);
    }

    // ── find_matches_in_range ────────────────────────────────────────────────

    #[test]
    fn range_matches_bounded() {
        // "ab" at (1,2), (3,4), (5,6) in "aababab\n". Range 3..6 should
        // return the two matches that fall entirely within it.
        let b = buf("aababab\n");
        let matches = find_matches_in_range(&b, &re("ab"), 3, 6);
        assert_eq!(matches, vec![(3, 4), (5, 6)]);
    }

    #[test]
    fn range_matches_at_boundaries() {
        // Range exactly covering one match.
        let b = buf("aababab\n");
        let matches = find_matches_in_range(&b, &re("ab"), 1, 2);
        assert_eq!(matches, vec![(1, 2)]);
    }

    #[test]
    fn range_matches_excludes_partial() {
        // Range 0..1 doesn't fully contain "ab" at (1,2) — only the 'a' at 1.
        // The regex engine with set_range won't match across the boundary.
        let b = buf("aababab\n");
        let matches = find_matches_in_range(&b, &re("ab"), 0, 0);
        assert_eq!(matches, vec![]);
    }

    #[test]
    fn range_matches_no_hits() {
        let b = buf("hello world\n");
        let matches = find_matches_in_range(&b, &re("xyz"), 0, 10);
        assert_eq!(matches, vec![]);
    }

    #[test]
    fn range_matches_full_buffer() {
        // Full buffer range should behave like find_all_matches.
        let b = buf("aababab\n");
        let all = find_all_matches(&b, &re("ab"));
        let ranged = find_matches_in_range(&b, &re("ab"), 0, 7);
        assert_eq!(all, ranged);
    }

    // ── escape_regex ─────────────────────────────────────────────────────────

    #[test]
    fn escape_regex_plain() {
        assert_eq!(escape_regex("hello"), "hello");
    }

    #[test]
    fn escape_regex_metacharacters() {
        assert_eq!(escape_regex("a.b*c?"), "a\\.b\\*c\\?");
        assert_eq!(escape_regex("[foo]"), "\\[foo\\]");
        assert_eq!(escape_regex("(a|b)"), "\\(a\\|b\\)");
    }

    #[test]
    fn escape_regex_backslash() {
        assert_eq!(escape_regex("a\\b"), "a\\\\b");
    }

    #[test]
    fn escape_regex_roundtrip() {
        // Escaped pattern should match the original text literally.
        let text = "foo.bar*baz";
        let pattern = escape_regex(text);
        let r = re(&pattern);
        let b = buf(&format!("{text}\n"));
        let matches = find_all_matches(&b, &r);
        assert_eq!(matches.len(), 1);
        assert_eq!(matches[0], (0, text.len() - 1));
    }
}
