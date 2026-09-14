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
use hume_editing::word::{CharClass, WordChars};
use hume_rope::grapheme::prev_str_boundary;
use hume_rope::offset::{CharOffset, InclusiveRange};

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
/// `Some((span, wrapped))` on success, where:
/// - `span` is the inclusive char range of the match (HUME's inclusive
///   selection model — `anchor == head` is a 1-char selection)
/// - `wrapped` is `true` when the match was found after wrapping around the
///   buffer boundary
///
/// Returns `None` when no match exists anywhere in the buffer, or when the
/// match is zero-width (which would cause the cursor to appear stuck).
pub fn find_next_match(
    text: &BufferText,
    regex: &Regex,
    from_char: CharOffset,
    direction: SearchDirection,
) -> Option<(InclusiveRange<CharOffset>, bool)> {
    let from_byte = text.char_to_byte(from_char);
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

// ── find_all_matches ──────────────────────────────────────────────────────────

/// Return all non-overlapping regex matches in `text` as inclusive char
/// ranges, in document order. Zero-width matches are skipped.
///
/// Used by `SearchMatchHighlighter` to convert matches to line-relative byte
/// ranges for the engine's highlight provider system.
pub fn find_all_matches(text: &BufferText, regex: &Regex) -> Vec<InclusiveRange<CharOffset>> {
    find_matches_in_range(
        text,
        regex,
        InclusiveRange::new(CharOffset::new(0), text.last_char()),
    )
}

// ── find_matches_in_range ─────────────────────────────────────────────────────

/// Return all non-overlapping regex matches within a char range of `text`.
///
/// Only matches that fall entirely within `range` are returned, as inclusive
/// char ranges in document order. Zero-width matches are skipped.
pub fn find_matches_in_range(
    text: &BufferText,
    regex: &Regex,
    range: InclusiveRange<CharOffset>,
) -> Vec<InclusiveRange<CharOffset>> {
    let start_byte = text.char_to_byte(range.start);
    // range.end is inclusive — we need the byte after the last char in range.
    let end_byte = text.char_to_byte(range.end.shift(1));

    let cursor = RopeyCursor::new(text.full_slice());
    let mut input = Input::new(cursor);
    input.set_range(start_byte..end_byte);

    regex
        .find_iter(input)
        .filter(|m| m.start() < m.end()) // skip zero-width matches
        .map(|m| {
            let s = text.byte_to_char(m.start());
            let e = text.byte_to_char(m.end()).shift(-1);
            InclusiveRange::new(s, e)
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

/// Build a `*` (search-word-under-cursor) pattern: `word`, escaped, with
/// `\b` anchoring each edge independently rather than the run as a whole.
///
/// `word` is always an already-resolved word/punct run (`inner_word_impl`'s
/// result) — that run's own first and last grapheme *cluster* decide the two
/// edges.
/// An edge is anchored only when *both* notions of "word character" agree:
/// `chars` (this buffer's `word-chars`-aware `hume_editing::word::WordChars`,
/// the rule that decided the run) and [`regex_syntax::try_is_word_character`]
/// (the exact Unicode `\w` class rust-regex's own `\b` is defined against —
/// `\p{Alphabetic} + \p{M} + \d + \p{Pc} + \p{Join_Control}`). Requiring
/// agreement matters in both directions:
/// - `chars` can be *wider* — e.g. `-` configured as a word char merges
///   "foo-bar" into one run, but rust-regex still sees `-` itself as
///   non-word, so `\bfoo-bar\b` still matches inside "foo-bar-baz" (rust-regex
///   has neither a configurable `\w` class nor lookbehind to express the
///   wider rule). `*` can over-match at an edge like this.
/// - rust-regex's class is *wider* on marks, non-`_` connector punctuation,
///   and join controls, which `chars` (absent that char in `word-chars`)
///   classifies as `Punctuation` — e.g. U+FF3F. Anchoring on rust-regex's
///   answer alone there would emit `\b＿\b`, which can never match: the
///   neighbouring characters are word characters to rust-regex too, so
///   neither boundary can hold. Requiring `chars` to agree drops the anchor
///   on that edge instead, so `*` never under-matches.
pub fn word_search_pattern(word: &str, chars: WordChars<'_>) -> String {
    let escaped = escape_regex(word);
    // Each edge is judged by its own cluster's *base* char — the first
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
    // combining mark of an NFD "café" (U+0301 — `Punctuation` to HUME),
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
pub fn search_match_info(
    matches: &[InclusiveRange<CharOffset>],
    cursor_head: CharOffset,
) -> (usize, usize) {
    let total = matches.len();
    // partition_point gives the first index where start > cursor_head, so
    // idx-1 is the last match that could contain cursor_head. If cursor_head
    // also falls within its end, the cursor is on that match.
    let idx = matches.partition_point(|span| span.start <= cursor_head);
    let current = idx
        .checked_sub(1)
        .filter(|&i| cursor_head <= matches[i].end)
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
/// Returns `Some((span, wrapped))` otherwise.
pub fn find_match_from_cache(
    matches: &[InclusiveRange<CharOffset>],
    from_char: CharOffset,
    direction: SearchDirection,
) -> Option<(InclusiveRange<CharOffset>, bool)> {
    if matches.is_empty() {
        return None;
    }
    match direction {
        SearchDirection::Forward => {
            // First match with start >= from_char.
            let idx = matches.partition_point(|span| span.start < from_char);
            if let Some(&span) = matches.get(idx) {
                Some((span, false))
            } else {
                // Wrap: take the very first match in the buffer.
                Some((matches[0], true)) // non-empty guard above
            }
        }
        SearchDirection::Backward => {
            // Last match with start < from_char.
            let idx = matches.partition_point(|span| span.start < from_char);
            if let Some(&span) = idx.checked_sub(1).and_then(|i| matches.get(i)) {
                Some((span, false))
            } else {
                // Wrap: take the very last match in the buffer.
                Some((matches[matches.len() - 1], true)) // non-empty guard above
            }
        }
    }
}

// ── Helpers ───────────────────────────────────────────────────────────────────

/// Find a non-zero-width match in `byte_range`, returning its inclusive char
/// range or `None`.
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
) -> Option<InclusiveRange<CharOffset>> {
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
    let end_incl = text.byte_to_char(m.end()).shift(-1);
    Some(InclusiveRange::new(start, end_incl))
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests;
