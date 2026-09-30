use super::*;
use hume_editing::marked::{bound_at, clusters};
use hume_editing::text::BufferText;
use hume_rope::offset::CharOffset;

fn co(n: usize) -> CharOffset {
    CharOffset::new(n)
}

/// `range` as `(first char, last char)`, both inclusive.
fn incl(range: ClusterRange) -> (usize, usize) {
    let chars = range.chars();
    (chars.start.index(), chars.end.index() - 1)
}

fn incls(ranges: &[ClusterRange]) -> Vec<(usize, usize)> {
    ranges.iter().copied().map(incl).collect()
}

/// `scan` advanced `count` matches from `sel`, with the result's covered
/// chars as `(first, last)`, both inclusive.
fn advance(
    scan: &MatchScan<'_>,
    sel: Selection,
    count: usize,
) -> Option<(Selection, (CharOffset, CharOffset), bool)> {
    let text = scan.text;
    let state =
        test_fixtures::testing::state(text.clone(), test_fixtures::testing::single(text, sel));
    let (new_sel, wrapped) = scan.advance(state.view().primary(), count)?;
    let covered = state
        .view()
        .primary()
        .with_selection(new_sel)
        .covered()
        .chars();
    Some((new_sel, (covered.start, covered.end.retreat(1)), wrapped))
}

fn re(pattern: &str) -> Regex {
    Regex::new(pattern).expect("test regex should be valid")
}

fn buf(text: &str) -> BufferText {
    BufferText::from(text)
}

fn fl(multi: bool, verbatim: bool) -> SearchFlags {
    SearchFlags { multi, verbatim }
}

// ── parse_search_input / render_search_input / compile_search_input ────────

#[test]
fn parse_no_flags() {
    assert_eq!(parse_search_input("bar"), (SearchFlags::default(), "bar"));
}

#[test]
fn parse_multi_flag() {
    assert_eq!(parse_search_input("m/bar"), (fl(true, false), "bar"));
}

#[test]
fn parse_multi_verbatim_flags() {
    assert_eq!(parse_search_input("mv/.rs"), (fl(true, true), ".rs"));
}

#[test]
fn parse_flag_order_independent() {
    assert_eq!(parse_search_input("vm/.rs"), (fl(true, true), ".rs"));
}

#[test]
fn parse_leading_slash_is_literal() {
    assert_eq!(
        parse_search_input("/usr/bin"),
        (SearchFlags::default(), "/usr/bin")
    );
    assert_eq!(parse_search_input("/m/s"), (SearchFlags::default(), "/m/s"));
}

#[test]
fn parse_multi_flag_then_literal_leading_slash_pattern() {
    assert_eq!(
        parse_search_input("m//usr/bin"),
        (fl(true, false), "/usr/bin")
    );
}

#[test]
fn parse_unknown_letter_falls_back_to_literal() {
    assert_eq!(
        parse_search_input("x/foo"),
        (SearchFlags::default(), "x/foo")
    );
}

#[test]
fn parse_no_slash_at_all() {
    assert_eq!(parse_search_input("mv"), (SearchFlags::default(), "mv"));
}

#[test]
fn parse_duplicate_flag_letter() {
    assert_eq!(parse_search_input("mm/foo"), (fl(true, false), "foo"));
}

#[test]
fn parse_empty_input() {
    assert_eq!(parse_search_input(""), (SearchFlags::default(), ""));
}

#[test]
fn render_round_trips_through_parse() {
    let flags = fl(true, true);
    let rendered = render_search_input(flags, ".rs");
    assert_eq!(parse_search_input(&rendered), (flags, ".rs"));
}

#[test]
fn render_no_flags_has_no_prefix() {
    assert_eq!(render_search_input(SearchFlags::default(), "bar"), "bar");
}

#[test]
fn render_leaves_leading_slash_pattern_untouched() {
    // "//" is exactly what `*` renders for a punctuation run of two slashes
    // (e.g. the "//" of a "// comment"). It must round-trip, not be read
    // back as an (empty, thus literal-pattern) flag run plus "/".
    assert_eq!(render_search_input(SearchFlags::default(), "//"), "//");
    assert_eq!(parse_search_input("//"), (SearchFlags::default(), "//"));
}

