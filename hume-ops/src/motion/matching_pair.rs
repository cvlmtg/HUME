use hume_editing::grapheme::snap_to_cluster_start;
use hume_editing::selection::Selection;
use hume_editing::text::BufferText;
use hume_rope::offset::CharOffset;

use crate::pair::matching_bracket;
use crate::tag::matching_tag;

/// `%`-style jump: a bracket anywhere in `sel` goes to its partner; a cursor
/// inside a tag's markup goes to the partner tag's `<`; anything else is a
/// no-op. Tags resolve from the head only, because the tag scan is unbounded
/// per position.
///
/// Neither scan is grapheme-aware, so a hit can land inside a cluster (e.g.
/// after a `GC_Prepend` codepoint) and is snapped to its start.
pub(super) fn goto_matching_pair(text: &BufferText, sel: &Selection) -> CharOffset {
    matching_bracket(text, *sel)
        .or_else(|| matching_tag(text, sel.head()))
        .map(|target| snap_to_cluster_start(text, target))
        .unwrap_or(sel.head())
}
