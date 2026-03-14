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
/// `Move` always produces a collapsed cursor (anchor == head).
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
/// Uses `map_and_merge` so that cursors which converge to the same position
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
/// "Blank" means ASCII space or tab. If the line is entirely blank, the cursor
/// lands at the line start (the newline or end of buffer).
fn goto_first_nonblank(buf: &Buffer, head: usize) -> usize {
    let line = buf.char_to_line(head);
    let line_start = buf.line_to_char(line);
    let end_excl = line_end_exclusive(buf, line);

    let mut pos = line_start;
    while pos < end_excl {
        match buf.char_at(pos) {
            Some(' ') | Some('\t') => pos += 1,
            _ => break,
        }
    }
    pos
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
        // Line is all spaces + newline — no non-blank found, cursor lands on the '\n'.
        assert_state!("|   \n", |(buf, sels)| cmd_goto_first_nonblank(buf, sels), "   |\n");
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
}
