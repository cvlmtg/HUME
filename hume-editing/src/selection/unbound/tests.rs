use hume_rope::offset::{CharOffset, ExclusiveRange};
use pretty_assertions::assert_eq;

use super::*;
use crate::changeset::Assoc;
use crate::edit::{EditBuilder, Landing, Landings, edit};
use crate::marked::{parse, render};

fn co(n: usize) -> CharOffset {
    CharOffset::new(n)
}

/// `input` edited by `f`, which returns its result selection.
fn edited(
    input: &str,
    f: impl for<'a, 'id> FnOnce(&mut EditBuilder<'a, 'id>, Selection) -> Landing<'id>,
) -> String {
    let state = parse(input);
    let sel = state.view().primary().selection();
    let edited = edit(&state, |b| Landings::new(vec![f(b, sel)], 0));
    render(edited.state().view())
}

#[test]
fn kept_ends_follow_their_assoc_across_an_insertion() {
    let before = edited("-[a]>b\n", |b, sel| {
        b.insert(co(0), "X");
        Landing::kept_with(sel, Assoc::Before)
    });
    let after = edited("-[a]>b\n", |b, sel| {
        b.insert(co(0), "X");
        Landing::kept(sel)
    });
    assert_eq!(before, "-[X]>ab\n");
    assert_eq!(after, "X-[a]>b\n");
}

#[test]
fn a_kept_end_left_inside_a_cluster_lands_on_its_start() {
    let out = edited("a\n-[\u{301}]>b\n", |b, sel| {
        b.delete(ExclusiveRange::new(co(1), co(2)));
        Landing::kept(sel)
    });
    assert_eq!(out, "-[a\u{301}]>b\n");
}

/// The primary selection of `input`, its ends moved to `first` and `last`
/// (0-based lines of the same text), as the edit that changes nothing lands it.
fn at_lines(input: &str, first: usize, last: usize) -> String {
    let state = parse(input);
    let sel = state.view().primary();
    let unbound = UnboundSelection::at_lines(
        sel,
        hume_rope::line::ContentLine::new(first),
        hume_rope::line::ContentLine::new(last),
    );
    let edited = edit(&state, |_| Landings::new(vec![unbound.into()], 0));
    render(edited.state().view())
}

#[test]
fn at_lines_keeps_each_ends_column_on_its_new_line() {
    assert_eq!(at_lines("ab\nc-[d\nef]>\n", 0, 1), "a-[b\ncd]>\nef\n");
}

#[test]
fn at_lines_clamps_a_column_to_the_last_content_cluster() {
    assert_eq!(at_lines("x\nab-[c]>\n", 0, 0), "-[x]>\nabc\n");
}

#[test]
fn at_lines_lands_a_column_on_an_empty_lines_break() {
    assert_eq!(at_lines("\nab-[c]>\n", 0, 0), "-[\n]>abc\n");
}

#[test]
fn at_lines_keeps_an_end_on_a_line_break_on_the_new_lines_break() {
    assert_eq!(at_lines("xyz\n-[a\n]>", 0, 0), "-[xyz\n]>a\n");
}

#[test]
#[should_panic(expected = "past the last line")]
fn at_lines_refuses_a_line_past_the_text() {
    at_lines("-[a]>\n", 3, 3);
}
