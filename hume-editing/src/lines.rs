//! Line-boundary helpers: computing the extent of a line and snapping
//! char offsets to grapheme cluster boundaries.
//!
//! All positions are **char offsets** into a [`Text`] buffer.

use crate::grapheme::{next_grapheme_boundary, prev_grapheme_boundary};
use crate::selection::Selection;
use crate::text::Text;

/// Returns `true` if the start of `sel` is the first char of its line (or the
/// buffer start).
///
/// Equivalent to "the char before `sel.start()` is a `\n`, or `sel.start()` is
/// 0", but expressed via line arithmetic — no grapheme-stepping needed.
pub fn is_line_start(buf: &Text, sel: &Selection) -> bool {
    let pos = sel.start();
    let line = buf.char_to_line(pos);
    pos == buf.line_to_char(line)
}

/// Exclusive end of `line`: char offset of the first char on the *next* line,
/// or `buf.len_chars()` for the last line.
pub fn line_end_exclusive(buf: &Text, line: usize) -> usize {
    if line + 1 < buf.len_lines() {
        buf.line_to_char(line + 1)
    } else {
        buf.len_chars()
    }
}

/// Char offset of the first non-whitespace char on `line`, or the line's
/// exclusive end if the whole line is whitespace (including empty lines,
/// where that end is `line_start`). Always within `[line_start, line_end]`.
///
/// Single source of truth for "where does leading whitespace end":
/// [`leading_whitespace`] copies the slice up to it, and the editor's
/// dedent-on-Backspace gate consults it so the two agree on the boundary.
///
/// Leading whitespace is always ASCII (`' '`/`'\t'`), and those are single
/// bytes in UTF-8, so a byte-level scan of the rope slice advances char-by-char
/// without needing grapheme iteration.
pub fn leading_whitespace_end(buf: &Text, line: usize) -> usize {
    let line_start = buf.line_to_char(line);
    let end_excl = line_end_exclusive(buf, line);
    let slice = buf.slice(line_start..end_excl);
    // Count leading whitespace bytes. Each is ASCII (single byte == single
    // char), so the byte count is also the char count — no grapheme stepping
    // needed. `n` (not `pos`) is used because this is ASCII byte scanning, not
    // char-position stepping; the raw-stepping lint exempts this form.
    let mut n = 0usize;
    for chunk in slice.chunks() {
        for b in chunk.bytes() {
            if b == b' ' || b == b'\t' {
                n += 1;
            } else {
                return line_start + n;
            }
        }
    }
    line_start + n
}

/// The leading whitespace (spaces and tabs) of `line`, as an owned string.
///
/// Returns `""` when the line has no leading whitespace or is empty. The
/// returned string is suitable for re-insertion (e.g. auto-indent on Enter
/// copies the current line's leading whitespace onto the new line). Built on
/// [`leading_whitespace_end`], the shared definition of where leading
/// whitespace ends.
pub fn leading_whitespace(buf: &Text, line: usize) -> String {
    let line_start = buf.line_to_char(line);
    let end = leading_whitespace_end(buf, line);
    buf.slice(line_start..end).to_string()
}

/// Snap `target` back to the nearest grapheme boundary at or before it,
/// walking forward from `line_start`. Used by vertical motions after computing
/// a char-offset column target, ensuring the cursor always lands on a cluster
/// boundary.
pub fn snap_to_grapheme_boundary(buf: &Text, line_start: usize, target: usize) -> usize {
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
/// line is empty (no other character to sit on).
pub fn line_content_end(buf: &Text, line: usize) -> usize {
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

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::selection::testing::parse_state;

    // ── is_line_start ─────────────────────────────────────────────────────────

    #[test]
    fn is_line_start_buffer_start() {
        // "hello\n" — char 0 is the buffer start, which is a line start.
        let (buf, _) = parse_state("-[h]>ello\n");
        assert!(is_line_start(&buf, &Selection::collapsed(0)));
    }

    #[test]
    fn is_line_start_mid_line_is_false() {
        // "hello\n" — char 2 ('l') is not at a line start.
        let (buf, _) = parse_state("-[h]>ello\n");
        assert!(!is_line_start(&buf, &Selection::collapsed(2)));
    }

    #[test]
    fn is_line_start_second_line_start() {
        // "hi\nbye\n" — line 1 starts at char 3 ('b').
        // h=0, i=1, \n=2, b=3, y=4, e=5, \n=6
        let (buf, _) = parse_state("-[h]>i\nbye\n");
        assert!(is_line_start(&buf, &Selection::collapsed(3)));
        // Verify a non-boundary on line 1 is false (independent oracle: char 4 = 'y').
        assert!(!is_line_start(&buf, &Selection::collapsed(4)));
    }

    #[test]
    fn is_line_start_newline_itself_is_not_line_start() {
        // "hi\n" — the '\n' is at char 2, which is NOT the start of its line
        // (line 0 starts at char 0). This test verifies the function uses line
        // arithmetic rather than just checking the previous char.
        let (buf, _) = parse_state("-[h]>i\n");
        assert!(!is_line_start(&buf, &Selection::collapsed(2))); // '\n' at end of line 0
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

    // ── leading_whitespace ───────────────────────────────────────────────────

    #[test]
    fn leading_whitespace_empty_line() {
        // "a\n\nb\n" — line 1 is empty.
        let (buf, _) = parse_state("-[a]>\n\nb\n");
        assert_eq!(leading_whitespace(&buf, 1), "");
    }

    #[test]
    fn leading_whitespace_none() {
        // "foo\n" — no leading whitespace.
        let (buf, _) = parse_state("-[f]>oo\n");
        assert_eq!(leading_whitespace(&buf, 0), "");
    }

    #[test]
    fn leading_whitespace_spaces() {
        // "    bar\n" — 4 spaces.
        let (buf, _) = parse_state("    -[b]>ar\n");
        assert_eq!(leading_whitespace(&buf, 0), "    ");
    }

    #[test]
    fn leading_whitespace_tabs() {
        // "\t\tfoo\n" — 2 tabs.
        let (buf, _) = parse_state("\t\t-[f]>oo\n");
        assert_eq!(leading_whitespace(&buf, 0), "\t\t");
    }

    #[test]
    fn leading_whitespace_mixed() {
        // "\t  x\n" — tab + 2 spaces.
        let (buf, _) = parse_state("\t  -[x]>\n");
        assert_eq!(leading_whitespace(&buf, 0), "\t  ");
    }

    #[test]
    fn leading_whitespace_only_whitespace_line() {
        // "   \n" — whole line is whitespace (3 spaces + structural \n).
        let (buf, _) = parse_state("-[ ]>  \n");
        assert_eq!(leading_whitespace(&buf, 0), "   ");
    }

    #[test]
    fn leading_whitespace_second_line() {
        // "a\n  b\n" — line 1 has 2-space indent.
        let (buf, _) = parse_state("-[a]>\n  b\n");
        assert_eq!(leading_whitespace(&buf, 1), "  ");
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
