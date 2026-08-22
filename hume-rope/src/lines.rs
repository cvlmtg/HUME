use ropey::Rope;
use std::ops::Range;

/// True if `rope` satisfies the trailing-newline invariant every HUME
/// buffer upholds by construction: empty, or ending in `'\n'`. The single
/// source of truth for that check — [`content_line_count`] asserts it
/// (a caller violating it is exactly the bug class this crate exists to
/// surface), while callers that must reject a violation at runtime instead
/// of trusting it (constructing a `Text`, applying a `ChangeSet`) check it
/// directly.
pub fn ends_with_newline(rope: &Rope) -> bool {
    let len = rope.len_chars();
    len == 0 || rope.char(len - 1) == '\n'
}

/// Raw ropey line count. The structural trailing `\n` every HUME buffer
/// ends with makes ropey report one line past the buffer's real content —
/// this is that raw count, phantom line included. Valid on any rope;
/// always `>= 1` (ropey defines an empty rope as having one, empty, line).
pub fn ropey_line_count(rope: &Rope) -> usize {
    rope.len_lines()
}

/// Index of the last ropey line — the phantom trailing line, under the
/// trailing-newline invariant. Gutter sizing and LSP wire-position clamps
/// use this: both must stay addressable up to ropey's own last line, not
/// just the last line with real content.
pub fn last_ropey_line(rope: &Rope) -> usize {
    // ropey_line_count() is always >= 1, so this never underflows.
    ropey_line_count(rope) - 1
}

/// `0..ropey_line_count(rope)` — every line index ropey considers valid,
/// phantom line included.
pub fn ropey_lines_range(rope: &Rope) -> Range<usize> {
    0..ropey_line_count(rope)
}

/// Number of content lines: `ropey_line_count()` minus the structural
/// trailing-`\n` line ropey counts past the buffer's real content. The
/// single source of truth for "how many lines does this buffer have" from
/// a caller's point of view — line counts shown to the user, range-checked
/// line indices.
///
/// Assumes the trailing-newline invariant (debug-asserted) — every
/// `hume_editing::Text` upholds it by construction. Callers that instead
/// need the last valid *ropey* line, phantom line included, want
/// [`last_ropey_line`].
pub fn content_line_count(rope: &Rope) -> usize {
    debug_assert!(
        ends_with_newline(rope),
        "content_line_count: rope does not end with '\\n' — trailing-newline invariant violated"
    );
    ropey_line_count(rope).saturating_sub(1)
}

/// Index of the last content line (`content_line_count() - 1`). `0` on an
/// empty buffer (`"\n"`, one content line: the empty first line). Callers
/// clamping a target line to stay within real content use this.
pub fn last_content_line(rope: &Rope) -> usize {
    content_line_count(rope).saturating_sub(1)
}

/// `0..content_line_count(rope)` — every real content line index.
/// `range.contains(&line)` is the canonical "is this a real content line"
/// bounds check.
pub fn content_lines_range(rope: &Rope) -> Range<usize> {
    0..content_line_count(rope)
}

/// The line breaks ropey's `Rope::lines()` (and hence line tokenization)
/// splits on — its default `unicode_lines` feature: LF, CR, CRLF, VT, FF,
/// NEL, LS, PS. `hume_editing::text::Text::from` only collapses `\r\n`
/// pairs, so every other form survives into the rope and can terminate a
/// line token.
pub(crate) const LINE_BREAKS: [char; 7] = [
    '\n', '\r', '\u{0B}', '\u{0C}', '\u{85}', '\u{2028}', '\u{2029}',
];

/// Strips a single trailing line break from a line-tokenization token —
/// never just `'\n'`, since the break set above is wider. A break char
/// always terminates a token, never sits interior to one, so the greedy
/// `trim_end_matches` is exact — including collapsing a two-char `"\r\n"`
/// token in one pass.
pub fn strip_line_break(line: &str) -> &str {
    line.trim_end_matches(LINE_BREAKS)
}

