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
pub fn classify_char(ch: char) -> CharClass {
    if ch == '\n' {
        CharClass::Eol
    } else if ch == ' ' || ch == '\t' {
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
/// `is_long_word_boundary` returns `true` only at whitespace-or-newline ↔
/// non-whitespace-non-newline transitions.
pub fn is_long_word_boundary(a: CharClass, b: CharClass) -> bool {
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
mod tests {
    use super::*;

    // ── classify_char ────────────────────────────────────────────────────────

    #[test]
    fn classify_newline_is_eol() {
        assert_eq!(classify_char('\n'), CharClass::Eol);
    }

    #[test]
    fn classify_space_and_tab_are_space() {
        assert_eq!(classify_char(' '), CharClass::Space);
        assert_eq!(classify_char('\t'), CharClass::Space);
    }

    #[test]
    fn classify_ascii_alnum_and_underscore_are_word() {
        assert_eq!(classify_char('a'), CharClass::Word);
        assert_eq!(classify_char('Z'), CharClass::Word);
        assert_eq!(classify_char('5'), CharClass::Word);
        assert_eq!(classify_char('_'), CharClass::Word);
    }

    #[test]
    fn classify_unicode_letters_are_word() {
        // Accented letter (Latin Extended) — alphanumeric in Unicode
        assert_eq!(classify_char('é'), CharClass::Word);
        // CJK ideograph — alphanumeric in Unicode
        assert_eq!(classify_char('文'), CharClass::Word);
    }

    #[test]
    fn classify_punctuation_and_symbols() {
        for ch in ['.', ',', '!', '(', ')', '-', '+', '@', '#'] {
            assert_eq!(
                classify_char(ch),
                CharClass::Punctuation,
                "expected Punctuation for {:?}",
                ch
            );
        }
    }

    // ── is_word_boundary ─────────────────────────────────────────────────────

    #[test]
    fn word_boundary_any_class_change() {
        use CharClass::*;
        // Same class → no boundary
        assert!(!is_word_boundary(Word, Word));
        assert!(!is_word_boundary(Space, Space));
        assert!(!is_word_boundary(Punctuation, Punctuation));
        assert!(!is_word_boundary(Eol, Eol));
        // Different class → boundary
        assert!(is_word_boundary(Word, Punctuation));
        assert!(is_word_boundary(Word, Space));
        assert!(is_word_boundary(Word, Eol));
        assert!(is_word_boundary(Punctuation, Space));
        assert!(is_word_boundary(Space, Eol));
    }

    // ── is_long_word_boundary ─────────────────────────────────────────────────

    #[test]
    fn long_word_boundary_merges_word_and_punctuation() {
        use CharClass::*;
        // Word ↔ Punctuation are merged — no long-word boundary between them
        assert!(!is_long_word_boundary(Word, Punctuation));
        assert!(!is_long_word_boundary(Punctuation, Word));
        // Same class → no boundary
        assert!(!is_long_word_boundary(Word, Word));
        assert!(!is_long_word_boundary(Space, Space));
        // Space/Eol transitions are still boundaries
        assert!(is_long_word_boundary(Word, Space));
        assert!(is_long_word_boundary(Punctuation, Space));
        assert!(is_long_word_boundary(Word, Eol));
        assert!(is_long_word_boundary(Space, Eol));
    }
}
