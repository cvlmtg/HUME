use super::*;
use hume_editing::changeset::ChangeSetBuilder;
use hume_editing::selection::StickyDisplayCol;
use hume_editing::text::BufferText;
use hume_rope::column::BufferLineCol;
use test_fixtures::testing::{cursor, sel, single};

/// The two chars of `text` from char `n`, selected.
fn at(text: &BufferText, n: usize) -> SelectionSet {
    single(text, sel(text, n, n + 1))
}

fn replaces(placement: Placement) -> bool {
    placement.replaces.is_some()
}

/// Selections that moved to each selection in `ns` in turn, oldest first,
/// each move inserting an entry.
fn recorded(text: &BufferText, ns: &[usize]) -> PaneSelections {
    let mut sels = PaneSelections::new(single(text, cursor(text, 0)));
    for &n in ns {
        let placement = sels.placement(Record::Insert);
        sels.set(at(text, n));
        sels.record(placement);
    }
    sels
}

/// The selections one step from `sels`' in direction `walk` through the
/// live list alone, `None` when it has no step.
fn step(sels: &mut PaneSelections, walk: Walk) -> Option<SelectionSet> {
    let before = sels.current().clone();
    match sels.walk(walk, 1, None) {
        Ok(()) | Err(TreeRefused) if *sels.current() != before => Some(sels.current().clone()),
        Ok(()) | Err(TreeRefused) => None,
    }
}

#[test]
fn what_each_effect_records_in_each_mode() {
    let cases = [
        (SelectionEffect::Extends, false, None),
        (SelectionEffect::Extends, true, Some(Record::Continue)),
        (SelectionEffect::Establishes, false, Some(Record::Insert)),
        (SelectionEffect::Establishes, true, Some(Record::Continue)),
        (SelectionEffect::Composes, false, Some(Record::Insert)),
        (SelectionEffect::Composes, true, Some(Record::Insert)),
    ];
    for (effect, extend, expected) in cases {
        assert_eq!(
            Record::of(effect, extend),
            expected,
            "{effect:?}, extend {extend}"
        );
    }
}

#[test]
fn only_a_continuing_change_from_the_last_entry_replaces_it() {
    let text = BufferText::from("abcdefghij\n");
    let mut sels = recorded(&text, &[1, 2]);

    assert!(replaces(sels.placement(Record::Continue)));
    assert!(!replaces(sels.placement(Record::Insert)));
    sels.set(at(&text, 1));
    assert!(!replaces(sels.placement(Record::Continue)));
    assert!(!replaces(
        PaneSelections::new(at(&text, 2)).placement(Record::Continue)
    ));
}

#[test]
fn continuing_from_the_last_entry_ignores_a_latched_sticky_column() {
    let text = BufferText::from("abcdefghij\n");
    let mut sels = recorded(&text, &[1, 2]);
    sels.set(single(
        &text,
        sel(&text, 2, 3).with_sticky(StickyDisplayCol::BufferLine {
            display_col: BufferLineCol::new(3),
        }),
    ));

    assert!(replaces(sels.placement(Record::Continue)));
}

#[test]
fn a_single_cursor_is_not_recorded() {
    let text = BufferText::from("abcdefghij\n");
    let mut sels = recorded(&text, &[1]);

    let placement = sels.placement(Record::Insert);
    sels.set(single(&text, cursor(&text, 5)));
    sels.record(placement);

    assert_eq!(step(&mut sels, Walk::Back), Some(at(&text, 1)));
}

#[test]
fn back_from_other_selections_returns_to_the_last_entry_first() {
    let text = BufferText::from("abcdefghij\n");
    let mut sels = recorded(&text, &[1, 2]);
    sels.set(at(&text, 5));

    assert_eq!(step(&mut sels, Walk::Back), Some(at(&text, 2)));
    assert_eq!(step(&mut sels, Walk::Back), Some(at(&text, 1)));
    assert_eq!(step(&mut sels, Walk::Back), None);
}

#[test]
fn back_from_selections_moved_off_a_walked_entry_returns_to_it() {
    let text = BufferText::from("abcdefghij\n");
    let mut sels = recorded(&text, &[1, 2, 3]);
    step(&mut sels, Walk::Back);
    sels.set(at(&text, 7));

    assert_eq!(step(&mut sels, Walk::Back), Some(at(&text, 2)));
    assert_eq!(step(&mut sels, Walk::Back), Some(at(&text, 1)));
}

