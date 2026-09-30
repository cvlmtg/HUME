use pretty_assertions::assert_eq;

use super::*;
use crate::marked::parse;

/// The anchor and head of the primary selection of `input`.
fn ends(input: &str) -> Selection {
    parse(input).view().primary().selection()
}

#[test]
fn a_cursor_faces_forward_and_is_a_cursor() {
    let cursor = ends("a-[b]>c\n");
    assert!(cursor.is_cursor());
    assert_eq!(cursor.facing(), Facing::Forward);
    assert_eq!(cursor.anchor(), cursor.head());
}

#[test]
fn facing_follows_which_end_is_the_head() {
    let forward = ends("-[abc]>\n");
    let backward = ends("<[abc]-\n");
    assert_eq!(forward.facing(), Facing::Forward);
    assert_eq!(backward.facing(), Facing::Backward);
    assert_eq!(forward.anchor(), backward.head());
    assert_eq!(forward.head(), backward.anchor());
}

#[test]
fn flip_swaps_the_ends_and_clears_the_sticky_column() {
    let col = StickyDisplayCol::BufferLine {
        display_col: BufferLineCol::new(3),
    };
    let sel = ends("-[abc]>\n").with_sticky(col);
    let flipped = sel.flip();
    assert_eq!(
        (flipped.anchor(), flipped.head()),
        (sel.head(), sel.anchor())
    );
    assert_eq!(flipped.sticky_display_col(), None);
    assert_eq!(sel.sticky_display_col(), Some(col));
}

#[test]
fn collapsing_keeps_the_chosen_end() {
    let sel = ends("-[abc]>\n");
    assert_eq!(sel.to_head(), Selection::cursor(sel.head()));
    assert_eq!(sel.to_anchor(), Selection::cursor(sel.anchor()));
}

#[test]
fn with_head_keeps_the_anchor() {
    let sel = ends("-[abc]>\n");
    let moved = sel.with_head(sel.anchor());
    assert_eq!(moved, Selection::cursor(sel.anchor()));
}

#[test]
fn covering_puts_the_head_at_the_facing_end() {
    let state = parse("x-[ye\u{301}]>z\n");
    let range = state.view().primary().covered();
    let forward = Selection::covering(range, Facing::Forward);
    let backward = Selection::covering(range, Facing::Backward);
    assert_eq!(
        (forward.anchor(), forward.head()),
        (range.start(), range.last())
    );
    assert_eq!(
        (backward.anchor(), backward.head()),
        (range.last(), range.start())
    );
}
