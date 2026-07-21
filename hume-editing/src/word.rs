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
pub fn classify_char(ch: char) -> CharClass {
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

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests;