#[test]
fn forward_retraces_the_steps_back() {
    let text = BufferText::from("abcdefghij\n");
    let mut sels = recorded(&text, &[1, 2, 3]);
    step(&mut sels, Walk::Back);
    step(&mut sels, Walk::Back);

    assert_eq!(step(&mut sels, Walk::Forward), Some(at(&text, 2)));
    assert_eq!(step(&mut sels, Walk::Forward), Some(at(&text, 3)));
    assert_eq!(step(&mut sels, Walk::Forward), None);
}

#[test]
fn a_counted_walk_goes_as_far_as_the_steps_do() {
    let text = BufferText::from("abcdefghij\n");
    let mut sels = recorded(&text, &[1, 2, 3]);

    assert_eq!(sels.walk(Walk::Back, 2, None), Ok(()));
    assert_eq!(sels.current(), &at(&text, 1));
    assert_eq!(sels.walk(Walk::Forward, 5, None), Ok(()));
    assert_eq!(sels.current(), &at(&text, 3));
}

#[test]
fn a_step_back_past_the_live_list_needs_the_undo_tree() {
    let text = BufferText::from("abcdefghij\n");
    let mut sels = recorded(&text, &[1, 2]);

    assert_eq!(sels.walk(Walk::Back, 3, None), Err(TreeRefused));
    assert_eq!(sels.current(), &at(&text, 1));
}

#[test]
fn continuing_from_the_last_entry_replaces_it() {
    let text = BufferText::from("abcdefghij\n");
    let mut sels = recorded(&text, &[1, 2]);

    let placement = sels.placement(Record::Continue);
    sels.set(at(&text, 4));
    sels.record(placement);

    assert_eq!(step(&mut sels, Walk::Back), Some(at(&text, 1)));
}

#[test]
fn a_replace_decided_before_another_record_inserts() {
    let text = BufferText::from("abcdefghij\n");
    let mut sels = recorded(&text, &[1, 2]);
    let placement = sels.placement(Record::Continue);
    let inserting = sels.placement(Record::Insert);
    sels.set(at(&text, 3));
    sels.record(inserting);

    sels.set(at(&text, 4));
    sels.record(placement);

    assert_eq!(step(&mut sels, Walk::Back), Some(at(&text, 3)));
}

#[test]
fn recording_after_stepping_back_drops_the_entries_ahead() {
    let text = BufferText::from("abcdefghij\n");
    let mut sels = recorded(&text, &[1, 2, 3]);
    step(&mut sels, Walk::Back);

    let placement = sels.placement(Record::Insert);
    sels.set(at(&text, 5));
    sels.record(placement);

    assert_eq!(step(&mut sels, Walk::Forward), None);
    assert_eq!(step(&mut sels, Walk::Back), Some(at(&text, 2)));
}

#[test]
fn updating_after_stepping_back_drops_the_entries_ahead() {
    let text = BufferText::from("abcdefghij\n");
    let mut sels = recorded(&text, &[1, 2, 3]);
    step(&mut sels, Walk::Back);

    let placement = sels.placement(Record::Continue);
    sels.set(at(&text, 4));
    sels.record(placement);

    assert_eq!(step(&mut sels, Walk::Forward), None);
    assert_eq!(step(&mut sels, Walk::Back), Some(at(&text, 1)));
}

#[test]
fn the_oldest_entry_goes_first_past_the_capacity() {
    let text = BufferText::from(format!("{}\n", "a".repeat(CAPACITY + 3)).as_str());
    let ns: Vec<usize> = (0..=CAPACITY).collect();
    let mut sels = recorded(&text, &ns);

    let mut walked = 0;
    while step(&mut sels, Walk::Back).is_some() {
        walked += 1;
    }
    assert_eq!(walked, CAPACITY - 1);
    assert_eq!(sels.current(), &at(&text, 1));
}

#[test]
fn an_edit_of_the_pane_forgets_the_live_list() {
    let text = BufferText::from("abcdefghij\n");
    let mut sels = recorded(&text, &[1, 2]);

    sels.edited();

    assert_eq!(step(&mut sels, Walk::Back), None);
}

#[test]
fn another_panes_edit_carries_the_live_list() {
    let pre = BufferText::from("abcdefghij\n");
    let mut sels = recorded(&pre, &[1, 2]);
    let mut b = ChangeSetBuilder::new(pre.end());
    b.insert("XX");
    let cs = b.finish();
    let post = cs.apply(&pre).expect("built for pre");

    sels.carry(&TextChange::new(&pre, &post, &cs));

    // "XX" lands before every stored selection, so each moves two right.
    assert_eq!(sels.current(), &at(&post, 4));
    assert_eq!(step(&mut sels, Walk::Back), Some(at(&post, 3)));
}

