//! Fuzzy scoring wrapper over `nucleo-matcher`. Hides the crate behind a
//! small API so no caller names it directly (mirrors the `ropey`/`termina`
//! wrapping precedent elsewhere in the editor).
//!
//! Consumed by `PickerSession` (`editor/picker.rs`) and `CompletionSession`
//! (`editor/lsp/completion/mod.rs`), one instance per profile — see
//! [`FuzzyProfile`].

use nucleo_matcher::pattern::{AtomKind, CaseMatching, Normalization, Pattern};
use nucleo_matcher::{Config, Matcher, Utf32Str};

/// Which caller is scoring, and therefore which of two nucleo behaviors it
/// wants. The two axes below must agree, so they're bound to one enum instead
/// of two independent constructor args that could drift out of sync:
///
/// - **Query parsing.** The picker is an fzf-style finder over a query the
///   user typed as *search syntax* — `^ $ ! '` at word boundaries select
///   prefix/postfix/negated/substring matching (`Pattern::parse`). Completion's
///   query is a raw slice of buffer text (`insert.rs`'s `doc.text().slice(..)`)
///   where those characters are legitimate identifier content (`$var`,
///   `println!`) — parsing them as operators would misfire, so completion uses
///   `Pattern::new(.., AtomKind::Fuzzy)`, which segments on whitespace only.
/// - **Prefix bonus.** nucleo's `Config::prefer_prefix` doc says it's "only
///   recommended for autocompletion usecases where the expectation is that the
///   user is typing the entire match... For a full fzf-like fuzzy
///   matcher/picker word segmentation and explicit prefix literals should be
///   used instead" — exactly the split above, so Autocomplete turns it on and
///   Picker leaves nucleo's default (`false`).
#[derive(Clone, Copy)]
pub(crate) enum FuzzyProfile {
    Picker,
    Autocomplete,
}

/// A parsed query, reusable across every haystack scored against it in one
/// re-rank pass. Re-parse on every query edit — parsing is cheap relative to
/// scoring thousands of haystacks per keystroke.
pub(crate) struct FuzzyPattern(Pattern);

/// Owns the reusable scoring engine. One instance per picker/completion
/// session (parallels `CompletionSession::rank_scratch` — caller-owned state
/// reused across every keystroke, never rebuilt per call).
pub(crate) struct FuzzyMatcher {
    matcher: Matcher,
    haystack_buf: Vec<char>,
    profile: FuzzyProfile,
}

impl FuzzyMatcher {
    pub(crate) fn new(profile: FuzzyProfile) -> Self {
        let mut config = Config::DEFAULT;
        config.prefer_prefix = matches!(profile, FuzzyProfile::Autocomplete);
        Self {
            matcher: Matcher::new(config),
            haystack_buf: Vec::new(),
            profile,
        }
    }

    /// Parse `query` under this matcher's profile — smart case (lowercase
    /// query matches any case; mixed/upper case is case-sensitive) and smart
    /// Unicode normalization either way, differing only in whether `query`'s
    /// `^ $ ! '` are search-syntax operators (see [`FuzzyProfile`]).
    pub(crate) fn parse(&self, query: &str) -> FuzzyPattern {
        FuzzyPattern(match self.profile {
            FuzzyProfile::Picker => {
                Pattern::parse(query, CaseMatching::Smart, Normalization::Smart)
            }
            FuzzyProfile::Autocomplete => Pattern::new(
                query,
                CaseMatching::Smart,
                Normalization::Smart,
                AtomKind::Fuzzy,
            ),
        })
    }

    /// Score `haystack` against `pattern`. Higher is a better match; `None`
    /// means `haystack` doesn't contain `pattern` as a match at all (nucleo's
    /// underlying algorithm is subsequence-based, so this is a superset of a
    /// plain subsequence check with much richer ranking).
    ///
    /// An empty pattern matches every haystack with score `0` — verified
    /// empirically below since `nucleo-matcher`'s docs don't state it.
    pub(crate) fn score(&mut self, pattern: &FuzzyPattern, haystack: &str) -> Option<u32> {
        let haystack = Utf32Str::new(haystack, &mut self.haystack_buf);
        pattern.0.score(haystack, &mut self.matcher)
    }
}

#[cfg(test)]
mod tests {
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

    /// Independent oracle: a contiguous/boundary match must outrank a
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
    /// matching on "foo"), but literal needle characters for Autocomplete —
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
    /// start of the haystack — nucleo's own doc recommends this only for
    /// "autocompletion usecases where the expectation is that the user is
    /// typing the entire match." Both haystacks hold the same contiguous run
    /// at a non-zero offset with the same single-class filler on both sides
    /// (nucleo also gives a boundary bonus to a match starting exactly at
    /// index 0 regardless of this setting, so neither haystack starts there —
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
    }

    /// Guardrail regression test: scoring + ranking 100k synthetic paths
    /// against one query, simulating a single re-rank pass (parse once,
    /// score every haystack, sort survivors) under a loose release-mode
    /// bound. `#[ignore]` by default — run explicitly with
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
}
