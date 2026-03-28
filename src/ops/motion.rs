use crate::core::buffer::Buffer;
use crate::editor::FindKind;
use crate::core::grapheme::{next_grapheme_boundary, prev_grapheme_boundary};
use crate::helpers::{classify_char, is_word_boundary, is_WORD_boundary, line_content_end, line_end_exclusive, snap_to_grapheme_boundary, CharClass};
use crate::core::selection::{Selection, SelectionSet};

// ── Motion mode ───────────────────────────────────────────────────────────────

/// Controls how a motion updates the selection's anchor and head.
///
/// | Mode | Anchor | Head | Typical keys |
/// |------|--------|------|-------------|
/// | `Move`   | `new_head` | `new_head` | `h`, `j`, `k`, `l` — plain cursor move |
/// | `Extend` | `old_anchor` | `new_head` | extend-mode variants — grow selection |
///
/// `Move` always produces a collapsed single-character selection (anchor == head).
/// `Extend` keeps the existing anchor, only moving the head.
///
/// Word motions (`w`/`b`/`W`/`B`) use [`apply_word_select`] instead of this
/// enum — they return `(word_start, word_end)` pairs that become fresh
/// forward selections without any accumulated anchor. In extend mode, word
/// motions use [`apply_word_select_extend_forward`] /
/// [`apply_word_select_extend_backward`] instead, which union the new word
/// range with the existing selection rather than replacing it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum MotionMode {
    Move,
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
            MotionMode::Extend => Selection::new(sel.anchor, new_head),
        }
    });
    result.debug_assert_valid(buf);
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
/// **Column model (current simplification):** column is a char offset from line
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

// ── Word-select helpers ───────────────────────────────────────────────────────

/// Scan forward from the first char of a known word group, returning the
/// position of its last char.
///
/// Starts at `start` (which must be the first char of a word or punct group),
/// advances forward while `classify_char` stays in the same class, and stops
/// when the class changes or the buffer ends.
///
/// This is Phase 2 of `next_word_end` run from a known starting position,
/// without the initial skip-whitespace step.
fn find_word_end_from(
    buf: &Buffer,
    start: usize,
    is_boundary: impl Fn(CharClass, CharClass) -> bool,
) -> usize {
    let len = buf.len_chars();
    if start >= len {
        return start.saturating_sub(1);
    }

    let cat = classify_char(buf.char_at(start).expect("start < len"));
    let mut pos = start;

    loop {
        let next_pos = next_grapheme_boundary(buf, pos);
        // `next_pos - 1` is the last codepoint of the grapheme cluster that
        // starts at `pos`. For a single-codepoint cluster (the common case)
        // this equals `pos`; for a multi-codepoint cluster such as "e\u{0301}"
        // (é = base letter + combining accent) it includes the trailing
        // combining marks that logically belong to the same grapheme.
        if next_pos >= len {
            return next_pos - 1; // grapheme-safe: next_pos is a grapheme boundary; -1 is the last codepoint of the current cluster
        }
        let next_cat = classify_char(buf.char_at(next_pos).expect("next_pos < len"));
        if is_boundary(cat, next_cat) {
            return next_pos - 1; // grapheme-safe: next_pos is a grapheme boundary; -1 is the last codepoint of the current cluster
        }
        pos = next_pos;
    }
}

/// Find the next word (or WORD) from `pos` and return `(word_start, word_end)`.
///
/// Returns `None` when there is no next word — at the last word in the buffer
/// (no-op) or on an empty buffer.
///
/// Unlike `next_word_start`, this function crosses line boundaries: if the
/// scan lands on a newline between lines, it calls `next_word_start` a second
/// time from the newline to reach the first word on the next line.
fn select_next_word(
    buf: &Buffer,
    pos: usize,
    is_boundary: impl Fn(CharClass, CharClass) -> bool + Copy,
) -> Option<(usize, usize)> {
    let len = buf.len_chars();

    // Find the start of the next word.
    let mut word_start = next_word_start(buf, pos, is_boundary);

    // If we landed on a newline that is NOT the trailing '\n', cross the line:
    // call next_word_start again from that newline to get to the next line's word.
    if word_start < len.saturating_sub(1) {
        let cat = classify_char(buf.char_at(word_start).expect("word_start < len"));
        if cat == CharClass::Eol {
            word_start = next_word_start(buf, word_start, is_boundary);
        }
    }

    // If we've hit the trailing '\n' (last char in the buffer), there is no
    // next word — treat this as a no-op.
    if word_start >= len.saturating_sub(1) {
        return None;
    }

    // Guard: if we somehow landed on whitespace, also a no-op.
    let cat = classify_char(buf.char_at(word_start).expect("word_start < len"));
    if cat == CharClass::Space || cat == CharClass::Eol {
        return None;
    }

    let word_end = find_word_end_from(buf, word_start, is_boundary);
    Some((word_start, word_end))
}

/// Find the previous word (or WORD) from `pos` and return `(word_start, word_end)`.
///
/// Returns `None` when there is no previous word — already at or before the
/// first word in the buffer (no-op).
///
/// If `pos` is inside a word, we jump to the word BEFORE the current one (not
/// the start of the current word). If `pos` is in whitespace or at the start
/// of a word, we jump to the preceding word.
fn select_prev_word(
    buf: &Buffer,
    pos: usize,
    is_boundary: impl Fn(CharClass, CharClass) -> bool + Copy,
) -> Option<(usize, usize)> {
    if pos == 0 {
        return None;
    }

    // Find the start of the word `prev_word_start` would land on.
    let word_start = prev_word_start(buf, pos, is_boundary);

    // If that position is whitespace (e.g. buffer starts with spaces), there
    // is no actual word to jump to.
    let cat = classify_char(buf.char_at(word_start).expect("word_start < len"));
    if cat == CharClass::Space || cat == CharClass::Eol {
        return None;
    }

    let word_end = find_word_end_from(buf, word_start, is_boundary);

    // If pos is within [word_start, word_end], prev_word_start landed on the
    // CURRENT word, not the previous one. We need one more step backward.
    if pos >= word_start && pos <= word_end {
        if word_start == 0 {
            return None; // already at the first word — no-op
        }
        let prev_start = prev_word_start(buf, word_start, is_boundary);
        let prev_cat = classify_char(buf.char_at(prev_start).expect("prev_start < len"));
        if prev_cat == CharClass::Space || prev_cat == CharClass::Eol {
            return None; // no word before this one
        }
        let prev_end = find_word_end_from(buf, prev_start, is_boundary);
        return Some((prev_start, prev_end));
    }

    Some((word_start, word_end))
}

