use hume_editing::grapheme::{next_grapheme_boundary, prev_grapheme_boundary};
use hume_editing::text::BufferText;

use crate::pair::matching_bracket;
use crate::tag::matching_tag;

/// `%`-style jump: cursor on a bracket goes to its partner; cursor anywhere
/// inside a tag's own markup (`<name…>` or `</name>`) goes to its partner
/// tag's `<`; anything else is a no-op. Brackets are tried first since a
/// single-character bracket match is cheaper to rule out than a tag scan.
///
/// Both scans return a raw char offset with no grapheme awareness (matching
/// a single-char delimiter can't itself land mid-cluster, but a tag's
/// `lt_pos` can sit right after a `GC_Prepend` codepoint that joins forward
/// into the following cluster). Snap once here, where both converge:
/// `next` then `prev` is a no-op on an already-boundary offset and pulls a
/// mid-cluster one back to where its cluster starts.
pub(super) fn goto_matching_pair(text: &BufferText, pos: usize) -> usize {
    let target = matching_bracket(text, pos)
        .or_else(|| matching_tag(text, pos))
        .unwrap_or(pos);
    prev_grapheme_boundary(text, next_grapheme_boundary(text, target))
}