/// Exclusive end of `line`: char offset of the first char on the *next*
/// line, or `rope.len_chars()` for the last line.
pub fn line_end_exclusive(rope: &Rope, line: usize) -> usize {
    if line + 1 < ropey_line_count(rope) {
        rope.line_to_char(line + 1)
    } else {
        rope.len_chars()
    }
}

/// Char offset of the line-break char that terminates `line` — the
/// inclusive counterpart to [`line_end_exclusive`].
///
/// Content domain: `line` must be a real content line. On the phantom
/// trailing line this would return the *previous* line's terminator, so
/// that case is debug-asserted rather than silently mis-answered.
///
/// With a `\r\n` terminator this is the `\n`, not the `\r` — `Text::from`
/// normalizes CRLF, so a HUME buffer never has one.
pub fn line_break_char(rope: &Rope, line: usize) -> usize {
    debug_assert!(
        line < content_line_count(rope),
        "line_break_char: line {line} is not a real content line (buffer has {} content lines)",
        content_line_count(rope)
    );
    line_end_exclusive(rope, line) - 1
}

/// Char offset one past the last content char on `line` — the offset the
/// byte-domain wire helpers in [`crate::position_encoding`] use, expressed
/// in bytes instead of chars.
pub fn line_end_exclusive_byte(rope: &Rope, line: usize) -> usize {
    if line + 1 < ropey_line_count(rope) {
        rope.line_to_byte(line + 1)
    } else {
        rope.len_bytes()
    }
}

