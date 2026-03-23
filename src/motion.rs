use crate::buffer::Buffer;
use crate::grapheme::{next_grapheme_boundary, prev_grapheme_boundary};
use crate::helpers::{classify_char, is_word_boundary, is_WORD_boundary, line_content_end, line_end_exclusive, snap_to_grapheme_boundary, CharClass};
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

/// Apply an inner motion to every selection in the set, repeated `count` times.
///
/// `motion` is a plain function `fn(&Buffer, head) -> new_head`. It knows
/// nothing about anchors or multi-cursor — it computes exactly one new
/// position from one old position. `apply_motion` handles the anchor
/// semantics (via `mode`) and multi-cursor bookkeeping.
///
/// `count` controls how many times the motion is applied per selection.
/// The motion is folded `count` times *inside* `map_and_merge` — each
/// selection independently accumulates N steps before anchor/merge logic
/// runs. This is semantically "move 3 words" (not "apply 1w to the whole
/// selection set three times"), which prevents premature merging of
/// multi-cursor selections between steps.
///
/// Uses `map_and_merge` so that selections which converge to the same position
/// after the motion are automatically merged.
pub(crate) fn apply_motion(
    buf: &Buffer,
    sels: SelectionSet,
    mode: MotionMode,
    count: usize,
    motion: impl Fn(&Buffer, usize) -> usize,
) -> SelectionSet {
    let result = sels.map_and_merge(|sel| {
        // Apply the motion `count` times, feeding each result as the next
        // input. `fold` starting from the current head position.
        let new_head = (0..count).fold(sel.head, |h, _| motion(buf, h));
        match mode {
            MotionMode::Move => Selection::cursor(new_head),
            MotionMode::Select => Selection::new(sel.head, new_head),
            MotionMode::Extend => Selection::new(sel.anchor, new_head),
        }
    });
    result.debug_assert_valid(buf.len_chars());
    result
}

// ── Character motions (inner) ─────────────────────────────────────────────────

/// Move one grapheme cluster to the right.
///
/// Clamps to `buf.len_chars() - 1` so the cursor never moves past the
/// trailing `\n` (which is always the last character in the buffer).
fn move_right(buf: &Buffer, head: usize) -> usize {
    let next = next_grapheme_boundary(buf, head);
    // len_chars() - 1 is safe: the buffer always has at least one char (\n).
    next.min(buf.len_chars() - 1)
}

