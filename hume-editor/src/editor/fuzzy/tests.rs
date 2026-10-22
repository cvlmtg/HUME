use super::*;

#[test]
fn subsequence_match_scores_some() {
    let mut m = FuzzyMatcher::new(FuzzyProfile::Picker);
    let p = m.parse("fb");
    assert!(m.score(&p, "foo/bar").is_some());
}

#[test]
fn non_subsequence_scores_none() {
    let mut m = FuzzyMatcher::new(FuzzyProfile::Picker);
    let p = m.parse("xyz");
    assert!(m.score(&p, "foo/bar").is_none());
}

#[test]
fn empty_query_matches_everything() {
    let mut m = FuzzyMatcher::new(FuzzyProfile::Picker);
    let p = m.parse("");
    assert!(m.score(&p, "anything").is_some());
    assert!(m.score(&p, "").is_some());
}

/// An empty pattern scoring every haystack `0` (rather than, say, a
/// length- or content-dependent value) is what lets completion's
/// empty-filter rank fall through entirely to `sortText`: every item ties
/// at the same score, so the tie-break is the only thing that decides
/// order.
#[test]
fn empty_query_scores_every_haystack_equally() {
    let mut m = FuzzyMatcher::new(FuzzyProfile::Autocomplete);
    let p = m.parse("");
    assert_eq!(m.score(&p, "short"), m.score(&p, "a much longer haystack"));
}

/// A space in the query is completion's own token boundary, typed past,
/// never a term separator the way it is for `Picker`, so it must
/// score every candidate `None`, not fall through to an effectively
/// empty pattern that matches everything. `Atom` (`AtomKind::Fuzzy`)
/// scores the whole string including whitespace, unlike `Pattern::
/// parse`, which would word-split on the space and drop the trailing
/// empty atom.
#[test]
fn autocomplete_query_with_a_space_matches_nothing() {
    let mut m = FuzzyMatcher::new(FuzzyProfile::Autocomplete);
    let p = m.parse("foo ");
    assert!(m.score(&p, "foobar").is_none());
    let p = m.parse(" ");
    assert!(m.score(&p, "foobar").is_none());
}

/// Smart case applies to `Autocomplete` too, not just `Picker`: an
/// uppercase-bearing query must be case-sensitive and reject a
/// lowercase-only haystack outright, the same guarantee
/// `smart_case_uppercase_query_is_case_sensitive` pins for `Picker`.
#[test]
fn autocomplete_uppercase_query_is_case_sensitive() {
    let mut m = FuzzyMatcher::new(FuzzyProfile::Autocomplete);
    let p = m.parse("FB");
    assert!(m.score(&p, "foobar").is_none());
    assert!(m.score(&p, "FooBar").is_some());
}

#[test]
fn smart_case_lowercase_query_matches_mixed_case_haystack() {
    let mut m = FuzzyMatcher::new(FuzzyProfile::Picker);
    let p = m.parse("fb");
    assert!(m.score(&p, "FooBar").is_some());
}

#[test]
fn smart_case_uppercase_query_is_case_sensitive() {
    let mut m = FuzzyMatcher::new(FuzzyProfile::Picker);
    let p = m.parse("FB");
    assert!(m.score(&p, "foobar").is_none());
    assert!(m.score(&p, "FooBar").is_some());
}

/// A contiguous/boundary match must outrank a
/// scattered subsequence match for the same query, regardless of the
/// exact score values nucleo assigns (those are the crate's internal
/// tuning and would make the test brittle to a version bump).
#[test]
fn boundary_match_outranks_scattered_subsequence() {
    let mut m = FuzzyMatcher::new(FuzzyProfile::Picker);
    let p = m.parse("fb");
    let boundary = m.score(&p, "foo/bar").expect("query is a subsequence");
    let scattered = m.score(&p, "fxxbxx").expect("query is a subsequence");
    assert!(
        boundary > scattered,
        "boundary match ({boundary}) should outrank scattered subsequence ({scattered})"
    );
}

#[test]
fn unicode_haystack_does_not_panic_and_matches() {
    let mut m = FuzzyMatcher::new(FuzzyProfile::Picker);
    // Smart normalization maps the precomposed "é" to plain "e" for
    // matching purposes, so an ASCII query still finds it.
    let p = m.parse("cafe");
    assert!(m.score(&p, "café").is_some());
    // Emoji ZWJ / modifier sequences must not panic the UTF-32 path.
    let p_emoji = m.parse("👍");
    assert!(m.score(&p_emoji, "👍🏽 thumbs up").is_some());
}

