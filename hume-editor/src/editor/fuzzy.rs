//! Fuzzy scoring wrapper over `nucleo-matcher`. Hides the crate behind a
//! small API so no caller names it directly (mirrors the `ropey`/`termina`
//! wrapping precedent elsewhere in the editor).
//!
//! One instance per profile; see [`FuzzyProfile`].

use nucleo_matcher::pattern::{Atom, AtomKind, CaseMatching, Normalization, Pattern};
use nucleo_matcher::{Config, Matcher, Utf32Str};

/// Which caller is scoring. Query parsing and prefix bonus must agree, so one
/// enum picks both.
///
/// - Query parsing: the picker parses fzf-style search syntax (`^ $ ! '`,
///   whitespace-separated terms) into a [`Pattern`]. Completion's query is raw
///   buffer text where those characters are identifier content (`$var`,
///   `println!`), so it is one [`Atom`] (`AtomKind::Fuzzy`) spanning the whole
///   query, whitespace included. A typed space then matches no LSP label, which
///   closes the menu.
/// - Prefix bonus: nucleo recommends `Config::prefer_prefix` only for
///   autocompletion, so Autocomplete enables it and Picker keeps the default.
#[derive(Clone, Copy)]
pub(in crate::editor) enum FuzzyProfile {
    Picker,
    Autocomplete,
}

/// A parsed query, reusable across every haystack scored against it in one
/// re-rank pass. Re-parse on every query edit: parsing is cheap relative to
/// scoring thousands of haystacks per keystroke. Two variants because the
/// two profiles parse into different nucleo types (see [`FuzzyProfile`]'s
/// query-parsing point): `Pattern` is itself a sequence of atoms, while
/// completion needs exactly one atom spanning the whole query, whitespace
/// included, which only a bare `Atom` gives.
pub(in crate::editor) enum FuzzyPattern {
    Words(Pattern),
    Whole(Atom),
}

/// Owns the reusable scoring engine. One instance per picker/completion
/// session (parallels `SlotSet::ranked`: caller-owned state reused across
/// every keystroke, never rebuilt per call).
pub(in crate::editor) struct FuzzyMatcher {
    matcher: Matcher,
    haystack_buf: Vec<char>,
    profile: FuzzyProfile,
}

impl FuzzyMatcher {
    pub(in crate::editor) fn new(profile: FuzzyProfile) -> Self {
        let mut config = Config::DEFAULT;
        config.prefer_prefix = matches!(profile, FuzzyProfile::Autocomplete);
        Self {
            matcher: Matcher::new(config),
            haystack_buf: Vec::new(),
            profile,
        }
    }

    /// Parse `query` under this matcher's profile: smart case (lowercase
    /// query matches any case; mixed/upper case is case-sensitive) and smart
    /// Unicode normalization either way, differing in whether `query` is a
    /// whitespace-segmented multi-term `Pattern` or one whole-string `Atom`
    /// (see [`FuzzyProfile`]).
    pub(in crate::editor) fn parse(&self, query: &str) -> FuzzyPattern {
        match self.profile {
            FuzzyProfile::Picker => FuzzyPattern::Words(Pattern::parse(
                query,
                CaseMatching::Smart,
                Normalization::Smart,
            )),
            FuzzyProfile::Autocomplete => FuzzyPattern::Whole(Atom::new(
                query,
                CaseMatching::Smart,
                Normalization::Smart,
                AtomKind::Fuzzy,
                false, // no `\ ` escaping: the query is buffer text, not search syntax
            )),
        }
    }

    /// Score `haystack` against `pattern`. Higher is a better match; `None`
    /// means `haystack` doesn't contain `pattern` as a match at all (nucleo's
    /// underlying algorithm is subsequence-based, so this is a superset of a
    /// plain subsequence check with much richer ranking).
    ///
    /// An empty pattern matches every haystack with score `0`, verified
    /// empirically below since `nucleo-matcher`'s docs don't state it.
    pub(in crate::editor) fn score(
        &mut self,
        pattern: &FuzzyPattern,
        haystack: &str,
    ) -> Option<u32> {
        let haystack = Utf32Str::new(haystack, &mut self.haystack_buf);
        match pattern {
            FuzzyPattern::Words(p) => p.score(haystack, &mut self.matcher),
            FuzzyPattern::Whole(a) => a.score(haystack, &mut self.matcher).map(u32::from),
        }
    }
}

#[cfg(test)]
mod tests;
