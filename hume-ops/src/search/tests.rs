use super::*;
use hume_editing::text::BufferText;

fn re(pattern: &str) -> Regex {
    Regex::new(pattern).expect("test regex should be valid")
}

fn buf(text: &str) -> BufferText {
    BufferText::from(text)
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
    assert_eq!(
        find_all_matches(&b, &re("ab")),
        vec![(1, 2), (3, 4), (5, 6)]
    );
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
    // search_match_in(.., take_last: true) fires and the wrap leg does all
    // the work.
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
    // Text chars: [é, space, b, o, n, \n]
    let b = buf("é bon\n");
    let (s, e, _) = find_next_match(&b, &re("bon"), 0, SearchDirection::Forward).unwrap();
    assert_eq!((s, e), (2, 4));
}

#[test]
fn unicode_combining_sequence() {
    // "é" as combining sequence: e (U+0065) + combining acute (U+0301) = 2 chars.
    // Text chars: [e, \u{0301}, space, b, o, n, \n]  (7 chars total)
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
    // Full buffer range returns all matches.
    let b = buf("aababab\n");
    let ranged = find_matches_in_range(&b, &re("ab"), 0, 7);
    assert_eq!(ranged, vec![(1, 2), (3, 4), (5, 6)]);
}

#[test]
fn range_matches_with_combining_graphemes() {
    // "café\n" — 'é' is e + U+0301 (2 codepoints, chars 3 and 4).
    // Searching for "é" within the full range should find it.
    let b = buf("caf\u{0065}\u{0301}\n");
    let matches = find_matches_in_range(&b, &re("\u{0065}\u{0301}"), 0, 5);
    assert_eq!(matches, vec![(3, 4)]);
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
