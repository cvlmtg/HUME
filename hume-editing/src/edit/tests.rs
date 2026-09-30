use pretty_assertions::assert_eq;

use super::*;
use crate::changeset::ChangeSetBuilder;
use crate::marked::{bound_at, parse, render};

/// `state` with an `X` inserted at `at`, its one cursor after it.
fn insert_x(state: EditState, at: usize) -> Edited {
    edit(&state, |b| {
        let mark = b.insert(bound_at(b.text(), at), "X");
        Landings::new(vec![Landing::cursor(mark.end())], 0)
    })
}

#[test]
fn an_edit_records_the_version_it_started_from() {
    let state = parse("-[a]>b\n");
    let base = state.text().version();
    let edited = insert_x(state, 0);
    assert_eq!(edited.base(), base);
    assert!(edited.state().text().version().is_later_than(base));
}

#[test]
fn unchanged_keeps_text_and_selections() {
    let edited = Edited::unchanged(parse("a-[b]>\n"));
    assert!(edited.changes().is_identity());
    assert_eq!(render(edited.state().view()), "a-[b]>\n");
}

#[test]
fn from_changes_carries_selections_past_inserted_text() {
    let state = parse("a-[b]>c\n");
    let mut b = ChangeSetBuilder::new(state.text().end());
    b.insert("XX");
    b.retain_rest();
    let edited = Edited::from_changes(state, b.finish()).expect("fits the text");
    assert_eq!(render(edited.state().view()), "XXa-[b]>c\n");
}

#[test]
fn from_changes_keeps_a_sticky_column_on_an_untouched_line() {
    let sticky = crate::selection::StickyDisplayCol::BufferLine {
        display_col: hume_rope::column::BufferLineCol::new(5),
    };
    let state = parse("ab\nc-[d]>\n").map(|v| v.selection().with_sticky(sticky));
    let mut b = ChangeSetBuilder::new(state.text().end());
    b.insert("X");
    b.retain_rest();
    let edited = Edited::from_changes(state, b.finish()).expect("fits the text");
    assert_eq!(
        edited
            .state()
            .view()
            .primary()
            .selection()
            .sticky_display_col(),
        Some(sticky)
    );
}

#[test]
fn then_composes_two_edits_into_one() {
    let start = parse("-[a]>b\n");
    let text = start.text().clone();
    let both = insert_x(start, 0).then(|s| insert_x(s, 2));
    assert_eq!(render(both.state().view()), "XaX-[b]>\n");
    assert_eq!(
        both.changes().apply(&text).expect("composed").to_string(),
        "XaXb\n"
    );
}

#[test]
#[should_panic(expected = "another text than this edit's result")]
fn then_refuses_an_edit_of_another_text() {
    Edited::unchanged(parse("-[a]>\n")).then(|_| Edited::unchanged(parse("-[b]>\n")));
}

#[test]
#[should_panic]
fn a_text_change_refuses_texts_its_changes_do_not_map() {
    let before = BufferText::from("ab");
    let after = BufferText::from("abc");
    let changes = ChangeSet::identity(before.len_chars());
    TextChange::new(&before, &after, &changes);
}
