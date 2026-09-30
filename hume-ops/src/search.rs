//! Incremental search over a rope buffer using `regex-cursor`.
//!
//! All functions here are pure: they read `BufferText` and a compiled `Regex`,
//! return cluster ranges, and never modify editor state.
//!
//! # Coordinate system
//!
//! `regex-cursor` operates on byte offsets; HUME's selection model uses
//! grapheme clusters. A match converts once, in [`match_range`]: its bytes
//! become chars, and the chars widen to the clusters that hold them, so a
//! pattern matching a lone combining mark selects the whole cluster it sits
//! in.

use regex_cursor::{Input, RopeyCursor, engines::meta::Regex};

use hume_editing::selection::{Facing, Selection, SelectionView};
use hume_editing::state::EditState;
use hume_editing::text::BufferText;
use hume_editing::word::{CharClass, WordChars};
use hume_rope::cluster::{ClusterBound, ClusterRange, ClusterStart};
use hume_rope::grapheme::prev_str_boundary;
use hume_rope::offset::ExclusiveRange;

use crate::MotionMode;

/// Direction for `search-forward` / `search-backward` and `search-next` / `search-prev`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SearchDirection {
    Forward,
    Backward,
}

/// Compile a search pattern with **smart case**: all-lowercase patterns become
/// case-insensitive; patterns containing any uppercase character stay
/// case-sensitive.
///
/// Explicit `(?i)`/`(?-i)` in the pattern wins: smart-case only prepends `(?i)`,
/// so a later flag group in the pattern overrides it.
///
/// Private: [`compile_search_input`] is the crate's one compilation path from
/// prompt input to a `Regex`. Every producer of a `SearchPattern` goes
/// through it rather than calling this directly.
fn compile_search_regex(pattern: &str) -> Option<Regex> {
    let effective;
    let pat = if pattern.chars().any(|c| c.is_uppercase()) {
        pattern
    } else {
        // Prepend (?i); an explicit (?-i) later in the pattern will override.
        effective = format!("(?i){pattern}");
        &effective
    };
    Regex::new(pat).ok()
}

// ── search input flags ────────────────────────────────────────────────────────

/// Leading flags on a search/sift prompt's raw input (`m/bar`, `v/.rs`).
///
/// `multi`: every selection searches independently and moves to its own next
/// match, instead of only the primary. Inert at the sift prompt: sift already
/// operates on every selection.
///
/// `verbatim`: the pattern is matched literally (via `escape_regex`) instead
/// of as a regex.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct SearchFlags {
    pub multi: bool,
    pub verbatim: bool,
}

/// Split a raw prompt input into its leading flags and the pattern.
///
/// Grammar: if every character before the first `/` is a known flag letter
/// (`m`, `v`) *and that run is non-empty*, it is the flag set and everything
/// after the `/` (verbatim, including any further `/`) is the pattern.
/// Otherwise there are no flags and `input` is the pattern as-is.
///
/// Only the first `/` after a non-empty flag run is ever a separator, so a
/// pattern never needs escaping for `/`s of its own (`v/a/b/c` means literal
/// `a/b/c`), including a pattern that itself starts with `/`: `input`
/// `"/usr/bin"` has an empty prefix before its first `/`, which is not a
/// flag run, so the whole string is the pattern. There is no way to write a
/// literal pattern that starts with a flag letter followed by `/` (e.g. the
/// two characters `m/`) other than going through `v/`, which forces the
/// question moot: `v/m/s` matches the literal text `m/s`. An unknown letter
/// (`x/foo`) falls back to "no flags" rather than erroring: during live
/// search every keystroke is parsed, so a typo or a pattern that happens to
/// start with letters must resolve to *something* sensible, not a rejected
/// input.
pub fn parse_search_input(input: &str) -> (SearchFlags, &str) {
    let Some(slash) = input.find('/') else {
        return (SearchFlags::default(), input);
    };
    let prefix = &input[..slash];
    if prefix.is_empty() {
        return (SearchFlags::default(), input);
    }
    let mut flags = SearchFlags::default();
    for c in prefix.chars() {
        match c {
            'm' => flags.multi = true,
            'v' => flags.verbatim = true,
            _ => return (SearchFlags::default(), input),
        }
    }
    (flags, &input[slash + 1..])
}