/// Apply a word-select motion to every selection in the set, repeated `count` times.
///
/// Unlike `apply_motion`, `motion` returns `(word_start, word_end)` — both
/// endpoints of the selected word — rather than a single new head position.
/// The result is always a fresh forward selection `[word_start, word_end]`
/// that replaces the old selection (no anchor accumulation).
///
/// If `motion` returns `None` (no next/previous word), the iteration stops
/// early for that selection and the last selection is kept unchanged.
fn apply_word_select(
    buf: &Buffer,
    sels: SelectionSet,
    count: usize,
    motion: impl Fn(&Buffer, usize) -> Option<(usize, usize)>,
) -> SelectionSet {
    let result = sels.map_and_merge(|sel| {
        let mut current = sel;
        for _ in 0..count {
            match motion(buf, current.head) {
                Some((anchor, head)) => current = Selection::new(anchor, head),
                None => break, // no more words — stop early, keep last selection
            }
        }
        current
    });
    result.debug_assert_valid(buf);
    result
}

/// Apply a forward word-select motion in extend mode: union the returned word range
/// with the existing selection rather than replacing it.
///
/// The motion origin is `sel.end()` so that pressing `w`/`W` always searches
/// *ahead* of the current selection, regardless of how far it already extends.
/// If `motion` returns `None`, iteration stops early and the last selection is kept.
fn apply_word_select_extend_forward(
    buf: &Buffer,
    sels: SelectionSet,
    count: usize,
    motion: impl Fn(&Buffer, usize) -> Option<(usize, usize)>,
) -> SelectionSet {
    let result = sels.map_and_merge(|sel| {
        let mut current = sel;
        // Preserve direction of the original selection through all union steps.
        let forward = current.anchor <= current.head;
        for _ in 0..count {
            // Start from the far end so the search goes past the selection.
            match motion(buf, current.end()) {
                Some((word_start, word_end)) => {
                    let new_start = current.start().min(word_start);
                    let new_end = current.end().max(word_end);
                    current = Selection::directed(new_start, new_end, forward);
                }
                None => break,
            }
        }
        current
    });
    result.debug_assert_valid(buf);
    result
}

