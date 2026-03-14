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
}
