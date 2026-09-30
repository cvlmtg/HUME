use pretty_assertions::assert_eq;

use super::*;
use crate::marked::{bound_at, clusters, parse, render};
use crate::selection::{Facing, UnboundSelection};

/// `f`'s edit of `input`, its one result selection covering `f`'s mark.
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

/// `f`'s edit of `input`, its one result selection a cursor at `f`'s position.
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
        marked("-[a]>bc\n", |b| b.insert(bound_at(b.text(), 1), "XY")),
        "a-[XY]>bc\n"
    );
}

#[test]
fn an_insert_inside_a_deletion_lands_at_the_deletion_point() {
    let out = marked("-[a]>bcd\n", |b| {
        b.delete(clusters(b.text(), 0, 2));
        b.insert(bound_at(b.text(), 1), "X")
    });
    assert_eq!(out, "-[X]>cd\n");
}

#[test]
fn ops_recorded_out_of_order_apply_in_position_order() {
    let out = marked("-[a]>bcd\n", |b| {
        let later = b.insert(bound_at(b.text(), 3), "Y");
        b.insert(bound_at(b.text(), 1), "X");
        later
    });
    assert_eq!(out, "aXbc-[Y]>d\n");
}

#[test]
fn inserts_at_one_position_keep_their_call_order() {
    let out = marked("-[a]>bc\n", |b| {
        b.insert(bound_at(b.text(), 1), "X");
        b.insert(bound_at(b.text(), 1), "Y")
    });
    assert_eq!(out, "aX-[Y]>bc\n");
}

#[test]
fn overlapping_deletions_remove_their_union() {
    let out = cursor_at("-[a]>bcdef\n", |b| {
        b.delete(clusters(b.text(), 1, 4));
        b.delete(clusters(b.text(), 3, 5))
    });
    assert_eq!(out, "a-[f]>\n");
}

#[test]
fn a_deletion_never_reaches_the_structural_break() {
    let out = cursor_at("-[a]>bc\n", |b| b.delete(clusters(b.text(), 1, 99)));
    assert_eq!(out, "a-[\n]>");
}

#[test]
fn text_without_a_final_break_lands_before_the_structural_one() {
    assert_eq!(
        marked("-[a]>b\n", |b| b.insert(bound_at(b.text(), 3), "X")),
        "ab-[X]>\n"
    );
}

#[test]
fn lines_inserted_at_the_end_go_after_the_last_line() {
    assert_eq!(
        marked("-[a]>b\n", |b| b.insert(bound_at(b.text(), 3), "X\n")),
        "ab\n-[X\n]>"
    );
}

#[test]
#[should_panic(expected = "past the text end")]
fn an_insert_past_the_text_end_is_refused() {
    let longer = BufferText::from("abcdef\n");
    marked("-[a]>b\n", |b| {
        b.insert(hume_rope::grapheme::text_end(longer.full_slice()), "X")
    });
}

#[test]
#[should_panic(expected = "past the text end")]
fn a_deletion_past_the_text_end_is_refused() {
    let longer = BufferText::from("abcdef\n");
    let range = clusters(&longer, 2, 6);
    cursor_at("-[a]>b\n", |b| b.delete(range));
}

#[test]
fn replacing_through_the_structural_break_keeps_it() {
    assert_eq!(
        marked("-[a]>bc\n", |b| b.replace(clusters(b.text(), 1, 4), "XY\n")),
        "a-[XY\n]>"
    );
    assert_eq!(
        marked("-[a]>bc\n", |b| b.replace(clusters(b.text(), 1, 4), "XY")),
        "a-[XY]>\n"
    );
}

#[test]
fn replacing_after_lines_were_inserted_at_the_end_keeps_the_structural_break_before_them() {
    let out = marked("-[a]>b\n", |b| {
        b.insert(bound_at(b.text(), 3), "X\n");
        b.replace(clusters(b.text(), 1, 3), "Y\n")
    });
    assert_eq!(out, "a-[Y\n]>X\n");
}

#[test]
fn keep_returns_what_a_range_became() {
    let state = parse("-[ab]>cd\n");
    let covered = state.view().primary().covered();
    let edited = edit(&state, |b| {
        b.insert(bound_at(b.text(), 0), "X");
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
        b.insert(bound_at(b.text(), 0), "XY");
        Landings::new(vec![UnboundSelection::kept(sel).into()], 0)
    });
    assert_eq!(render(edited.state().view()), "XYa-[b]>c\n");
}

#[test]
fn an_anchor_inside_a_replaced_range_resolves_to_the_deletion_point() {
    let out = cursor_at("-[a]>bcd\n", |b| {
        b.replace(clusters(b.text(), 1, 3), "XY");
        b.at(bound_at(b.text(), 2))
    });
    assert_eq!(out, "a-[X]>Yd\n");
}