#[test]
fn compile_verbatim_dot_matches_literal_dot_only() {
    let (flags, r) = compile_search_input("v/a.b").expect("valid pattern");
    assert!(flags.verbatim);
    let b = buf("a.b axb\n");
    let matches = find_all_matches(&b, &r);
    assert_eq!(incls(&matches), vec![(0, 2)]);
}

#[test]
fn compile_non_verbatim_dot_matches_any_char() {
    let (flags, r) = compile_search_input("a.b").expect("valid pattern");
    assert!(!flags.verbatim);
    let b = buf("a.b axb\n");
    let matches = find_all_matches(&b, &r);
    assert_eq!(matches.len(), 2);
}

#[test]
fn compile_verbatim_reaches_a_pattern_that_looks_like_flags() {
    // "m/s" typed bare would be read as the multi flag plus pattern "s";
    // `v/` is how a literal "m/s" is matched instead.
    let (flags, r) = compile_search_input("v/m/s").expect("valid pattern");
    assert!(flags.verbatim);
    assert!(!flags.multi);
    let b = buf("m/s other\n");
    let matches = find_all_matches(&b, &r);
    assert_eq!(incls(&matches), vec![(0, 2)]);
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
    assert_eq!(incl(matches[0]), (0, 4));
}

#[test]
fn smart_case_override_force_sensitive() {
    // Explicit (?-i) on a lowercase pattern forces case-sensitive.
    let r = compile_search_regex("(?-i)hello").expect("valid pattern");
    let b = buf("Hello HELLO hello\n");
    let matches = find_all_matches(&b, &r);
    assert_eq!(matches.len(), 1);
    assert_eq!(incl(matches[0]), (12, 16));
}

// ── find_all_matches ──────────────────────────────────────────────────────

#[test]
fn all_matches_empty_buffer() {
    // Empty buffer is just "\n", so no "foo" match.
    let b = buf("\n");
    assert_eq!(find_all_matches(&b, &re("foo")), vec![]);
}

#[test]
fn all_matches_single_hit() {
    let b = buf("hello world\n");
    // "world" starts at char 6, ends at 10 (inclusive).
    assert_eq!(incls(&find_all_matches(&b, &re("world"))), vec![(6, 10)]);
}

