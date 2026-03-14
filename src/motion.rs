use crate::buffer::Buffer;
use crate::grapheme::{next_grapheme_boundary, prev_grapheme_boundary};
use crate::selection::{Selection, SelectionSet};

// ── Motion mode ───────────────────────────────────────────────────────────────

/// Controls how a motion updates the selection's anchor and head.
///
/// In Helix's select-then-act model, the same underlying position calculation
/// can produce three distinct selection behaviours depending on context:
///
/// | Mode | Anchor | Head | Typical keys |
/// |------|--------|------|-------------|
/// | `Move`   | `new_head` | `new_head` | `h`, `l` — plain cursor move |
/// | `Select` | `old_head` | `new_head` | `w`, `b` — select from here to target |
/// | `Extend` | `old_anchor` | `new_head` | shift-variants — grow selection |
///
/// `Move` always produces a single-character selection (anchor == head).
/// `Select` creates a fresh selection whose anchor is the *current* cursor position.
/// `Extend` keeps the existing anchor, only moving the head.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum MotionMode {
    Move,
    Select,
    Extend,
}

// ── Motion framework ──────────────────────────────────────────────────────────

/// Apply an inner motion to every selection in the set.
///
/// `motion` is a plain function `fn(&Buffer, head) -> new_head`. It knows
/// nothing about anchors or multi-cursor — it computes exactly one new
/// position from one old position. `apply_motion` handles the anchor
/// semantics (via `mode`) and multi-cursor bookkeeping.
///
/// Uses `map_and_merge` so that selections which converge to the same position
/// after the motion are automatically merged.
pub(crate) fn apply_motion(
    buf: &Buffer,
    sels: SelectionSet,
    mode: MotionMode,
    motion: impl Fn(&Buffer, usize) -> usize,
) -> SelectionSet {
    sels.map_and_merge(|sel| {
        let new_head = motion(buf, sel.head);
        match mode {
            MotionMode::Move => Selection::cursor(new_head),
            MotionMode::Select => Selection::new(sel.head, new_head),
            MotionMode::Extend => Selection::new(sel.anchor, new_head),
        }
    })
}

// ── Character motions (inner) ─────────────────────────────────────────────────

/// Move one grapheme cluster to the right.
///
/// Returns `buf.len_chars()` when already at or past the end — the grapheme
/// API handles clamping so callers never get an out-of-bounds offset.
fn move_right(buf: &Buffer, head: usize) -> usize {
    next_grapheme_boundary(buf, head)
}

/// Move one grapheme cluster to the left.
///
/// Returns `0` when already at the start of the buffer.
fn move_left(buf: &Buffer, head: usize) -> usize {
    prev_grapheme_boundary(buf, head)
}

// ── Line motion helpers ───────────────────────────────────────────────────────