/// Render `flags` and `pattern` back into the prompt input that
/// `parse_search_input` recovers unchanged: the inverse of `parse_search_input`,
/// over the range that grammar can represent. A default-flags `pattern` that
/// itself starts with a flag letter followed by `/` (e.g. `"m/s"`) is outside
/// that range: `parse_search_input` cannot tell it apart from an actual `m`
/// flag. The `debug_assert!` below catches a producer that hands this function
/// such a pattern. `set_search_pattern` re-parses this function's own output
/// to get the regex it stores, so an unrepresentable pair would silently
/// compile the *wrong regex* in release, not just mangle the register text.
/// Every current caller is safe: `*`'s patterns are `\b`-anchored word runs or
/// pure punctuation runs (never flag letters immediately followed by `/`), and
/// Ctrl-`/` always sets `verbatim`, which puts a non-empty `v` run in front
/// regardless of `pattern`'s own text.
pub fn render_search_input(flags: SearchFlags, pattern: &str) -> String {
    let mut prefix = String::new();
    if flags.multi {
        prefix.push('m');
    }
    if flags.verbatim {
        prefix.push('v');
    }
    let rendered = if prefix.is_empty() {
        pattern.to_string()
    } else {
        format!("{prefix}/{pattern}")
    };
    debug_assert_eq!(
        parse_search_input(&rendered),
        (flags, pattern),
        "pattern {pattern:?} under flags {flags:?} is not representable by the flag grammar"
    );
    rendered
}

/// Parse `input` for leading flags, then compile the remaining pattern:
/// literally (via `escape_regex`) when `verbatim` is set, as smart-case
/// regex otherwise. `None` when the resulting pattern is not a valid regex.
pub fn compile_search_input(input: &str) -> Option<(SearchFlags, Regex)> {
    let (flags, pattern) = parse_search_input(input);
    let owned;
    let effective = if flags.verbatim {
        owned = escape_regex(pattern);
        &owned
    } else {
        pattern
    };
    let regex = compile_search_regex(effective)?;
    Some((flags, regex))
}

/// Find the next regex match in `text`, starting from char offset `from_char`.
///
/// # Direction
///
/// - **Forward**: finds the first match whose start is ≥ `from`. Wraps to the
///   start of the buffer if no match is found forward.
/// - **Backward**: finds the last match whose start is < `from`. Wraps to
///   the end of the buffer if no match is found backward.
///
/// # Return value
///
/// `Some((span, wrapped))` on success, where:
/// - `span` is the clusters holding the match
/// - `wrapped` is `true` when the match was found after wrapping around the
///   buffer boundary
///
/// Returns `None` when no match exists anywhere in the buffer, or when the
/// match is zero-width (which would cause the cursor to appear stuck).
pub fn find_next_match(
    text: &BufferText,
    regex: &Regex,
    from: ClusterBound,
    direction: SearchDirection,
) -> Option<(ClusterRange, bool)> {
    let from_byte = text.char_to_byte(from.offset());
    let total_bytes = text.len_bytes();

    match direction {
        SearchDirection::Forward => {
            // Primary: search from_byte..end
            if let Some(span) = search_match_in(text, regex, from_byte..total_bytes, false) {
                return Some((span, false));
            }
            // Wrap: search 0..from_byte
            if let Some(span) = search_match_in(text, regex, 0..from_byte, false) {
                return Some((span, true));
            }
        }
        SearchDirection::Backward => {
            // Primary: search 0..from_byte, take the last match
            if let Some(span) = search_match_in(text, regex, 0..from_byte, true) {
                return Some((span, false));
            }
            // Wrap: search from_byte..end, take the last match
            if let Some(span) = search_match_in(text, regex, from_byte..total_bytes, true) {
                return Some((span, true));
            }
        }
    }

    None
}

/// Return all non-overlapping regex matches in `text`, in document order.
/// Zero-width matches are skipped.
pub fn find_all_matches(text: &BufferText, regex: &Regex) -> Vec<ClusterRange> {
    matches_in_bytes(text, regex, 0..text.len_bytes())
}

