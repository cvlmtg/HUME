use super::{FindKind, MotionMode, apply_motion};
use hume_editing::grapheme::{clusters_before, graphemes_at, next_cluster, prev_cluster};
use hume_editing::lines::{line_break, line_start};
use hume_editing::state::EditState;
use hume_editing::text::BufferText;
use hume_rope::cluster::ClusterStart;
use hume_rope::grapheme::Cluster;
use unicode_normalization::UnicodeNormalization;

// ── Find/till character motions ───────────────────────────────────────────────

/// Whether `cluster` is the character `ch`: the cluster's text and `ch` are
/// equal after NFC normalization, so a typed `é` finds both `é` and `e` +
/// U+0301, and a typed `e` skips the accented one. A single ASCII char on
/// both sides answers without allocating.
fn cluster_is(text: &BufferText, cluster: Cluster, ch: char) -> bool {
    let chars = cluster.range().chars();
    let single_char = chars.end.chars_since(chars.start) == 1;
    if single_char && cluster.first().is_ascii() && ch.is_ascii() {
        return cluster.first() == ch;
    }
    text.slice(chars)
        .to_string()
        .nfc()
        .eq(std::iter::once(ch).nfc())
}

/// Scan forward on `head`'s line for `ch`, starting one grapheme after `head`.
///
/// Returns the first match, or `None` if not found before the line's
/// terminating `\n`. The newline itself is never matched: it is a structural
/// boundary, not content.
pub(super) fn find_char_on_line_forward(
    text: &BufferText,
    head: ClusterStart,
    ch: char,
) -> Option<ClusterStart> {
    let newline = line_break(text, text.char_to_line(head.offset()));
    graphemes_at(text, head.into())
        .skip(1)
        .take_while(|cluster| cluster.start() < newline)
        .find(|&cluster| cluster_is(text, cluster, ch))
        .map(|cluster| cluster.start())
}

/// Scan backward on `head`'s line for `ch`, starting one grapheme before
/// `head`. Returns the first match, or `None` if not found before the line
/// start.
pub(super) fn find_char_on_line_backward(
    text: &BufferText,
    head: ClusterStart,
    ch: char,
) -> Option<ClusterStart> {
    let first = line_start(text, text.char_to_line(head.offset()));
    clusters_before(text, head.into())
        .take_while(|cluster| cluster.start() >= first)
        .find(|&cluster| cluster_is(text, cluster, ch))
        .map(|cluster| cluster.start())
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
    state: EditState,
    count: usize,
    mode: MotionMode,
    ch: char,
    kind: FindKind,
) -> EditState {
    apply_motion(state, mode, count, |s| {
        let head = s.head();
        match find_char_on_line_forward(s.text(), head, ch) {
            Some(pos) => match kind {
                FindKind::Inclusive => pos,
                // One cluster back from the match. If that is the head (the
                // char was adjacent), the motion is a no-op.
                FindKind::Exclusive => prev_cluster(s.text(), pos.into()).unwrap_or(pos),
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
    state: EditState,
    count: usize,
    mode: MotionMode,
    ch: char,
    kind: FindKind,
) -> EditState {
    apply_motion(state, mode, count, |s| {
        let head = s.head();
        match find_char_on_line_backward(s.text(), head, ch) {
            Some(pos) => match kind {
                FindKind::Inclusive => pos,
                // One cluster past the match, between `ch` and the cursor.
                FindKind::Exclusive => next_cluster(s.text(), pos).unwrap_or(pos),
            },
            None => head, // not found, stay put
        }
    })
}
