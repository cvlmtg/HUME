use hume_editing::text::BufferText;

use crate::pair::matching_bracket;
use crate::tag::matching_tag;

/// `%`-style jump: cursor on a bracket goes to its partner; cursor anywhere
/// inside a tag's own markup (`<name…>` or `</name>`) goes to its partner
/// tag's `<`; anything else is a no-op. Brackets are tried first since a
/// single-character bracket match is cheaper to rule out than a tag scan.
pub(super) fn goto_matching_pair(text: &BufferText, pos: usize) -> usize {
    matching_bracket(text, pos)
        .or_else(|| matching_tag(text, pos))
        .unwrap_or(pos)
}