/// Return all non-overlapping regex matches that fall entirely within
/// `range`, in document order. Zero-width matches are skipped.
pub fn find_matches_in_range(
    text: &BufferText,
    regex: &Regex,
    range: ClusterRange,
) -> Vec<ClusterRange> {
    let chars = range.chars();
    matches_in_bytes(
        text,
        regex,
        text.char_to_byte(chars.start)..text.char_to_byte(chars.end),
    )
}

/// The matches in `byte_range` as cluster ranges. Two matches that widen to
/// the same cluster (a base char and its combining mark matched apart) keep
/// only the first, so the list stays sorted and non-overlapping.
fn matches_in_bytes(
    text: &BufferText,
    regex: &Regex,
    byte_range: std::ops::Range<usize>,
) -> Vec<ClusterRange> {
    let cursor = RopeyCursor::new(text.full_slice());
    let mut input = Input::new(cursor);
    input.set_range(byte_range);

    let mut matches: Vec<ClusterRange> = Vec::new();
    for m in regex.find_iter(input) {
        let Some(range) = match_range(text, m.start()..m.end()) else {
            continue;
        };
        if matches
            .last()
            .is_none_or(|prev| prev.end() <= ClusterBound::from(range.start()))
        {
            matches.push(range);
        }
    }
    matches
}

/// The clusters holding the match at `bytes`, or `None` for a zero-width
/// match.
fn match_range(text: &BufferText, bytes: std::ops::Range<usize>) -> Option<ClusterRange> {
    text.covering(ExclusiveRange::new(
        text.byte_to_char(bytes.start),
        text.byte_to_char(bytes.end),
    ))
}

/// Escape regex metacharacters so the string matches literally.
///
/// `regex_syntax::escape` escapes a few characters (`-`, `#`, `&`, `~`) that
/// only have meaning inside `[...]` classes. Harmless here since none of
/// this module's patterns are ever spliced into one.
fn escape_regex(s: &str) -> String {
    regex_syntax::escape(s)
}

/// Build a `*` (search-word-under-cursor) pattern: `word`, escaped, with
/// `\b` anchoring each edge independently rather than the run as a whole.
///
/// `word` is always an already-resolved word/punct run (`inner_word_impl`'s
/// result); that run's own first and last grapheme *cluster* decide the two
/// edges.
/// An edge is anchored only when *both* notions of "word character" agree:
/// `chars` (this buffer's `word-chars`-aware `hume_editing::word::WordChars`,
/// the rule that decided the run) and [`regex_syntax::try_is_word_character`]
/// (the exact Unicode `\w` class rust-regex's own `\b` is defined against:
/// `\p{Alphabetic} + \p{M} + \d + \p{Pc} + \p{Join_Control}`). Requiring
/// agreement matters in both directions:
/// - `chars` can be *wider*: e.g. `-` configured as a word char merges
///   "foo-bar" into one run, but rust-regex still sees `-` itself as
///   non-word, so `\bfoo-bar\b` still matches inside "foo-bar-baz" (rust-regex
///   has neither a configurable `\w` class nor lookbehind to express the
///   wider rule). `*` can over-match at an edge like this.
/// - rust-regex's class is *wider* on marks, non-`_` connector punctuation,
///   and join controls, which `chars` (absent that char in `word-chars`)
///   classifies as `Punctuation`, e.g. U+FF3F. Anchoring on rust-regex's
///   answer alone there would emit `\b＿\b`, which can never match: the
///   neighbouring characters are word characters to rust-regex too, so
///   neither boundary can hold. Requiring `chars` to agree drops the anchor
///   on that edge instead, so `*` never under-matches.
pub fn word_search_pattern(word: &str, chars: WordChars<'_>) -> String {
    let escaped = escape_regex(word);
    // Each edge is judged by its own cluster's *base* char: the first
    // codepoint of that cluster, never a trailing combining mark.
    let anchorable_at = |s: &str| {
        s.chars().next().is_some_and(|c| {
            chars.classify(c) == CharClass::Word
                && regex_syntax::try_is_word_character(c).unwrap_or(false)
        })
    };
    let lead = anchorable_at(word);
    // The trailing edge needs the cluster boundary; the leading one doesn't
    // (`s.chars().next()` is by definition the base char of `s`'s first
    // cluster). `word.chars().next_back()` would hand `classify` the
    // combining mark of an NFD "café" (U+0301, `Punctuation` to HUME),
    // silently dropping the `\b` so `*` also matches inside "cafétéria".
    // Anchoring *after* a mark is right: rust-regex's `\w` includes `\p{M}`,
    // so the boundary holds against whatever letter follows.
    let trail = anchorable_at(&word[prev_str_boundary(word, word.len())..]);
    format!(
        "{}{escaped}{}",
        if lead { r"\b" } else { "" },
        if trail { r"\b" } else { "" },
    )
}

