use hume_editing::grapheme::snap_to_cluster_start;
use hume_editing::text::BufferText;

use crate::pair::matching_bracket;
use crate::tag::matching_tag;

/// `%`-style jump: cursor on a bracket goes to its partner; cursor anywhere
/// inside a tag's own markup (`<name…>` or `</name>`) goes to its partner
/// tag's `<`; anything else is a no-op. Brackets are tried first since a
/// single-character bracket match is cheaper to rule out than a tag scan.
///
/// Neither scan is grapheme-aware — both return a raw char offset that can
/// sit right after a `GC_Prepend` codepoint joining forward into the
/// following cluster (e.g. `(\u{0600})`: matching the `(` lands one char
/// into the cluster the `)` belongs to). Snap once here, where both
/// converge, and only on an actual hit — the no-op path already sits on a
/// grapheme boundary by the buffer invariant, so snapping it is a
/// guaranteed-identity round trip through two `GraphemeCursor` runs, wasted
/// on the most common outcome of a mis-pressed `#`.
pub(super) fn goto_matching_pair(text: &BufferText, pos: usize) -> usize {
    matching_bracket(text, pos)
        .or_else(|| matching_tag(text, pos))
        .map(|target| snap_to_cluster_start(text, target))
        .unwrap_or(pos)
}
