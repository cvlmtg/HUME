use hume_editing::grapheme::snap_to_cluster_start;
use hume_editing::selection::Selection;
use hume_editing::text::BufferText;

use crate::pair::matching_bracket;
use crate::tag::matching_tag;

/// `%`-style jump: a bracket anywhere in `sel` goes to its partner; cursor
/// anywhere inside a tag's own markup (`<name…>` or `</name>`) goes to its
/// partner tag's `<`; anything else is a no-op. Brackets are tried first
/// since a bracket match is cheaper to rule out than a tag scan.
///
/// Brackets resolve against the whole selection, nearest the head — a
/// `w`-motion selection like `") "` leaves the head on the trailing space,
/// not the bracket itself, and pressing `#` should still jump. Tags resolve
/// from the head alone: `matching_tag`'s backward tag-boundary scan is
/// unbounded per position, so probing every position in the selection would
/// be a per-keystroke cost proportional to selection length rather than a
/// single fixed-cost scan.
///
/// Neither scan is grapheme-aware — both return a raw char offset that can
/// sit right after a `GC_Prepend` codepoint joining forward into the
/// following cluster (e.g. `(\u{0600})`: matching the `(` lands one char
/// into the cluster the `)` belongs to). Snap once here, where both
/// converge, and only on an actual hit — the no-op path already sits on a
/// grapheme boundary by the buffer invariant, so snapping it is a
/// guaranteed-identity round trip through two `GraphemeCursor` runs, wasted
/// on the most common outcome of a mis-pressed `#`.
pub(super) fn goto_matching_pair(text: &BufferText, sel: &Selection) -> usize {
    matching_bracket(text, sel)
        .or_else(|| matching_tag(text, sel.head()))
        .map(|target| snap_to_cluster_start(text, target))
        .unwrap_or(sel.head())
}
