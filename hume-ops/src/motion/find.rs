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

/// Which press of a find/till this is. A repeated till steps over a match
/// adjacent to the head, since the first press already stopped against it.
#[derive(Clone, Copy)]
enum Press {
    First,
    Repeat,
}

/// How many clusters next to `head` a scan passes over before matching: a
/// repeated till passes over the adjacent one.
fn adjacent_skipped(kind: FindKind, press: Press) -> usize {
    match (kind, press) {
        (FindKind::Exclusive, Press::Repeat) => 1,
        _ => 0,
    }
}

/// The `n`th match of `ch` after `head` on its line, 1-based, with the
/// first `skipped` clusters after `head` passed over. The line's `\n` is
/// never matched: it is a structural boundary, not content.
fn nth_char_on_line_forward(
    text: &BufferText,
    head: ClusterStart,
    ch: char,
    n: usize,
    skipped: usize,
) -> Option<ClusterStart> {
    let newline = line_break(text, text.char_to_line(head.offset()));
    graphemes_at(text, head.into())
        .skip(1 + skipped)
        .take_while(|cluster| cluster.start() < newline)
        .filter(|&cluster| cluster_is(text, cluster, ch))
        .nth(n.checked_sub(1)?)
        .map(|cluster| cluster.start())
}

/// The `n`th match of `ch` before `head` on its line, nearest first,
/// 1-based, with the first `skipped` clusters before `head` passed over.
fn nth_char_on_line_backward(
    text: &BufferText,
    head: ClusterStart,
    ch: char,
    n: usize,
    skipped: usize,
) -> Option<ClusterStart> {
    let first = line_start(text, text.char_to_line(head.offset()));
    clusters_before(text, head.into())
        .skip(skipped)
        .take_while(|cluster| cluster.start() >= first)
        .filter(|&cluster| cluster_is(text, cluster, ch))
        .nth(n.checked_sub(1)?)
        .map(|cluster| cluster.start())
}

fn find_forward(
    state: EditState,
    count: usize,
    mode: MotionMode,
    ch: char,
    kind: FindKind,
    press: Press,
) -> EditState {
    let skipped = adjacent_skipped(kind, press);
    apply_motion(state, mode, 1, |s| {
        let head = s.head();
        match nth_char_on_line_forward(s.text(), head, ch, count, skipped) {
            Some(pos) => match kind {
                FindKind::Inclusive => pos,
                FindKind::Exclusive => prev_cluster(s.text(), pos.into()).unwrap_or(pos),
            },
            None => head,
        }
    })
}

fn find_backward(
    state: EditState,
    count: usize,
    mode: MotionMode,
    ch: char,
    kind: FindKind,
    press: Press,
) -> EditState {
    let skipped = adjacent_skipped(kind, press);
    apply_motion(state, mode, 1, |s| {
        let head = s.head();
        match nth_char_on_line_backward(s.text(), head, ch, count, skipped) {
            Some(pos) => match kind {
                FindKind::Inclusive => pos,
                FindKind::Exclusive => next_cluster(s.text(), pos).unwrap_or(pos),
            },
            None => head,
        }
    })
}

/// Move to the `count`th occurrence of `ch` after the head on its line.
///
/// `kind` controls cursor placement:
/// - `Inclusive` (`f`): cursor lands ON `ch`.
/// - `Exclusive` (`t`): cursor lands one grapheme *before* `ch`. A match
///   adjacent to the head counts, so `ta` with `a` next to the cursor is a
///   no-op and `2ta` lands before the `a` after it, as in Vim.
///
/// No-op per selection if there are fewer than `count` matches.
pub fn find_char_forward(
    state: EditState,
    count: usize,
    mode: MotionMode,
    ch: char,
    kind: FindKind,
) -> EditState {
    find_forward(state, count, mode, ch, kind, Press::First)
}

/// Move to the `count`th occurrence of `ch` before the head on its line.
///
/// `kind` controls cursor placement:
/// - `Inclusive` (`F`): cursor lands ON `ch`.
/// - `Exclusive` (`T`): cursor lands one grapheme *after* `ch`. An adjacent
///   match counts, as in [`find_char_forward`].
///
/// No-op per selection if there are fewer than `count` matches.
pub fn find_char_backward(
    state: EditState,
    count: usize,
    mode: MotionMode,
    ch: char,
    kind: FindKind,
) -> EditState {
    find_backward(state, count, mode, ch, kind, Press::First)
}

/// [`find_char_forward`] for a repeated find. A till passes over a match
/// adjacent to the head, so repeating `ta` reaches the next `a` instead of
/// staying against the one it stopped at.
pub fn repeat_find_char_forward(
    state: EditState,
    count: usize,
    mode: MotionMode,
    ch: char,
    kind: FindKind,
) -> EditState {
    find_forward(state, count, mode, ch, kind, Press::Repeat)
}

/// [`find_char_backward`] for a repeated find, passing over an adjacent
/// match the way [`repeat_find_char_forward`] does.
pub fn repeat_find_char_backward(
    state: EditState,
    count: usize,
    mode: MotionMode,
    ch: char,
    kind: FindKind,
) -> EditState {
    find_backward(state, count, mode, ch, kind, Press::Repeat)
}
