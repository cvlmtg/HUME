use crate::buffer::Buffer;
use crate::grapheme::{next_grapheme_boundary, prev_grapheme_boundary};

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