/// Exclusive end of `line`: char offset of the first char on the *next* line,
/// or `buf.len_chars()` for the last line.
fn line_end_exclusive(buf: &Buffer, line: usize) -> usize {
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
fn snap_to_grapheme_boundary(buf: &Buffer, line_start: usize, target: usize) -> usize {
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

// ── Line motions (inner) ──────────────────────────────────────────────────────

/// Jump to the first character on the current line.
fn goto_line_start(buf: &Buffer, head: usize) -> usize {
    buf.line_to_char(buf.char_to_line(head))
}

/// Jump to the last non-newline grapheme cluster on the current line.
///
/// On an empty line (containing only `\n`), the cursor stays on the newline —
/// there is no other character to land on.
fn goto_line_end(buf: &Buffer, head: usize) -> usize {
    let line = buf.char_to_line(head);
    let line_start = buf.line_to_char(line);
    let end_excl = line_end_exclusive(buf, line);

    if end_excl == line_start {
        // Empty buffer.
        return line_start;
    }

    // Check for a trailing newline.
    let last = end_excl - 1;
    if buf.char_at(last) == Some('\n') {
        if last == line_start {
            // Empty line — only a newline. Cursor stays on it.
            line_start
        } else {
            // Step back past the newline to the last content grapheme cluster.
            prev_grapheme_boundary(buf, last)
        }
    } else {
        // Last line with no trailing newline.
        prev_grapheme_boundary(buf, end_excl)
    }
}

/// Jump to the first non-blank character on the current line.
///
/// "Blank" means ASCII space or tab. Matches Helix behaviour: if no non-blank
/// character exists on the line (e.g. a line of only spaces), the motion is a
/// no-op and the cursor stays at its current position.
fn goto_first_nonblank(buf: &Buffer, head: usize) -> usize {
    let line = buf.char_to_line(head);
    let line_start = buf.line_to_char(line);
    let end_excl = line_end_exclusive(buf, line);

    let mut pos = line_start;
    while pos < end_excl {
        match buf.char_at(pos) {
            Some(' ') | Some('\t') => pos += 1,
            Some('\n') | None => break, // end of line content without finding non-blank
            Some(_) => return pos,      // found a non-blank char
        }
    }
    head // no non-blank found — no-op, matching Helix
}

/// Move the cursor down one line, preserving the char-offset column.
///
/// `preferred_col` overrides the column computed from the current position.
/// Pass `None` to use the current column. A `Some` value supports sticky-column
/// behaviour once the editor layer tracks it.
///
/// **Column model (M1 simplification):** column is a char offset from line
/// start, not a display column. This is correct for ASCII. When the renderer
/// adds tab/wide-char support, vertical motions will switch to display columns.
fn move_down_inner(buf: &Buffer, head: usize, preferred_col: Option<usize>) -> usize {
    let line = buf.char_to_line(head);
    if line + 1 >= buf.len_lines() {
        return head; // already on the last line
    }

    let col = preferred_col.unwrap_or_else(|| head - buf.line_to_char(line));
    let target_start = buf.line_to_char(line + 1);
    let target_end = line_end_exclusive(buf, line + 1);
    let target = target_start + col;

    if target >= target_end {
        // Column overshoots the target line — clamp to last char.
        goto_line_end(buf, target_start)
    } else {
        snap_to_grapheme_boundary(buf, target_start, target)
    }
}

/// Move the cursor up one line, preserving the char-offset column.
///
/// See `move_down_inner` for the column model and `preferred_col` semantics.
fn move_up_inner(buf: &Buffer, head: usize, preferred_col: Option<usize>) -> usize {
    let line = buf.char_to_line(head);
    if line == 0 {
        return head; // already on the first line
    }

    let col = preferred_col.unwrap_or_else(|| head - buf.line_to_char(line));
    let target_start = buf.line_to_char(line - 1);
    let target_end = line_end_exclusive(buf, line - 1);
    let target = target_start + col;

    if target >= target_end {
        goto_line_end(buf, target_start)
    } else {
        snap_to_grapheme_boundary(buf, target_start, target)
    }
}

// ── Word motion helpers ───────────────────────────────────────────────────────

/// Broad category of a character for word-boundary detection.
///
/// `Eol` is distinct from `Space` so that `w` can stop at newlines (matching
/// Helix), rather than treating `\n` as ordinary whitespace to skip over.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CharClass {
    Word,        // alphanumeric + underscore
    Punctuation, // other non-whitespace, non-newline
    Space,       // space, tab
    Eol,         // newline
}

fn classify_char(ch: char) -> CharClass {
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
fn is_word_boundary(a: CharClass, b: CharClass) -> bool {
    a != b
}

/// Word and Punctuation are treated as the same "long word" class — only
/// transitions involving Space or Eol count.
fn is_long_word_boundary(a: CharClass, b: CharClass) -> bool {
    let merge = |c: CharClass| {
        if c == CharClass::Punctuation { CharClass::Word } else { c }
    };
    merge(a) != merge(b)
}

// ── Word motions (inner) ──────────────────────────────────────────────────────

/// Move to the start of the next word.
///
/// Pair-scan forward: stop when the category changes AND the next char is
/// either Eol or not Space. This skips the current word/punct, skips spaces
/// (but not newlines), and lands on the next word/punct start or on a newline.
///
/// The `is_boundary` parameter is `is_word_boundary` for `w` and
/// `is_long_word_boundary` for `W`.
fn next_word_start(
    buf: &Buffer,
    head: usize,
    is_boundary: impl Fn(CharClass, CharClass) -> bool,
) -> usize {
    let len = buf.len_chars();
    if head >= len {
        return head;
    }

    let mut pos = head;
    // Unwrap is safe: pos < len is guaranteed by loop condition and entry guard.
    let mut prev_class = classify_char(buf.char_at(pos).expect("pos < len"));
    pos += 1;

    while pos < len {
        let cur_class = classify_char(buf.char_at(pos).expect("pos < len"));
        if is_boundary(prev_class, cur_class)
            && (cur_class == CharClass::Eol || cur_class != CharClass::Space)
        {
            return pos;
        }
        prev_class = cur_class;
        pos += 1;
    }
    pos // EOF
}

/// Move to the start of the previous word.
///
/// Two-phase backward scan: skip Space/Eol backward, then skip backward while
/// in the same category, landing on the first char of that group.
fn prev_word_start(
    buf: &Buffer,
    head: usize,
    is_boundary: impl Fn(CharClass, CharClass) -> bool,
) -> usize {
    if head == 0 {
        return 0;
    }

    let mut pos = head - 1;

    // Phase 1: skip Space and Eol backward.
    loop {
        let cat = classify_char(buf.char_at(pos).expect("pos < len"));
        if cat != CharClass::Space && cat != CharClass::Eol {
            break;
        }
        if pos == 0 {
            return 0; // nothing but whitespace before — land at buffer start
        }
        pos -= 1;
    }

    // Phase 2: skip backward while in the same category.
    let cat = classify_char(buf.char_at(pos).expect("pos < len"));
    while pos > 0 {
        let prev_cat = classify_char(buf.char_at(pos - 1).expect("pos-1 < len"));
        if is_boundary(prev_cat, cat) {
            break;
        }
        pos -= 1;
    }

    pos
}

/// Move to the end of the next word.
///
/// Two-phase forward scan: skip Space/Eol forward, then skip forward while
/// in the same category, landing on the last char of that group.
fn next_word_end(
    buf: &Buffer,
    head: usize,
    is_boundary: impl Fn(CharClass, CharClass) -> bool,
) -> usize {
    let len = buf.len_chars();
    if head + 1 >= len {
        return head; // at or past last char — no-op
    }

    let mut pos = head + 1;

    // Phase 1: skip Space and Eol forward.
    while pos < len {
        let cat = classify_char(buf.char_at(pos).expect("pos < len"));
        if cat != CharClass::Space && cat != CharClass::Eol {
            break;
        }
        pos += 1;
    }
    if pos >= len {
        return len - 1; // only whitespace to EOF — clamp to last char
    }

    // Phase 2: skip forward while in the same category.
    let cat = classify_char(buf.char_at(pos).expect("pos < len"));
    while pos + 1 < len {
        let next_cat = classify_char(buf.char_at(pos + 1).expect("pos+1 < len"));
        if is_boundary(cat, next_cat) {
            break;
        }
        pos += 1;
    }

    pos
}

// ── Paragraph motion helpers ─────────────────────────────────────────────────

/// Returns `true` if `line` is an empty line — either zero chars or exactly
/// one newline. Whitespace-only lines are NOT empty (matching Helix semantics).
fn is_empty_line(buf: &Buffer, line: usize) -> bool {
    let start = buf.line_to_char(line);
    let end = line_end_exclusive(buf, line);
    // Zero chars (last line of an empty buffer) or exactly one '\n'.
    end == start || (end == start + 1 && buf.char_at(start) == Some('\n'))
}

// ── Paragraph motions (inner) ─────────────────────────────────────────────────

/// Move to the start of the next paragraph (`]p`).
///
/// Two-phase forward scan:
/// 1. Skip non-empty lines (the current paragraph).
/// 2. Skip empty lines (the gap after the paragraph).
///
/// Lands on the first char of the next paragraph, or `len_chars()` if there is
/// no paragraph below (EOF). At EOF already: no-op.
fn next_paragraph(buf: &Buffer, head: usize) -> usize {
    let mut line = buf.char_to_line(head);
    let total = buf.len_lines();

    // Phase 1: skip the current paragraph (non-empty lines).
    while line < total && !is_empty_line(buf, line) {
        line += 1;
    }
    // Phase 2: skip the gap (empty lines).
    while line < total && is_empty_line(buf, line) {
        line += 1;
    }

    if line >= total {
        buf.len_chars() // no paragraph below — land at EOF
    } else {
        buf.line_to_char(line)
    }
}

/// Move to the first empty line above the current paragraph (`[p`).
///
/// Three-phase backward scan:
/// 1. Skip empty lines backward (if already in a gap — jump over it).
/// 2. Skip non-empty lines backward (the current paragraph).
/// 3. Scan to the TOP of the gap above (in case there are multiple empty lines).
///
/// Lands on the first (topmost) empty line of the gap above, or line 0 if
/// there is no paragraph above. At line 0 already: no-op.
fn prev_paragraph(buf: &Buffer, head: usize) -> usize {
    let mut line = buf.char_to_line(head);

    // Phase 1: skip empty lines backward (handles starting inside a gap).
    while line > 0 && is_empty_line(buf, line) {
        line -= 1;
    }
    // Phase 2: skip non-empty lines backward (current paragraph).
    while line > 0 && !is_empty_line(buf, line) {
        line -= 1;
    }
    // Phase 3: scan to the top of the gap — there may be multiple empty lines.
    while line > 0 && is_empty_line(buf, line - 1) {
        line -= 1;
    }

    buf.line_to_char(line)
}

// ── Named commands (public API) ───────────────────────────────────────────────
//
// Named commands follow the edit convention — `(Buffer, SelectionSet) ->
// (Buffer, SelectionSet)` — so they can be used directly with `assert_state!`
// and, eventually, the command dispatch table.
//
// Pure motions do not modify the buffer, so `buf` passes through unchanged.

/// Move all cursors one grapheme to the right (collapsed, no selection).
pub(crate) fn cmd_move_right(buf: Buffer, sels: SelectionSet) -> (Buffer, SelectionSet) {
    let new_sels = apply_motion(&buf, sels, MotionMode::Move, move_right);
    (buf, new_sels)
}

/// Move all cursors one grapheme to the left (collapsed, no selection).
pub(crate) fn cmd_move_left(buf: Buffer, sels: SelectionSet) -> (Buffer, SelectionSet) {
    let new_sels = apply_motion(&buf, sels, MotionMode::Move, move_left);
    (buf, new_sels)
}

/// Extend all selections one grapheme to the right (anchor stays, head moves).
pub(crate) fn cmd_extend_right(buf: Buffer, sels: SelectionSet) -> (Buffer, SelectionSet) {
    let new_sels = apply_motion(&buf, sels, MotionMode::Extend, move_right);
    (buf, new_sels)
}

/// Extend all selections one grapheme to the left (anchor stays, head moves).
pub(crate) fn cmd_extend_left(buf: Buffer, sels: SelectionSet) -> (Buffer, SelectionSet) {
    let new_sels = apply_motion(&buf, sels, MotionMode::Extend, move_left);
    (buf, new_sels)
}

/// Move all cursors to the start of their current line.
pub(crate) fn cmd_goto_line_start(buf: Buffer, sels: SelectionSet) -> (Buffer, SelectionSet) {
    let new_sels = apply_motion(&buf, sels, MotionMode::Move, goto_line_start);
    (buf, new_sels)
}

/// Move all cursors to the last non-newline character on their current line.
pub(crate) fn cmd_goto_line_end(buf: Buffer, sels: SelectionSet) -> (Buffer, SelectionSet) {
    let new_sels = apply_motion(&buf, sels, MotionMode::Move, goto_line_end);
    (buf, new_sels)
}

/// Move all cursors to the first non-blank character on their current line.
pub(crate) fn cmd_goto_first_nonblank(buf: Buffer, sels: SelectionSet) -> (Buffer, SelectionSet) {
    let new_sels = apply_motion(&buf, sels, MotionMode::Move, goto_first_nonblank);
    (buf, new_sels)
}

/// Move all cursors down one line, preserving the char-offset column.
pub(crate) fn cmd_move_down(buf: Buffer, sels: SelectionSet) -> (Buffer, SelectionSet) {
    let new_sels = apply_motion(&buf, sels, MotionMode::Move, |b, h| move_down_inner(b, h, None));
    (buf, new_sels)
}

/// Move all cursors up one line, preserving the char-offset column.
pub(crate) fn cmd_move_up(buf: Buffer, sels: SelectionSet) -> (Buffer, SelectionSet) {
    let new_sels = apply_motion(&buf, sels, MotionMode::Move, |b, h| move_up_inner(b, h, None));
    (buf, new_sels)
}

/// Extend all selections down one line (anchor stays, head moves).
pub(crate) fn cmd_extend_down(buf: Buffer, sels: SelectionSet) -> (Buffer, SelectionSet) {
    let new_sels = apply_motion(&buf, sels, MotionMode::Extend, |b, h| move_down_inner(b, h, None));
    (buf, new_sels)
}

/// Extend all selections up one line (anchor stays, head moves).
pub(crate) fn cmd_extend_up(buf: Buffer, sels: SelectionSet) -> (Buffer, SelectionSet) {
    let new_sels = apply_motion(&buf, sels, MotionMode::Extend, |b, h| move_up_inner(b, h, None));
    (buf, new_sels)
}

/// Select to the start of the next word (w).
pub(crate) fn cmd_next_word_start(buf: Buffer, sels: SelectionSet) -> (Buffer, SelectionSet) {
    let new_sels = apply_motion(&buf, sels, MotionMode::Select, |b, h| {
        next_word_start(b, h, is_word_boundary)
    });
    (buf, new_sels)
}

/// Select to the start of the next WORD (W — treats word+punct as one class).
pub(crate) fn cmd_next_long_word_start(buf: Buffer, sels: SelectionSet) -> (Buffer, SelectionSet) {
    let new_sels = apply_motion(&buf, sels, MotionMode::Select, |b, h| {
        next_word_start(b, h, is_long_word_boundary)
    });
    (buf, new_sels)
}

/// Select to the start of the previous word (b).
pub(crate) fn cmd_prev_word_start(buf: Buffer, sels: SelectionSet) -> (Buffer, SelectionSet) {
    let new_sels = apply_motion(&buf, sels, MotionMode::Select, |b, h| {
        prev_word_start(b, h, is_word_boundary)
    });
    (buf, new_sels)
}

/// Select to the start of the previous WORD (B).
pub(crate) fn cmd_prev_long_word_start(buf: Buffer, sels: SelectionSet) -> (Buffer, SelectionSet) {
    let new_sels = apply_motion(&buf, sels, MotionMode::Select, |b, h| {
        prev_word_start(b, h, is_long_word_boundary)
    });
    (buf, new_sels)
}

/// Select to the end of the next word (e).
pub(crate) fn cmd_next_word_end(buf: Buffer, sels: SelectionSet) -> (Buffer, SelectionSet) {
    let new_sels = apply_motion(&buf, sels, MotionMode::Select, |b, h| {
        next_word_end(b, h, is_word_boundary)
    });
    (buf, new_sels)
}

/// Select to the end of the next WORD (E).
pub(crate) fn cmd_next_long_word_end(buf: Buffer, sels: SelectionSet) -> (Buffer, SelectionSet) {
    let new_sels = apply_motion(&buf, sels, MotionMode::Select, |b, h| {
        next_word_end(b, h, is_long_word_boundary)
    });
    (buf, new_sels)
}

/// Extend selection to the start of the next word (shift-w variant).
pub(crate) fn cmd_extend_next_word_start(buf: Buffer, sels: SelectionSet) -> (Buffer, SelectionSet) {
    let new_sels = apply_motion(&buf, sels, MotionMode::Extend, |b, h| {
        next_word_start(b, h, is_word_boundary)
    });
    (buf, new_sels)
}

/// Extend selection to the start of the previous word (shift-b variant).
pub(crate) fn cmd_extend_prev_word_start(buf: Buffer, sels: SelectionSet) -> (Buffer, SelectionSet) {
    let new_sels = apply_motion(&buf, sels, MotionMode::Extend, |b, h| {
        prev_word_start(b, h, is_word_boundary)
    });
    (buf, new_sels)
}

/// Extend selection to the end of the next word (shift-e variant).
pub(crate) fn cmd_extend_next_word_end(buf: Buffer, sels: SelectionSet) -> (Buffer, SelectionSet) {
    let new_sels = apply_motion(&buf, sels, MotionMode::Extend, |b, h| {
        next_word_end(b, h, is_word_boundary)
    });
    (buf, new_sels)
}

/// Move all cursors to the start of the next paragraph (`]p`).
pub(crate) fn cmd_next_paragraph(buf: Buffer, sels: SelectionSet) -> (Buffer, SelectionSet) {
    let new_sels = apply_motion(&buf, sels, MotionMode::Move, next_paragraph);
    (buf, new_sels)
}

/// Move all cursors to the first empty line above the current paragraph (`[p`).
pub(crate) fn cmd_prev_paragraph(buf: Buffer, sels: SelectionSet) -> (Buffer, SelectionSet) {
    let new_sels = apply_motion(&buf, sels, MotionMode::Move, prev_paragraph);
    (buf, new_sels)
}

/// Extend selection to the start of the next paragraph.
pub(crate) fn cmd_extend_next_paragraph(buf: Buffer, sels: SelectionSet) -> (Buffer, SelectionSet) {
    let new_sels = apply_motion(&buf, sels, MotionMode::Extend, next_paragraph);
    (buf, new_sels)
}

/// Extend selection to the first empty line above the current paragraph.
pub(crate) fn cmd_extend_prev_paragraph(buf: Buffer, sels: SelectionSet) -> (Buffer, SelectionSet) {
    let new_sels = apply_motion(&buf, sels, MotionMode::Extend, prev_paragraph);
    (buf, new_sels)
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::assert_state;

    // ── move_right ────────────────────────────────────────────────────────────

    #[test]
    fn move_right_basic() {
        assert_state!("|hello", |(buf, sels)| cmd_move_right(buf, sels), "h|ello");
    }

    #[test]
    fn move_right_to_eof() {
        assert_state!("hell|o", |(buf, sels)| cmd_move_right(buf, sels), "hello|");
    }

    #[test]
    fn move_right_clamp_at_eof() {
        assert_state!("hello|", |(buf, sels)| cmd_move_right(buf, sels), "hello|");
    }

    #[test]
    fn move_right_empty_buffer() {
        assert_state!("|", |(buf, sels)| cmd_move_right(buf, sels), "|");
    }

    #[test]
    fn move_right_multi_cursor() {
        assert_state!("|h|ello", |(buf, sels)| cmd_move_right(buf, sels), "h|e|llo");
    }

    #[test]
    fn move_right_grapheme_cluster() {
        // "e\u{0301}" is two chars but one grapheme cluster (e + combining acute).
        // move_right from offset 0 must skip the entire cluster to offset 2.
        assert_state!(
            "|e\u{0301}x",
            |(buf, sels)| cmd_move_right(buf, sels),
            "e\u{0301}|x"
        );
    }

    // ── move_left ─────────────────────────────────────────────────────────────

    #[test]
    fn move_left_basic() {
        assert_state!("h|ello", |(buf, sels)| cmd_move_left(buf, sels), "|hello");
    }

    #[test]
    fn move_left_clamp_at_start() {
        assert_state!("|hello", |(buf, sels)| cmd_move_left(buf, sels), "|hello");
    }

    #[test]
    fn move_left_empty_buffer() {
        assert_state!("|", |(buf, sels)| cmd_move_left(buf, sels), "|");
    }

    #[test]
    fn move_left_grapheme_cluster() {
        // "e\u{0301}" is two chars but one grapheme cluster.
        // move_left from offset 2 (after the cluster) must jump to 0.
        assert_state!(
            "e\u{0301}|x",
            |(buf, sels)| cmd_move_left(buf, sels),
            "|e\u{0301}x"
        );
    }

    #[test]
    fn move_left_multi_cursor_merge() {
        // Cursors at 0 and 1. Both move left: 0→0 and 1→0. Same position → merge.
        assert_state!("|a|bc", |(buf, sels)| cmd_move_left(buf, sels), "|abc");
    }

    // ── extend_right ──────────────────────────────────────────────────────────

    #[test]
    fn extend_right_from_cursor() {
        // Collapsed cursor at 0. Extend right: anchor stays at 0, head moves to 1.
        // Forward selection anchor=0, head=1 → "#[h|e]#llo".
        assert_state!(
            "|hello",
            |(buf, sels)| cmd_extend_right(buf, sels),
            "#[h|e]#llo"
        );
    }

    #[test]
    fn extend_right_grows_selection() {
        // Existing forward selection anchor=0, head=1. Extend right: head moves to 2.
        // anchor=0, head=2 → "#[he|l]#lo".
        assert_state!(
            "#[h|e]#llo",
            |(buf, sels)| cmd_extend_right(buf, sels),
            "#[he|l]#lo"
        );
    }

    #[test]
    fn extend_right_clamp_at_eof() {
        assert_state!("hello|", |(buf, sels)| cmd_extend_right(buf, sels), "hello|");
    }

    // ── extend_left ───────────────────────────────────────────────────────────

    #[test]
    fn extend_left_from_cursor() {
        // Collapsed cursor at 1. Extend left: anchor stays at 1, head moves to 0.
        // Backward selection anchor=1, head=0 → "#[|h]#ello".
        assert_state!(
            "h|ello",
            |(buf, sels)| cmd_extend_left(buf, sels),
            "#[|h]#ello"
        );
    }

    #[test]
    fn extend_left_shrinks_forward_selection() {
        // Forward selection anchor=0, head=2. Extend left: head moves to 1.
        // anchor=0, head=1 → "#[h|e]#llo".
        assert_state!(
            "#[he|l]#lo",
            |(buf, sels)| cmd_extend_left(buf, sels),
            "#[h|e]#llo"
        );
    }

    #[test]
    fn extend_left_clamp_at_start() {
        assert_state!("|hello", |(buf, sels)| cmd_extend_left(buf, sels), "|hello");
    }

    // ── goto_line_start ───────────────────────────────────────────────────────

    #[test]
    fn goto_line_start_from_middle() {
        assert_state!("hel|lo", |(buf, sels)| cmd_goto_line_start(buf, sels), "|hello");
    }

    #[test]
    fn goto_line_start_already_at_start() {
        assert_state!("|hello", |(buf, sels)| cmd_goto_line_start(buf, sels), "|hello");
    }

    #[test]
    fn goto_line_start_second_line() {
        assert_state!("hello\nwor|ld", |(buf, sels)| cmd_goto_line_start(buf, sels), "hello\n|world");
    }

    #[test]
    fn goto_line_start_empty_buffer() {
        assert_state!("|", |(buf, sels)| cmd_goto_line_start(buf, sels), "|");
    }

    // ── goto_line_end ─────────────────────────────────────────────────────────

    #[test]
    fn goto_line_end_from_start() {
        assert_state!("|hello", |(buf, sels)| cmd_goto_line_end(buf, sels), "hell|o");
    }

    #[test]
    fn goto_line_end_already_at_end() {
        assert_state!("hell|o", |(buf, sels)| cmd_goto_line_end(buf, sels), "hell|o");
    }

    #[test]
    fn goto_line_end_stops_before_newline() {
        // Cursor must land on 'o', not on '\n'.
        assert_state!("|hello\nworld", |(buf, sels)| cmd_goto_line_end(buf, sels), "hell|o\nworld");
    }

    #[test]
    fn goto_line_end_empty_line() {
        // Line contains only '\n'. Cursor stays on it.
        assert_state!("|\n", |(buf, sels)| cmd_goto_line_end(buf, sels), "|\n");
    }

    #[test]
    fn goto_line_end_last_line_no_newline() {
        assert_state!("|hello", |(buf, sels)| cmd_goto_line_end(buf, sels), "hell|o");
    }

    #[test]
    fn goto_line_end_empty_buffer() {
        assert_state!("|", |(buf, sels)| cmd_goto_line_end(buf, sels), "|");
    }

    // ── goto_first_nonblank ───────────────────────────────────────────────────

    #[test]
    fn goto_first_nonblank_skips_spaces() {
        assert_state!("|  hello", |(buf, sels)| cmd_goto_first_nonblank(buf, sels), "  |hello");
    }

    #[test]
    fn goto_first_nonblank_from_middle() {
        assert_state!("  hel|lo", |(buf, sels)| cmd_goto_first_nonblank(buf, sels), "  |hello");
    }

    #[test]
    fn goto_first_nonblank_skips_tab() {
        assert_state!("|\thello", |(buf, sels)| cmd_goto_first_nonblank(buf, sels), "\t|hello");
    }

    #[test]
    fn goto_first_nonblank_no_leading_whitespace() {
        assert_state!("|hello", |(buf, sels)| cmd_goto_first_nonblank(buf, sels), "|hello");
    }

    #[test]
    fn goto_first_nonblank_all_blank_line() {
        // Line is all spaces — no non-blank found, cursor is unchanged (Helix behaviour).
        assert_state!("|   \n", |(buf, sels)| cmd_goto_first_nonblank(buf, sels), "|   \n");
        assert_state!(" | \n", |(buf, sels)| cmd_goto_first_nonblank(buf, sels), " | \n");
    }

    // ── move_down ─────────────────────────────────────────────────────────────

    #[test]
    fn move_down_basic() {
        assert_state!("|hello\nworld", |(buf, sels)| cmd_move_down(buf, sels), "hello\n|world");
    }

    #[test]
    fn move_down_preserves_column() {
        assert_state!("hel|lo\nworld", |(buf, sels)| cmd_move_down(buf, sels), "hello\nwor|ld");
    }

    #[test]
    fn move_down_clamps_to_shorter_line() {
        assert_state!("hel|lo\nab", |(buf, sels)| cmd_move_down(buf, sels), "hello\na|b");
    }

    #[test]
    fn move_down_clamp_on_last_line() {
        assert_state!("hello\n|world", |(buf, sels)| cmd_move_down(buf, sels), "hello\n|world");
    }

    #[test]
    fn move_down_to_empty_line() {
        assert_state!("|hello\n\nworld", |(buf, sels)| cmd_move_down(buf, sels), "hello\n|\nworld");
    }

    #[test]
    fn move_down_empty_buffer() {
        assert_state!("|", |(buf, sels)| cmd_move_down(buf, sels), "|");
    }

    #[test]
    fn move_down_multi_cursor_merge() {
        // Two cursors on line 0. Both move to line 1 — they converge and merge.
        assert_state!("|hello\n|world", |(buf, sels)| cmd_move_down(buf, sels), "hello\n|world");
    }

    // ── move_up ───────────────────────────────────────────────────────────────

    #[test]
    fn move_up_basic() {
        assert_state!("hello\n|world", |(buf, sels)| cmd_move_up(buf, sels), "|hello\nworld");
    }

    #[test]
    fn move_up_preserves_column() {
        assert_state!("hello\nwor|ld", |(buf, sels)| cmd_move_up(buf, sels), "hel|lo\nworld");
    }

    #[test]
    fn move_up_clamp_on_first_line() {
        assert_state!("|hello\nworld", |(buf, sels)| cmd_move_up(buf, sels), "|hello\nworld");
    }

    #[test]
    fn move_up_clamps_to_shorter_line() {
        // "ab" is 2 chars, "hello" is 5. Cursor at col 3 on "hello" → clamps to end of "ab".
        assert_state!("ab\nhel|lo", |(buf, sels)| cmd_move_up(buf, sels), "a|b\nhello");
    }

    // ── extend_down / extend_up ───────────────────────────────────────────────

    #[test]
    fn extend_down_creates_selection() {
        // Cursor at offset 0. Extend down: anchor stays at 0, head moves to 6 ('w').
        // Forward selection: "#[hello\n|w]#orld"
        assert_state!("|hello\nworld", |(buf, sels)| cmd_extend_down(buf, sels), "#[hello\n|w]#orld");
    }

    #[test]
    fn extend_up_creates_selection() {
        // Cursor at offset 6 ('w'). Extend up: anchor stays at 6, head moves to 0 ('h').
        // Backward selection: anchor=6, head=0 → "#[|hello\n]#world"
        assert_state!("hello\n|world", |(buf, sels)| cmd_extend_up(buf, sels), "#[|hello\n]#world");
    }

    // ── next_word_start (w) ───────────────────────────────────────────────────

    #[test]
    fn next_word_start_basic() {
        // Skip "hello" + space, land on 'w'. Selection: anchor=0, head=6.
        assert_state!("|hello world", |(buf, sels)| cmd_next_word_start(buf, sels), "#[hello |w]#orld");
    }

    #[test]
    fn next_word_start_word_to_punct() {
        // word→punct is a boundary; land on '.'.
        assert_state!("|hello.world", |(buf, sels)| cmd_next_word_start(buf, sels), "#[hello|.]#world");
    }

    #[test]
    fn next_word_start_punct_to_word() {
        assert_state!("|.hello", |(buf, sels)| cmd_next_word_start(buf, sels), "#[.|h]#ello");
    }

    #[test]
    fn next_word_start_from_mid_word() {
        // Cursor in the middle of "hello" — skips the rest of it.
        assert_state!("hel|lo world", |(buf, sels)| cmd_next_word_start(buf, sels), "hel#[lo |w]#orld");
    }

    #[test]
    fn next_word_start_from_whitespace() {
        // From whitespace, skip to next non-whitespace.
        assert_state!("|  hello", |(buf, sels)| cmd_next_word_start(buf, sels), "#[  |h]#ello");
    }

    #[test]
    fn next_word_start_stops_at_newline() {
        // w stops at the newline, not at the next line's first word.
        assert_state!("|hello\nworld", |(buf, sels)| cmd_next_word_start(buf, sels), "#[hello|\n]#world");
    }

    #[test]
    fn next_word_start_from_newline() {
        // From a newline, next w skips it and lands on the next word.
        assert_state!("hello|\nworld", |(buf, sels)| cmd_next_word_start(buf, sels), "hello#[\n|w]#orld");
    }

    #[test]
    fn next_word_start_at_eof() {
        assert_state!("hello|", |(buf, sels)| cmd_next_word_start(buf, sels), "hello|");
    }

    #[test]
    fn next_word_start_empty_buffer() {
        assert_state!("|", |(buf, sels)| cmd_next_word_start(buf, sels), "|");
    }

    // ── prev_word_start (b) ───────────────────────────────────────────────────

    #[test]
    fn prev_word_start_basic() {
        // Cursor mid-word, jump back to start of current word.
        assert_state!("hello wor|ld", |(buf, sels)| cmd_prev_word_start(buf, sels), "hello #[|wor]#ld");
    }

    #[test]
    fn prev_word_start_from_word_to_punct() {
        assert_state!("hello.wor|ld", |(buf, sels)| cmd_prev_word_start(buf, sels), "hello.#[|wor]#ld");
    }

    #[test]
    fn prev_word_start_skips_space() {
        // From a word, skip space backward, land on start of previous word.
        assert_state!("hello |world", |(buf, sels)| cmd_prev_word_start(buf, sels), "#[|hello ]#world");
    }

    #[test]
    fn prev_word_start_at_start() {
        assert_state!("|hello", |(buf, sels)| cmd_prev_word_start(buf, sels), "|hello");
    }

    #[test]
    fn prev_word_start_across_newline() {
        // Skip newline backward, land on word start on the previous line.
        assert_state!("hello\n|world", |(buf, sels)| cmd_prev_word_start(buf, sels), "#[|hello\n]#world");
    }

    // ── next_word_end (e) ─────────────────────────────────────────────────────

    #[test]
    fn next_word_end_basic() {
        // From start of word, land on last char.
        assert_state!("|hello world", |(buf, sels)| cmd_next_word_end(buf, sels), "#[hell|o]# world");
    }

    #[test]
    fn next_word_end_from_word_end_skips_to_next() {
        // Already at end of word — skip space, land on end of next word.
        assert_state!("hell|o world", |(buf, sels)| cmd_next_word_end(buf, sels), "hell#[o worl|d]#");
    }

    #[test]
    fn next_word_end_word_to_punct() {
        // word→punct boundary: land on last punct char.
        assert_state!("|hello.world", |(buf, sels)| cmd_next_word_end(buf, sels), "#[hell|o]#.world");
    }

    #[test]
    fn next_word_end_at_eof() {
        assert_state!("hello|", |(buf, sels)| cmd_next_word_end(buf, sels), "hello|");
    }

    // ── WORD variants (W / B / E) ─────────────────────────────────────────────

    #[test]
    fn next_long_word_start_skips_punct() {
        // W treats word+punct as one class — "hello.world" is a single WORD.
        assert_state!("|hello.world bar", |(buf, sels)| cmd_next_long_word_start(buf, sels), "#[hello.world |b]#ar");
    }

    #[test]
    fn next_word_start_stops_at_punct() {
        // w (lowercase) stops at punct — "hello" and ".world" are separate words.
        assert_state!("|hello.world bar", |(buf, sels)| cmd_next_word_start(buf, sels), "#[hello|.]#world bar");
    }

    #[test]
    fn prev_long_word_start_skips_punct() {
        // B: "hello.world" is one WORD, jump to its start.
        assert_state!("hello.wor|ld bar", |(buf, sels)| cmd_prev_long_word_start(buf, sels), "#[|hello.wor]#ld bar");
    }

    #[test]
    fn next_long_word_end_skips_punct() {
        // E: land on last char of the WORD (including adjacent punct).
        assert_state!("|hello.world bar", |(buf, sels)| cmd_next_long_word_end(buf, sels), "#[hello.worl|d]# bar");
    }

    // ── extend variants ───────────────────────────────────────────────────────

    #[test]
    fn extend_next_word_start_grows_selection() {
        // Existing forward selection — extend its head to next word start.
        assert_state!("#[hel|l]#o world", |(buf, sels)| cmd_extend_next_word_start(buf, sels), "#[hello |w]#orld");
    }

    #[test]
    fn extend_prev_word_start_backward() {
        assert_state!("hello |world", |(buf, sels)| cmd_extend_prev_word_start(buf, sels), "#[|hello ]#world");
    }

    // ── next_paragraph (]p) ───────────────────────────────────────────────────

    #[test]
    fn next_paragraph_basic() {
        // Skip "hello\nworld" paragraph and the empty gap line, land on "foo".
        assert_state!(
            "|hello\nworld\n\nfoo",
            |(buf, sels)| cmd_next_paragraph(buf, sels),
            "hello\nworld\n\n|foo"
        );
    }

    #[test]
    fn next_paragraph_no_paragraph_below() {
        // No empty line below — land at EOF.
        assert_state!(
            "|hello\nworld",
            |(buf, sels)| cmd_next_paragraph(buf, sels),
            "hello\nworld|"
        );
    }

    #[test]
    fn next_paragraph_from_empty_line() {
        // Starting on an empty line — skip the gap, land on the next paragraph.
        assert_state!(
            "|\n\nfoo",
            |(buf, sels)| cmd_next_paragraph(buf, sels),
            "\n\n|foo"
        );
    }

    #[test]
    fn next_paragraph_multiple_empty_lines() {
        // Multiple empty lines in the gap — skip all of them.
        assert_state!(
            "|\n\n\nfoo",
            |(buf, sels)| cmd_next_paragraph(buf, sels),
            "\n\n\n|foo"
        );
    }

    #[test]
    fn next_paragraph_empty_buffer() {
        assert_state!("|", |(buf, sels)| cmd_next_paragraph(buf, sels), "|");
    }

    #[test]
    fn next_paragraph_at_eof() {
        assert_state!("hello|", |(buf, sels)| cmd_next_paragraph(buf, sels), "hello|");
    }

    // ── prev_paragraph ([p) ───────────────────────────────────────────────────

    #[test]
    fn prev_paragraph_basic() {
        // Land on the empty gap line above "world".
        assert_state!(
            "hello\n\nwor|ld",
            |(buf, sels)| cmd_prev_paragraph(buf, sels),
            "hello\n|\nworld"
        );
    }

    #[test]
    fn prev_paragraph_multiple_empty_lines() {
        // Multiple empty lines — land on the first (topmost) one.
        assert_state!(
            "hello\n\n\nwor|ld",
            |(buf, sels)| cmd_prev_paragraph(buf, sels),
            "hello\n|\n\nworld"
        );
    }

    #[test]
    fn prev_paragraph_no_paragraph_above() {
        // No gap above — land on line 0 (no-op if already there).
        assert_state!(
            "|hello\nworld",
            |(buf, sels)| cmd_prev_paragraph(buf, sels),
            "|hello\nworld"
        );
    }

    #[test]
    fn prev_paragraph_from_empty_line() {
        // Starting on the empty gap line — skip gap + paragraph, land on the
        // empty line above the paragraph before it.
        assert_state!(
            "hello\n|\nworld",
            |(buf, sels)| cmd_prev_paragraph(buf, sels),
            "|hello\n\nworld"
        );
    }

    // ── multi-paragraph navigation ────────────────────────────────────────────

    #[test]
    fn next_paragraph_sequential() {
        // Two consecutive ]p motions walk through three paragraphs.
        assert_state!(
            "|a\n\nb\n\nc",
            |(buf, sels)| cmd_next_paragraph(buf, sels),
            "a\n\n|b\n\nc"
        );
        assert_state!(
            "a\n\n|b\n\nc",
            |(buf, sels)| cmd_next_paragraph(buf, sels),
            "a\n\nb\n\n|c"
        );
    }

    #[test]
    fn prev_paragraph_sequential() {
        // Two consecutive [p motions walk backward through three paragraphs.
        assert_state!(
            "a\n\nb\n\n|c",
            |(buf, sels)| cmd_prev_paragraph(buf, sels),
            "a\n\nb\n|\nc"
        );
        assert_state!(
            "a\n\nb\n|\nc",
            |(buf, sels)| cmd_prev_paragraph(buf, sels),
            "a\n|\nb\n\nc"
        );
    }

    // ── extend variants ───────────────────────────────────────────────────────

    #[test]
    fn extend_next_paragraph_creates_selection() {
        // Anchor stays at 0, head moves to 'w' at the start of "world".
        assert_state!(
            "|hello\n\nworld",
            |(buf, sels)| cmd_extend_next_paragraph(buf, sels),
            "#[hello\n\n|w]#orld"
        );
    }

    #[test]
    fn extend_prev_paragraph_creates_selection() {
        // Anchor stays on 'w', head moves back to the empty gap line.
        assert_state!(
            "hello\n\n|world",
            |(buf, sels)| cmd_extend_prev_paragraph(buf, sels),
            "hello\n#[|\n]#world"
        );
    }
}
