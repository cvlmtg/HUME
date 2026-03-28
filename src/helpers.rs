use crate::core::buffer::Buffer;
use crate::core::grapheme::{next_grapheme_boundary, prev_grapheme_boundary};

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::parse_state;

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

    // ── is_WORD_boundary ─────────────────────────────────────────────────────

    #[test]
    #[allow(non_snake_case)]
    fn WORD_boundary_merges_word_and_punctuation() {
        use CharClass::*;
        // Word ↔ Punctuation are merged — no WORD boundary between them
        assert!(!is_WORD_boundary(Word, Punctuation));
        assert!(!is_WORD_boundary(Punctuation, Word));
        // Same class → no boundary
        assert!(!is_WORD_boundary(Word, Word));
        assert!(!is_WORD_boundary(Space, Space));
        // Space/Eol transitions are still boundaries
        assert!(is_WORD_boundary(Word, Space));
        assert!(is_WORD_boundary(Punctuation, Space));
        assert!(is_WORD_boundary(Word, Eol));
        assert!(is_WORD_boundary(Space, Eol));
    }

    // ── line_end_exclusive ────────────────────────────────────────────────────

    #[test]
    fn line_end_exclusive_first_line_of_two() {
        // "hello\nworld\n" — line 0 ends exclusive at char 6 (start of "world")
        let (buf, _) = parse_state("-[h]>ello\nworld\n");
        assert_eq!(line_end_exclusive(&buf, 0), 6); // 'h','e','l','l','o','\n' = 6 chars
    }

    #[test]
    fn line_end_exclusive_last_line() {
        // Last line — returns buf.len_chars()
        let (buf, _) = parse_state("-[h]>ello\n");
        // single line: len = 6, line_end_exclusive(0) == len_chars() == 6
        assert_eq!(line_end_exclusive(&buf, 0), buf.len_chars());
    }

    #[test]
    fn line_end_exclusive_empty_line_between() {
        // "a\n\nb\n" — line 1 is empty ("\n"), its exclusive end is char 3
        let (buf, _) = parse_state("-[a]>\n\nb\n");
        // line 0: 'a','\n' = 2 chars → line_end_exclusive(0) = 2
        // line 1: '\n'     = 1 char  → line_end_exclusive(1) = 3
        assert_eq!(line_end_exclusive(&buf, 1), 3);
    }

    // ── line_content_end ──────────────────────────────────────────────────────

    #[test]
    fn line_content_end_normal_line() {
        // "hello\nworld\n" — line 0: last non-newline char is 'o' at offset 4
        let (buf, _) = parse_state("-[h]>ello\nworld\n");
        assert_eq!(line_content_end(&buf, 0), 4);
    }

    #[test]
    fn line_content_end_empty_line_returns_newline_pos() {
        // "hello\n\nworld\n" — line 1 is empty; cursor sits on the '\n'
        let (buf, _) = parse_state("-[h]>ello\n\nworld\n");
        // line 1 starts at char 6, its only char is '\n' → content_end = 6
        assert_eq!(line_content_end(&buf, 1), 6);
    }

    #[test]
    fn line_content_end_single_char_line() {
        // "a\nb\n" — line 0 content end is at 'a' (offset 0)
        let (buf, _) = parse_state("-[a]>\nb\n");
        assert_eq!(line_content_end(&buf, 0), 0);
    }

    #[test]
    fn line_content_end_combining_grapheme_before_newline() {
        // "cafe\u{0301}\n" = c(0) a(1) f(2) e(3) combining_acute(4) \n(5)
        // The grapheme "e\u{0301}" starts at char 3. line_content_end must
        // return 3 (the grapheme cluster start), not 4 (mid-cluster).
        let (buf, _) = parse_state("-[c]>afe\u{0301}\n");
        assert_eq!(line_content_end(&buf, 0), 3);
    }

    // ── snap_to_grapheme_boundary ─────────────────────────────────────────────

    #[test]
    fn snap_to_grapheme_boundary_ascii_lands_exactly() {
        let (buf, _) = parse_state("-[h]>ello\n");
        // Target 3 in ASCII — all single-char graphemes, so snap returns 3
        assert_eq!(snap_to_grapheme_boundary(&buf, 0, 3), 3);
    }

    #[test]
    fn snap_to_grapheme_boundary_target_at_line_start() {
        let (buf, _) = parse_state("-[h]>ello\n");
        assert_eq!(snap_to_grapheme_boundary(&buf, 0, 0), 0);
    }

    #[test]
    fn snap_to_grapheme_boundary_target_beyond_line_returns_len_chars() {
        // snap walks forward until `next > target || next == pos`. When target
        // is past all graphemes, the loop walks all the way to len_chars (where
        // next_grapheme_boundary clamps and returns the same position, triggering
        // the `next == pos` stop). The result is len_chars, not the last char.
        // Callers (vertical motion) apply their own clamping to len_chars - 1.
        let (buf, _) = parse_state("-[h]>i\n");
        // "hi\n": h=0, i=1, \n=2; len_chars=3
        assert_eq!(snap_to_grapheme_boundary(&buf, 0, 100), buf.len_chars());
    }

    #[test]
    fn snap_to_grapheme_boundary_mid_cluster_snaps_back() {
        // "e\u{0301}\n" — 'e' + combining acute = one grapheme cluster (2 chars).
        // snap with target=1 (inside the cluster) should return 0 (start of cluster).
        let (buf, _) = parse_state("-[e]>\u{0301}\n");
        // The combining char is at char index 1. target=1 is inside the cluster.
        assert_eq!(snap_to_grapheme_boundary(&buf, 0, 1), 0);
    }
}