/// Return `(current_1based, total)` for a pre-computed match list.
///
/// `total` is the number of matches in `matches`.
/// `current_1based` is the 1-based index of the match whose range contains
/// `head`, or `0` when the cursor is not on any match (e.g. during
/// live search before a hit is found).
///
/// `matches` must be in document order (sorted by start position, non-overlapping),
/// as produced by [`find_all_matches`].
pub fn search_match_info(matches: &[ClusterRange], head: ClusterStart) -> (usize, usize) {
    let total = matches.len();
    // partition_point gives the first index where start > head, so idx-1 is
    // the last match that could contain head. If that match also contains
    // head, the cursor is on it.
    let idx = matches.partition_point(|span| span.start() <= head);
    let current = idx
        .checked_sub(1)
        .filter(|&i| matches[i].contains(head))
        .map(|i| i + 1) // convert to 1-based
        .unwrap_or(0);
    (current, total)
}

/// Find the next match relative to `from` by binary-searching a
/// pre-computed, sorted match list rather than re-scanning the buffer.
///
/// This is O(log M) where M is the number of matches, vs O(buffer_size) for
/// the regex-scan path ([`find_next_match`]). [`MatchScan`] uses this
/// whenever its `cached` list is warm (both live search and `n`/`N` warm it
/// before scanning), falling back to [`find_next_match`] only when it isn't
/// (a buffer whose match cache has never been built).
///
/// A cache-derived match is the leftmost non-overlapping one from offset 0,
/// which can differ from a scan restarted mid-buffer for a self-overlapping
/// pattern (`aa` over `aaa`). This is what makes live preview agree with
/// where `n` lands right after confirm, rather than a discrepancy.
///
/// # Direction
///
/// - **Forward**: first match whose `start ≥ from`. Wraps to `matches[0]`
///   if none is found at or after `from`.
/// - **Backward**: last match whose `start < from`. Wraps to
///   `matches.last()` if none is found before `from`.
///
/// Returns `None` only when `matches` is empty.
/// Returns `Some((span, wrapped))` otherwise.
pub fn find_match_from_cache(
    matches: &[ClusterRange],
    from: ClusterBound,
    direction: SearchDirection,
) -> Option<(ClusterRange, bool)> {
    if matches.is_empty() {
        return None;
    }
    let before_from = |span: &ClusterRange| ClusterBound::from(span.start()) < from;
    match direction {
        SearchDirection::Forward => {
            // First match with start >= from.
            let idx = matches.partition_point(before_from);
            if let Some(&span) = matches.get(idx) {
                Some((span, false))
            } else {
                // Wrap: take the very first match in the buffer.
                Some((matches[0], true)) // non-empty guard above
            }
        }
        SearchDirection::Backward => {
            // Last match with start < from.
            let idx = matches.partition_point(before_from);
            if let Some(&span) = idx.checked_sub(1).and_then(|i| matches.get(i)) {
                Some((span, false))
            } else {
                // Wrap: take the very last match in the buffer.
                Some((matches[matches.len() - 1], true)) // non-empty guard above
            }
        }
    }
}

// ── MatchScan ──────────────────────────────────────────────────────────────────

/// Where a per-selection scan starts: at the selection itself (live preview:
/// a selection already sitting on a match should stay put) or past it (`n`/`N`,
/// which must not re-find the match a selection is already on).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MatchSeed {
    AtSelection,
    PastSelection,
}

