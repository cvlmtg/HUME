use hume_rope::cluster::ClusterStart;
use hume_rope::column::BufferLineCol;
use hume_rope::offset::CharOffset;
use pretty_assertions::assert_eq;

use super::*;
use crate::changeset::{ChangeSet, ChangeSetBuilder};
use crate::marked::{parse, render};
use crate::state::EditState;

fn co(n: usize) -> CharOffset {
    CharOffset::new(n)
}

/// The cluster starting at char `n` of an ASCII test text.
fn at(state: &EditState, n: usize) -> ClusterStart {
    let pos = state.text().snap(co(n));
    assert_eq!(pos.offset(), co(n), "char {n} is not a cluster start");
    pos
}

/// Every latch here is `BufferLine`: these tests pin merge and translation,
/// not the variants, which `hume-ops` and `hume-editor` cover.
fn sticky(display_col: u32) -> StickyDisplayCol {
    StickyDisplayCol::BufferLine {
        display_col: BufferLineCol::new(display_col),
    }
}

/// `state` with the selections `ends` names as `(anchor, head)` char pairs,
/// the first primary.
fn with(state: EditState, ends: &[(usize, usize)]) -> EditState {
    let sels = ends
        .iter()
        .map(|&(a, h)| Selection::new(at(&state, a), at(&state, h)))
        .collect();
    state.with_selections(sels, 0)
}

fn shown(state: &EditState) -> String {
    render(state.view())
}

// ── Normalizing ───────────────────────────────────────────────────────────

#[test]
fn disjoint_selections_stay_apart() {
    let state = with(parse("-[a]>bcdefghij\n"), &[(0, 3), (5, 8)]);
    assert_eq!(shown(&state), "-[abcd]>e-[fghi]>j\n");
}

#[test]
fn overlapping_selections_merge() {
    let state = with(parse("-[a]>bcdefghij\n"), &[(0, 5), (3, 8)]);
    assert_eq!(shown(&state), "-[abcdefghi]>j\n");
}

#[test]
fn selections_sharing_their_edge_cluster_merge() {
    let state = with(parse("-[a]>bcdefghij\n"), &[(0, 3), (3, 6)]);
    assert_eq!(shown(&state), "-[abcdefg]>hij\n");
}

#[test]
fn selections_touching_without_sharing_a_cluster_stay_apart() {
    let state = with(parse("-[a]>bcdefghij\n"), &[(0, 2), (3, 6)]);
    assert_eq!(shown(&state), "-[abc]>-[defg]>hij\n");
}

#[test]
fn duplicate_and_contained_selections_merge() {
    let state = with(parse("-[a]>bcdefghij\n"), &[(2, 5), (2, 5)]);
    assert_eq!(shown(&state), "ab-[cdef]>ghij\n");
    let state = with(parse("-[a]>bcdefghij\n"), &[(0, 8), (2, 5)]);
    assert_eq!(shown(&state), "-[abcdefghi]>j\n");
}

#[test]
fn three_chained_selections_merge_into_one() {
    let state = with(parse("-[a]>bcdefghij\n"), &[(0, 3), (2, 6), (5, 9)]);
    assert_eq!(shown(&state), "-[abcdefghij]>\n");
}

#[test]
fn a_merge_extended_by_a_backward_selection_faces_backward() {
    let state = with(parse("-[a]>bcdefghij\n"), &[(0, 3), (6, 2)]);
    assert_eq!(shown(&state), "<[abcdefg]-hij\n");
}

#[test]
fn unsorted_selections_are_sorted() {
    let state = with(parse("-[a]>bcdefghij\n"), &[(6, 7), (0, 1)]);
    assert_eq!(shown(&state), "-[ab]>cdef-[gh]>ij\n");
    assert_eq!(state.view().primary().start(), at(&state, 6));
}

#[test]
fn a_merge_that_extends_a_selection_clears_its_sticky_column() {
    let state = parse("-[a]>bcdefghij\n");
    let a = Selection::new(at(&state, 0), at(&state, 5)).with_sticky(sticky(42));
    let b = Selection::new(at(&state, 3), at(&state, 8)).with_sticky(sticky(99));
    let merged = state.with_selections(vec![a, b], 0);
    assert_eq!(merged.view().len(), 1);
    assert_eq!(
        merged.view().primary().selection().sticky_display_col(),
        None
    );
}

