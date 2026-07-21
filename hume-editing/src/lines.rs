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

/// Returns `true` if `line` is an empty line — either zero chars or exactly
/// one newline. Whitespace-only lines are NOT empty (matching Helix semantics).
pub fn is_empty_line(buf: &Text, line: usize) -> bool {
    let start = buf.line_to_char(line);
    let end = line_end_exclusive(buf, line);
    // Zero chars (last line of an empty buffer) or exactly one '\n'.
    end == start || (end == start + 1 && buf.char_at(start) == Some('\n'))
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

/// Place the cursor at `col` chars from the start of `line`, clamping to the
/// last content character and snapping to a grapheme boundary.
///
/// Shared by vertical motions (`move_down_inner`/`move_up_inner`, which land
/// on an adjacent buffer line) and vertical selection copy (which lands on a
/// line an arbitrary number of lines away) — both want "column N of line L,
/// clamped and grapheme-snapped."
pub fn place_column(buf: &Text, line: usize, col: usize) -> usize {
    let line_start = buf.line_to_char(line);
    let end_excl = line_end_exclusive(buf, line);
    let target = line_start + col;

    if target >= end_excl {
        // Column overshoots — clamp to the last content char on the line.
        line_content_end(buf, line)
    } else {
        snap_to_grapheme_boundary(buf, line_start, target)
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests;
