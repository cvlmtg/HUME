use hume_rope::offset::{CharOffset, ExclusiveRange};
use pretty_assertions::assert_eq;

use super::*;
use crate::marked::{parse, render};
use crate::selection::{Facing, UnboundSelection};

fn co(n: usize) -> CharOffset {
    CharOffset::new(n)
}

fn range(a: usize, b: usize) -> ExclusiveRange<CharOffset> {
    ExclusiveRange::new(co(a), co(b))
}

/// `f`'s plan over `input`, its one result selection covering `f`'s mark.
fn marked(
    input: &str,
    f: impl for<'a, 'id> FnOnce(&mut EditBuilder<'a, 'id>) -> Mark<'id>,
) -> String {
    let state = parse(input);
    let edited = edit(&state, |b| {
        let mark = f(b);
        Landings::new(vec![Landing::covering(mark, Facing::Forward)], 0)
    });
    render(edited.state().view())
}

/// `f`'s plan over `input`, its one result selection a cursor at `f`'s position.
fn cursor_at(
    input: &str,
    f: impl for<'a, 'id> FnOnce(&mut EditBuilder<'a, 'id>) -> NewPos<'id>,
) -> String {
    let state = parse(input);
    let edited = edit(&state, |b| {
        let at = f(b);
        Landings::new(vec![Landing::cursor(at)], 0)
    });
    render(edited.state().view())
}

#[test]
fn insert_returns_what_the_insertion_became() {
    assert_eq!(
        marked("-[a]>bc\n", |b| b.insert(co(1), "XY")),
        "a-[XY]>bc\n"
    );
}

#[test]
fn an_insert_inside_a_deletion_lands_at_the_deletion_point() {
    let out = marked("-[a]>bcd\n", |b| {
        b.delete(range(0, 2));
        b.insert(co(1), "X")
    });
    assert_eq!(out, "-[X]>cd\n");
}

#[test]
fn ops_recorded_out_of_order_apply_in_position_order() {
    let out = marked("-[a]>bcd\n", |b| {
        let later = b.insert(co(3), "Y");
        b.insert(co(1), "X");
        later
    });
    assert_eq!(out, "aXbc-[Y]>d\n");
}

#[test]
fn inserts_at_one_position_keep_their_call_order() {
    let out = marked("-[a]>bc\n", |b| {
        b.insert(co(1), "X");
        b.insert(co(1), "Y")
    });
    assert_eq!(out, "aX-[Y]>bc\n");
}

#[test]
fn overlapping_deletions_remove_their_union() {
    let out = cursor_at("-[a]>bcdef\n", |b| {
        b.delete(range(1, 4));
        b.delete(range(3, 5))
    });
    assert_eq!(out, "a-[f]>\n");
}

#[test]
fn a_deletion_never_reaches_the_structural_break() {
    let out = cursor_at("-[a]>bc\n", |b| b.delete(range(1, 99)));
    assert_eq!(out, "a-[\n]>");
}

#[test]
fn text_without_a_final_break_lands_before_the_structural_one() {
    assert_eq!(marked("-[a]>b\n", |b| b.insert(co(3), "X")), "ab-[X]>\n");
}

#[test]
fn lines_inserted_at_the_end_go_after_the_last_line() {
    assert_eq!(
        marked("-[a]>b\n", |b| b.insert(co(3), "X\n")),
        "ab\n-[X\n]>"
    );
}

#[test]
#[should_panic(expected = "past the text end")]
fn an_insert_past_the_text_end_is_refused() {
    marked("-[a]>b\n", |b| b.insert(co(4), "X"));
}

#[test]
fn replacing_through_the_structural_break_keeps_it() {
    assert_eq!(
        marked("-[a]>bc\n", |b| b.replace(range(1, 4), "XY\n")),
        "a-[XY\n]>"
    );
    assert_eq!(
        marked("-[a]>bc\n", |b| b.replace(range(1, 4), "XY")),
        "a-[XY]>\n"
    );
}

#[test]
fn replacing_after_lines_were_inserted_at_the_end_keeps_the_structural_break_before_them() {
    let out = marked("-[a]>b\n", |b| {
        b.insert(co(3), "X\n");
        b.replace(range(1, 3), "Y\n")
    });
    assert_eq!(out, "a-[Y\n]>X\n");
}

#[test]
fn keep_returns_what_a_range_became() {
    let state = parse("-[ab]>cd\n");
    let covered = state.view().primary().covered();
    let edited = edit(&state, |b| {
        b.insert(co(0), "X");
        let mark = b.keep(covered);
        Landings::new(vec![Landing::covering(mark, Facing::Backward)], 0)
    });
    assert_eq!(render(edited.state().view()), "X<[ab]-cd\n");
}

