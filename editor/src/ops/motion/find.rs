use super::{FindKind, MotionMode, apply_motion};
use crate::core::grapheme::{next_grapheme_boundary, prev_grapheme_boundary};
use crate::core::selection::SelectionSet;
use crate::core::text::Text;
use crate::helpers::line_end_exclusive;

// ── Find/till character motions ───────────────────────────────────────────────

/// Scan forward on `head`'s line for `ch`, starting one grapheme after `head`.
///
/// Returns the char offset of the first match, or `None` if not found before
/// the line's terminating `\n`. The newline itself is never matched — it is a
/// structural boundary, not content.
pub(super) fn find_char_on_line_forward(buf: &Text, head: usize, ch: char) -> Option<usize> {
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
pub(super) fn find_char_on_line_backward(buf: &Text, head: usize, ch: char) -> Option<usize> {
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
    buf: &Text,
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
    buf: &Text,
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