/// Defers a change of `record` that moves the selections to `to`, as a
/// command in a script session does.
fn defer_move(sels: &mut PaneSelections, text: &BufferText, record: Record, to: SelectionSet) {
    let placement = sels.placement(record);
    let before = Tracked::new(sels.current().clone(), text);
    sels.set(to);
    sels.defer(placement, before, record);
}

#[test]
fn deferred_continues_replace_the_entry_they_started_from_once() {
    let text = BufferText::from("abcdefghij\n");
    let mut sels = recorded(&text, &[1, 2]);

    defer_move(&mut sels, &text, Record::Continue, at(&text, 3));
    defer_move(&mut sels, &text, Record::Continue, at(&text, 4));
    sels.record_deferred(&text);

    assert_eq!(sels.current(), &at(&text, 4));
    assert_eq!(step(&mut sels, Walk::Back), Some(at(&text, 1)));
}

#[test]
fn a_deferred_continue_then_insert_inserts() {
    let text = BufferText::from("abcdefghij\n");
    let mut sels = recorded(&text, &[1, 2]);

    defer_move(&mut sels, &text, Record::Continue, at(&text, 3));
    defer_move(&mut sels, &text, Record::Insert, at(&text, 4));
    sels.record_deferred(&text);

    assert_eq!(step(&mut sels, Walk::Back), Some(at(&text, 2)));
}

#[test]
fn a_deferral_that_ends_where_it_started_records_nothing() {
    let text = BufferText::from("abcdefghij\n");
    let mut sels = recorded(&text, &[1, 2, 3]);
    step(&mut sels, Walk::Back);

    defer_move(&mut sels, &text, Record::Insert, at(&text, 6));
    defer_move(&mut sels, &text, Record::Insert, at(&text, 2));
    sels.record_deferred(&text);

    assert_eq!(step(&mut sels, Walk::Forward), Some(at(&text, 3)));
}

#[test]
fn an_edit_of_the_pane_drops_the_deferred_record() {
    let text = BufferText::from("abcdefghij\n");
    let mut sels = recorded(&text, &[1, 2]);

    defer_move(&mut sels, &text, Record::Insert, at(&text, 3));
    sels.edited();
    sels.record_deferred(&text);
    sels.set(at(&text, 5));

    assert_eq!(step(&mut sels, Walk::Back), None);
}

#[test]
fn a_walk_drops_the_deferred_record() {
    let text = BufferText::from("abcdefghij\n");
    let mut sels = recorded(&text, &[1, 2, 3]);

    defer_move(&mut sels, &text, Record::Continue, at(&text, 4));
    step(&mut sels, Walk::Back);
    step(&mut sels, Walk::Back);
    sels.record_deferred(&text);

    assert_eq!(sels.current(), &at(&text, 2));
    assert_eq!(step(&mut sels, Walk::Forward), Some(at(&text, 3)));
}

#[test]
fn another_panes_edit_carries_the_deferred_start() {
    let pre = BufferText::from("abcdefghij\n");
    let mut sels = recorded(&pre, &[1, 2, 3]);
    step(&mut sels, Walk::Back);
    defer_move(&mut sels, &pre, Record::Insert, at(&pre, 6));
    defer_move(&mut sels, &pre, Record::Insert, at(&pre, 2));
    let mut b = ChangeSetBuilder::new(pre.end());
    b.insert("XX");
    let cs = b.finish();
    let post = cs.apply(&pre).expect("built for pre");

    sels.carry(&TextChange::new(&pre, &post, &cs));
    sels.record_deferred(&post);

    // "XX" lands before every stored selection, so each moves two right.
    assert_eq!(step(&mut sels, Walk::Forward), Some(at(&post, 5)));
}

#[test]
fn a_split_does_not_copy_the_deferred_record() {
    let text = BufferText::from("abcdefghij\n");
    let mut sels = recorded(&text, &[1, 2]);
    defer_move(&mut sels, &text, Record::Insert, at(&text, 3));

    let mut copy = sels.split();
    copy.record_deferred(&text);
    copy.set(at(&text, 5));

    assert_eq!(step(&mut copy, Walk::Back), Some(at(&text, 2)));
}
