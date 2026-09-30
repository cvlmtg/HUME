use hume_editing::selection::SelectionView;
use hume_rope::cluster::ClusterStart;

use crate::pair::matching_bracket;
use crate::tag::matching_tag;

/// `%`-style jump: a bracket anywhere in `sel` goes to its partner; a cursor
/// inside a tag's markup goes to the partner tag's `<`; anything else is a
/// no-op. Tags resolve from the head only, because the tag scan is unbounded
/// per position.
pub(super) fn goto_matching_pair(sel: SelectionView<'_>) -> ClusterStart {
    matching_bracket(sel)
        .or_else(|| matching_tag(sel.text(), sel.head()))
        .unwrap_or(sel.head())
}