/// Char offset of the first non-whitespace char on `line`, or the line's
/// exclusive end if the whole line is whitespace (including empty lines,
/// where that end is `line_start`). Always within `[line_start, line_end]`.
///
/// Single source of truth for "where does leading whitespace end": the
/// editor's auto-indent-on-Enter and dedent-on-Backspace paths both consult
/// it so they agree on the boundary.
///
/// Leading whitespace is always ASCII (`' '`/`'\t'`), and those are single
/// bytes in UTF-8, so a byte-level scan of the rope slice advances char-by-char
/// without needing grapheme iteration.
pub fn leading_whitespace_end(rope: &Rope, line: usize) -> usize {
    let line_start = rope.line_to_char(line);
    let end_excl = line_end_exclusive(rope, line);
    let slice = rope.slice(line_start..end_excl);
    // Count leading whitespace bytes. Each is ASCII (single byte == single
    // char), so the byte count is also the char count — no grapheme stepping
    // needed.
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

/// Snap `target` back to the nearest grapheme boundary at or before it,
/// walking forward from `line_start`. Used by vertical motions after computing
/// a char-offset column target, ensuring the cursor always lands on a cluster
/// boundary.
pub fn snap_to_grapheme_boundary(rope: &Rope, line_start: usize, target: usize) -> usize {
    let mut pos = line_start;
    loop {
        let next = crate::grapheme::next_grapheme_boundary(rope.slice(..), pos);
        // `next == pos` when at EOF (the function clamps to len_chars).
        if next > target || next == pos {
            return pos;
        }
        pos = next;
    }
}

/// Returns `true` if `line` is an empty line — either zero chars or exactly
/// one newline. Whitespace-only lines are NOT empty (matching Helix semantics).
pub fn is_empty_line(rope: &Rope, line: usize) -> bool {
    let start = rope.line_to_char(line);
    let end = line_end_exclusive(rope, line);
    // Zero chars (last line of an empty buffer) or exactly one '\n'.
    end == start || (end == start + 1 && rope.get_char(start) == Some('\n'))
}

/// The last char offset a cursor can land on for `line`.
///
/// Returns the last non-`\n` char on the line, or the `\n` itself when the
/// line is empty (no other character to sit on).
///
/// This is the motion-domain "end of line": where a cursor may land,
/// including a position sitting *on* the `\n`. [`crate::position_encoding`]'s
/// `line_content_end_char` is the wire-domain counterpart — defined by
/// content length instead — and the two disagree by design for an empty
/// line; do not substitute one for the other.
pub fn line_content_end(rope: &Rope, line: usize) -> usize {
    let line_start = rope.line_to_char(line);
    let end_excl = line_end_exclusive(rope, line);

    if end_excl == line_start {
        return line_start; // empty buffer (no content at all)
    }

    let last = end_excl - 1;
    if rope.get_char(last) == Some('\n') {
        if last == line_start {
            line_start // empty line — cursor on the `\n`
        } else {
            crate::grapheme::prev_grapheme_boundary(rope.slice(..), last) // step back past the `\n`
        }
    } else {
        crate::grapheme::prev_grapheme_boundary(rope.slice(..), end_excl) // last line with no trailing newline
    }
}

/// 0-based char column of `char_pos` within `line` — `char_pos` minus the
/// line's start offset. The char-unit sibling of
/// [`crate::grapheme::grapheme_col_in_line`] /
/// [`crate::grapheme::display_col_in_line`]; inverse of `place_char_column`'s
/// `line_start + char_col`.
pub fn char_col_in_line(rope: &Rope, line: usize, char_pos: usize) -> usize {
    let line_start = rope.line_to_char(line);
    debug_assert!(
        char_pos >= line_start,
        "char_col_in_line: char_pos {char_pos} is before line {line}'s start {line_start}"
    );
    char_pos - line_start
}

/// Place the cursor at `char_col` **chars** from the start of `line` (not
/// display columns — every non-tab grapheme counts 1, a tab counts 1),
/// clamping to the last content character and snapping to a grapheme
/// boundary.
///
/// Callers are those with no channel to a per-buffer `tab_width`: vertical
/// selection copy (`copy_selection_vertically`), whose commands are registered
/// directly in `CommandRegistry` as bare `fn` pointers and so can't receive
/// settings the way an `EditorCmd` can; buffer reload, which re-places every
/// cursor against the new text; and `goto-location!`'s char-indexed target
/// shape. See [`place_display_column`] for the display-column-aware sibling
/// vertical motion uses instead, and its doc for why the two aren't unified.
///
/// The clamp compares against the line's *content* end, not
/// [`line_end_exclusive`]: the latter counts the terminating `\n`, which would
/// make a `char_col` of exactly the line's content length land on the newline
/// while any larger one clamped back to the last real character — a
/// non-monotonic result where moving further right moves the cursor left. An
/// empty line still lands on its `\n`, since there `line_content_end` *is*
/// that newline. Mirrors [`place_display_column`]'s boundary rule, in char
/// units.
pub fn place_char_column(rope: &Rope, line: usize, char_col: usize) -> usize {
    let line_start = rope.line_to_char(line);
    let content_end = line_content_end(rope, line);
    let target = line_start + char_col;

    if target >= content_end {
        content_end
    } else {
        snap_to_grapheme_boundary(rope, line_start, target)
    }
}

/// Place the cursor at display column `target_display_col` of `line`,
/// clamping to the last content character (or the line's own `\n` when it's
/// empty) and snapping to a grapheme boundary. Tab-aware and
/// unicode-width-aware — the display-column model `move_down_inner`/
/// `move_up_inner` (`9j`/`9k`) share with `editor::visual_move::move_vertical`'s
/// bare `j`/`k`.
///
/// Doesn't just delegate the overshoot case to
/// [`crate::grapheme::char_pos_at_display_col`]: that function always lands
/// ON the line's trailing `\n` once `target_display_col` reaches or passes
/// the line's width — correct for its own caller (`dedent_tab_backward`,
/// which wants an exact column), but not what vertical motion wants. Moving
/// onto a *shorter* line should stick to the last real character (the
/// vim/helix convention), landing on `\n` only when the line is genuinely
/// empty — so this checks the line's own display width first and only
/// defers to `char_pos_at_display_col` when `target_display_col` lands
/// strictly inside the line, falling back to [`line_content_end`] otherwise.
///
/// The comparison is `>=`, not `>`: a target *equal* to the line's width is
/// already one column past its last character, which is exactly the `\n`'s
/// own column. Letting that case through to `char_pos_at_display_col` would
/// land `9j` on a non-empty line's newline while bare `j` — which resolves
/// through `RowMap`'s `NearestContent`, and so excludes the EOL sentinel —
/// lands on its last real character, splitting the two column models this
/// function exists to unify. An empty line still lands on its `\n`: its width
/// is 0, so `0 >= 0` takes the [`line_content_end`] branch, which for an
/// empty line *is* the newline. Mirrors [`place_char_column`]'s two-tier
/// clamp shape, just in display-column units.
pub fn place_display_column(
    rope: &Rope,
    line: usize,
    target_display_col: usize,
    tab_width: u8,
) -> usize {
    let slice = rope.slice(..);
    let line_width =
        crate::grapheme::display_col_in_line(slice, line, line_break_char(rope, line), tab_width);
    if target_display_col >= line_width {
        line_content_end(rope, line)
    } else {
        crate::grapheme::char_pos_at_display_col(slice, line, target_display_col, tab_width)
    }
}

/// Convert a char-offset position to a line-relative byte offset.
///
/// Returns `(line_idx, byte_in_line)` — the byte offset from the start of
/// the line. Used to build tree-sitter `Point`s and line-relative highlight
/// spans.
pub fn char_to_line_byte(rope: &Rope, char_pos: usize) -> (usize, usize) {
    let line = rope.char_to_line(char_pos);
    let line_start_byte = rope.line_to_byte(line);
    let byte = rope.char_to_byte(char_pos).saturating_sub(line_start_byte);
    (line, byte)
}

/// `(row, byte_col)` of the end of `inserted` written starting at
/// `(row, byte_col)` — tree-sitter's `Point` convention (row is a line
/// index, `byte_col` a line-relative byte offset), used to build the
/// `new_end_position` of an `InputEdit` without a second rope lookup: same
/// row with `byte_col` advanced by `inserted`'s byte length when `inserted`
/// has no `'\n'`; otherwise `row` advances by the newline count and
/// `byte_col` becomes the byte count after the last `'\n'`. Splits on
/// `'\n'` only, matching tree-sitter's own convention — callers feed
/// CRLF-normalized buffer text.
pub fn advance_byte_point(row: usize, byte_col: usize, inserted: &str) -> (usize, usize) {
    match inserted.rfind('\n') {
        None => (row, byte_col + inserted.len()),
        Some(last_nl) => {
            let newline_count = inserted.bytes().filter(|&b| b == b'\n').count();
            (row + newline_count, inserted.len() - last_nl - 1)
        }
    }
}

/// Yield `(line, byte_start, byte_end)` for each line the *non-empty*
/// `[start, end_char_excl)` char range touches, clipped to that line's own
/// content (up to but excluding its trailing `\n`). Caller must check
/// `start < end_char_excl` first.
///
/// A single-line range yields one triple, byte-identical to converting
/// `start`/`end_char_excl` directly with [`char_to_line_byte`]. A multi-line
/// range yields one triple per touched line. The clip point is deliberately
/// the `\n` char's own position, not [`line_end_exclusive`] — the latter is
/// the *next* line's start, which `char_to_line_byte` would resolve to the
/// wrong line (byte 0 of the line after).
pub fn line_segments(
    rope: &Rope,
    start: usize,
    end_char_excl: usize,
) -> impl Iterator<Item = (usize, usize, usize)> + '_ {
    let last_char = end_char_excl - 1;
    let start_line = rope.char_to_line(start);
    let end_line = rope.char_to_line(last_char);
    (start_line..=end_line).map(move |line| {
        let line_newline = line_break_char(rope, line);
        let seg_start = start.max(rope.line_to_char(line));
        let seg_end = end_char_excl.min(line_newline);
        let (_, byte_start) = char_to_line_byte(rope, seg_start);
        let (_, byte_end) = char_to_line_byte(rope, seg_end);
        (line, byte_start, byte_end)
    })
}

#[cfg(test)]
mod tests;
