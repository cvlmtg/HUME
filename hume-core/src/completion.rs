//! Completion vocabulary shared by the scripts that register and answer
//! sources and the editor's completion sessions.

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

/// Whether an answer is a source's whole answer to its invocation, or one
/// part of it with more to follow (`#:more`). Each part replaces the one
/// before it, so a part holds every item so far. A `Final` answer closes
/// the request: a later answer to it is dropped.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub enum AnswerEnd {
    #[default]
    Final,
    /// More follows: the menu shows this part and its session stays
    /// pending, so what the minibuffer does once a source has answered,
    /// and an Insert-mode report of nothing found, wait for the final part.
    More,
}

/// What the first Tab puts over a minibuffer word while no row is picked.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub enum FirstTab {
    /// What the insert texts all start with, when that extends the word.
    #[default]
    Common,
    /// This text, from a source whose insert texts have a syntax the editor
    /// cannot cut (a quoted shell word).
    Named(NamedPrefix),
    /// Nothing: the first Tab only opens the menu.
    Nothing,
}

/// The text of a [`FirstTab::Named`]: never empty, since an empty one
/// would erase the word it is put over.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NamedPrefix(String);

impl NamedPrefix {
    /// `None` for `""`.
    pub fn new(text: String) -> Option<Self> {
        (!text.is_empty()).then_some(Self(text))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}
