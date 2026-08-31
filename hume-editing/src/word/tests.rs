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
fn classify_invisible_unicode_spaces_are_space() {
    assert_eq!(classify_char('\u{A0}'), CharClass::Space); // NBSP
    assert_eq!(classify_char('\u{3000}'), CharClass::Space); // ideographic space
}

#[test]
fn classify_other_unicode_whitespace_stays_punctuation() {
    // Deliberate: form feed and bare \r are rare enough that `w` stopping
    // on them beats silently skipping them.
    assert_eq!(classify_char('\u{C}'), CharClass::Punctuation); // form feed
    assert_eq!(classify_char('\r'), CharClass::Punctuation);
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

// ── is_uppercase_word_boundary ─────────────────────────────────────────────────

#[test]
fn uppercase_word_boundary_merges_word_and_punctuation() {
    use CharClass::*;
    // Word ↔ Punctuation are merged — no long-word boundary between them
    assert!(!is_uppercase_word_boundary(Word, Punctuation));
    assert!(!is_uppercase_word_boundary(Punctuation, Word));
    // Same class → no boundary
    assert!(!is_uppercase_word_boundary(Word, Word));
    assert!(!is_uppercase_word_boundary(Space, Space));
    // Space/Eol transitions are still boundaries
    assert!(is_uppercase_word_boundary(Word, Space));
    assert!(is_uppercase_word_boundary(Punctuation, Space));
    assert!(is_uppercase_word_boundary(Word, Eol));
    assert!(is_uppercase_word_boundary(Space, Eol));
}

// ── WordChars ────────────────────────────────────────────────────────────

#[test]
fn word_chars_promotes_listed_punctuation() {
    assert_eq!(WordChars::new("-").classify('-'), CharClass::Word);
}

#[test]
fn word_chars_leaves_unlisted_punctuation_alone() {
    assert_eq!(WordChars::new("-").classify('.'), CharClass::Punctuation);
}

#[test]
fn word_chars_empty_matches_classify_char() {
    let chars = WordChars::default();
    for ch in [
        'a', 'Z', '5', '_', 'é', '文', '.', ',', '-', '+', ' ', '\t', '\u{A0}', '\u{3000}', '\n',
        '\r', '\u{C}',
    ] {
        assert_eq!(chars.classify(ch), classify_char(ch), "mismatch for {ch:?}");
    }
}

#[test]
fn word_chars_cannot_promote_space_or_eol() {
    // Defense in depth: even if a caller bypasses `validate`, `classify` must
    // never turn whitespace/newline into `Word` — every scan here finds a
    // word's end by hitting one of those two classes.
    assert_eq!(WordChars::new(" ").classify(' '), CharClass::Space);
    assert_eq!(WordChars::new("\n").classify('\n'), CharClass::Eol);
}

#[test]
fn validate_rejects_space_and_eol_chars() {
    for ch in [' ', '\t', '\u{A0}', '\u{3000}', '\n'] {
        assert!(
            WordChars::validate(&ch.to_string()).is_err(),
            "expected {ch:?} to be rejected"
        );
    }
}

/// `validate` must reject every Unicode whitespace character, not just the
/// five `classify_char` calls `Space`/`Eol`. `\r`/U+2028/U+2029 are ropey
/// line breaks — configuring one as a word char would make `char_to_line`
/// disagree with the word runs `miw`/`mm` compute.
#[test]
fn validate_rejects_every_unicode_whitespace_char() {
    for ch in [
        '\r', '\u{B}', '\u{C}', '\u{85}', '\u{2028}', '\u{2029}', '\u{202F}',
    ] {
        assert!(
            WordChars::validate(&ch.to_string()).is_err(),
            "expected {ch:?} to be rejected"
        );
    }
}

#[test]
fn validate_accepts_redundant_word_char() {
    assert!(WordChars::validate("_a").is_ok());
}

#[test]
fn validate_accepts_punctuation() {
    assert!(WordChars::validate("-*?!<>/").is_ok());
}