// ── Line helpers ───────────────────────────────────────────────────────────────

/// Exclusive end of `line`: char offset of the first char on the *next* line,
/// or `buf.len_chars()` for the last line.
pub(crate) fn line_end_exclusive(buf: &Buffer, line: usize) -> usize {
    if line + 1 < buf.len_lines() {
        buf.line_to_char(line + 1)
    } else {
        buf.len_chars()
    }
}

/// Snap `target` back to the nearest grapheme boundary at or before it,
/// walking forward from `line_start`. Used by vertical motions after computing
/// a char-offset column target, ensuring the cursor always lands on a cluster
/// boundary.
pub(crate) fn snap_to_grapheme_boundary(buf: &Buffer, line_start: usize, target: usize) -> usize {
    let mut pos = line_start;
    loop {
        let next = next_grapheme_boundary(buf, pos);
        // `next == pos` when at EOF (the function clamps to len_chars).
        if next > target || next == pos {
            return pos;
        }
        pos = next;
    }
}

/// The last char offset a cursor can land on for `line`.
///
/// Returns the last non-`\n` char on the line, or the `\n` itself when the
/// line is empty (no other character to sit on). This is the single
/// authoritative implementation — shared by `goto_line_end` in `motion.rs`
/// and the multi-line expand/shrink commands in `selection_cmd.rs`.
pub(crate) fn line_content_end(buf: &Buffer, line: usize) -> usize {
    let line_start = buf.line_to_char(line);
    let end_excl = line_end_exclusive(buf, line);

    if end_excl == line_start {
        return line_start; // empty buffer (no content at all)
    }

    let last = end_excl - 1;
    if buf.char_at(last) == Some('\n') {
        if last == line_start {
            line_start // empty line — cursor on the `\n`
        } else {
            prev_grapheme_boundary(buf, last) // step back past the `\n`
        }
    } else {
        prev_grapheme_boundary(buf, end_excl) // last line with no trailing newline
    }
}

// ── Word boundary helpers ──────────────────────────────────────────────────────

/// Broad category of a character for word-boundary detection.
///
/// `Eol` is distinct from `Space` so that `w` can stop at newlines (matching
/// Helix), rather than treating `\n` as ordinary whitespace to skip over.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CharClass {
    Word,        // alphanumeric + underscore
    Punctuation, // other non-whitespace, non-newline
    Space,       // space, tab
    Eol,         // newline
}

pub(crate) fn classify_char(ch: char) -> CharClass {
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

/// Any category change is a word boundary.
pub(crate) fn is_word_boundary(a: CharClass, b: CharClass) -> bool {
    a != b
}

/// Word and Punctuation are treated as the same "long word" class — only
/// transitions involving Space or Eol count.
#[allow(non_snake_case)]
pub(crate) fn is_WORD_boundary(a: CharClass, b: CharClass) -> bool {
    let merge = |c: CharClass| {
        if c == CharClass::Punctuation { CharClass::Word } else { c }
    };
    merge(a) != merge(b)
}
