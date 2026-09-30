use hume_rope::offset::CharOffset;
use pretty_assertions::assert_eq;

use super::*;
use crate::changeset::ChangeSetBuilder;
use crate::marked::{parse, render};

fn co(n: usize) -> CharOffset {
    CharOffset::new(n)
}

/// `text` with `!` inserted at its start.
fn bang(text: &BufferText) -> ChangeSet {
    let mut b = ChangeSetBuilder::new(text.end());
    b.insert("!");
    b.retain_rest();
    b.finish()
}

fn identity(text: &BufferText) -> ChangeSet {
    let mut b = ChangeSetBuilder::new(text.end());
    b.retain_rest();
    b.finish()
}

#[test]
fn undoing_lands_the_recorded_selections_on_the_recorded_version() {
    let before = parse("h-[el]>lo\n");
    let forward = bang(before.text());
    let after = forward.apply(before.text()).expect("the insertion applies");
    let undo = Transaction::new(
        forward.invert(before.text()),
        before.clone().into_selections(),
    );

    let undone = undo.apply(&after).expect("the inverse applies");
    assert_eq!(undone.text().version(), before.text().version());
    assert!(undone.text().generation() > after.generation());
    assert_eq!(render(undone.view()), "h-[el]>lo\n");
}

#[test]
fn a_transaction_that_changes_nothing_relabels_the_text_in_place() {
    let before = parse("-[a]>b\n");
    let inserted = bang(before.text())
        .apply(before.text())
        .expect("the insertion applies");
    let mut b = ChangeSetBuilder::new(inserted.end());
    b.delete(1);
    b.retain_rest();
    let same = b.finish().apply(&inserted).expect("the deletion applies");
    assert_ne!(
        same.version(),
        before.text().version(),
        "setup: a new version"
    );

    let walk = Transaction::new(identity(&same), before.clone().into_selections());
    let landed = walk.apply(&same).expect("the identity applies");
    assert_eq!(landed.text().version(), before.text().version());
    assert_eq!(landed.text().generation(), same.generation());
    assert_eq!(render(landed.view()), "-[a]>b\n");
}

#[test]
fn composing_keeps_the_last_transactions_selections() {
    let start = parse("-[a]>b\n");
    let first = bang(start.text());
    let middle = first
        .apply(start.text())
        .expect("the first insertion applies");
    let second = bang(&middle);
    let end = second.apply(&middle).expect("the second insertion applies");
    let end_state = crate::state::EditState::with_cursor(end.clone(), end.snap(co(2)));
    let txns = vec![
        Transaction::new(first, start.clone().into_selections()),
        Transaction::new(second, end_state.clone().into_selections()),
    ];

    let composed = Transaction::compose_all(txns).expect("two transactions compose");
    assert_eq!(composed.selection(), &end_state.into_selections());
    let landed = composed
        .apply(start.text())
        .expect("the composition applies");
    assert_eq!(render(landed.view()), "!!-[a]>b\n");
}

#[test]
fn composing_nothing_gives_nothing() {
    assert!(Transaction::compose_all(Vec::new()).is_none());
}

#[test]
fn transaction_apply_rejects_length_mismatch() {
    let text = BufferText::from("hi");
    let mut b = ChangeSetBuilder::new(co(10));
    b.retain_rest();
    let txn = Transaction::new(b.finish(), parse("-[h]>i\n").into_selections());
    assert_eq!(
        txn.apply(&text).err(),
        Some(ApplyError::LengthMismatch {
            buf_len: 3,
            expected: 10
        })
    );
}