#[test]
fn a_kept_cursor_inside_a_replaced_range_lands_where_an_anchor_does() {
    let state = parse("ab-[c]>d\n");
    let sel = state.view().primary().selection();
    let edited = edit(&state, |b| {
        b.replace(clusters(b.text(), 1, 3), "XY");
        Landings::new(vec![Landing::kept(sel)], 0)
    });
    assert_eq!(render(edited.state().view()), "a-[X]>Yd\n");
}

/// The text `input` becomes when the clusters covering chars `a..b` of it
/// are deleted, for each `(a, b)`.
fn text_after_deleting(input: &str, ranges: &[(usize, usize)]) -> String {
    let state = parse(input);
    let edited = edit(&state, |b| {
        let items = ranges
            .iter()
            .map(|&(a, e)| Landing::cursor(b.delete(clusters(b.text(), a, e))))
            .collect();
        Landings::new(items, 0)
    });
    edited.state().text().to_string()
}

#[test]
fn a_deletion_of_the_structural_break_alone_changes_nothing() {
    assert_eq!(text_after_deleting("-[a]>b\n", &[(2, 3)]), "ab\n");
}

#[test]
fn deleting_the_only_break_of_an_empty_text_changes_nothing() {
    assert_eq!(text_after_deleting("-[\n]>", &[(0, 1)]), "\n");
}

#[test]
fn text_inserted_at_the_end_of_an_empty_text_lands_before_its_break() {
    assert_eq!(
        marked("-[\n]>", |b| b.insert(bound_at(b.text(), 1), "x")),
        "-[x]>\n"
    );
}

#[test]
fn cursor_ending_at_the_text_end_sits_on_the_structural_break() {
    let out = {
        let state = parse("-[a]>\n");
        let edited = edit(&state, |b| {
            let end = b.at(bound_at(b.text(), 2));
            Landings::new(vec![Landing::cursor_ending_at(end)], 0)
        });
        render(edited.state().view())
    };
    assert_eq!(out, "a-[\n]>");
}

#[test]
fn a_deletion_reaching_the_end_keeps_a_final_break() {
    assert_eq!(text_after_deleting("-[a]>bc\n", &[(1, 4)]), "a\n");
}

#[test]
fn a_deletion_of_an_empty_last_line_removes_it() {
    assert_eq!(text_after_deleting("-[a]>\n\n", &[(2, 3)]), "a\n");
}

#[test]
fn touching_deletions_through_the_last_break_remove_the_whole_line() {
    assert_eq!(text_after_deleting("-[a]>\nb\n", &[(2, 3), (3, 4)]), "a\n");
}

#[test]
fn ops_at_distinct_positions_give_one_changeset_in_any_recording_order() {
    let state = parse("-[a]>bcdefgh\n");
    let record: [fn(&mut EditBuilder<'_, '_>); 4] = [
        |b| {
            b.insert(bound_at(b.text(), 1), "X");
        },
        |b| {
            b.insert(bound_at(b.text(), 3), "Y\n");
        },
        |b| {
            b.delete(clusters(b.text(), 4, 5));
        },
        |b| {
            b.replace(clusters(b.text(), 6, 7), "Z");
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
fn cursor_ending_at_the_text_start_sits_at_it() {
    let state = parse("-[a]>b\n");
    let edited = edit(&state, |b| {
        let empty = b.insert(bound_at(b.text(), 0), "");
        Landings::new(vec![Landing::cursor_ending_at(empty.end())], 0)
    });
    assert_eq!(render(edited.state().view()), "-[a]>b\n");
}

#[test]
fn cursor_ending_at_a_line_start_stays_on_that_line() {
    let state = parse("a\n-[b]>\n");
    let edited = edit(&state, |b| {
        let mark = b.insert(bound_at(b.text(), 2), "");
        Landings::new(vec![Landing::cursor_ending_at(mark.end())], 0)
    });
    assert_eq!(render(edited.state().view()), "a\n-[b]>\n");
}

#[test]
fn a_cursor_lands_on_the_cluster_holding_its_position() {
    // The inserted mark joins the `a` before it.
    let out = cursor_at("-[a]>b\n", |b| {
        b.insert(bound_at(b.text(), 1), "\u{301}").start()
    });
    assert_eq!(out, "-[a\u{301}]>b\n");
}

#[test]
fn cursor_ending_at_takes_the_cluster_before_the_position() {
    let state = parse("-[a]>b\n");
    let edited = edit(&state, |b| {
        let mark = b.insert(bound_at(b.text(), 1), "e\u{301}");
        Landings::new(vec![Landing::cursor_ending_at(mark.end())], 0)
    });
    assert_eq!(render(edited.state().view()), "a-[e\u{301}]>b\n");
}

#[test]
fn covering_an_empty_mark_is_a_cursor_at_it() {
    let out = marked("-[a]>b\n", |b| b.insert(bound_at(b.text(), 1), ""));
    assert_eq!(out, "a-[b]>\n");
}
