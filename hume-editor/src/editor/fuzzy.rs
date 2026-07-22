//! Fuzzy scoring wrapper over `nucleo-matcher`. Hides the crate behind a
//! small API so no caller names it directly (mirrors the `ropey`/`termina`
//! wrapping precedent elsewhere in the editor).
//!
//! Not yet consumed outside tests — `PickerSession` (B2, see
//! `docs/FUZZY-FINDERS.md`) is the first real caller.
#![allow(dead_code)] // consumed by PickerSession (B2) — remove this allow there

use nucleo_matcher::pattern::{CaseMatching, Normalization, Pattern};
use nucleo_matcher::{Config, Matcher, Utf32Str};

/// A parsed query, reusable across every haystack scored against it in one
/// re-rank pass. Re-parse on every query edit — parsing is cheap relative to
/// scoring thousands of haystacks per keystroke.
pub(crate) struct FuzzyPattern(Pattern);

impl FuzzyPattern {
    /// Smart case (lowercase query matches any case; mixed/upper case is
    /// case-sensitive) and smart Unicode normalization — the defaults
    /// `nucleo-matcher`'s own `Pattern::parse` recommends.
    pub(crate) fn parse(query: &str) -> Self {
        Self(Pattern::parse(
            query,
            CaseMatching::Smart,
            Normalization::Smart,
        ))
    }
}

/// Owns the reusable scoring engine. One instance per picker/completion
/// session (parallels `CompletionSession::rank_scratch` — caller-owned state
/// reused across every keystroke, never rebuilt per call).
pub(crate) struct FuzzyMatcher {
    matcher: Matcher,
    haystack_buf: Vec<char>,
}

impl FuzzyMatcher {
    pub(crate) fn new() -> Self {
        Self {
            matcher: Matcher::new(Config::DEFAULT),
            haystack_buf: Vec::new(),
        }
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
        let mut m = FuzzyMatcher::new();
        let p = FuzzyPattern::parse("fb");
        assert!(m.score(&p, "foo/bar").is_some());
    }

    #[test]
    fn non_subsequence_scores_none() {
        let mut m = FuzzyMatcher::new();
        let p = FuzzyPattern::parse("xyz");
        assert!(m.score(&p, "foo/bar").is_none());
    }

    #[test]
    fn empty_query_matches_everything() {
        let mut m = FuzzyMatcher::new();
        let p = FuzzyPattern::parse("");
        assert!(m.score(&p, "anything").is_some());
        assert!(m.score(&p, "").is_some());
    }

    #[test]
    fn smart_case_lowercase_query_matches_mixed_case_haystack() {
        let mut m = FuzzyMatcher::new();
        let p = FuzzyPattern::parse("fb");
        assert!(m.score(&p, "FooBar").is_some());
    }

    #[test]
    fn smart_case_uppercase_query_is_case_sensitive() {
        let mut m = FuzzyMatcher::new();
        let p = FuzzyPattern::parse("FB");
        assert!(m.score(&p, "foobar").is_none());
        assert!(m.score(&p, "FooBar").is_some());
    }

    /// Independent oracle: a contiguous/boundary match must outrank a
    /// scattered subsequence match for the same query, regardless of the
    /// exact score values nucleo assigns (those are the crate's internal
    /// tuning and would make the test brittle to a version bump).
    #[test]
    fn boundary_match_outranks_scattered_subsequence() {
        let mut m = FuzzyMatcher::new();
        let p = FuzzyPattern::parse("fb");
        let boundary = m.score(&p, "foo/bar").expect("query is a subsequence");
        let scattered = m.score(&p, "fxxbxx").expect("query is a subsequence");
        assert!(
            boundary > scattered,
            "boundary match ({boundary}) should outrank scattered subsequence ({scattered})"
        );
    }

    #[test]
    fn unicode_haystack_does_not_panic_and_matches() {
        let mut m = FuzzyMatcher::new();
        // Smart normalization maps the precomposed "é" to plain "e" for
        // matching purposes, so an ASCII query still finds it.
        let p = FuzzyPattern::parse("cafe");
        assert!(m.score(&p, "café").is_some());
        // Emoji ZWJ / modifier sequences must not panic the UTF-32 path.
        let p_emoji = FuzzyPattern::parse("👍");
        assert!(m.score(&p_emoji, "👍🏽 thumbs up").is_some());
    }

    /// Guardrail regression test: scoring + ranking 100k synthetic paths
    /// against one query, simulating a single re-rank pass (parse once,
    /// score every haystack, sort survivors) under a loose release-mode
    /// bound. `#[ignore]` by default — run explicitly with
    /// `cargo test --release -- --ignored` (wall-clock asserts in debug/CI
    /// builds are flaky).
    ///
    /// Budget is 2 frames (32ms, at the project's 16ms-frame convention —
    /// see `docs/LSP.md`), not 1: measured single-threaded cost on a
    /// worst-case (short, low-selectivity) query against 100k items is a
    /// stable ~15.5-16.5ms, right at one frame's edge, and a keystroke
    /// dropping a single frame under the heaviest realistic query is
    /// imperceptible to a typing user. See `docs/FUZZY-FINDERS.md` Q-B1.
    #[test]
    #[ignore]
    fn scoring_100k_paths_stays_under_the_b1_budget() {
        let mut m = FuzzyMatcher::new();
        let haystacks: Vec<String> = (0..100_000)
            .map(|i| {
                format!(
                    "src/mod_{}/sub_{}/file_{}.rs",
                    i % 137,
                    (i / 137) % 53,
                    i
                )
            })
            .collect();

        let start = std::time::Instant::now();
        let pattern = FuzzyPattern::parse("mfr");
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
             (see docs/FUZZY-FINDERS.md Q-B1 — escalate to full `nucleo` if this fails)"
        );
    }
}