/// Apply a backward word-select motion in extend mode: union the returned word range
/// with the existing selection rather than replacing it.
///
/// The motion origin is `sel.start()` so that pressing `b`/`B` always searches
/// *behind* the current selection, regardless of how far it already extends.
/// Without this, `select_prev_word(sel.head)` finds a word already inside the
/// selection, making union a no-op.
fn apply_word_select_extend_backward(
    buf: &Buffer,
    sels: SelectionSet,
    count: usize,
    motion: impl Fn(&Buffer, usize) -> Option<(usize, usize)>,
) -> SelectionSet {
    let result = sels.map_and_merge(|sel| {
        let mut current = sel;
        let forward = current.anchor <= current.head;
        for _ in 0..count {
            // Start from the near end so the search goes past the selection.
            match motion(buf, current.start()) {
                Some((word_start, word_end)) => {
                    let new_start = current.start().min(word_start);
                    let new_end = current.end().max(word_end);
                    current = Selection::directed(new_start, new_end, forward);
                }
                None => break,
            }
        }
        current
    });
    result.debug_assert_valid(buf);
    result
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
/// ```text
/// motion_cmd!(/// doc, cmd_move_right, Move, move_right);
/// ```
///
/// **Curried** — the motion function needs an extra argument (a boundary
/// predicate or a target-column hint). The macro generates the closure
/// `|b, h| inner(b, h, arg)`:
/// ```text
/// motion_cmd!(/// doc, cmd_extend_down, Extend, move_down_inner(None));
/// motion_cmd!(/// doc, cmd_move_down,   Move,   move_down_inner(None));
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
motion_cmd!(/// Extend all selections to the start of their current line (anchor stays, head moves).
    cmd_extend_line_start, Extend, goto_line_start);
motion_cmd!(/// Extend all selections to the last non-newline character on their current line.
    cmd_extend_line_end, Extend, goto_line_end);
motion_cmd!(/// Extend all selections to the first non-blank character on their current line.
    cmd_extend_first_nonblank, Extend, goto_first_nonblank);

// Vertical motion passes `None` as the target-column hint (no sticky column yet).
motion_cmd!(/// Move all cursors down one line, preserving the char-offset column.
    cmd_move_down, Move, move_down_inner(None));
motion_cmd!(/// Move all cursors up one line, preserving the char-offset column.
    cmd_move_up, Move, move_up_inner(None));
motion_cmd!(/// Extend all selections down one line (anchor stays, head moves).
    cmd_extend_down, Extend, move_down_inner(None));
motion_cmd!(/// Extend all selections up one line (anchor stays, head moves).
    cmd_extend_up, Extend, move_up_inner(None));

// Word motions — select the entire next/previous word (HUME model).
//
// `w` / `W` jump to the next word/WORD and select it as a fresh forward
// selection (anchor = word start, head = word end). `b` / `B` do the same
// for the previous word. This replaces Helix's "extend from current position"
// semantics: every motion re-anchors rather than growing a drag selection.
//
// `e` / `E` are removed — they were only needed to compensate for `w` landing
// on the first char of the next word. With the new model, `w` already selects
// the whole word so `e` is redundant.

/// Select the next word entirely (`w`): jump to the next word and select it
/// from its first to its last character. Re-anchors on each press; crosses
/// line boundaries. No-op at the last word in the buffer.
#[allow(non_snake_case)]
pub(crate) fn cmd_select_next_word(buf: &Buffer, sels: SelectionSet, count: usize) -> SelectionSet {
    apply_word_select(buf, sels, count, |b, pos| select_next_word(b, pos, is_word_boundary))
}

/// Select the next WORD entirely (`W`): like `w` but treats word+punct as one class.
#[allow(non_snake_case)]
pub(crate) fn cmd_select_next_WORD(buf: &Buffer, sels: SelectionSet, count: usize) -> SelectionSet {
    apply_word_select(buf, sels, count, |b, pos| select_next_word(b, pos, is_WORD_boundary))
}

/// Select the previous word entirely (`b`): jump to the previous word and
/// select it from its first to its last character. Re-anchors on each press;
/// crosses line boundaries. No-op at the first word in the buffer.
#[allow(non_snake_case)]
pub(crate) fn cmd_select_prev_word(buf: &Buffer, sels: SelectionSet, count: usize) -> SelectionSet {
    apply_word_select(buf, sels, count, |b, pos| select_prev_word(b, pos, is_word_boundary))
}

/// Select the previous WORD entirely (`B`): like `b` but treats word+punct as one class.
#[allow(non_snake_case)]
pub(crate) fn cmd_select_prev_WORD(buf: &Buffer, sels: SelectionSet, count: usize) -> SelectionSet {
    apply_word_select(buf, sels, count, |b, pos| select_prev_word(b, pos, is_WORD_boundary))
}

/// Word motions — Extend mode, union semantics: current selection ∪ next/prev word.
/// Extend selection to encompass both the current selection and the next word (`w` in extend mode).
#[allow(non_snake_case)]
pub(crate) fn cmd_extend_select_next_word(buf: &Buffer, sels: SelectionSet, count: usize) -> SelectionSet {
    apply_word_select_extend_forward(buf, sels, count, |b, pos| select_next_word(b, pos, is_word_boundary))
}
/// Extend selection to encompass both the current selection and the next WORD (`W` in extend mode).
#[allow(non_snake_case)]
pub(crate) fn cmd_extend_select_next_WORD(buf: &Buffer, sels: SelectionSet, count: usize) -> SelectionSet {
    apply_word_select_extend_forward(buf, sels, count, |b, pos| select_next_word(b, pos, is_WORD_boundary))
}
/// Extend selection to encompass both the current selection and the previous word (`b` in extend mode).
#[allow(non_snake_case)]
pub(crate) fn cmd_extend_select_prev_word(buf: &Buffer, sels: SelectionSet, count: usize) -> SelectionSet {
    apply_word_select_extend_backward(buf, sels, count, |b, pos| select_prev_word(b, pos, is_word_boundary))
}
/// Extend selection to encompass both the current selection and the previous WORD (`B` in extend mode).
#[allow(non_snake_case)]
pub(crate) fn cmd_extend_select_prev_WORD(buf: &Buffer, sels: SelectionSet, count: usize) -> SelectionSet {
    apply_word_select_extend_backward(buf, sels, count, |b, pos| select_prev_word(b, pos, is_WORD_boundary))
}

// Paragraph motions.
motion_cmd!(/// Move all cursors to the start of the next paragraph (`]p`).
    cmd_next_paragraph, Move, next_paragraph);
motion_cmd!(/// Move all cursors to the first empty line above the current paragraph (`[p`).
    cmd_prev_paragraph, Move, prev_paragraph);
motion_cmd!(/// Extend selection to the start of the next paragraph.
    cmd_extend_next_paragraph, Extend, next_paragraph);
motion_cmd!(/// Extend selection to the first empty line above the current paragraph.
    cmd_extend_prev_paragraph, Extend, prev_paragraph);

// ── Line selection motions ────────────────────────────────────────────────────

/// Select the full line (`x`): from line start to the trailing `\n` (inclusive).
/// If the selection already ends on a `\n`, jumps to the next line (replaces
/// the selection — does not extend). Always produces a forward selection.
pub(crate) fn cmd_select_line(buf: &Buffer, sels: SelectionSet) -> SelectionSet {
    let result = sels.map_and_merge(|sel| {
        let bottom_line = buf.char_to_line(sel.end());
        let end_excl = line_end_exclusive(buf, bottom_line);
        // If selection already ends on the trailing `\n`, jump to the next line.
        let target_line = if sel.end() + 1 >= end_excl && end_excl < buf.len_chars() {
            bottom_line + 1
        } else {
            buf.char_to_line(sel.start())
        };
        let start = buf.line_to_char(target_line);
        let end = line_end_exclusive(buf, target_line) - 1; // inclusive `\n`
        Selection::new(start, end)
    });
    result.debug_assert_valid(buf);
    result
}

/// Extend-mode / Ctrl+`x`: grow the selection to cover the current line.
/// If the selection already ends on a `\n`, accumulates the next line instead.
/// Always produces a forward selection (anchor=start, head=`\n`).
pub(crate) fn cmd_extend_select_line(buf: &Buffer, sels: SelectionSet) -> SelectionSet {
    let result = sels.map_and_merge(|sel| {
        let bottom_line = buf.char_to_line(sel.end());
        let end_excl = line_end_exclusive(buf, bottom_line);
        if sel.end() + 1 >= end_excl && end_excl >= buf.len_chars() {
            return sel; // already at last line — clamp
        }
        let (tgt_start, tgt_end) = if sel.end() + 1 >= end_excl {
            // Already ends on `\n` — target next line.
            let next_line = bottom_line + 1;
            (buf.line_to_char(next_line), line_end_exclusive(buf, next_line) - 1)
        } else {
            // Expand to cover full lines.
            let top_line = buf.char_to_line(sel.start());
            (buf.line_to_char(top_line), end_excl - 1)
        };
        let new_start = sel.start().min(tgt_start);
        let new_end = sel.end().max(tgt_end);
        Selection::new(new_start, new_end) // always forward
    });
    result.debug_assert_valid(buf);
    result
}

/// Select the full line backward (`X`): anchor on the trailing `\n`, head on
/// line start. If the selection already starts at a line boundary, jumps to the
/// previous line (replaces the selection — does not extend).
pub(crate) fn cmd_select_line_backward(buf: &Buffer, sels: SelectionSet) -> SelectionSet {
    let result = sels.map_and_merge(|sel| {
        let top_line = buf.char_to_line(sel.start());
        let top_line_start = buf.line_to_char(top_line);
        // If selection already starts at line start, jump to previous line.
        let target_line = if sel.start() == top_line_start && top_line > 0 {
            top_line - 1
        } else {
            top_line
        };
        let start = buf.line_to_char(target_line);
        let end = line_end_exclusive(buf, target_line) - 1; // inclusive `\n`
        Selection::new(end, start) // backward: anchor=`\n`, head=line_start
    });
    result.debug_assert_valid(buf);
    result
}

/// Extend-mode / Ctrl+`X`: grow the selection to cover the current line.
/// If the selection already starts at a line boundary, accumulates the previous
/// line. Always produces a backward selection (anchor=bottom `\n`, head=top start).
pub(crate) fn cmd_extend_select_line_backward(buf: &Buffer, sels: SelectionSet) -> SelectionSet {
    let result = sels.map_and_merge(|sel| {
        let top_line = buf.char_to_line(sel.start());
        let top_line_start = buf.line_to_char(top_line);
        if sel.start() == top_line_start && top_line == 0 {
            return sel; // already at first line — clamp
        }
        let (tgt_start, tgt_end) = if sel.start() == top_line_start {
            // Already starts at line boundary — target previous line.
            let prev_line = top_line - 1;
            (buf.line_to_char(prev_line), line_end_exclusive(buf, prev_line) - 1)
        } else {
            // Expand to cover full lines.
            let bottom_line = buf.char_to_line(sel.end());
            (top_line_start, line_end_exclusive(buf, bottom_line) - 1)
        };
        let new_start = sel.start().min(tgt_start);
        let new_end = sel.end().max(tgt_end);
        Selection::new(new_end, new_start) // backward: anchor=bottom, head=top
    });
    result.debug_assert_valid(buf);
    result
}

// ── Find/till character motions ───────────────────────────────────────────────

/// Scan forward on `head`'s line for `ch`, starting one grapheme after `head`.
///
/// Returns the char offset of the first match, or `None` if not found before
/// the line's terminating `\n`. The newline itself is never matched — it is a
/// structural boundary, not content.
fn find_char_on_line_forward(buf: &Buffer, head: usize, ch: char) -> Option<usize> {
    let line = buf.char_to_line(head);
    // Exclude the '\n': stop iteration once pos reaches the newline position.
    // The buffer always ends with '\n', so line_end_exclusive >= 1.
    let newline = line_end_exclusive(buf, line) - 1;
    let mut pos = next_grapheme_boundary(buf, head);
    while pos < newline {
        if buf.char_at(pos) == Some(ch) {
            return Some(pos);
        }
        pos = next_grapheme_boundary(buf, pos);
    }
    None
}

/// Scan backward on `head`'s line for `ch`, starting one grapheme before `head`.
///
/// Returns the char offset of the first match, or `None` if not found before
/// the line start.
fn find_char_on_line_backward(buf: &Buffer, head: usize, ch: char) -> Option<usize> {
    let line = buf.char_to_line(head);
    let line_start = buf.line_to_char(line);
    if head == line_start {
        return None; // already at line start, nothing to the left
    }
    let mut pos = prev_grapheme_boundary(buf, head);
    loop {
        if buf.char_at(pos) == Some(ch) {
            return Some(pos);
        }
        if pos == line_start {
            break;
        }
        pos = prev_grapheme_boundary(buf, pos);
    }
    None
}

/// Find the next occurrence of `ch` on the current line (forward).
///
/// `kind` controls cursor placement:
/// - `Inclusive` (`f`): cursor lands ON `ch`.
/// - `Exclusive` (`t`): cursor lands one grapheme *before* `ch`.
///   If `ch` is exactly one grapheme ahead, the adjusted position equals `head`
///   and the motion is a no-op — this matches Helix/Vim `t` behaviour.
///
/// `count` is supported via `apply_motion`'s fold: `3fa` skips to the 3rd `a`.
/// No-op per selection if `ch` is not found.
pub(crate) fn find_char_forward(
    buf: &Buffer,
    sels: SelectionSet,
    mode: MotionMode,
    count: usize,
    ch: char,
    kind: FindKind,
) -> SelectionSet {
    apply_motion(buf, sels, mode, count, |b, head| {
        match find_char_on_line_forward(b, head, ch) {
            Some(pos) => match kind {
                FindKind::Inclusive => pos,
                // Step back one grapheme from the found position. If that lands
                // back at head (char was adjacent), the motion is a no-op.
                FindKind::Exclusive => prev_grapheme_boundary(b, pos),
            },
            None => head, // not found — stay put
        }
    })
}

/// Find the previous occurrence of `ch` on the current line (backward).
///
/// `kind` controls cursor placement:
/// - `Inclusive` (`F`): cursor lands ON `ch`.
/// - `Exclusive` (`T`): cursor lands one grapheme *after* `ch` (the cursor stays
///   between the found char and its original position).
///
/// No-op per selection if `ch` is not found.
pub(crate) fn find_char_backward(
    buf: &Buffer,
    sels: SelectionSet,
    mode: MotionMode,
    count: usize,
    ch: char,
    kind: FindKind,
) -> SelectionSet {
    apply_motion(buf, sels, mode, count, |b, head| {
        match find_char_on_line_backward(b, head, ch) {
            Some(pos) => match kind {
                FindKind::Inclusive => pos,
                // Step forward one grapheme from the found position, landing
                // just after `ch` (between `ch` and the original cursor).
                FindKind::Exclusive => next_grapheme_boundary(b, pos),
            },
            None => head, // not found — stay put
        }
    })
}

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

    #[test]
    fn extend_left_reverses_direction() {
        // Forward selection anchor=3,head=3. Extend left 3 times: head→0.
        // anchor=3 > head=0 → becomes a backward selection spanning "hell".
        assert_state!("hel-[l]>o\n", |(buf, sels)| cmd_extend_left(&buf, sels, 3), "<[hell]-o\n");
    }

    #[test]
    fn extend_right_crosses_newline() {
        // Cursor on '\n' at end of first line. Extend right: head crosses newline
        // onto the first char of the next line.
        // "hello\nworld\n": '\n'=5, 'w'=6. anchor=5, head→6.
        assert_state!(
            "hello-[\n]>world\n",
            |(buf, sels)| cmd_extend_right(&buf, sels, 1),
            "hello-[\nw]>orld\n"
        );
    }

    #[test]
    fn extend_left_crosses_newline() {
        // Cursor on first char of second line. Extend left: head crosses newline
        // onto the '\n' of the previous line. "hello\nworld\n": '\n'=5, 'w'=6.
        // anchor=6 stays on 'w'; head→5 ('\n'). Backward selection covers "\nw".
        assert_state!(
            "hello\n-[w]>orld\n",
            |(buf, sels)| cmd_extend_left(&buf, sels, 1),
            "hello<[\nw]-orld\n"
        );
    }

    #[test]
    fn extend_right_multi_cursor() {
        // Two independent cursors both extend right by 2. They grow their own
        // selections without merging (ranges remain disjoint).
        // "foo bar\n": f=0,o=1,o=2,' '=3,b=4,a=5,r=6,'\n'=7.
        // cursor1 anchor=0,head=0 → head=2 → "-[foo]>"
        // cursor2 anchor=4,head=4 → head=6 → "-[bar]>"
        assert_state!(
            "-[f]>oo -[b]>ar\n",
            |(buf, sels)| cmd_extend_right(&buf, sels, 2),
            "-[foo]> -[bar]>\n"
        );
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

    // ── cmd_select_next_word (w) ──────────────────────────────────────────────

    #[test]
    fn select_next_word_basic() {
        // From 'h', selects "world" (the next word). Fresh anchor at word start.
        assert_state!("-[h]>ello world\n", |(buf, sels)| cmd_select_next_word(&buf, sels, 1), "hello -[world]>\n");
    }

    #[test]
    fn select_next_word_from_mid_word() {
        // Cursor in the middle of "hello" — still jumps to next word "world".
        assert_state!("hel-[l]>o world\n", |(buf, sels)| cmd_select_next_word(&buf, sels, 1), "hello -[world]>\n");
    }

    #[test]
    fn select_next_word_from_whitespace() {
        // From the space between words, selects the next word "world".
        assert_state!("hello-[ ]>world\n", |(buf, sels)| cmd_select_next_word(&buf, sels, 1), "hello -[world]>\n");
    }

    #[test]
    fn select_next_word_crosses_newline() {
        // w crosses the newline and selects the first word on the next line.
        assert_state!("-[h]>ello\nworld\n", |(buf, sels)| cmd_select_next_word(&buf, sels, 1), "hello\n-[world]>\n");
    }

    #[test]
    fn select_next_word_crosses_multiple_blank_lines() {
        // Multiple blank lines between words — w still reaches the next word.
        assert_state!("-[h]>ello\n\n\nworld\n", |(buf, sels)| cmd_select_next_word(&buf, sels, 1), "hello\n\n\n-[world]>\n");
    }

    #[test]
    fn select_next_word_at_last_word_is_noop() {
        // Cursor on the last word in the buffer — no-op.
        assert_state!("hello -[world]>\n", |(buf, sels)| cmd_select_next_word(&buf, sels, 1), "hello -[world]>\n");
    }

    #[test]
    fn select_next_word_at_eof_is_noop() {
        // Cursor on trailing '\n' — no-op.
        assert_state!("hello-[\n]>", |(buf, sels)| cmd_select_next_word(&buf, sels, 1), "hello-[\n]>");
    }

    #[test]
    fn select_next_word_empty_buffer_is_noop() {
        assert_state!("-[\n]>", |(buf, sels)| cmd_select_next_word(&buf, sels, 1), "-[\n]>");
    }

    #[test]
    fn select_next_word_word_to_punct() {
        // "hello" and "." are different word classes — w selects ".".
        assert_state!("-[h]>ello.world\n", |(buf, sels)| cmd_select_next_word(&buf, sels, 1), "hello-[.]>world\n");
    }

    #[test]
    fn select_next_word_punct_to_word() {
        // From ".", the next word class token is "hello".
        assert_state!("-[.]>hello\n", |(buf, sels)| cmd_select_next_word(&buf, sels, 1), ".-[hello]>\n");
    }

    #[test]
    fn select_next_word_count_2() {
        // count=2: skips "world", selects "foo".
        assert_state!("-[h]>ello world foo\n", |(buf, sels)| cmd_select_next_word(&buf, sels, 2), "hello world -[foo]>\n");
    }

    #[test]
    fn select_next_word_count_stops_at_last_word() {
        // count=3 but only 2 words remain after cursor — stops at "foo".
        assert_state!("-[h]>ello world foo\n", |(buf, sels)| cmd_select_next_word(&buf, sels, 3), "hello world -[foo]>\n");
    }

    // ── cmd_select_prev_word (b) ──────────────────────────────────────────────

    #[test]
    fn select_prev_word_basic() {
        // From "world", selects the previous word "hello".
        assert_state!("hello -[world]>\n", |(buf, sels)| cmd_select_prev_word(&buf, sels, 1), "-[hello]> world\n");
    }

    #[test]
    fn select_prev_word_from_mid_word() {
        // Cursor in the middle of "world" — jumps to previous word "hello".
        assert_state!("hello wor-[l]>d\n", |(buf, sels)| cmd_select_prev_word(&buf, sels, 1), "-[hello]> world\n");
    }

    #[test]
    fn select_prev_word_from_whitespace() {
        // From the space between words, selects the previous word "hello".
        assert_state!("hello-[ ]>world\n", |(buf, sels)| cmd_select_prev_word(&buf, sels, 1), "-[hello]> world\n");
    }

    #[test]
    fn select_prev_word_from_punct() {
        // Cursor on the '.' punctuation — selects the preceding word "hello".
        assert_state!("hello-[.]>world\n", |(buf, sels)| cmd_select_prev_word(&buf, sels, 1), "-[hello]>.world\n");
    }

    #[test]
    fn select_prev_word_from_trailing_newline() {
        // Cursor on the trailing '\n' — selects the last word on the line.
        assert_state!("hello world-[\n]>", |(buf, sels)| cmd_select_prev_word(&buf, sels, 1), "hello -[world]>\n");
    }

    #[test]
    fn select_prev_word_crosses_newline() {
        // b crosses the newline and selects the last word on the previous line.
        assert_state!("hello\n-[world]>\n", |(buf, sels)| cmd_select_prev_word(&buf, sels, 1), "-[hello]>\nworld\n");
    }

    #[test]
    fn select_prev_word_at_first_word_is_noop() {
        // Cursor on first word — no-op.
        assert_state!("-[hello]> world\n", |(buf, sels)| cmd_select_prev_word(&buf, sels, 1), "-[hello]> world\n");
    }

    #[test]
    fn select_prev_word_in_first_word_mid_is_noop() {
        // Cursor in the middle of the first word — no previous word, no-op.
        assert_state!("hel-[l]>o world\n", |(buf, sels)| cmd_select_prev_word(&buf, sels, 1), "hel-[l]>o world\n");
    }

    #[test]
    fn select_prev_word_at_buffer_start_is_noop() {
        assert_state!("-[h]>ello\n", |(buf, sels)| cmd_select_prev_word(&buf, sels, 1), "-[h]>ello\n");
    }

    #[test]
    fn select_prev_word_empty_buffer_is_noop() {
        assert_state!("-[\n]>", |(buf, sels)| cmd_select_prev_word(&buf, sels, 1), "-[\n]>");
    }

    #[test]
    fn select_prev_word_count_2() {
        // count=2: from "foo", skips "world", selects "hello".
        assert_state!("hello world -[foo]>\n", |(buf, sels)| cmd_select_prev_word(&buf, sels, 2), "-[hello]> world foo\n");
    }

    #[test]
    fn select_prev_word_count_overshoots() {
        // count=5 but only 2 words precede "foo" — stops at "hello" rather than erroring.
        assert_state!("hello world -[foo]>\n", |(buf, sels)| cmd_select_prev_word(&buf, sels, 5), "-[hello]> world foo\n");
    }

    // ── WORD variants (W / B) ─────────────────────────────────────────────────

    #[test]
    fn select_next_WORD_skips_punct() {
        // W: "hello.world" is a single WORD — W selects it entirely.
        assert_state!("-[h]>ello.world bar\n", |(buf, sels)| cmd_select_next_WORD(&buf, sels, 1), "hello.world -[bar]>\n");
    }

    #[test]
    fn select_next_WORD_crosses_newline() {
        // W at end of a line crosses the newline and selects the first WORD on the next line.
        assert_state!("-[h]>ello.world\nbar\n", |(buf, sels)| cmd_select_next_WORD(&buf, sels, 1), "hello.world\n-[bar]>\n");
    }

    #[test]
    fn select_next_word_stops_at_punct() {
        // w (lowercase): "hello" and "." are separate word-class tokens.
        assert_state!("-[h]>ello.world bar\n", |(buf, sels)| cmd_select_next_word(&buf, sels, 1), "hello-[.]>world bar\n");
    }

    #[test]
    fn select_prev_WORD_skips_punct() {
        // B: from "bar", jumps back over "hello.world" as ONE WORD (the dot is not
        // a WORD boundary), selecting the whole token.
        assert_state!("hello.world -[bar]>\n", |(buf, sels)| cmd_select_prev_WORD(&buf, sels, 1), "-[hello.world]> bar\n");
    }

    #[test]
    fn select_prev_WORD_crosses_newline() {
        // B at the start of a line crosses the newline and selects the last WORD on the previous line.
        assert_state!("hello.world\n-[bar]>\n", |(buf, sels)| cmd_select_prev_WORD(&buf, sels, 1), "-[hello.world]>\nbar\n");
    }

    // ── grapheme cluster correctness ──────────────────────────────────────────

    #[test]
    fn select_next_word_skips_combining_grapheme() {
        // Buffer: "cafe\u{0301} world\n" — graphemes: {c}{a}{f}{e◌́}{ }{w}{o}{r}{l}{d}{\n}
        // The combining codepoint U+0301 (offset 4) must not create a false word
        // boundary inside the grapheme cluster {e◌́}. w selects "world".
        assert_state!(
            "-[c]>afe\u{0301} world\n",
            |(buf, sels)| cmd_select_next_word(&buf, sels, 1),
            "cafe\u{0301} -[world]>\n"
        );
    }

    #[test]
    fn select_prev_word_skips_combining_grapheme() {
        // Buffer: "cafe\u{0301} world\n", cursor on 'w'.
        // b must step over the combining grapheme {e◌́} as a unit (Word class)
        // and select all of "cafe\u{0301}" as one word.
        assert_state!(
            "cafe\u{0301} -[w]>orld\n",
            |(buf, sels)| cmd_select_prev_word(&buf, sels, 1),
            "-[cafe\u{0301}]> world\n"
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
    fn select_next_word_multi_cursor() {
        // Two cursors: each independently selects the next word from its position.
        // Cursor 1 at 'h'(0): next word is "foo"(6..8).
        // Cursor 2 at 'f'(6): next word is "bar"(10..12).
        assert_state!(
            "-[h]>ello -[f]>oo bar\n",
            |(buf, sels)| cmd_select_next_word(&buf, sels, 1),
            "hello -[foo]> -[bar]>\n"
        );
    }

    #[test]
    fn select_prev_word_multi_cursor() {
        // Two cursors each jump to the previous word independently.
        // Cursor 1 on "hello" (head=8) → prev word "foo" → [0,2].
        // Cursor 2 on "world" (head=14) → prev word "hello" → [4,8].
        // No merging because [0,2] and [4,8] are disjoint.
        assert_state!(
            "foo -[hello]> -[world]> bar\n",
            |(buf, sels)| cmd_select_prev_word(&buf, sels, 1),
            "-[foo]> -[hello]> world bar\n"
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
    fn prev_paragraph_empty_buffer() {
        assert_state!("-[\n]>", |(buf, sels)| cmd_prev_paragraph(&buf, sels, 1), "-[\n]>");
    }

    // ── extend line-start / line-end / first-nonblank ─────────────────────────

    #[test]
    fn extend_line_start_from_mid_line() {
        // Cursor on 'l' in "hello"; extend to line start: anchor stays at 'l', head at 'h'.
        assert_state!(
            "hel-[l]>o\n",
            |(buf, sels)| cmd_extend_line_start(&buf, sels, 1),
            "<[hell]-o\n"
        );
    }

    #[test]
    fn extend_line_start_already_at_start() {
        // Already at line start — no-op.
        assert_state!(
            "-[h]>ello\n",
            |(buf, sels)| cmd_extend_line_start(&buf, sels, 1),
            "-[h]>ello\n"
        );
    }

    #[test]
    fn extend_line_end_from_start() {
        // Cursor on 'h'; extend to end: anchor stays at 'h', head at 'o'.
        assert_state!(
            "-[h]>ello\n",
            |(buf, sels)| cmd_extend_line_end(&buf, sels, 1),
            "-[hello]>\n"
        );
    }

    #[test]
    fn extend_line_end_already_at_end() {
        // Already at line end — no-op.
        assert_state!(
            "hell-[o]>\n",
            |(buf, sels)| cmd_extend_line_end(&buf, sels, 1),
            "hell-[o]>\n"
        );
    }

    #[test]
    fn extend_first_nonblank_from_mid_line() {
        // Cursor on 'l'; extend to first nonblank 'h': backward extension.
        assert_state!(
            "hel-[l]>o\n",
            |(buf, sels)| cmd_extend_first_nonblank(&buf, sels, 1),
            "<[hell]-o\n"
        );
    }

    #[test]
    fn extend_first_nonblank_from_indent() {
        // Buffer "  hello\n" (2 spaces), cursor at ' '(0); extend to 'h'(2).
        // anchor stays at 0, head = 2 → selection covers "  h".
        // Serialized with ]> after head: "-[  h]>ello\n".
        assert_state!(
            "-[ ]> hello\n",
            |(buf, sels)| cmd_extend_first_nonblank(&buf, sels, 1),
            "-[  h]>ello\n"
        );
    }

    // ── extend_select word motions (union semantics) ──────────────────────────

    #[test]
    fn extend_select_next_word_from_cursor() {
        // From a collapsed cursor at 'h', extend-w unions cursor pos with next word.
        // select_next_word from pos 0 jumps to "world" (6,10).
        // Union: min(0,6)=0, max(0,10)=10 → selection (0,10) = "hello world".
        assert_state!(
            "-[h]>ello world foo\n",
            |(buf, sels)| cmd_extend_select_next_word(&buf, sels, 1),
            "-[hello world]> foo\n"
        );
    }

    #[test]
    fn extend_select_next_word_grows_selection() {
        // Start with "world" selected via `w`; extend-w unions with "foo".
        // s1 = "world" (6,10); motion from pos 10 → "foo" (12,14).
        // Union: min(6,12)=6, max(10,14)=14 → "world foo".
        assert_state!(
            "-[h]>ello world foo\n",
            |(buf, sels)| {
                let s1 = cmd_select_next_word(&buf, sels, 1); // selects "world" (6,10)
                cmd_extend_select_next_word(&buf, s1, 1)       // union with "foo" (12,14)
            },
            "hello -[world foo]>\n"
        );
    }

    #[test]
    fn extend_select_prev_word_extends_backward() {
        // Start with "world" selected via `w`; extend-b unions with "hello".
        // s1 = "world" (6,10); backward motion from start()=6 → "hello" (0,4).
        // Union: min(6,0)=0, max(10,4)=10 → "hello world".
        assert_state!(
            "-[h]>ello world\n",
            |(buf, sels)| {
                let s1 = cmd_select_next_word(&buf, sels, 1); // selects "world" (6,10)
                cmd_extend_select_prev_word(&buf, s1, 1)       // union with "hello" (0,4)
            },
            "-[hello world]>\n"
        );
    }

    #[test]
    fn extend_select_prev_word_from_multi_word_selection() {
        // Regression: from a multi-word selection "-[bar baz]>", pressing extend-b
        // should grow backward to include "foo", not be a no-op.
        //
        // Bug: old code used sel.head (=end of "baz") as motion origin.
        // select_prev_word from inside the selection found "baz" itself → union was
        // a no-op. Fix: backward variant uses sel.start() as origin, which is at
        // the start of "bar", so select_prev_word finds "foo".
        //
        // "foo bar baz\n": f=0,o=1,o=2,' '=3,b=4,a=5,r=6,' '=7,b=8,a=9,z=10,'\n'=11
        // "-[bar baz]>" = anchor=4, head=10; start()=4, end()=10.
        // select_prev_word(buf, start()=4) → "foo" at (0,2).
        // Union: min(4,0)=0, max(10,2)=10 → (0,10) = "foo bar baz".
        assert_state!(
            "foo -[bar baz]>\n",
            |(buf, sels)| cmd_extend_select_prev_word(&buf, sels, 1),
            "-[foo bar baz]>\n"
        );
    }

    #[test]
    fn extend_select_next_word_at_buffer_end_is_noop() {
        // From a selection covering the only word in the buffer, extend-w finds
        // no next word (only '\n' remains) and leaves the selection unchanged.
        assert_state!(
            "-[hello]>\n",
            |(buf, sels)| cmd_extend_select_next_word(&buf, sels, 1),
            "-[hello]>\n"
        );
    }

    #[test]
    fn extend_select_prev_word_at_buffer_start_is_noop() {
        // The selection starts at pos 0; there is no previous word. Noop.
        assert_state!(
            "-[hello]> world\n",
            |(buf, sels)| cmd_extend_select_prev_word(&buf, sels, 1),
            "-[hello]> world\n"
        );
    }

    #[test]
    fn extend_select_next_word_multi_cursor() {
        // Two cursors each independently union with the next word. Because
        // select_next_word skips the word under the cursor and returns the
        // *following* word, each cursor unites with the word after its current one.
        //
        // "foo bar baz qux\n": f=0..2,' '=3,b=4..6,' '=7,b=8..10,' '=11,q=12..14
        // cursor1 at 'f'(0): end()=0, select_next_word → "bar"(4,6). union(0,0,4,6)=(0,6)="foo bar".
        // cursor2 at 'b'(8): end()=8, select_next_word → "qux"(12,14). union(8,8,12,14)=(8,14)="baz qux".
        // Results (0,6) and (8,14) are disjoint — no merge.
        assert_state!(
            "-[f]>oo bar -[b]>az qux\n",
            |(buf, sels)| cmd_extend_select_next_word(&buf, sels, 1),
            "-[foo bar]> -[baz qux]>\n"
        );
    }

    // ── cmd_select_line / cmd_select_line_backward ────────────────────────────

    #[test]
    fn select_line_from_mid_line() {
        // Cursor mid-line → select full line forward.
        assert_state!(
            "hello -[w]>orld\nfoo\n",
            |(buf, sels)| cmd_select_line(&buf, sels),
            "-[hello world\n]>foo\n"
        );
    }

    #[test]
    fn select_line_already_full_line_jumps_to_next() {
        // Selection already covers full line → jump to next line.
        assert_state!(
            "-[hello world\n]>foo\n",
            |(buf, sels)| cmd_select_line(&buf, sels),
            "hello world\n-[foo\n]>"
        );
    }

    #[test]
    fn select_line_clamps_at_last_line() {
        // Already on last line → no change.
        assert_state!(
            "hello\n-[foo\n]>",
            |(buf, sels)| cmd_select_line(&buf, sels),
            "hello\n-[foo\n]>"
        );
    }

    #[test]
    fn select_line_backward_from_mid_line() {
        // Cursor mid-line → select full line backward (anchor=`\n`, head=start).
        assert_state!(
            "hello -[w]>orld\nfoo\n",
            |(buf, sels)| cmd_select_line_backward(&buf, sels),
            "<[hello world\n]-foo\n"
        );
    }

    #[test]
    fn select_line_backward_already_at_start_jumps_to_prev() {
        // Selection already starts at line boundary → jump to previous line.
        assert_state!(
            "aaa\n<[bbb\n]-ccc\n",
            |(buf, sels)| cmd_select_line_backward(&buf, sels),
            "<[aaa\n]-bbb\nccc\n"
        );
    }

    #[test]
    fn select_line_backward_clamps_at_first_line() {
        // Already on first line → no change.
        assert_state!(
            "<[hello\n]-world\n",
            |(buf, sels)| cmd_select_line_backward(&buf, sels),
            "<[hello\n]-world\n"
        );
    }

    // ── cmd_extend_select_line / cmd_extend_select_line_backward ─────────────

    #[test]
    fn extend_select_line_accumulates_downward() {
        // Each press accumulates one more line.
        assert_state!(
            "-[hello\n]>foo\nbar\n",
            |(buf, sels)| cmd_extend_select_line(&buf, sels),
            "-[hello\nfoo\n]>bar\n"
        );
    }

    #[test]
    fn extend_select_line_clamps_at_last_line() {
        // Already at last line → no change.
        assert_state!(
            "hello\n-[foo\n]>",
            |(buf, sels)| cmd_extend_select_line(&buf, sels),
            "hello\n-[foo\n]>"
        );
    }

    #[test]
    fn extend_select_line_backward_accumulates_upward() {
        // Each press accumulates one more line upward.
        assert_state!(
            "aaa\n<[bbb\n]-ccc\n",
            |(buf, sels)| cmd_extend_select_line_backward(&buf, sels),
            "<[aaa\nbbb\n]-ccc\n"
        );
    }

    #[test]
    fn extend_select_line_backward_clamps_at_first_line() {
        // Already at first line → no change.
        assert_state!(
            "<[hello\n]-world\n",
            |(buf, sels)| cmd_extend_select_line_backward(&buf, sels),
            "<[hello\n]-world\n"
        );
    }

    #[test]
    fn extend_select_line_from_mid_line() {
        // Starting from a partial selection, the first extend covers the full line.
        assert_state!(
            "hello -[w]>orld\nfoo\n",
            |(buf, sels)| cmd_extend_select_line(&buf, sels),
            "-[hello world\n]>foo\n"
        );
    }

    #[test]
    fn extend_select_line_backward_from_mid_line() {
        // Starting from a partial selection, the first backward extend covers the full line.
        assert_state!(
            "hello -[w]>orld\nfoo\n",
            |(buf, sels)| cmd_extend_select_line_backward(&buf, sels),
            "<[hello world\n]-foo\n"
        );
    }

    #[test]
    fn select_line_empty_line() {
        // A bare `\n` line: the cursor is already on the only character (the `\n`),
        // so `x` immediately jumps to the next line.
        assert_state!(
            "hello\n-[\n]>world\n",
            |(buf, sels)| cmd_select_line(&buf, sels),
            "hello\n\n-[world\n]>"
        );
    }

    #[test]
    fn select_line_backward_empty_line() {
        // A bare `\n` line: cursor is at line start → `X` jumps to the previous line.
        assert_state!(
            "hello\n-[\n]>world\n",
            |(buf, sels)| cmd_select_line_backward(&buf, sels),
            "<[hello\n]-\nworld\n"
        );
    }

    #[test]
    fn select_line_multi_cursor() {
        // Two cursors on different lines each independently select their full line.
        // The resulting line selections are non-overlapping and stay separate.
        assert_state!(
            "hello -[w]>orld\nfoo -[b]>ar\nbaz\n",
            |(buf, sels)| cmd_select_line(&buf, sels),
            "-[hello world\n]>-[foo bar\n]>baz\n"
        );
    }

    #[test]
    fn select_line_multi_cursor_same_line_merges() {
        // Two cursors on the same line both produce identical line selections,
        // which map_and_merge collapses to a single selection.
        assert_state!(
            "hell-[o]> -[w]>orld\nfoo\n",
            |(buf, sels)| cmd_select_line(&buf, sels),
            "-[hello world\n]>foo\n"
        );
    }

    #[test]
    fn extend_select_line_multi_cursor_merges() {
        // Two adjacent full-line selections each extend to the next line; because the
        // resulting ranges overlap, map_and_merge unifies them into one selection.
        //
        // sel1 (-[hello world\n]>) end=11 → extends to line 1 → (0,15)
        // sel2 (-[foo\n]>)         end=15 → extends to line 2 → (12,19)
        // (0,15) and (12,19) overlap → merged to (0,19)
        assert_state!(
            "-[hello world\n]>-[foo\n]>bar\n",
            |(buf, sels)| cmd_extend_select_line(&buf, sels),
            "-[hello world\nfoo\nbar\n]>"
        );
    }

    // ── find_char_forward / find_char_backward ────────────────────────────────

    // Helper wrappers with fixed mode so assert_state! closures stay tidy.
    fn fwd(buf: Buffer, sels: SelectionSet, ch: char, kind: FindKind) -> SelectionSet {
        find_char_forward(&buf, sels, MotionMode::Move, 1, ch, kind)
    }
    fn bwd(buf: Buffer, sels: SelectionSet, ch: char, kind: FindKind) -> SelectionSet {
        find_char_backward(&buf, sels, MotionMode::Move, 1, ch, kind)
    }
    fn fwd_ext(buf: Buffer, sels: SelectionSet, ch: char, kind: FindKind) -> SelectionSet {
        find_char_forward(&buf, sels, MotionMode::Extend, 1, ch, kind)
    }
    fn fwd_count(buf: Buffer, sels: SelectionSet, ch: char, kind: FindKind, n: usize) -> SelectionSet {
        find_char_forward(&buf, sels, MotionMode::Move, n, ch, kind)
    }

    #[test]
    fn find_forward_inclusive_basic() {
        // Cursor on 'h'; `fa` jumps to the first 'a'.
        assert_state!(
            "-[h]>ello a world\n",
            |(buf, sels)| fwd(buf, sels, 'a', FindKind::Inclusive),
            "hello -[a]> world\n"
        );
    }

    #[test]
    fn find_forward_inclusive_first_char_on_line() {
        // Target is the very last content char.
        assert_state!(
            "-[h]>ello\n",
            |(buf, sels)| fwd(buf, sels, 'o', FindKind::Inclusive),
            "hell-[o]>\n"
        );
    }

    #[test]
    fn find_forward_inclusive_not_found() {
        // No 'z' on this line — no-op.
        assert_state!(
            "-[h]>ello\n",
            |(buf, sels)| fwd(buf, sels, 'z', FindKind::Inclusive),
            "-[h]>ello\n"
        );
    }

    #[test]
    fn find_forward_does_not_cross_newline() {
        // 'a' appears only on the second line — the motion must not cross '\n'.
        assert_state!(
            "-[h]>ello\nabc\n",
            |(buf, sels)| fwd(buf, sels, 'a', FindKind::Inclusive),
            "-[h]>ello\nabc\n"
        );
    }

    #[test]
    fn find_forward_skips_char_under_cursor() {
        // Cursor is already on 'a'; `fa` should find the *next* 'a', not the current one.
        assert_state!(
            "-[a]>bc a def\n",
            |(buf, sels)| fwd(buf, sels, 'a', FindKind::Inclusive),
            "abc -[a]> def\n"
        );
    }

    #[test]
    fn find_forward_exclusive_basic() {
        // `ta` stops one grapheme before 'a' — the space is one grapheme before 'a'.
        assert_state!(
            "-[h]>ello a world\n",
            |(buf, sels)| fwd(buf, sels, 'a', FindKind::Exclusive),
            "hello-[ ]>a world\n"
        );
    }

    #[test]
    fn find_forward_exclusive_adjacent_is_noop() {
        // 'a' is the immediately next grapheme; exclusive adjustment lands back at head.
        assert_state!(
            "-[h]>a world\n",
            |(buf, sels)| fwd(buf, sels, 'a', FindKind::Exclusive),
            "-[h]>a world\n"
        );
    }

    #[test]
    fn find_forward_count() {
        // `2fa` jumps to the second 'a'.
        assert_state!(
            "-[h]>a ba\n",
            |(buf, sels)| fwd_count(buf, sels, 'a', FindKind::Inclusive, 2),
            "ha b-[a]>\n"
        );
    }

    #[test]
    fn find_backward_inclusive_basic() {
        // `Fa` finds the previous 'a'.
        assert_state!(
            "hello a worl-[d]>\n",
            |(buf, sels)| bwd(buf, sels, 'a', FindKind::Inclusive),
            "hello -[a]> world\n"
        );
    }

    #[test]
    fn find_backward_inclusive_not_found() {
        assert_state!(
            "hell-[o]>\n",
            |(buf, sels)| bwd(buf, sels, 'z', FindKind::Inclusive),
            "hell-[o]>\n"
        );
    }

    #[test]
    fn find_backward_does_not_cross_newline() {
        // 'z' is only on the first line; cursor on second line must not find it.
        assert_state!(
            "z\n-[a]>bc\n",
            |(buf, sels)| bwd(buf, sels, 'z', FindKind::Inclusive),
            "z\n-[a]>bc\n"
        );
    }

    #[test]
    fn find_backward_exclusive_basic() {
        // `Ta` stops one grapheme after 'a' (cursor is between 'a' and its original pos).
        assert_state!(
            "hello a worl-[d]>\n",
            |(buf, sels)| bwd(buf, sels, 'a', FindKind::Exclusive),
            "hello a-[ ]>world\n"
        );
    }

    #[test]
    fn find_backward_exclusive_adjacent_is_noop() {
        // Cursor is immediately right of 'a'; exclusive adjustment steps forward
        // from the found position back to head — so the motion is a no-op,
        // symmetric to the forward exclusive adjacent case.
        assert_state!(
            "hello a-[x]>\n",
            |(buf, sels)| bwd(buf, sels, 'a', FindKind::Exclusive),
            "hello a-[x]>\n"
        );
    }

    #[test]
    fn find_forward_extend_mode() {
        // Extend mode: anchor stays, head moves to found char.
        assert_state!(
            "-[h]>ello a\n",
            |(buf, sels)| fwd_ext(buf, sels, 'a', FindKind::Inclusive),
            "-[hello a]>\n"
        );
    }

    #[test]
    fn find_forward_multi_cursor() {
        // Two cursors on the same line each find their own next 'a'.
        // cursor1 at 'h'(0) → next 'a' at 1.
        // cursor2 at 'a'(4) → skips it, next 'a' at 8.
        assert_state!(
            "-[h]>a b-[a]> c a\n",
            |(buf, sels)| fwd(buf, sels, 'a', FindKind::Inclusive),
            "h-[a]> ba c -[a]>\n"
        );
    }

    #[test]
    fn find_backward_at_line_start_noop() {
        // Cursor at line start — nothing to the left, no-op.
        assert_state!(
            "-[h]>ello\n",
            |(buf, sels)| bwd(buf, sels, 'x', FindKind::Inclusive),
            "-[h]>ello\n"
        );
    }
}