#[test]
fn the_primary_follows_the_selection_that_holds_it_through_a_merge() {
    let state = parse("-[a]>bcdefghij\n");
    let sels = vec![
        Selection::new(at(&state, 8), at(&state, 9)),
        Selection::new(at(&state, 0), at(&state, 2)),
        Selection::new(at(&state, 1), at(&state, 4)),
    ];
    let merged = state.with_selections(sels, 2);
    assert_eq!(merged.view().primary().start(), at(&merged, 0));
    assert_eq!(merged.view().primary().last(), at(&merged, 4));
}

#[test]
#[should_panic(expected = "must not be empty")]
fn an_empty_selection_list_panics() {
    parse("-[a]>\n")
        .with_selections(Vec::new(), 0)
        .into_selections();
}

#[test]
#[should_panic(expected = "primary index out of bounds")]
fn a_primary_past_the_list_panics() {
    let state = parse("-[a]>\n");
    let cursor = Selection::cursor(at(&state, 0));
    state.with_selections(vec![cursor], 1).into_selections();
}

// ── Translating through an edit ───────────────────────────────────────────

fn replace(text: &crate::text::BufferText, from: usize, to: usize, with: &str) -> ChangeSet {
    let mut b = ChangeSetBuilder::new(text.end());
    b.retain(from);
    b.delete(to - from);
    b.insert(with);
    b.retain_rest();
    b.finish()
}

fn translate(state: &EditState, cs: &ChangeSet) -> EditState {
    let post = cs
        .apply(state.text())
        .expect("changeset built for the text");
    let mut set = state.clone().into_selections();
    set.translate(&crate::edit::TextChange::new(state.text(), &post, cs));
    assert_eq!(set.version(), post.version());
    EditState::bind(&post, set)
}

#[test]
fn translation_moves_selections_and_keeps_sticky_columns_only_on_untouched_lines() {
    let state = parse("aaa\nbbb\nccc\n-[x]>\n");
    let sels = vec![
        Selection::cursor(at(&state, 1)).with_sticky(sticky(5)),
        Selection::new(at(&state, 5), at(&state, 6)).with_sticky(sticky(9)),
        Selection::cursor(at(&state, 9)).with_sticky(sticky(7)),
    ];
    let state = state.with_selections(sels, 0);
    let cs = replace(state.text(), 4, 7, "XY");

    let out = translate(&state, &cs);

    assert_eq!(shown(&out), "a-[a]>a\n-[X]>Y\nc-[c]>c\nx\n");
    let stickies: Vec<_> = out
        .view()
        .iter()
        .map(|v| v.selection().sticky_display_col())
        .collect();
    assert_eq!(stickies, vec![Some(sticky(5)), None, Some(sticky(7))]);
}

#[test]
fn an_insertion_at_a_line_start_touches_that_line() {
    let state = parse("-[a]>a\nbb\n");
    let sels = vec![
        Selection::cursor(at(&state, 1)).with_sticky(sticky(5)),
        Selection::cursor(at(&state, 4)).with_sticky(sticky(9)),
    ];
    let state = state.with_selections(sels, 0);
    let out = translate(&state, &replace(state.text(), 3, 3, "X"));
    let stickies: Vec<_> = out
        .view()
        .iter()
        .map(|v| v.selection().sticky_display_col())
        .collect();
    assert_eq!(stickies, vec![Some(sticky(5)), None]);
}

#[test]
fn a_backward_selection_stays_backward_through_a_translation() {
    let state = parse("a<[bcd]-e\n");
    let out = translate(&state, &replace(state.text(), 0, 0, "XX"));
    assert_eq!(shown(&out), "XXa<[bcd]-e\n");
}

#[test]
fn selections_folded_onto_one_point_merge() {
    let state = parse("a-[b]>cd-[e]>f\n");
    let out = translate(&state, &replace(state.text(), 0, 6, ""));
    assert_eq!(shown(&out), "-[\n]>");
}

#[test]
fn a_position_left_inside_a_cluster_lands_on_its_start() {
    // Deleting the line break joins the lone mark to the `e`: the cursor on
    // the mark lands on the new cluster's start.
    let state = parse("e\n-[\u{301}]>x\n");
    let out = translate(&state, &replace(state.text(), 1, 2, ""));
    assert_eq!(shown(&out), "-[e\u{301}]>x\n");
}