/// `^`/`$`/`!`/`'` at a word boundary are fzf-style search-syntax
/// operators for the Picker profile (`^foo` selects literal-prefix
/// matching on "foo"), but literal needle characters for Autocomplete:
/// completion's query is raw buffer text, where those characters are
/// legitimate identifier content (`$var`, `println!`), not syntax.
#[test]
fn autocomplete_profile_treats_search_operators_as_literal_chars() {
    let mut picker = FuzzyMatcher::new(FuzzyProfile::Picker);
    let picker_pattern = picker.parse("^foo");
    assert!(
        picker.score(&picker_pattern, "foobar").is_some(),
        "picker profile: `^foo` is a prefix operator, matches literal prefix \"foo\""
    );

    let mut autocomplete = FuzzyMatcher::new(FuzzyProfile::Autocomplete);
    let autocomplete_pattern = autocomplete.parse("^foo");
    assert!(
        autocomplete
            .score(&autocomplete_pattern, "foobar")
            .is_none(),
        "autocomplete profile: `^foo` needs a literal '^' char, absent from \"foobar\""
    );
}

/// `prefer_prefix` gives Autocomplete a bonus for matches closer to the
/// start of the haystack. Nucleo's own doc recommends this only for
/// "autocompletion usecases where the expectation is that the user is
/// typing the entire match." Both haystacks hold the same contiguous run
/// at a non-zero offset with the same single-class filler on both sides
/// (nucleo also gives a boundary bonus to a match starting exactly at
/// index 0 regardless of this setting, so neither haystack starts there;
/// that would confound the comparison with a bonus this profile isn't
/// responsible for).
#[test]
fn autocomplete_profile_prefers_the_earlier_match() {
    let earlier = "xabxxxxxxxxxxxxxxx";
    let later = "xxxxxxxxxxxxxxxabx";

    let mut m = FuzzyMatcher::new(FuzzyProfile::Autocomplete);
    let pattern = m.parse("ab");
    let earlier_score = m.score(&pattern, earlier).expect("contiguous match");
    let later_score = m.score(&pattern, later).expect("contiguous match");
    assert!(
        earlier_score > later_score,
        "earlier match ({earlier_score}) should outrank later match ({later_score})"
    );

    // Control: Picker leaves `prefer_prefix` off, so the two haystacks
    // must score identically there. Without this, the assertion above
    // wouldn't actually prove the bonus is profile-gated (it would still
    // pass if `prefer_prefix` were on unconditionally).
    let mut picker = FuzzyMatcher::new(FuzzyProfile::Picker);
    let picker_pattern = picker.parse("ab");
    assert_eq!(
        picker.score(&picker_pattern, earlier),
        picker.score(&picker_pattern, later),
        "Picker leaves prefer_prefix off, so match offset must not affect the score"
    );
}

/// Guardrail regression test: scoring + ranking 100k synthetic paths
/// against one query, simulating a single re-rank pass (parse once,
/// score every haystack, sort survivors) under a loose release-mode
/// bound. `#[ignore]` by default; run explicitly with
/// `cargo test --release -- --ignored` (wall-clock asserts in debug/CI
/// builds are flaky).
///
/// Budget is 2 frames (32ms, at the project's 16ms-frame convention),
/// not 1: measured single-threaded cost on a
/// worst-case (short, low-selectivity) query against 100k items is a
/// stable ~15.5-16.5ms, right at one frame's edge, and a keystroke
/// dropping a single frame under the heaviest realistic query is
/// imperceptible to a typing user.
#[test]
#[ignore]
fn scoring_100k_paths_stays_under_the_b1_budget() {
    let mut m = FuzzyMatcher::new(FuzzyProfile::Picker);
    let haystacks: Vec<String> = (0..100_000)
        .map(|i| format!("src/mod_{}/sub_{}/file_{}.rs", i % 137, (i / 137) % 53, i))
        .collect();

    let start = std::time::Instant::now();
    let pattern = m.parse("mfr");
    let mut scored: Vec<(u32, usize)> = haystacks
        .iter()
        .enumerate()
        .filter_map(|(idx, h)| m.score(&pattern, h).map(|score| (score, idx)))
        .collect();
    scored.sort_unstable_by_key(|&(score, _)| std::cmp::Reverse(score));
    let elapsed = start.elapsed();

    assert!(
        elapsed.as_millis() < 32,
        "scoring+ranking 100k paths took {elapsed:?}, over the 32ms (2-frame) budget \
         (escalate to full `nucleo`'s background/incremental matching if this fails)"
    );
}
