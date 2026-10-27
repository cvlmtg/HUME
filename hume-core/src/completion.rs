//! Completion vocabulary shared by the scripting builtin that registers a
//! source and the editor that matches its items.

/// How a source's items are matched against its token's typed text. Declared
/// per source, since one session mixes sources.
///
/// - `Fuzzy`: nucleo scoring. The candidate universe is stable (or replaced
///   by an LSP `isIncomplete` re-invocation), so the session re-scores
///   locally each keystroke.
/// - `String { case_sensitive }`: a prefix gate (`starts_with`, or
///   `eq_ignore_ascii_case` on the head when not case-sensitive) with a tied
///   score of `0`. Nucleo scores any non-empty match above `0`, so once the
///   user types, `Fuzzy` items (e.g. from an LSP server) outrank `String` ones;
///   `#:priority` only breaks ties on the empty pattern.
/// - `Delegated`: the source returned a finished, ordered result. Scores are
///   tied and the sortText tiebreak is skipped, so the source's order is kept.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum MatchKind {
    Fuzzy,
    String { case_sensitive: bool },
    Delegated,
}
