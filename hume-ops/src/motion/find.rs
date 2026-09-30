use super::{FindKind, MotionMode, apply_motion};
use hume_editing::grapheme::{graphemes_at, next_grapheme_boundary, prev_grapheme_boundary};
use hume_editing::lines::line_break_char;
use hume_editing::selection::{Selection, SelectionSet};
use hume_editing::text::BufferText;
use hume_rope::grapheme::Cluster;
use hume_rope::offset::{CharOffset, ExclusiveRange};
use unicode_normalization::UnicodeNormalization;

// ── Find/till character motions ───────────────────────────────────────────────

/// Whether `cluster` is the character `ch`: the cluster's text and `ch` are
/// equal after NFC normalization, so a typed `é` finds both `é` and `e` +
/// U+0301, and a typed `e` skips the accented one. A single ASCII char on
/// both sides answers without allocating.
fn cluster_is(text: &BufferText, cluster: Cluster, ch: char) -> bool {
    let single_char = cluster.end == cluster.start.shift(1);
    if single_char && cluster.first.is_ascii() && ch.is_ascii() {
        return cluster.first == ch;
    }
    text.slice(ExclusiveRange::new(cluster.start, cluster.end))
        .to_string()
        .nfc()
        .eq(std::iter::once(ch).nfc())
}

/// Scan forward on `head`'s line for `ch`, starting one grapheme after `head`.
///
/// Returns the char offset of the first match, or `None` if not found before
/// the line's terminating `\n`. The newline itself is never matched: it is a
/// structural boundary, not content.
pub(super) fn find_char_on_line_forward(
    text: &BufferText,
    head: CharOffset,
    ch: char,
) -> Option<CharOffset> {
    let line = text.char_to_line(head);
    // Exclude the '\n': stop iteration once pos reaches the newline position.
    let newline = line_break_char(text, line);
    graphemes_at(text, head)
        .skip(1)
        .take_while(|cluster| cluster.start < newline)
        .find(|&cluster| cluster_is(text, cluster, ch))
        .map(|cluster| cluster.start)
}

/// Scan backward on `head`'s line for `ch`, starting one grapheme before `head`.
///
/// Returns the char offset of the first match, or `None` if not found before
/// the line start.
pub(super) fn find_char_on_line_backward(
    text: &BufferText,
    head: CharOffset,
    ch: char,
) -> Option<CharOffset> {
    let line = text.char_to_line(head);
    let line_start = text.line_to_char(line.into());
    if head == line_start {
        return None; // already at line start, nothing to the left
    }
    let mut pos = prev_grapheme_boundary(text, head);
    loop {
        let cluster = graphemes_at(text, pos)
            .next()
            .expect("pos < head lies inside the buffer");
        if cluster_is(text, cluster, ch) {
            return Some(pos);
        }
        if pos == line_start {
            break;
        }
        pos = prev_grapheme_boundary(text, pos);
    }
    None
}

/// Find the next occurrence of `ch` on the current line (forward).
///
/// `kind` controls cursor placement:
/// - `Inclusive` (`f`): cursor lands ON `ch`.
/// - `Exclusive` (`t`): cursor lands one grapheme *before* `ch`.
///   If `ch` is exactly one grapheme ahead, the adjusted position equals `head`
///   and the motion is a no-op. This matches Helix/Vim `t` behaviour.
///
/// `count` is supported via `apply_motion`'s fold: `3fa` skips to the 3rd `a`.
/// No-op per selection if `ch` is not found.
pub fn find_char_forward(
    text: &BufferText,
    sels: SelectionSet,
    count: usize,
    mode: MotionMode,
    ch: char,
    kind: FindKind,
) -> SelectionSet {
    apply_motion(text, sels, mode, count, |b, s: &Selection| {
        let head = s.head();
        match find_char_on_line_forward(b, head, ch) {
            Some(pos) => match kind {
                FindKind::Inclusive => pos,
                // Step back one grapheme from the found position. If that lands
                // back at head (char was adjacent), the motion is a no-op.
                FindKind::Exclusive => prev_grapheme_boundary(b, pos),
            },
            None => head, // not found, stay put
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
pub fn find_char_backward(
    text: &BufferText,
    sels: SelectionSet,
    count: usize,
    mode: MotionMode,
    ch: char,
    kind: FindKind,
) -> SelectionSet {
    apply_motion(text, sels, mode, count, |b, s: &Selection| {
        let head = s.head();
        match find_char_on_line_backward(b, head, ch) {
            Some(pos) => match kind {
                FindKind::Inclusive => pos,
                // Step forward one grapheme from the found position, landing
                // just after `ch` (between `ch` and the original cursor).
                FindKind::Exclusive => next_grapheme_boundary(b, pos),
            },
            None => head, // not found, stay put
        }
    })
}