/// Move one grapheme cluster to the left.
///
/// Returns `0` when already at the start of the buffer.
fn move_left(buf: &Buffer, head: usize) -> usize {
    prev_grapheme_boundary(buf, head)
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
    // The core logic lives in helpers::line_content_end, which is also used by
    // selection_cmd.rs — one implementation, two callers.
    line_content_end(buf, buf.char_to_line(head))
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
            // Step by grapheme boundary to respect the project invariant even
            // for space/tab (both are always single-codepoint, but be consistent).
            Some(' ') | Some('\t') => pos = next_grapheme_boundary(buf, pos),
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

    // The phantom trailing line (produced by the structural trailing \n) has
    // target_start == len_chars(). Moving into it would place the cursor past
    // all characters — stay put instead.
    if target_start >= buf.len_chars() {
        return head;
    }

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

// ── Word motions (inner) ──────────────────────────────────────────────────────

/// Move to the start of the next word.
///
/// Pair-scan forward: stop when the category changes AND the next char is
/// either Eol or not Space. This skips the current word/punct, skips spaces
/// (but not newlines), and lands on the next word/punct start or on a newline.
///
/// The `is_boundary` parameter is `is_word_boundary` for `w` and
/// `is_WORD_boundary` for `W`.
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
    let mut prev_class = classify_char(buf.char_at(pos).expect("pos < len"));
    // Advance by a full grapheme cluster so we never land mid-cluster.
    // This matters for combining sequences like e + U+0301 (combining acute):
    // stepping by 1 would land on the combining codepoint, which classify_char
    // sees as Punctuation — creating a false word boundary inside the grapheme.
    pos = next_grapheme_boundary(buf, pos);

    while pos < len {
        let cur_class = classify_char(buf.char_at(pos).expect("pos < len"));
        if is_boundary(prev_class, cur_class)
            && (cur_class == CharClass::Eol || cur_class != CharClass::Space)
        {
            return pos;
        }
        prev_class = cur_class;
        pos = next_grapheme_boundary(buf, pos);
    }
    // Clamp to last valid position (the trailing \n). len - 1 is safe because
    // the buffer always has at least one character.
    pos.min(len - 1)
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

    // Step back by a full grapheme cluster so we never start mid-cluster.
    // For a combining sequence like "café" (e + U+0301), stepping by 1 from
    // the position after the cluster would land on the combining codepoint —
    // which classify_char treats as Punctuation, creating a false boundary.
    let mut pos = prev_grapheme_boundary(buf, head);

    // Phase 1: skip Space and Eol backward.
    loop {
        let cat = classify_char(buf.char_at(pos).expect("pos < len"));
        if cat != CharClass::Space && cat != CharClass::Eol {
            break;
        }
        if pos == 0 {
            return 0; // nothing but whitespace before — land at buffer start
        }
        pos = prev_grapheme_boundary(buf, pos);
    }

    // Phase 2: skip backward while in the same category.
    let cat = classify_char(buf.char_at(pos).expect("pos < len"));
    while pos > 0 {
        // Use prev_grapheme_boundary rather than pos - 1 so we always examine
        // the first codepoint of each grapheme cluster (the base character),
        // not a combining codepoint that may report a different class.
        let prev_pos = prev_grapheme_boundary(buf, pos);
        let prev_cat = classify_char(buf.char_at(prev_pos).expect("prev_pos < len"));
        if is_boundary(prev_cat, cat) {
            break;
        }
        pos = prev_pos;
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
    // Step forward by a full grapheme cluster to get our starting position.
    // Using next_grapheme_boundary instead of head + 1 ensures we skip over
    // any combining codepoints that trail the current grapheme.
    let first = next_grapheme_boundary(buf, head);
    if first >= len {
        return head; // at or past last char — no-op
    }

    let mut pos = first;

    // Phase 1: skip Space and Eol forward.
    while pos < len {
        let cat = classify_char(buf.char_at(pos).expect("pos < len"));
        if cat != CharClass::Space && cat != CharClass::Eol {
            break;
        }
        pos = next_grapheme_boundary(buf, pos);
    }
    if pos >= len {
        return len - 1; // only whitespace to EOF — clamp to last char
    }

    // Phase 2: skip forward while in the same category.
    let cat = classify_char(buf.char_at(pos).expect("pos < len"));
    loop {
        // Peek at the next grapheme cluster's class. If the category changes,
        // we've reached the end of this word — stop at the current grapheme.
        let next_pos = next_grapheme_boundary(buf, pos);
        if next_pos >= len {
            break;
        }
        let next_cat = classify_char(buf.char_at(next_pos).expect("next_pos < len"));
        if is_boundary(cat, next_cat) {
            break;
        }
        pos = next_pos;
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
        // No paragraph below — land on the trailing \n (last valid position).
        // len_chars() - 1 is safe: every buffer has at least one char.
        buf.len_chars() - 1
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
//
// Every named command is structurally identical: call `apply_motion` with a
// mode and a motion function, return `(buf, new_sels)`. The `motion_cmd!`
// macro captures that skeleton so the table below is just data — name, mode,
// motion — with no repeated scaffolding.

/// Generate a named motion command.
///
/// Two arms handle the two motion shapes that exist in this codebase:
///
/// **Direct** — the motion function takes only `(&Buffer, head)`:
/// ```ignore
/// motion_cmd!(/// doc, cmd_move_right, Move, move_right);
/// ```
///
/// **Curried** — the motion function needs an extra argument (a boundary
/// predicate or a target-column hint). The macro generates the closure
/// `|b, h| inner(b, h, arg)`:
/// ```ignore
/// motion_cmd!(/// doc, cmd_next_word_start, Select, next_word_start(is_word_boundary));
/// motion_cmd!(/// doc, cmd_move_down,       Move,   move_down_inner(None));
/// ```
///
/// The curried arm is listed first so that `ident(expr)` syntax is tried
/// before the bare-`expr` arm — without this ordering, `inner(arg)` would
/// match the direct arm as an expression and generate a call-site type error.
///
/// `$(#[$attr:meta])*` forwards doc comments (and any other attributes) from
/// the invocation into the generated function. In Rust, `/// text` is
/// syntactic sugar for `#[doc = "text"]`, so it is captured by `:meta`.
///
/// `#[allow(non_snake_case)]` is emitted unconditionally. It is a no-op for
/// snake_case names and suppresses the expected warning for WORD variants
/// (`cmd_next_WORD_start` etc.) without needing a separate macro arm.
macro_rules! motion_cmd {
    // Curried arm: motion needs an extra argument — generates a closure.
    ($(#[$attr:meta])* $name:ident, $mode:ident, $inner:ident($arg:expr)) => {
        $(#[$attr])*
        #[allow(non_snake_case)]
        pub(crate) fn $name(buf: &Buffer, sels: SelectionSet, count: usize) -> SelectionSet {
            apply_motion(buf, sels, MotionMode::$mode, count, |b, h| $inner(b, h, $arg))
        }
    };
    // Direct arm: motion function takes only (&Buffer, head).
    ($(#[$attr:meta])* $name:ident, $mode:ident, $motion:expr) => {
        $(#[$attr])*
        #[allow(non_snake_case)]
        pub(crate) fn $name(buf: &Buffer, sels: SelectionSet, count: usize) -> SelectionSet {
            apply_motion(buf, sels, MotionMode::$mode, count, $motion)
        }
    };
}

// ── Command table ─────────────────────────────────────────────────────────────

motion_cmd!(/// Move all cursors one grapheme to the right (collapsed, no selection).
    cmd_move_right, Move, move_right);
motion_cmd!(/// Move all cursors one grapheme to the left (collapsed, no selection).
    cmd_move_left, Move, move_left);
motion_cmd!(/// Extend all selections one grapheme to the right (anchor stays, head moves).
    cmd_extend_right, Extend, move_right);
motion_cmd!(/// Extend all selections one grapheme to the left (anchor stays, head moves).
    cmd_extend_left, Extend, move_left);

motion_cmd!(/// Move all cursors to the start of their current line.
    cmd_goto_line_start, Move, goto_line_start);
motion_cmd!(/// Move all cursors to the last non-newline character on their current line.
    cmd_goto_line_end, Move, goto_line_end);
motion_cmd!(/// Move all cursors to the first non-blank character on their current line.
    cmd_goto_first_nonblank, Move, goto_first_nonblank);

// Vertical motion passes `None` as the target-column hint (no sticky column yet).
motion_cmd!(/// Move all cursors down one line, preserving the char-offset column.
    cmd_move_down, Move, move_down_inner(None));
motion_cmd!(/// Move all cursors up one line, preserving the char-offset column.
    cmd_move_up, Move, move_up_inner(None));
motion_cmd!(/// Extend all selections down one line (anchor stays, head moves).
    cmd_extend_down, Extend, move_down_inner(None));
motion_cmd!(/// Extend all selections up one line (anchor stays, head moves).
    cmd_extend_up, Extend, move_up_inner(None));

// Word motions — Select mode (anchor = old head, head = new position).
motion_cmd!(/// Select to the start of the next word (w).
    cmd_next_word_start, Select, next_word_start(is_word_boundary));
motion_cmd!(/// Select to the start of the next WORD (W — treats word+punct as one class).
    cmd_next_WORD_start, Select, next_word_start(is_WORD_boundary));
motion_cmd!(/// Select to the start of the previous word (b).
    cmd_prev_word_start, Select, prev_word_start(is_word_boundary));
motion_cmd!(/// Select to the start of the previous WORD (B).
    cmd_prev_WORD_start, Select, prev_word_start(is_WORD_boundary));
motion_cmd!(/// Select to the end of the next word (e).
    cmd_next_word_end, Select, next_word_end(is_word_boundary));
motion_cmd!(/// Select to the end of the next WORD (E).
    cmd_next_WORD_end, Select, next_word_end(is_WORD_boundary));

// Word motions — Extend mode (anchor stays, head = new position).
motion_cmd!(/// Extend selection to the start of the next word (shift-w variant).
    cmd_extend_next_word_start, Extend, next_word_start(is_word_boundary));
motion_cmd!(/// Extend selection to the start of the previous word (shift-b variant).
    cmd_extend_prev_word_start, Extend, prev_word_start(is_word_boundary));
motion_cmd!(/// Extend selection to the end of the next word (shift-e variant).
    cmd_extend_next_word_end, Extend, next_word_end(is_word_boundary));
motion_cmd!(/// Extend selection to the start of the next WORD.
    cmd_extend_next_WORD_start, Extend, next_word_start(is_WORD_boundary));
motion_cmd!(/// Extend selection to the start of the previous WORD.
    cmd_extend_prev_WORD_start, Extend, prev_word_start(is_WORD_boundary));
motion_cmd!(/// Extend selection to the end of the next WORD.
    cmd_extend_next_WORD_end, Extend, next_word_end(is_WORD_boundary));

// Paragraph motions.
motion_cmd!(/// Move all cursors to the start of the next paragraph (`]p`).
    cmd_next_paragraph, Move, next_paragraph);
motion_cmd!(/// Move all cursors to the first empty line above the current paragraph (`[p`).
    cmd_prev_paragraph, Move, prev_paragraph);
motion_cmd!(/// Extend selection to the start of the next paragraph.
    cmd_extend_next_paragraph, Extend, next_paragraph);
motion_cmd!(/// Extend selection to the first empty line above the current paragraph.
    cmd_extend_prev_paragraph, Extend, prev_paragraph);

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
#[allow(non_snake_case)] // WORD (uppercase) is an intentional Vim concept, distinct from word (lowercase)
mod tests {
    use super::*;
    use crate::assert_state;

    // ── move_right ────────────────────────────────────────────────────────────

    #[test]
    fn move_right_basic() {
        assert_state!("-[h]>ello\n", |(buf, sels)| cmd_move_right(&buf, sels, 1), "h-[e]>llo\n");
    }

    #[test]
    fn move_right_to_eof() {
        assert_state!("hell-[o]>\n", |(buf, sels)| cmd_move_right(&buf, sels, 1), "hello-[\n]>");
    }

    #[test]
    fn move_right_clamp_at_eof() {
        assert_state!("hello-[\n]>", |(buf, sels)| cmd_move_right(&buf, sels, 1), "hello-[\n]>");
    }

    #[test]
    fn move_right_empty_buffer() {
        assert_state!("-[\n]>", |(buf, sels)| cmd_move_right(&buf, sels, 1), "-[\n]>");
    }

    #[test]
    fn move_right_multi_cursor() {
        assert_state!("-[h]>-[e]>llo\n", |(buf, sels)| cmd_move_right(&buf, sels, 1), "h-[e]>-[l]>lo\n");
    }

    #[test]
    fn move_right_grapheme_cluster() {
        // "e\u{0301}" is two chars but one grapheme cluster (e + combining acute).
        // move_right from offset 0 must skip the entire cluster to offset 2.
        assert_state!(
            "-[e\u{0301}]>x\n",
            |(buf, sels)| cmd_move_right(&buf, sels, 1),
            "e\u{0301}-[x]>\n"
        );
    }

    // ── move_left ─────────────────────────────────────────────────────────────

    #[test]
    fn move_left_basic() {
        assert_state!("h-[e]>llo\n", |(buf, sels)| cmd_move_left(&buf, sels, 1), "-[h]>ello\n");
    }

    #[test]
    fn move_left_clamp_at_start() {
        assert_state!("-[h]>ello\n", |(buf, sels)| cmd_move_left(&buf, sels, 1), "-[h]>ello\n");
    }

    #[test]
    fn move_left_empty_buffer() {
        assert_state!("-[\n]>", |(buf, sels)| cmd_move_left(&buf, sels, 1), "-[\n]>");
    }

    #[test]
    fn move_left_grapheme_cluster() {
        // "e\u{0301}" is two chars but one grapheme cluster.
        // move_left from offset 2 (after the cluster) must jump to 0.
        assert_state!(
            "e\u{0301}-[x]>\n",
            |(buf, sels)| cmd_move_left(&buf, sels, 1),
            "-[e]>\u{0301}x\n"
        );
    }

    #[test]
    fn move_left_multi_cursor_merge() {
        // Cursors at 0 and 1. Both move left: 0→0 and 1→0. Same position → merge.
        assert_state!("-[a]>-[b]>c\n", |(buf, sels)| cmd_move_left(&buf, sels, 1), "-[a]>bc\n");
    }

    // ── extend_right ──────────────────────────────────────────────────────────

    #[test]
    fn extend_right_from_cursor() {
        // Collapsed cursor at 0. Extend right: anchor stays at 0, head moves to 1.
        // Forward selection anchor=0, head=1 → "-[he]>llo\n".
        assert_state!(
            "-[h]>ello\n",
            |(buf, sels)| cmd_extend_right(&buf, sels, 1),
            "-[he]>llo\n"
        );
    }

    #[test]
    fn extend_right_grows_selection() {
        // Existing forward selection anchor=0, head=1. Extend right: head moves to 2.
        // anchor=0, head=2 → "-[hel]>lo\n".
        assert_state!(
            "-[he]>llo\n",
            |(buf, sels)| cmd_extend_right(&buf, sels, 1),
            "-[hel]>lo\n"
        );
    }

    #[test]
    fn extend_right_clamp_at_eof() {
        assert_state!("hello-[\n]>", |(buf, sels)| cmd_extend_right(&buf, sels, 1), "hello-[\n]>");
    }

    // ── extend_left ───────────────────────────────────────────────────────────

    #[test]
    fn extend_left_from_cursor() {
        // Collapsed cursor at 1. Extend left: anchor stays at 1, head moves to 0.
        // Backward selection anchor=1, head=0, selects "he" (2 chars).
        assert_state!(
            "h-[e]>llo\n",
            |(buf, sels)| cmd_extend_left(&buf, sels, 1),
            "<[he]-llo\n"
        );
    }

    #[test]
    fn extend_left_shrinks_forward_selection() {
        // Forward selection anchor=0, head=2. Extend left: head moves to 1.
        // anchor=0, head=1 → "-[he]>llo\n".
        assert_state!(
            "-[hel]>lo\n",
            |(buf, sels)| cmd_extend_left(&buf, sels, 1),
            "-[he]>llo\n"
        );
    }

    #[test]
    fn extend_left_clamp_at_start() {
        assert_state!("-[h]>ello\n", |(buf, sels)| cmd_extend_left(&buf, sels, 1), "-[h]>ello\n");
    }

    // ── goto_line_start ───────────────────────────────────────────────────────

    #[test]
    fn goto_line_start_from_middle() {
        assert_state!("hel-[l]>o\n", |(buf, sels)| cmd_goto_line_start(&buf, sels, 1), "-[h]>ello\n");
    }

    #[test]
    fn goto_line_start_already_at_start() {
        assert_state!("-[h]>ello\n", |(buf, sels)| cmd_goto_line_start(&buf, sels, 1), "-[h]>ello\n");
    }

    #[test]
    fn goto_line_start_second_line() {
        assert_state!("hello\nwor-[l]>d\n", |(buf, sels)| cmd_goto_line_start(&buf, sels, 1), "hello\n-[w]>orld\n");
    }

    #[test]
    fn goto_line_start_empty_buffer() {
        assert_state!("-[\n]>", |(buf, sels)| cmd_goto_line_start(&buf, sels, 1), "-[\n]>");
    }

    // ── goto_line_end ─────────────────────────────────────────────────────────

    #[test]
    fn goto_line_end_from_start() {
        assert_state!("-[h]>ello\n", |(buf, sels)| cmd_goto_line_end(&buf, sels, 1), "hell-[o]>\n");
    }

    #[test]
    fn goto_line_end_already_at_end() {
        assert_state!("hell-[o]>\n", |(buf, sels)| cmd_goto_line_end(&buf, sels, 1), "hell-[o]>\n");
    }

    #[test]
    fn goto_line_end_stops_before_newline() {
        // Cursor must land on 'o', not on '\n'.
        assert_state!("-[h]>ello\nworld\n", |(buf, sels)| cmd_goto_line_end(&buf, sels, 1), "hell-[o]>\nworld\n");
    }

    #[test]
    fn goto_line_end_empty_line() {
        // Line contains only '\n'. Cursor stays on it.
        assert_state!("-[\n]>", |(buf, sels)| cmd_goto_line_end(&buf, sels, 1), "-[\n]>");
    }

    #[test]
    fn goto_line_end_last_line_no_newline() {
        assert_state!("-[h]>ello\n", |(buf, sels)| cmd_goto_line_end(&buf, sels, 1), "hell-[o]>\n");
    }

    #[test]
    fn goto_line_end_empty_buffer() {
        assert_state!("-[\n]>", |(buf, sels)| cmd_goto_line_end(&buf, sels, 1), "-[\n]>");
    }

    // ── goto_first_nonblank ───────────────────────────────────────────────────

    #[test]
    fn goto_first_nonblank_skips_spaces() {
        assert_state!("-[ ]> hello\n", |(buf, sels)| cmd_goto_first_nonblank(&buf, sels, 1), "  -[h]>ello\n");
    }

    #[test]
    fn goto_first_nonblank_from_middle() {
        assert_state!("  hel-[l]>o\n", |(buf, sels)| cmd_goto_first_nonblank(&buf, sels, 1), "  -[h]>ello\n");
    }

    #[test]
    fn goto_first_nonblank_skips_tab() {
        assert_state!("-[\t]>hello\n", |(buf, sels)| cmd_goto_first_nonblank(&buf, sels, 1), "\t-[h]>ello\n");
    }

    #[test]
    fn goto_first_nonblank_no_leading_whitespace() {
        assert_state!("-[h]>ello\n", |(buf, sels)| cmd_goto_first_nonblank(&buf, sels, 1), "-[h]>ello\n");
    }

    #[test]
    fn goto_first_nonblank_all_blank_line() {
        // Line is all spaces — no non-blank found, cursor is unchanged (Helix behaviour).
        assert_state!("-[ ]>  \n", |(buf, sels)| cmd_goto_first_nonblank(&buf, sels, 1), "-[ ]>  \n");
        assert_state!(" -[ ]>\n", |(buf, sels)| cmd_goto_first_nonblank(&buf, sels, 1), " -[ ]>\n");
    }

    // ── move_down ─────────────────────────────────────────────────────────────

    #[test]
    fn move_down_basic() {
        assert_state!("-[h]>ello\nworld\n", |(buf, sels)| cmd_move_down(&buf, sels, 1), "hello\n-[w]>orld\n");
    }

    #[test]
    fn move_down_preserves_column() {
        assert_state!("hel-[l]>o\nworld\n", |(buf, sels)| cmd_move_down(&buf, sels, 1), "hello\nwor-[l]>d\n");
    }

    #[test]
    fn move_down_clamps_to_shorter_line() {
        assert_state!("hel-[l]>o\nab\n", |(buf, sels)| cmd_move_down(&buf, sels, 1), "hello\na-[b]>\n");
    }

    #[test]
    fn move_down_clamp_on_last_line() {
        assert_state!("hello\n-[w]>orld\n", |(buf, sels)| cmd_move_down(&buf, sels, 1), "hello\n-[w]>orld\n");
    }

    #[test]
    fn move_down_to_empty_line() {
        assert_state!("-[h]>ello\n\nworld\n", |(buf, sels)| cmd_move_down(&buf, sels, 1), "hello\n-[\n]>world\n");
    }

    #[test]
    fn move_down_empty_buffer() {
        assert_state!("-[\n]>", |(buf, sels)| cmd_move_down(&buf, sels, 1), "-[\n]>");
    }

    #[test]
    fn move_down_multi_cursor_merge() {
        // Two cursors on line 0. Both move to line 1 — they converge and merge.
        assert_state!("-[h]>ello\n-[w]>orld\n", |(buf, sels)| cmd_move_down(&buf, sels, 1), "hello\n-[w]>orld\n");
    }

    // ── move_up ───────────────────────────────────────────────────────────────

    #[test]
    fn move_up_basic() {
        assert_state!("hello\n-[w]>orld\n", |(buf, sels)| cmd_move_up(&buf, sels, 1), "-[h]>ello\nworld\n");
    }

    #[test]
    fn move_up_preserves_column() {
        assert_state!("hello\nwor-[l]>d\n", |(buf, sels)| cmd_move_up(&buf, sels, 1), "hel-[l]>o\nworld\n");
    }

    #[test]
    fn move_up_clamp_on_first_line() {
        assert_state!("-[h]>ello\nworld\n", |(buf, sels)| cmd_move_up(&buf, sels, 1), "-[h]>ello\nworld\n");
    }

    #[test]
    fn move_up_clamps_to_shorter_line() {
        // "ab" is 2 chars, "hello" is 5. Cursor at col 3 on "hello" → clamps to end of "ab".
        assert_state!("ab\nhel-[l]>o\n", |(buf, sels)| cmd_move_up(&buf, sels, 1), "a-[b]>\nhello\n");
    }

    // ── extend_down / extend_up ───────────────────────────────────────────────

    #[test]
    fn extend_down_creates_selection() {
        // Cursor at offset 0. Extend down: anchor stays at 0, head moves to 6 ('w').
        // Forward selection: "-[hello\nw]>orld\n"
        assert_state!("-[h]>ello\nworld\n", |(buf, sels)| cmd_extend_down(&buf, sels, 1), "-[hello\nw]>orld\n");
    }

    #[test]
    fn extend_up_creates_selection() {
        // Cursor at offset 6 ('w'). Extend up: anchor stays at 6, head moves to 0 ('h').
        // Backward selection: anchor=6, head=0, selects "hello\nw" (7 chars).
        assert_state!("hello\n-[w]>orld\n", |(buf, sels)| cmd_extend_up(&buf, sels, 1), "<[hello\nw]-orld\n");
    }

    // ── next_word_start (w) ───────────────────────────────────────────────────

    #[test]
    fn next_word_start_basic() {
        // Skip "hello" + space, land on 'w'. Selection: anchor=0, head=6.
        assert_state!("-[h]>ello world\n", |(buf, sels)| cmd_next_word_start(&buf, sels, 1), "-[hello w]>orld\n");
    }

    #[test]
    fn next_word_start_word_to_punct() {
        // word→punct is a boundary; land on '.'.
        assert_state!("-[h]>ello.world\n", |(buf, sels)| cmd_next_word_start(&buf, sels, 1), "-[hello.]>world\n");
    }

    #[test]
    fn next_word_start_punct_to_word() {
        assert_state!("-[.]>hello\n", |(buf, sels)| cmd_next_word_start(&buf, sels, 1), "-[.h]>ello\n");
    }

    #[test]
    fn next_word_start_from_mid_word() {
        // Cursor in the middle of "hello" — skips the rest of it.
        assert_state!("hel-[l]>o world\n", |(buf, sels)| cmd_next_word_start(&buf, sels, 1), "hel-[lo w]>orld\n");
    }

    #[test]
    fn next_word_start_from_whitespace() {
        // From whitespace, skip to next non-whitespace.
        assert_state!("-[ ]> hello\n", |(buf, sels)| cmd_next_word_start(&buf, sels, 1), "-[  h]>ello\n");
    }

    #[test]
    fn next_word_start_stops_at_newline() {
        // w stops at the newline, not at the next line's first word.
        assert_state!("-[h]>ello\nworld\n", |(buf, sels)| cmd_next_word_start(&buf, sels, 1), "-[hello\n]>world\n");
    }

    #[test]
    fn next_word_start_from_newline() {
        // From a newline, next w skips it and lands on the next word.
        assert_state!("hello-[\n]>world\n", |(buf, sels)| cmd_next_word_start(&buf, sels, 1), "hello-[\nw]>orld\n");
    }

    #[test]
    fn next_word_start_at_eof() {
        assert_state!("hello-[\n]>", |(buf, sels)| cmd_next_word_start(&buf, sels, 1), "hello-[\n]>");
    }

    #[test]
    fn next_word_start_empty_buffer() {
        assert_state!("-[\n]>", |(buf, sels)| cmd_next_word_start(&buf, sels, 1), "-[\n]>");
    }

    // ── prev_word_start (b) ───────────────────────────────────────────────────

    #[test]
    fn prev_word_start_basic() {
        // Cursor mid-word, jump back to start of current word.
        assert_state!("hello wor-[l]>d\n", |(buf, sels)| cmd_prev_word_start(&buf, sels, 1), "hello <[worl]-d\n");
    }

    #[test]
    fn prev_word_start_from_word_to_punct() {
        assert_state!("hello.wor-[l]>d\n", |(buf, sels)| cmd_prev_word_start(&buf, sels, 1), "hello.<[worl]-d\n");
    }

    #[test]
    fn prev_word_start_skips_space() {
        // From a word, skip space backward, land on start of previous word.
        assert_state!("hello -[w]>orld\n", |(buf, sels)| cmd_prev_word_start(&buf, sels, 1), "<[hello w]-orld\n");
    }

    #[test]
    fn prev_word_start_at_start() {
        assert_state!("-[h]>ello\n", |(buf, sels)| cmd_prev_word_start(&buf, sels, 1), "-[h]>ello\n");
    }

    #[test]
    fn prev_word_start_across_newline() {
        // Skip newline backward, land on word start on the previous line.
        assert_state!("hello\n-[w]>orld\n", |(buf, sels)| cmd_prev_word_start(&buf, sels, 1), "<[hello\nw]-orld\n");
    }

    // ── next_word_end (e) ─────────────────────────────────────────────────────

    #[test]
    fn next_word_end_basic() {
        // From start of word, land on last char.
        assert_state!("-[h]>ello world\n", |(buf, sels)| cmd_next_word_end(&buf, sels, 1), "-[hello]> world\n");
    }

    #[test]
    fn next_word_end_from_word_end_skips_to_next() {
        // Already at end of word — skip space, land on end of next word.
        assert_state!("hell-[o]> world\n", |(buf, sels)| cmd_next_word_end(&buf, sels, 1), "hell-[o world]>\n");
    }

    #[test]
    fn next_word_end_word_to_punct() {
        // word→punct boundary: land on last punct char.
        assert_state!("-[h]>ello.world\n", |(buf, sels)| cmd_next_word_end(&buf, sels, 1), "-[hello]>.world\n");
    }

    #[test]
    fn next_word_end_at_eof() {
        assert_state!("hello-[\n]>", |(buf, sels)| cmd_next_word_end(&buf, sels, 1), "hello-[\n]>");
    }

    // ── WORD variants (W / B / E) ─────────────────────────────────────────────

    #[test]
    fn next_WORD_start_skips_punct() {
        // W treats word+punct as one class — "hello.world" is a single WORD.
        assert_state!("-[h]>ello.world bar\n", |(buf, sels)| cmd_next_WORD_start(&buf, sels, 1), "-[hello.world b]>ar\n");
    }

    #[test]
    fn next_word_start_stops_at_punct() {
        // w (lowercase) stops at punct — "hello" and ".world" are separate words.
        assert_state!("-[h]>ello.world bar\n", |(buf, sels)| cmd_next_word_start(&buf, sels, 1), "-[hello.]>world bar\n");
    }

    #[test]
    fn prev_WORD_start_skips_punct() {
        // B: "hello.world" is one WORD, jump to its start.
        assert_state!("hello.wor-[l]>d bar\n", |(buf, sels)| cmd_prev_WORD_start(&buf, sels, 1), "<[hello.worl]-d bar\n");
    }

    #[test]
    fn next_WORD_end_skips_punct() {
        // E: land on last char of the WORD (including adjacent punct).
        assert_state!("-[h]>ello.world bar\n", |(buf, sels)| cmd_next_WORD_end(&buf, sels, 1), "-[hello.world]> bar\n");
    }

    // ── extend variants ───────────────────────────────────────────────────────

    #[test]
    fn extend_next_word_start_grows_selection() {
        // Existing forward selection — extend its head to next word start.
        assert_state!("-[hell]>o world\n", |(buf, sels)| cmd_extend_next_word_start(&buf, sels, 1), "-[hello w]>orld\n");
    }

    #[test]
    fn extend_prev_word_start_backward() {
        assert_state!("hello -[w]>orld\n", |(buf, sels)| cmd_extend_prev_word_start(&buf, sels, 1), "<[hello w]-orld\n");
    }

    // ── extend WORD variants ──────────────────────────────────────────────────

    #[test]
    fn extend_next_WORD_start_skips_punct() {
        // Cursor inside "hello.world" — extend to next WORD start, skipping punct as part of WORD.
        assert_state!("-[h]>ello.world foo\n", |(buf, sels)| cmd_extend_next_WORD_start(&buf, sels, 1), "-[hello.world f]>oo\n");
    }

    #[test]
    fn extend_next_WORD_start_grows_existing_selection() {
        // Existing forward selection — extend head to the next WORD start.
        assert_state!("-[hell]>o.world foo\n", |(buf, sels)| cmd_extend_next_WORD_start(&buf, sels, 1), "-[hello.world f]>oo\n");
    }

    #[test]
    fn extend_prev_WORD_start_backward() {
        // Cursor after a WORD that includes punctuation — extend back to its start.
        assert_state!("hello.world -[f]>oo\n", |(buf, sels)| cmd_extend_prev_WORD_start(&buf, sels, 1), "<[hello.world f]-oo\n");
    }

    #[test]
    fn extend_prev_WORD_start_from_inside_WORD() {
        // Cursor in middle of "hello.world" — extend back to the start of that WORD.
        assert_state!("hello.wor-[l]>d foo\n", |(buf, sels)| cmd_extend_prev_WORD_start(&buf, sels, 1), "<[hello.worl]-d foo\n");
    }

    #[test]
    fn extend_next_WORD_end_skips_punct() {
        // Cursor at start of "hello.world" — extend to its end (last char of the WORD).
        assert_state!("-[h]>ello.world foo\n", |(buf, sels)| cmd_extend_next_WORD_end(&buf, sels, 1), "-[hello.world]> foo\n");
    }

    #[test]
    fn extend_next_WORD_end_grows_existing_selection() {
        // Existing forward selection — extend head to end of next WORD.
        assert_state!("-[hell]>o.world foo\n", |(buf, sels)| cmd_extend_next_WORD_end(&buf, sels, 1), "-[hello.world]> foo\n");
    }

    // ── grapheme cluster correctness ──────────────────────────────────────────

    #[test]
    fn next_word_start_skips_combining_grapheme() {
        // Buffer: "cafe\u{0301} world\n"
        // char offsets: c(0) a(1) f(2) e(3) ◌́(4) ' '(5) w(6) ...
        // Grapheme clusters: {c}{a}{f}{e◌́}{ }{w}{o}{r}{l}{d}{\n}
        //
        // Old code (pos += 1) would stop at offset 4 (the combining codepoint
        // U+0301, classified as Punctuation), producing a false word boundary
        // and leaving the cursor mid-grapheme. New code steps by grapheme
        // boundary and correctly lands on 'w' at offset 6.
        assert_state!(
            "-[c]>afe\u{0301} world\n",
            |(buf, sels)| cmd_next_word_start(&buf, sels, 1),
            "-[cafe\u{0301} w]>orld\n"
        );
    }

    #[test]
    fn prev_word_start_skips_combining_grapheme() {
        // Buffer: "cafe\u{0301} world\n", cursor on 'w'.
        // Old code steps back by 1: lands on the combining codepoint (offset 4),
        // sees it as Punctuation ≠ the following Space — false boundary — and
        // returns offset 4 (mid-grapheme). New code steps by grapheme boundary,
        // skips the cluster {e◌́} as a unit (classified Word via 'e'), and
        // correctly backtracks all the way to 'c' at offset 0.
        assert_state!(
            "cafe\u{0301} -[w]>orld\n",
            |(buf, sels)| cmd_prev_word_start(&buf, sels, 1),
            "<[cafe\u{0301} w]-orld\n"
        );
    }

    #[test]
    fn next_word_end_skips_combining_grapheme() {
        // Buffer: "a\u{0301}b\n" — graphemes: {a◌́}{b}{\n}
        // Old code (pos + 1): the initial step lands at offset 1 (the combining
        // codepoint U+0301, Punctuation). Phase 2 then sees Punct→Word at 1→2
        // and stops, returning offset 1 — a mid-grapheme position.
        // New code: next_grapheme_boundary(0) = 2 ('b'), skipping the whole
        // {a◌́} cluster. 'b' is Word; next boundary is '\n' (Eol) — stop at 2.
        //
        // cmd_next_word_end uses MotionMode::Select (anchor = old head = 0),
        // so the result is a selection, not a cursor.
        // Old result: "-[a\u{0301}]>b\n"  (head=1, mid-grapheme)
        // New result: "-[a\u{0301}b]>\n"  (head=2, 'b')
        assert_state!(
            "-[a]>\u{0301}b\n",
            |(buf, sels)| cmd_next_word_end(&buf, sels, 1),
            "-[a\u{0301}b]>\n"
        );
    }

    // ── next_paragraph (]p) ───────────────────────────────────────────────────

    #[test]
    fn next_paragraph_basic() {
        // Skip "hello\nworld" paragraph and the empty gap line, land on "foo".
        assert_state!(
            "-[h]>ello\nworld\n\nfoo\n",
            |(buf, sels)| cmd_next_paragraph(&buf, sels, 1),
            "hello\nworld\n\n-[f]>oo\n"
        );
    }

    #[test]
    fn next_paragraph_no_paragraph_below() {
        // No empty line below — land at EOF.
        assert_state!(
            "-[h]>ello\nworld\n",
            |(buf, sels)| cmd_next_paragraph(&buf, sels, 1),
            "hello\nworld-[\n]>"
        );
    }

    #[test]
    fn next_paragraph_from_empty_line() {
        // Starting on an empty line — skip the gap, land on the next paragraph.
        assert_state!(
            "-[\n]>\nfoo\n",
            |(buf, sels)| cmd_next_paragraph(&buf, sels, 1),
            "\n\n-[f]>oo\n"
        );
    }

    #[test]
    fn next_paragraph_multiple_empty_lines() {
        // Multiple empty lines in the gap — skip all of them.
        assert_state!(
            "-[\n]>\n\nfoo\n",
            |(buf, sels)| cmd_next_paragraph(&buf, sels, 1),
            "\n\n\n-[f]>oo\n"
        );
    }

    #[test]
    fn next_paragraph_empty_buffer() {
        assert_state!("-[\n]>", |(buf, sels)| cmd_next_paragraph(&buf, sels, 1), "-[\n]>");
    }

    #[test]
    fn next_paragraph_at_eof() {
        assert_state!("hello-[\n]>", |(buf, sels)| cmd_next_paragraph(&buf, sels, 1), "hello-[\n]>");
    }

    // ── prev_paragraph ([p) ───────────────────────────────────────────────────

    #[test]
    fn prev_paragraph_basic() {
        // Land on the empty gap line above "world".
        assert_state!(
            "hello\n\nwor-[l]>d\n",
            |(buf, sels)| cmd_prev_paragraph(&buf, sels, 1),
            "hello\n-[\n]>world\n"
        );
    }

    #[test]
    fn prev_paragraph_multiple_empty_lines() {
        // Multiple empty lines — land on the first (topmost) one.
        assert_state!(
            "hello\n\n\nwor-[l]>d\n",
            |(buf, sels)| cmd_prev_paragraph(&buf, sels, 1),
            "hello\n-[\n]>\nworld\n"
        );
    }

    #[test]
    fn prev_paragraph_no_paragraph_above() {
        // No gap above — land on line 0 (no-op if already there).
        assert_state!(
            "-[h]>ello\nworld\n",
            |(buf, sels)| cmd_prev_paragraph(&buf, sels, 1),
            "-[h]>ello\nworld\n"
        );
    }

    #[test]
    fn prev_paragraph_from_empty_line() {
        // Starting on the empty gap line — skip gap + paragraph, land on the
        // empty line above the paragraph before it.
        assert_state!(
            "hello\n-[\n]>world\n",
            |(buf, sels)| cmd_prev_paragraph(&buf, sels, 1),
            "-[h]>ello\n\nworld\n"
        );
    }

    // ── multi-paragraph navigation ────────────────────────────────────────────

    #[test]
    fn next_paragraph_sequential() {
        // Two consecutive ]p motions walk through three paragraphs.
        assert_state!(
            "-[a]>\n\nb\n\nc\n",
            |(buf, sels)| cmd_next_paragraph(&buf, sels, 1),
            "a\n\n-[b]>\n\nc\n"
        );
        assert_state!(
            "a\n\n-[b]>\n\nc\n",
            |(buf, sels)| cmd_next_paragraph(&buf, sels, 1),
            "a\n\nb\n\n-[c]>\n"
        );
    }

    #[test]
    fn prev_paragraph_sequential() {
        // Two consecutive [p motions walk backward through three paragraphs.
        assert_state!(
            "a\n\nb\n\n-[c]>\n",
            |(buf, sels)| cmd_prev_paragraph(&buf, sels, 1),
            "a\n\nb\n-[\n]>c\n"
        );
        assert_state!(
            "a\n\nb\n-[\n]>c\n",
            |(buf, sels)| cmd_prev_paragraph(&buf, sels, 1),
            "a\n-[\n]>b\n\nc\n"
        );
    }

    // ── extend variants ───────────────────────────────────────────────────────

    #[test]
    fn extend_next_paragraph_creates_selection() {
        // Anchor stays at 0, head moves to 'w' at the start of "world".
        assert_state!(
            "-[h]>ello\n\nworld\n",
            |(buf, sels)| cmd_extend_next_paragraph(&buf, sels, 1),
            "-[hello\n\nw]>orld\n"
        );
    }

    #[test]
    fn extend_prev_paragraph_creates_selection() {
        // Anchor stays on 'w', head moves back to the empty gap line.
        assert_state!(
            "hello\n\n-[w]>orld\n",
            |(buf, sels)| cmd_extend_prev_paragraph(&buf, sels, 1),
            "hello\n<[\nw]-orld\n"
        );
    }

    // ── count prefix ──────────────────────────────────────────────────────────

    #[test]
    fn move_right_count_3() {
        // h(0) → e(1) → l(2) → l(3)
        assert_state!("-[h]>ello\n", |(buf, sels)| cmd_move_right(&buf, sels, 3), "hel-[l]>o\n");
    }

    #[test]
    fn move_right_count_clamps_at_eof() {
        // count=100 far exceeds the buffer length — clamps at the trailing '\n'.
        assert_state!("-[h]>ello\n", |(buf, sels)| cmd_move_right(&buf, sels, 100), "hello-[\n]>");
    }

    #[test]
    fn move_left_count_3() {
        // \n(5) → o(4) → l(3) → l(2)
        assert_state!("hello-[\n]>", |(buf, sels)| cmd_move_left(&buf, sels, 3), "he-[l]>lo\n");
    }

    #[test]
    fn extend_right_count_3() {
        // Extend: anchor stays at old head (0), head folds 3 steps: 0→1→2→3.
        // Selection anchor=0, head=3: covers "hell".
        assert_state!("-[h]>ello\n", |(buf, sels)| cmd_extend_right(&buf, sels, 3), "-[hell]>o\n");
    }

    #[test]
    fn move_down_count_3() {
        // From 'a' on line 0, move down 3 lines — lands on 'd'.
        assert_state!(
            "-[a]>\nb\nc\nd\ne\n",
            |(buf, sels)| cmd_move_down(&buf, sels, 3),
            "a\nb\nc\n-[d]>\ne\n"
        );
    }

    #[test]
    fn next_word_start_count_2() {
        // cmd_next_word_start uses Select mode (anchor = old head = 0).
        // Step 1: 0 → 6 ('w'). Step 2: 6 → 12 ('f').
        // Final selection: anchor=0, head=12.
        assert_state!(
            "-[h]>ello world foo\n",
            |(buf, sels)| cmd_next_word_start(&buf, sels, 2),
            "-[hello world f]>oo\n"
        );
    }

    #[test]
    fn move_right_count_grapheme_cluster() {
        // Buffer: "e◌́x\n". Grapheme clusters: {e◌́}(0..2), {x}(2), {\n}(3).
        // count=2 from offset 0: step1 → 2 (x), step2 → 3 (\n). Clamped to len-1=3.
        assert_state!(
            "-[e\u{0301}]>x\n",
            |(buf, sels)| cmd_move_right(&buf, sels, 2),
            "e\u{0301}x-[\n]>"
        );
    }

    #[test]
    fn multi_cursor_count_independent_movement() {
        // Two cursors: 'h'(0) and 'l'(2). move_right count=3.
        // Cursor 0: 0→1→2→3 (second 'l'). Cursor 2: 2→3→4→5 ('\n').
        // No merge — different positions.
        assert_state!(
            "-[h]>el-[l]>o\n",
            |(buf, sels)| cmd_move_right(&buf, sels, 3),
            "hel-[l]>o-[\n]>"
        );
    }

    // ── multi-cursor word motions ──────────────────────────────────────────────

    #[test]
    fn next_word_end_multi_cursor() {
        // Two cursors in different words. Select mode: anchor stays at old head, head moves.
        // Cursor 1 at 'h'(0): next_word_end → 'o'(4). Selection (0,4).
        // Cursor 2 at 'f'(6): next_word_end → 'o'(8). Selection (6,8).
        assert_state!(
            "-[h]>ello -[f]>oo\n",
            |(buf, sels)| cmd_next_word_end(&buf, sels, 1),
            "-[hello]> -[foo]>\n"
        );
    }

    #[test]
    fn next_word_start_multi_cursor() {
        // Two cursors that jump to non-overlapping positions.
        // "hello foo bar\n": cursor 1 at 'h'(0) → 'f'(6); cursor 2 at 'b'(10) → '\n'(13).
        assert_state!(
            "-[h]>ello foo -[b]>ar\n",
            |(buf, sels)| cmd_next_word_start(&buf, sels, 1),
            "-[hello f]>oo -[bar\n]>"
        );
    }

    #[test]
    fn prev_word_start_multi_cursor() {
        // "hello world\n": cursors at 'o'(4) and 'd'(10). Each jumps to start of its word.
        // Cursor 1: anchor=4, head=0 (backward, 'o' IS the anchor so it's included).
        // Cursor 2: anchor=10, head=6 (backward, 'd' IS the anchor so it's included).
        assert_state!(
            "hell-[o]> worl-[d]>\n",
            |(buf, sels)| cmd_prev_word_start(&buf, sels, 1),
            "<[hello]- <[world]-\n"
        );
    }

    // ── multi-cursor paragraph motions ────────────────────────────────────────

    #[test]
    fn next_paragraph_multi_cursor() {
        // Two cursors in different paragraphs, each jumps to the start of the next one.
        // "hello\n\nworld\n\nfoo\n": cursor at 'w'(7) → 'f'(14); cursor at 'f'(14) → '\n'(17).
        assert_state!(
            "hello\n\n-[w]>orld\n\n-[f]>oo\n",
            |(buf, sels)| cmd_next_paragraph(&buf, sels, 1),
            "hello\n\nworld\n\n-[f]>oo-[\n]>"
        );
    }

    #[test]
    fn prev_paragraph_multi_cursor() {
        // Same buffer; each cursor jumps backward to the gap above its paragraph.
        // Cursor at 'w'(7) → '\n'(6) (gap). Cursor at 'f'(14) → '\n'(13) (gap).
        assert_state!(
            "hello\n\n-[w]>orld\n\n-[f]>oo\n",
            |(buf, sels)| cmd_prev_paragraph(&buf, sels, 1),
            "hello\n-[\n]>world\n-[\n]>foo\n"
        );
    }

    // ── multi-cursor goto_line motions ────────────────────────────────────────

    #[test]
    fn goto_line_start_multi_cursor() {
        assert_state!(
            "hel-[l]>o\nwor-[l]>d\n",
            |(buf, sels)| cmd_goto_line_start(&buf, sels, 1),
            "-[h]>ello\n-[w]>orld\n"
        );
    }

    #[test]
    fn goto_line_end_multi_cursor() {
        assert_state!(
            "-[h]>ello\n-[w]>orld\n",
            |(buf, sels)| cmd_goto_line_end(&buf, sels, 1),
            "hell-[o]>\nworl-[d]>\n"
        );
    }

    #[test]
    fn goto_first_nonblank_multi_cursor() {
        // Both cursors are mid-line; each jumps to the first non-blank of its line.
        assert_state!(
            "  hel-[l]>o\n  wor-[l]>d\n",
            |(buf, sels)| cmd_goto_first_nonblank(&buf, sels, 1),
            "  -[h]>ello\n  -[w]>orld\n"
        );
    }

    // ── multi-cursor merge on move_up ─────────────────────────────────────────

    #[test]
    fn move_up_multi_cursor_merge() {
        // Line 0 is "a\n" (1 content char). Two cursors on line 1 at cols 0 and 2.
        // Both move up: col 0 → 'a'(0); col 2 → clamps to 'a'(0). They merge.
        // Buffer content "a\norld\n" is unchanged; only one cursor remains.
        assert_state!(
            "a\n-[o]>r-[l]>d\n",
            |(buf, sels)| cmd_move_up(&buf, sels, 1),
            "-[a]>\norld\n"
        );
    }

    // ── empty buffer edge cases ───────────────────────────────────────────────

    #[test]
    fn goto_first_nonblank_empty_buffer() {
        assert_state!("-[\n]>", |(buf, sels)| cmd_goto_first_nonblank(&buf, sels, 1), "-[\n]>");
    }

    #[test]
    fn prev_word_start_empty_buffer() {
        assert_state!("-[\n]>", |(buf, sels)| cmd_prev_word_start(&buf, sels, 1), "-[\n]>");
    }

    #[test]
    fn next_word_end_empty_buffer() {
        assert_state!("-[\n]>", |(buf, sels)| cmd_next_word_end(&buf, sels, 1), "-[\n]>");
    }

    #[test]
    fn prev_paragraph_empty_buffer() {
        assert_state!("-[\n]>", |(buf, sels)| cmd_prev_paragraph(&buf, sels, 1), "-[\n]>");
    }}