#[test]
fn all_matches_multiple_hits() {
    let b = buf("aababab\n");
    // "ab" at chars 1..2, 3..4, 5..6
    assert_eq!(
        incls(&find_all_matches(&b, &re("ab"))),
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
    for span in &matches {
        assert!(
            span.end() > ClusterBound::from(span.start()),
            "zero-width match found at {span:?}"
        );
    }
}

// ── find_next_match (forward) ─────────────────────────────────────────────

#[test]
fn forward_basic() {
    let b = buf("hello world\n");
    let (span, wrapped) =
        find_next_match(&b, &re("world"), bound_at(&b, 0), SearchDirection::Forward).unwrap();
    assert_eq!(incl(span), (6, 10));
    assert!(!wrapped);
}

#[test]
fn forward_from_match_start() {
    // Searching from the start of the existing match should find the same match.
    let b = buf("hello world\n");
    let (span, _) =
        find_next_match(&b, &re("world"), bound_at(&b, 6), SearchDirection::Forward).unwrap();
    assert_eq!(incl(span), (6, 10));
}

#[test]
fn forward_wraps() {
    let b = buf("hello world\n");
    // Searching from after "world" (char 11 = '\n') should wrap and find "world".
    let (span, wrapped) =
        find_next_match(&b, &re("world"), bound_at(&b, 11), SearchDirection::Forward).unwrap();
    assert_eq!(incl(span), (6, 10));
    assert!(wrapped);
}

#[test]
fn forward_no_match() {
    let b = buf("hello\n");
    assert!(find_next_match(&b, &re("xyz"), bound_at(&b, 0), SearchDirection::Forward).is_none());
}

#[test]
fn forward_multiple_matches_picks_first_after_from() {
    let b = buf("aababab\n");
    // Two "ab" matches at (1,2) and (3,4) and (5,6). Searching from char 3.
    let (span, _) =
        find_next_match(&b, &re("ab"), bound_at(&b, 3), SearchDirection::Forward).unwrap();
    assert_eq!(incl(span), (3, 4));
}

// ── find_next_match (backward) ────────────────────────────────────────────

#[test]
fn backward_basic() {
    let b = buf("hello world\n");
    // Search backward from position 11 ('\n'): should find "world" at (6,10).
    let (span, wrapped) = find_next_match(
        &b,
        &re("world"),
        bound_at(&b, 11),
        SearchDirection::Backward,
    )
    .unwrap();
    assert_eq!(incl(span), (6, 10));
    assert!(!wrapped);
}

#[test]
fn backward_wraps() {
    // Searching backward from before the only match should wrap.
    let b = buf("hello world\n");
    let (span, wrapped) =
        find_next_match(&b, &re("world"), bound_at(&b, 3), SearchDirection::Backward).unwrap();
    assert_eq!(incl(span), (6, 10));
    assert!(wrapped);
}

#[test]
fn backward_from_position_zero_wraps() {
    // Primary range is 0..0 (empty), so the entire buffer is searched as the
    // wrap range. This exercises the path where the early-return guard in
    // search_match_in(.., take_last: true) fires and the wrap leg does all
    // the work.
    let b = buf("hello world\n");
    let (span, wrapped) =
        find_next_match(&b, &re("world"), bound_at(&b, 0), SearchDirection::Backward).unwrap();
    assert_eq!(incl(span), (6, 10));
    assert!(wrapped);
}

#[test]
fn backward_multiple_matches_picks_last_before_from() {
    let b = buf("aababab\n");
    // Matches: (1,2), (3,4), (5,6). Searching backward from char 5.
    let (span, _) =
        find_next_match(&b, &re("ab"), bound_at(&b, 5), SearchDirection::Backward).unwrap();
    assert_eq!(incl(span), (3, 4));
}

// ── search_match_info ─────────────────────────────────────────────────────

#[test]
fn match_info_no_match_in_buffer() {
    // Empty match list: total=0, current=0.
    let t = buf("hello\n");
    assert_eq!(search_match_info(&[], t.snap(co(0))), (0, 0));
}

#[test]
fn match_info_cursor_on_only_match() {
    // "world" at chars 6..10; cursor on 'w' (6) → current=1, total=1.
    let t = buf("hello world\n");
    assert_eq!(
        search_match_info(&[clusters(&t, 6, 11)], t.snap(co(6))),
        (1, 1)
    );
}

#[test]
fn match_info_cursor_on_last_char_of_match() {
    // Cursor on 'd' (10, inclusive end) → still current=1.
    let t = buf("hello world\n");
    assert_eq!(
        search_match_info(&[clusters(&t, 6, 11)], t.snap(co(10))),
        (1, 1)
    );
}

#[test]
fn match_info_cursor_between_matches() {
    // "ab" at (1,2), (3,4), (5,6). Cursor on pos 0, not inside any match.
    assert_eq!(
        search_match_info(&cache(), cache_text().snap(co(0))),
        (0, 3)
    );
}

#[test]
fn match_info_cursor_on_second_of_three_matches() {
    // Cursor on char 3 (start of second "ab") → current=2, total=3.
    assert_eq!(
        search_match_info(&cache(), cache_text().snap(co(3))),
        (2, 3)
    );
}

// ── Unicode / grapheme cluster ────────────────────────────────────────────

#[test]
fn unicode_multibyte_char() {
    // "é" in NFC is a single codepoint (U+00E9, 2 bytes in UTF-8).
    // Text chars: [é, space, b, o, n, \n]
    let b = buf("é bon\n");
    let (span, _) =
        find_next_match(&b, &re("bon"), bound_at(&b, 0), SearchDirection::Forward).unwrap();
    assert_eq!(incl(span), (2, 4));
}

#[test]
fn unicode_combining_sequence() {
    // "é" as combining sequence: e (U+0065) + combining acute (U+0301) = 2 chars.
    // Text chars: [e, \u{0301}, space, b, o, n, \n]  (7 chars total)
    let b = buf("e\u{0301} bon\n");
    let (span, _) =
        find_next_match(&b, &re("bon"), bound_at(&b, 0), SearchDirection::Forward).unwrap();
    // "b" is at char 3, "bon" spans chars 3..5 inclusive
    assert_eq!(incl(span), (3, 5));
}

// ── find_match_from_cache ─────────────────────────────────────────────────

// Matches used in the cache tests: three "ab" spans at (1,2), (3,4), (5,6).
fn cache_text() -> BufferText {
    buf("aababab\n")
}

fn cache() -> Vec<ClusterRange> {
    let text = cache_text();
    vec![
        clusters(&text, 1, 3),
        clusters(&text, 3, 5),
        clusters(&text, 5, 7),
    ]
}

#[test]
fn cache_empty_returns_none() {
    assert!(
        find_match_from_cache(&[], bound_at(&cache_text(), 0), SearchDirection::Forward).is_none()
    );
    assert!(
        find_match_from_cache(&[], bound_at(&cache_text(), 0), SearchDirection::Backward).is_none()
    );
}

#[test]
fn cache_forward_first_match() {
    // from_char=0 → first match at (1,2), no wrap.
    let (span, w) = find_match_from_cache(
        &cache(),
        bound_at(&cache_text(), 0),
        SearchDirection::Forward,
    )
    .unwrap();
    assert_eq!(incl(span), (1, 2));
    assert!(!w);
}

#[test]
fn cache_forward_exact_start() {
    // from_char exactly on a match start → that match is returned.
    let (span, w) = find_match_from_cache(
        &cache(),
        bound_at(&cache_text(), 3),
        SearchDirection::Forward,
    )
    .unwrap();
    assert_eq!(incl(span), (3, 4));
    assert!(!w);
}

#[test]
fn cache_forward_between_matches() {
    // from_char=2 (gap between first and second match) → second match (3,4).
    let (span, w) = find_match_from_cache(
        &cache(),
        bound_at(&cache_text(), 2),
        SearchDirection::Forward,
    )
    .unwrap();
    assert_eq!(incl(span), (3, 4));
    assert!(!w);
}

#[test]
fn cache_forward_wraps() {
    // from_char past last match start → wrap to first match.
    let (span, w) = find_match_from_cache(
        &cache(),
        bound_at(&cache_text(), 6),
        SearchDirection::Forward,
    )
    .unwrap();
    assert_eq!(incl(span), (1, 2));
    assert!(w);
}

#[test]
fn cache_backward_last_before_cursor() {
    // from_char=5 → last match with start < 5 is (3,4).
    let (span, w) = find_match_from_cache(
        &cache(),
        bound_at(&cache_text(), 5),
        SearchDirection::Backward,
    )
    .unwrap();
    assert_eq!(incl(span), (3, 4));
    assert!(!w);
}

#[test]
fn cache_backward_exact_start_excluded() {
    // Backward uses start < from_char (strict), so from_char=3 excludes (3,4)
    // and returns the previous match (1,2).
    let (span, w) = find_match_from_cache(
        &cache(),
        bound_at(&cache_text(), 3),
        SearchDirection::Backward,
    )
    .unwrap();
    assert_eq!(incl(span), (1, 2));
    assert!(!w);
}

#[test]
fn cache_backward_wraps() {
    // from_char=0 → no match before 0, wrap to last match (5,6).
    let (span, w) = find_match_from_cache(
        &cache(),
        bound_at(&cache_text(), 0),
        SearchDirection::Backward,
    )
    .unwrap();
    assert_eq!(incl(span), (5, 6));
    assert!(w);
}

#[test]
fn cache_single_match_forward_wrap() {
    let text = buf("aaaabbbb\n");
    let single = &[clusters(&text, 4, 8)];
    // from_char past the only match → wrap to it.
    let (span, w) =
        find_match_from_cache(single, bound_at(&text, 8), SearchDirection::Forward).unwrap();
    assert_eq!(incl(span), (4, 7));
    assert!(w);
}

#[test]
fn cache_single_match_backward_wrap() {
    let text = buf("aaaabbbb\n");
    let single = &[clusters(&text, 4, 8)];
    // from_char before the only match → wrap to it.
    let (span, w) =
        find_match_from_cache(single, bound_at(&text, 2), SearchDirection::Backward).unwrap();
    assert_eq!(incl(span), (4, 7));
    assert!(w);
}

// ── MatchScan ────────────────────────────────────────────────────────────

// "bar" at (3,5) and (10,12).
// indices: a0 a1 sp2 b3 a4 r5 sp6 b7 b8 sp9 b10 a11 r12 sp13 c14 c15 \n16
fn scan_text() -> BufferText {
    buf("aa bar bb bar cc\n")
}

#[test]
fn match_scan_at_selection_forward_scans_regex_when_uncached() {
    let text = scan_text();
    let regex = re("bar");
    let scan = MatchScan {
        text: &text,
        regex: &regex,
        cached: None,
        direction: SearchDirection::Forward,
        mode: MotionMode::Move,
        seed: MatchSeed::AtSelection,
    };
    let (_, span, wrapped) =
        advance(&scan, test_fixtures::testing::cursor(&text, 0), 1).expect("bar exists");
    assert_eq!(span, (co(3), co(5)));
    assert!(!wrapped);
}

#[test]
fn match_scan_past_selection_steps_over_current_match() {
    let text = scan_text();
    let regex = re("bar");
    let scan = MatchScan {
        text: &text,
        regex: &regex,
        cached: None,
        direction: SearchDirection::Forward,
        mode: MotionMode::Move,
        seed: MatchSeed::PastSelection,
    };
    // Selection already sitting on the first "bar": PastSelection must land
    // on the second one, not re-find the first.
    let sel = test_fixtures::testing::sel(&text, 3, 5);
    let (_, span, _) = advance(&scan, sel, 1).expect("second bar exists");
    assert_eq!(span, (co(10), co(12)));
}

#[test]
fn match_scan_at_selection_stays_on_current_match_when_already_there() {
    let text = scan_text();
    let regex = re("bar");
    let scan = MatchScan {
        text: &text,
        regex: &regex,
        cached: None,
        direction: SearchDirection::Forward,
        mode: MotionMode::Move,
        seed: MatchSeed::AtSelection,
    };
    // AtSelection seeds from the selection's own start, so a selection
    // already on a match re-finds that same match instead of skipping it:
    // the live-preview seed rule (a keystroke shouldn't jump a selection
    // that's already correct).
    let sel = test_fixtures::testing::sel(&text, 3, 5);
    let (_, span, _) = advance(&scan, sel, 1).expect("bar exists");
    assert_eq!(span, (co(3), co(5)));
}

#[test]
fn match_scan_cached_empty_is_zero_matches_not_cold() {
    let text = scan_text();
    let regex = re("bar");
    // A warm cache with zero matches must not fall back to scanning `regex`;
    // that's exactly the bug an "empty means cold" heuristic would reintroduce
    // once the live-search path warms the cache on every keystroke.
    let scan = MatchScan {
        text: &text,
        regex: &regex,
        cached: Some(&[]),
        direction: SearchDirection::Forward,
        mode: MotionMode::Move,
        seed: MatchSeed::AtSelection,
    };
    assert!(advance(&scan, test_fixtures::testing::cursor(&text, 0), 1).is_none());
}

#[test]
fn match_scan_cached_populated_binary_searches_instead_of_scanning() {
    let text = scan_text();
    let regex = re("zzz"); // would find nothing if `cached` were ignored
    let cached = [clusters(&text, 3, 6), clusters(&text, 10, 13)];
    let scan = MatchScan {
        text: &text,
        regex: &regex,
        cached: Some(&cached),
        direction: SearchDirection::Forward,
        mode: MotionMode::Move,
        seed: MatchSeed::AtSelection,
    };
    let (_, span, _) = advance(&scan, test_fixtures::testing::cursor(&text, 0), 1)
        .expect("cache has matches even though regex would find none");
    assert_eq!(span, (co(3), co(5)));
}

#[test]
fn match_scan_count_of_two_advances_twice() {
    let text = scan_text();
    let regex = re("bar");
    let scan = MatchScan {
        text: &text,
        regex: &regex,
        cached: None,
        direction: SearchDirection::Forward,
        mode: MotionMode::Move,
        seed: MatchSeed::PastSelection,
    };
    let (_, span, _) =
        advance(&scan, test_fixtures::testing::cursor(&text, 0), 2).expect("two bars exist");
    assert_eq!(span, (co(10), co(12)));
}

#[test]
fn match_scan_returns_none_when_zero_matches_total() {
    // The only way a hop in the `count` chain can miss: both `find_next_match`
    // and `find_match_from_cache` wrap around the buffer boundary rather than
    // stopping, so a chain only fails atomically when no match exists at all:
    // wrapping otherwise guarantees every later hop in the chain succeeds too.
    let text = scan_text();
    let regex = re("zzz");
    let scan = MatchScan {
        text: &text,
        regex: &regex,
        cached: None,
        direction: SearchDirection::Forward,
        mode: MotionMode::Move,
        seed: MatchSeed::AtSelection,
    };
    assert!(advance(&scan, test_fixtures::testing::cursor(&text, 0), 1).is_none());
}

#[test]
fn match_scan_extend_keeps_original_anchor() {
    let text = scan_text();
    let regex = re("bar");
    let scan = MatchScan {
        text: &text,
        regex: &regex,
        cached: None,
        direction: SearchDirection::Forward,
        mode: MotionMode::Extend,
        seed: MatchSeed::PastSelection,
    };
    let sel = test_fixtures::testing::sel(&text, 0, 1);
    let (new_sel, _, _) = advance(&scan, sel, 1).expect("bar exists");
    assert_eq!(
        new_sel.anchor().offset(),
        co(0),
        "anchor stays at the original selection's anchor"
    );
    assert_eq!(
        new_sel.head().offset(),
        co(5),
        "head moves to the match's forward edge"
    );
}

#[test]
fn match_scan_advance_all_merges_converging_selections() {
    // "aa foo bb\n": one "foo" at (3,5); both selections sit before it, so
    // both independently land on it and merge into one.
    let text = buf("aa foo bb\n");
    let regex = re("foo");
    let scan = MatchScan {
        text: &text,
        regex: &regex,
        cached: None,
        direction: SearchDirection::Forward,
        mode: MotionMode::Move,
        seed: MatchSeed::AtSelection,
    };
    let sels = test_fixtures::testing::set(
        &text,
        vec![
            test_fixtures::testing::cursor(&text, 0),
            test_fixtures::testing::cursor(&text, 2),
        ],
        0,
    );
    let (new_sels, _wrapped) = scan
        .advance_all(test_fixtures::testing::state(text.clone(), sels), 1)
        .expect("both selections match");
    assert_eq!(
        new_sels.view().len(),
        1,
        "both selections converge on the one \"foo\" match"
    );
}

#[test]
fn match_scan_advance_all_none_when_nothing_matches() {
    let text = buf("aa bb\n");
    let regex = re("zzz");
    let scan = MatchScan {
        text: &text,
        regex: &regex,
        cached: None,
        direction: SearchDirection::Forward,
        mode: MotionMode::Move,
        seed: MatchSeed::AtSelection,
    };
    let sels = test_fixtures::testing::set(
        &text,
        vec![
            test_fixtures::testing::cursor(&text, 0),
            test_fixtures::testing::cursor(&text, 3),
        ],
        0,
    );
    assert!(
        scan.advance_all(test_fixtures::testing::state(text.clone(), sels), 1)
            .is_none()
    );
}

#[test]
fn match_scan_advance_all_wrapped_bit_tracks_only_the_primary() {
    // "bar" at (0,2), (7,9), (14,16). Primary (co(15), inside the third "bar")
    // has nothing ahead of it and must wrap to the first match; the secondary
    // (co(3), before the second "bar") finds one without wrapping.
    let text = buf("bar xx bar yy bar\n");
    let regex = re("bar");
    let scan = MatchScan {
        text: &text,
        regex: &regex,
        cached: None,
        direction: SearchDirection::Forward,
        mode: MotionMode::Move,
        seed: MatchSeed::AtSelection,
    };
    let sels = test_fixtures::testing::set(
        &text,
        vec![
            test_fixtures::testing::cursor(&text, 3),
            test_fixtures::testing::cursor(&text, 15),
        ],
        1, // co(15) is primary
    );
    let (new_sels, primary_wrapped) = scan
        .advance_all(test_fixtures::testing::state(text.clone(), sels), 1)
        .expect("both selections match");
    assert_eq!(
        new_sels.view().len(),
        2,
        "the two hops land on different matches"
    );
    assert!(primary_wrapped, "only the primary's own hop wrapped");
}

// ── find_matches_in_range ────────────────────────────────────────────────

#[test]
fn range_matches_bounded() {
    // "ab" at (1,2), (3,4), (5,6) in "aababab\n". Range 3..6 should
    // return the two matches that fall entirely within it.
    let b = buf("aababab\n");
    let matches = find_matches_in_range(&b, &re("ab"), clusters(&b, 3, 7));
    assert_eq!(incls(&matches), vec![(3, 4), (5, 6)]);
}

#[test]
fn range_matches_at_boundaries() {
    // Range exactly covering one match.
    let b = buf("aababab\n");
    let matches = find_matches_in_range(&b, &re("ab"), clusters(&b, 1, 3));
    assert_eq!(incls(&matches), vec![(1, 2)]);
}

#[test]
fn range_matches_excludes_partial() {
    // Range 0..1 doesn't fully contain "ab" at (1,2), only the 'a' at 1.
    // The regex engine with set_range won't match across the boundary.
    let b = buf("aababab\n");
    let matches = find_matches_in_range(&b, &re("ab"), clusters(&b, 0, 1));
    assert_eq!(matches, vec![]);
}

#[test]
fn range_matches_no_hits() {
    let b = buf("hello world\n");
    let matches = find_matches_in_range(&b, &re("xyz"), clusters(&b, 0, 11));
    assert_eq!(matches, vec![]);
}

#[test]
fn range_matches_full_buffer() {
    // Full buffer range returns all matches.
    let b = buf("aababab\n");
    let ranged = find_matches_in_range(&b, &re("ab"), clusters(&b, 0, 8));
    assert_eq!(incls(&ranged), vec![(1, 2), (3, 4), (5, 6)]);
}

#[test]
fn range_matches_with_combining_graphemes() {
    // "café\n": 'é' is e + U+0301 (2 codepoints, chars 3 and 4).
    // Searching for "é" within the full range should find it.
    let b = buf("caf\u{0065}\u{0301}\n");
    let matches = find_matches_in_range(&b, &re("\u{0065}\u{0301}"), clusters(&b, 0, 6));
    assert_eq!(incls(&matches), vec![(3, 4)]);
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
    assert_eq!(incl(matches[0]), (0, text.len() - 1));
}

// ── word_search_pattern ──────────────────────────────────────────────────

#[test]
fn word_search_pattern_anchors_a_plain_word() {
    let chars = hume_editing::word::WordChars::default();
    assert_eq!(word_search_pattern("hello", chars), r"\bhello\b");
}

/// NFD "café" (c, a, f, e, U+0301) ends on a *combining mark*, not on its
/// cluster's base char. Judging the trailing edge with `chars().next_back()`
/// hands `classify` the mark (`Punctuation` to HUME), dropping the `\b` and
/// letting `*` match the "café" prefix of "cafétéria". The edge belongs to
/// the cluster's base 'e', and `\b` after the mark holds because rust-regex's
/// own `\w` includes `\p{M}`.
#[test]
fn word_search_pattern_anchors_a_combining_sequence_on_its_base_char() {
    let chars = hume_editing::word::WordChars::default();
    let pattern = word_search_pattern("cafe\u{301}", chars);
    assert_eq!(pattern, "\\bcafe\u{301}\\b");

    let r = re(&pattern);
    let b = buf("cafe\u{301} cafe\u{301}teria\n");
    let matches = find_all_matches(&b, &r);
    assert_eq!(
        matches.len(),
        1,
        "the standalone word matches; the prefix of \"cafétéria\" must not"
    );
    assert_eq!(incl(matches[0]), (0, 4));
}

/// U+FF3F (FULLWIDTH LOW LINE) is `\p{Pc}`, a word character to
/// `regex_syntax::try_is_word_character`, but HUME classifies it as
/// `Punctuation` (not `_`, not in `word-chars`). Anchoring on the
/// regex-syntax answer alone produces `\b＿\b`, which can never match:
/// rust-regex also sees both neighbours as word characters, so neither
/// boundary can hold. The pattern must drop the anchor on this edge instead.
#[test]
fn word_search_pattern_skips_boundary_hume_does_not_consider_a_word_char() {
    let chars = hume_editing::word::WordChars::default();
    let pattern = word_search_pattern("\u{FF3F}", chars);
    assert_eq!(pattern, "\u{FF3F}");

    let r = re(&pattern);
    let b = buf("\u{FF41}\u{FF3F}\u{FF42}\n"); // fullwidth a, the char under test, fullwidth b
    let matches = find_all_matches(&b, &r);
    assert_eq!(matches.len(), 1, "the punctuation run must still match");
}

/// The opposite direction from the two tests above: `chars` *wider* than
/// rust-regex's `\w`. With `-` a word char, "foo-bar" is one run and both
/// edges anchor, but rust-regex still reads `-` itself as non-word, so
/// `\bfoo-bar\b` also holds inside "foo-bar-baz". `word_search_pattern`'s doc
/// accepts this over-match (rust-regex has neither a configurable `\w` class
/// nor lookbehind to express the wider rule); pinned here so a future
/// anchoring change can't alter it silently.
#[test]
fn word_search_pattern_over_matches_a_wider_word_chars_run() {
    let chars = hume_editing::word::WordChars::new("-");
    let pattern = word_search_pattern("foo-bar", chars);
    assert_eq!(pattern, r"\bfoo\-bar\b");

    let r = re(&pattern);
    let b = buf("foo-bar-baz\n");
    let matches = find_all_matches(&b, &r);
    assert_eq!(matches.len(), 1, "the anchors hold inside the longer run");
    assert_eq!(incl(matches[0]), (0, 6)); // inclusive end: 'r' is char index 6
}

// ── Corpus ────────────────────────────────────────────────────────────────

#[test]
fn find_next_match_spans_every_corpus_sample_exactly() {
    for s in test_fixtures::unicode::ALL {
        let text = BufferText::from(format!("x\n{s}b\n").as_str());
        let regex = regex_cursor::engines::meta::Regex::new(&regex_syntax::escape(s)).unwrap();
        let (span, _) =
            find_next_match(&text, &regex, bound_at(&text, 0), SearchDirection::Forward)
                .unwrap_or_else(|| panic!("no match for {s:?}"));
        assert_eq!(incl(span), (2, 1 + s.chars().count()), "{s:?}");
    }
}

#[test]
fn smart_case_uppercase_accented_letter_is_sensitive() {
    let r = compile_search_regex("\u{c9}").expect("valid pattern");
    let b = buf("\u{e9} \u{c9}\n");
    assert_eq!(incls(&find_all_matches(&b, &r)), vec![(2, 2)]);
}

#[test]
fn smart_case_lowercase_accented_letter_matches_both_cases() {
    let r = compile_search_regex("\u{e9}").expect("valid pattern");
    let b = buf("\u{e9} \u{c9}\n");
    assert_eq!(incls(&find_all_matches(&b, &r)), vec![(0, 0), (2, 2)]);
}

#[test]
fn smart_case_lowercase_sigma_matches_capital_and_final_sigma() {
    let r = compile_search_regex("\u{3c3}").expect("valid pattern");
    let b = buf("\u{3a3} \u{3c3} \u{3c2}\n");
    assert_eq!(
        incls(&find_all_matches(&b, &r)),
        vec![(0, 0), (2, 2), (4, 4)]
    );
}

#[test]
fn find_next_match_from_a_char_offset_after_multi_byte_text() {
    let r = compile_search_regex("a").expect("valid pattern");
    let b = buf("\u{e9}\u{6f22}\u{1f600}ab a\n");
    let from = |n, dir| find_next_match(&b, &r, bound_at(&b, n), dir).map(|(span, _)| incl(span));
    assert_eq!(from(3, SearchDirection::Forward), Some((3, 3)));
    assert_eq!(from(4, SearchDirection::Forward), Some((6, 6)));
    assert_eq!(from(6, SearchDirection::Backward), Some((3, 3)));
}

#[test]
fn match_scan_steps_over_multi_byte_text_between_matches() {
    let text = buf("ab\u{1f600}ab\n");
    let regex = compile_search_regex("ab").expect("valid pattern");
    let scan = MatchScan {
        text: &text,
        regex: &regex,
        cached: None,
        direction: SearchDirection::Forward,
        mode: MotionMode::Move,
        seed: MatchSeed::PastSelection,
    };
    let (sel, _, wrapped) =
        advance(&scan, test_fixtures::testing::sel(&text, 0, 1), 1).expect("a second match");
    assert!(!wrapped);
    assert_eq!((sel.anchor().offset(), sel.head().offset()), (co(3), co(4)));
}