/// The inputs a "move each selection to its next match" pass needs, bundled
/// once so [`advance`](Self::advance)/[`advance_all`](Self::advance_all) take
/// one argument instead of five. Live search (`m`-flag preview, every
/// keystroke) and `n`/`N` (`m`-flag repeat) are the same operation over
/// different seed/cache/count inputs, not two separate ones. This is their
/// single implementation.
pub struct MatchScan<'a> {
    pub text: &'a BufferText,
    pub regex: &'a Regex,
    /// `Some(matches)` binary-searches that pre-computed, sorted list
    /// ([`find_match_from_cache`], O(log M)); an empty slice means "cache
    /// warm, zero matches", not "cache cold". `None` scans `regex` directly
    /// ([`find_next_match`], O(buffer)): the cold-cache fallback.
    pub cached: Option<&'a [ClusterRange]>,
    pub direction: SearchDirection,
    pub mode: MotionMode,
    pub seed: MatchSeed,
}

impl MatchScan<'_> {
    /// Advance `sel` by `count` matches. A miss at any step fails the whole
    /// hop atomically (`None`) rather than leaving `sel` part-advanced,
    /// matching a count prefix's usual all-or-nothing semantics elsewhere in
    /// the editor. `wrapped` is true iff the last hop in the chain wrapped
    /// the buffer boundary.
    pub fn advance(&self, sel: SelectionView<'_>, count: usize) -> Option<(Selection, bool)> {
        let mut from: ClusterBound = match (self.seed, self.direction) {
            (MatchSeed::AtSelection, SearchDirection::Forward) => sel.start().into(),
            (MatchSeed::AtSelection, SearchDirection::Backward) => sel.last().into(),
            // Step past the current match so we don't re-find it.
            (MatchSeed::PastSelection, SearchDirection::Forward) => sel.covered().end(),
            (MatchSeed::PastSelection, SearchDirection::Backward) => sel.start().into(),
        };

        let mut last_match = None;
        let mut any_wrapped = false;
        for _ in 0..count {
            let (span, wrapped) = match self.cached {
                Some(matches) => find_match_from_cache(matches, from, self.direction),
                None => find_next_match(self.text, self.regex, from, self.direction),
            }?;
            any_wrapped |= wrapped;
            last_match = Some(span);
            from = match self.direction {
                SearchDirection::Forward => span.end(),
                SearchDirection::Backward => span.start().into(),
            };
        }

        let span = last_match?;
        let new_sel = match self.mode {
            // Keep the anchor, move the head to the match edge that faces
            // the search direction.
            MotionMode::Extend => sel.selection().with_head(match self.direction {
                SearchDirection::Forward => span.last(),
                SearchDirection::Backward => span.start(),
            }),
            MotionMode::Move => Selection::covering(span, Facing::Forward),
        };
        Some((new_sel, any_wrapped))
    }

    /// Advance every selection in `state` independently, merging any that
    /// converge on the same match (the selection set's own merge). A
    /// selection with no match of its own keeps its prior position.
    /// `None` when nothing matched at all; the returned `bool` is whether the
    /// *primary* selection's own hop wrapped (`false` when the primary itself
    /// had no match).
    pub fn advance_all(&self, state: EditState, count: usize) -> Option<(EditState, bool)> {
        let mut any_matched = false;
        let mut primary_wrapped = false;
        let new_state = state.map(|sel| match self.advance(sel, count) {
            Some((new_sel, wrapped)) => {
                any_matched = true;
                if sel.is_primary() {
                    primary_wrapped = wrapped;
                }
                new_sel
            }
            None => sel.selection(),
        });
        any_matched.then_some((new_state, primary_wrapped))
    }
}

// ── Helpers ───────────────────────────────────────────────────────────────────

/// Find a non-zero-width match in `byte_range`, or `None`.
///
/// `take_last`: `false` takes the first match found (forward search);
/// `true` scans every match in the range and takes the last one,
/// implemented by collecting all matches, which is correct and simple,
/// acceptable for typical buffer sizes. A reverse-DFA approach could be
/// added later for very large files.
fn search_match_in(
    text: &BufferText,
    regex: &Regex,
    byte_range: std::ops::Range<usize>,
    take_last: bool,
) -> Option<ClusterRange> {
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
    match_range(text, m.start()..m.end())
}

#[cfg(test)]
mod tests;
