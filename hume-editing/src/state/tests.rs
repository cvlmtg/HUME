use hume_rope::offset::CharOffset;
use pretty_assertions::assert_eq;

use super::*;
use crate::marked::{parse, render};

fn shown(state: &EditState) -> String {
    render(state.view())
}

#[test]
fn bind_pairs_a_set_with_its_own_text() {
    let state = parse("a-[b]>c\n");
    let (text, set) = (state.text().clone(), state.clone().into_selections());
    let rebound = EditState::bind(&text, set);
    assert_eq!(shown(&rebound), "a-[b]>c\n");
    assert_eq!(rebound.view().check(), Ok(()));
}

#[test]
fn with_cursor_and_at_text_start() {
    let text = BufferText::from("ab\n");
    let second = text.snap(CharOffset::new(1));
    assert_eq!(
        shown(&EditState::with_cursor(text.clone(), second)),
        "a-[b]>\n"
    );
    assert_eq!(shown(&EditState::at_text_start(text)), "-[a]>b\n");
}

#[test]
fn from_cluster_indices_counts_clusters_and_wraps() {
    let text = BufferText::from("ae\u{301}b\n");
    let state = EditState::from_cluster_indices(text, &[(1, 2), (7, 7)], 1);
    assert_eq!(shown(&state), "a-[e\u{301}b]>-{\n}>");
    assert_eq!(state.view().primary().index(), 1);
}

#[test]
fn map_replaces_each_selection_and_merges() {
    let state = parse("-{a}>b-[c]>d\n");
    let collapsed = state.map(|v| Selection::cursor(v.text().snap(CharOffset::new(0))));
    assert_eq!(shown(&collapsed), "-[a]>bcd\n");
}

#[test]
fn flat_map_can_split_and_drop_selections() {
    let state = parse("-{ab}>c-[d]>\n");
    let split = state.flat_map(|v| {
        if v.index() == 0 {
            v.clusters()
                .map(|c| Selection::cursor(c.start()))
                .collect::<Vec<_>>()
        } else {
            Vec::new()
        }
    });
    assert_eq!(shown(&split), "-{a}>-[b]>cd\n");
    assert_eq!(split.view().primary().index(), 0);
}

#[test]
fn flat_map_moves_the_primary_to_the_first_selection_it_produced() {
    let state = parse("-{a}>b-[cd]>\n").cycle_primary(1);
    let split = state.flat_map(|v| {
        v.clusters()
            .map(|c| Selection::cursor(c.start()))
            .collect::<Vec<_>>()
    });
    assert_eq!(split.view().primary().start().offset(), CharOffset::new(2));
}

#[test]
fn keep_primary_drops_the_others() {
    let state = parse("-{a}>b-[c]>d\n").cycle_primary(1).keep_primary();
    assert_eq!(shown(&state), "ab-[c]>d\n");
}

#[test]
fn replace_primary_merges_with_its_neighbours() {
    let state = parse("-{a}>b-[c]>d\n");
    let text = state.text().clone();
    let wide = Selection::new(text.snap(CharOffset::new(0)), text.snap(CharOffset::new(2)));
    assert_eq!(shown(&state.replace_primary(wide)), "-[abc]>d\n");
}

#[test]
fn remove_moves_the_primary_along() {
    let three = || parse("-{a}>b-[c]>d-[e]>\n");
    let primary = |s: EditState| s.view().primary().index();
    assert_eq!(primary(three().cycle_primary(1).remove(0)), 0);
    assert_eq!(primary(three().cycle_primary(1).remove(1)), 1);
    assert_eq!(primary(three().cycle_primary(2).remove(2)), 0);
    assert_eq!(primary(three().remove(2)), 0);
    assert_eq!(shown(&parse("-[a]>\n").remove(0)), "-[a]>\n");
}

#[test]
fn cycle_primary_wraps_both_ways() {
    let primary = |delta| {
        parse("-{a}>b-[c]>d-[e]>\n")
            .cycle_primary(delta)
            .view()
            .primary()
            .index()
    };
    assert_eq!(primary(1), 1);
    assert_eq!(primary(3), 0);
    assert_eq!(primary(-1), 2);
    assert_eq!(primary(-4), 2);
}

#[test]
fn all_cursors_is_false_once_one_selection_has_extent() {
    assert!(parse("-{a}>b-[c]>\n").view().all_cursors());
    assert!(!parse("-{a}>b-[cd]>\n").view().all_cursors());
}