#[test]
fn kept_and_old_landings_map_through_the_plan() {
    let state = parse("a-[b]>c\n");
    let sel = state.view().primary().selection();
    let edited = edit(&state, |b| {
        b.insert(co(0), "XY");
        Landings::new(vec![UnboundSelection::kept(sel).into()], 0)
    });
    assert_eq!(render(edited.state().view()), "XYa-[b]>c\n");
}

#[test]
fn touching_linewise_removals_at_the_end_leave_no_blank_line() {
    let state = parse("a\n-[b\n]>-[c\n]>");
    assert_eq!(state.view().iter().count(), 2, "two touching selections");
    let edited = edit(&state, |b| {
        let items = state
            .view()
            .iter()
            .map(|sel| b.remove(sel).expect("removable").cursor)
            .collect();
        Landings::new(items, 0)
    });
    assert_eq!(edited.state().text().to_string(), "a\n");
    assert_eq!(render(edited.state().view()), "-[a]>\n");
}

#[test]
fn a_linewise_removal_of_the_last_line_takes_the_break_before_it() {
    let state = parse("a\n-[b\n]>");
    let edited = edit(&state, |b| {
        let removed = b.remove(state.view().primary()).expect("removable");
        Landings::new(vec![removed.cursor], 0)
    });
    assert_eq!(render(edited.state().view()), "-[a]>\n");
}

#[test]
fn a_linewise_removal_before_the_last_line_lands_on_the_next_line() {
    let state = parse("a\n-[b\n]>c\n");
    let edited = edit(&state, |b| {
        let removed = b.remove(state.view().primary()).expect("removable");
        Landings::new(vec![removed.cursor], 0)
    });
    assert_eq!(render(edited.state().view()), "a\n-[c]>\n");
}

#[test]
fn ops_at_distinct_positions_give_one_changeset_in_any_recording_order() {
    let state = parse("-[a]>bcdefgh\n");
    let record: [fn(&mut EditBuilder<'_, '_>); 4] = [
        |b| {
            b.insert(co(1), "X");
        },
        |b| {
            b.insert(co(3), "Y\n");
        },
        |b| {
            b.delete(range(4, 5));
        },
        |b| {
            b.replace(range(6, 7), "Z");
        },
    ];
    let mut seen: Option<String> = None;
    for order in permutations(record.len()) {
        let edited = edit(&state, |b| {
            for &i in &order {
                record[i](b);
            }
            Landings::new(
                vec![UnboundSelection::kept(state.view().primary().selection()).into()],
                0,
            )
        });
        let outcome = format!("{:?}", edited.changes());
        match &seen {
            None => seen = Some(outcome),
            Some(first) => assert_eq!(&outcome, first, "order {order:?}"),
        }
    }
}

fn permutations(n: usize) -> Vec<Vec<usize>> {
    if n == 0 {
        return vec![vec![]];
    }
    let mut all = Vec::new();
    for rest in permutations(n - 1) {
        for at in 0..=rest.len() {
            let mut order = rest.clone();
            order.insert(at, n - 1);
            all.push(order);
        }
    }
    all
}

#[test]
#[should_panic(expected = "no cluster ends at the text start")]
fn a_cursor_ending_at_the_text_start_is_refused() {
    let state = parse("-[a]>b\n");
    edit(&state, |b| {
        let empty = b.insert(co(0), "");
        Landings::new(vec![Landing::cursor_ending_at(empty.end())], 0)
    });
}

#[test]
fn a_cursor_lands_on_the_cluster_holding_its_position() {
    // The inserted mark joins the `a` before it.
    let out = cursor_at("-[a]>b\n", |b| b.insert(co(1), "\u{301}").start());
    assert_eq!(out, "-[a\u{301}]>b\n");
}

#[test]
fn cursor_ending_at_takes_the_cluster_before_the_position() {
    let state = parse("-[a]>b\n");
    let edited = edit(&state, |b| {
        let mark = b.insert(co(1), "e\u{301}");
        Landings::new(vec![Landing::cursor_ending_at(mark.end())], 0)
    });
    assert_eq!(render(edited.state().view()), "a-[e\u{301}]>b\n");
}

#[test]
fn covering_an_empty_mark_is_a_cursor_at_it() {
    let out = marked("-[a]>b\n", |b| b.insert(co(1), ""));
    assert_eq!(out, "a-[b]>\n");
}
