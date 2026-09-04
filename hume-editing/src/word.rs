//! Word-boundary classification: character categories and boundary predicates
//! for `w`/`W`-style motion and text-object logic.

/// Broad category of a character for word-boundary detection.
///
/// `Eol` is distinct from `Space` so that `w` can stop at newlines (matching
/// Helix), rather than treating `\n` as ordinary whitespace to skip over.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CharClass {
    Word,        // alphanumeric + underscore
    Punctuation, // other non-whitespace, non-newline
    Space,       // space, tab
    Eol,         // newline
}

/// Classify a single character into a [`CharClass`].
///
/// Space covers ASCII blanks plus the two invisible Unicode spaces commonly
/// found in real text: NBSP (U+00A0) and ideographic space (U+3000). Other
/// Unicode whitespace (form feed, bare `\r`, …) stays `Punctuation` — rare
/// enough that stopping on it is more useful than skipping it.
///
/// Crate-private: a caller that distinguishes `Word` from `Punctuation`
/// (word motions, text objects, `*`) must go through [`WordChars::classify`]
/// instead, so a buffer's configured extra word characters are honored — and
/// a caller that only asks whether a char is blank must go through
/// [`blank_class`] instead. Keeping this function itself unreachable from
/// outside `hume-editing` makes bypassing either funnel a compile error
/// rather than a doc paragraph to remember.
pub(crate) fn classify_char(ch: char) -> CharClass {
    if ch == '\n' {
        CharClass::Eol
    } else if matches!(ch, ' ' | '\t' | '\u{A0}' | '\u{3000}') {
        CharClass::Space
    } else if ch.is_alphanumeric() || ch == '_' {
        CharClass::Word
    } else {
        CharClass::Punctuation
    }
}

/// `ch`'s blank class — `Some(Space)`, `Some(Eol)`, or `None` for every
/// non-blank character (`Word` or `Punctuation`). The one question about a
/// character a buffer's `word-chars` can never change: [`WordChars::validate`]
/// rejects any value that would promote a blank to `Word`, so this answer is
/// invariant under it, unlike [`WordChars::classify`]'s `Word`/`Punctuation`
/// split. Every caller that only distinguishes blank from non-blank — never
/// `Word` from `Punctuation` — goes through this rather than `classify_char`.
pub fn blank_class(ch: char) -> Option<CharClass> {
    match classify_char(ch) {
        c @ (CharClass::Space | CharClass::Eol) => Some(c),
        CharClass::Word | CharClass::Punctuation => None,
    }
}

/// Any category change is a word boundary (`w` semantics).
pub fn is_word_boundary(a: CharClass, b: CharClass) -> bool {
    a != b
}

/// Word and Punctuation are treated as the same "long word" class — only
/// transitions involving Space or Eol count (`W` semantics).
///
/// The name follows Vim's uppercase-W convention for WORD (long-word) motions:
/// `is_uppercase_word_boundary` returns `true` only at whitespace-or-newline ↔
/// non-whitespace-non-newline transitions.
pub fn is_uppercase_word_boundary(a: CharClass, b: CharClass) -> bool {
    let merge = |c: CharClass| {
        if c == CharClass::Punctuation {
            CharClass::Word
        } else {
            c
        }
    };
    merge(a) != merge(b)
}

/// A buffer's extra word characters (Vim's `iskeyword`, minus the range
/// syntax) — characters that classify as [`CharClass::Word`] on top of the
/// built-in alphanumeric-plus-`_` rule. Borrowed and `Copy`: the owning
/// `String` lives in the settings layer, and this rides inside a bare `fn`
/// pointer's argument list without a clone per call.
#[derive(Debug, Clone, Copy, Default)]
pub struct WordChars<'a>(&'a str);

impl<'a> WordChars<'a> {
    /// Wrap an already-validated `word-chars` value.
    pub fn new(s: &'a str) -> Self {
        Self(s)
    }

    /// Reject a value that would break every scan here: they all find a
    /// word's end by hitting a blank (`Space` or `Eol` in `classify_char`'s
    /// terms), so promoting one to `Word` would leave a word run with no
    /// terminator. Checked with `char::is_whitespace()` rather than
    /// `classify_char` itself — `classify_char` only calls out five specific
    /// blanks (` `, `\t`, NBSP, ideographic space, `\n`) and leaves every
    /// other Unicode whitespace character `Punctuation` on purpose (see its
    /// doc), which is too narrow a check here: a word run has to end
    /// *somewhere*, and promoting an arbitrary Unicode blank to `Word` is how
    /// a run ends up with no terminator at all on a line built from one.
    ///
    /// A char that is already `Word` (`a`, `_`) is accepted as a redundant
    /// no-op rather than an error — "already a word char" depends on
    /// `char::is_alphanumeric`'s Unicode tables, which shift between Rust
    /// releases, so rejecting it would make a config file break on a
    /// toolchain upgrade.
    pub fn validate(s: &str) -> Result<(), String> {
        for ch in s.chars() {
            if ch.is_whitespace() {
                return Err(format!(
                    "word-chars cannot contain whitespace or newline: {ch:?}"
                ));
            }
        }
        Ok(())
    }

    /// `classify_char`, with this buffer's extra word characters folded
    /// in. Only the `Punctuation` arm does a lookup, so the common path
    /// (letters, digits, space, newline) is exactly as fast as
    /// `classify_char` alone.
    #[inline]
    pub fn classify(self, ch: char) -> CharClass {
        match classify_char(ch) {
            CharClass::Punctuation if self.0.contains(ch) => CharClass::Word,
            base => base,
        }
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests;
