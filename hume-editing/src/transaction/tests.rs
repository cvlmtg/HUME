use hume_rope::cluster::ClusterStart;
use hume_rope::offset::CharOffset;
use pretty_assertions::assert_eq;

use super::*;
use crate::changeset::ChangeSetBuilder;
use crate::error::{ApplyError, InvariantViolation};
use crate::marked::render;
use crate::selection::{RecordedSelections, Selection};

fn co(n: usize) -> CharOffset {
    CharOffset::new(n)
}

/// The cluster at char `n` of a text the recorded selections came from,
/// wide enough that every char below it starts a cluster.
fn recorded_at(n: usize) -> ClusterStart {
    let earlier = BufferText::from(" ".repeat(n + 1).as_str());
    let pos = earlier.snap(co(n));
    assert_eq!(pos.offset(), co(n));
    pos
}

fn identity(text: &BufferText) -> ChangeSet {
    let mut b = ChangeSetBuilder::new(text.end());
    b.retain_rest();
    b.finish()
}

fn applied(txn: &Transaction, text: &BufferText) -> String {
    let state = txn.apply(text).expect("transaction applies");
    assert_eq!(state.view().check(), Ok(()));
    render(state.view())
}

/// `sels` recorded as they stand.
fn recorded(sels: Vec<Selection>) -> RecordedSelections {
    RecordedSelections::new(sels, 0)
}

#[test]
fn transaction_apply() {
    let text = BufferText::from("hello");
    let mut b = ChangeSetBuilder::new(co(6));
    b.insert("!");
    b.retain_rest();
    let txn = Transaction::new(
        b.finish(),
        recorded(vec![Selection::cursor(recorded_at(1))]),
    );
    assert_eq!(applied(&txn, &text), "!-[h]>ello\n");
}

#[test]
fn transaction_apply_rejects_a_selection_past_the_text() {
    let text = BufferText::from("hi");
    let txn = Transaction::new(
        identity(&text),
        recorded(vec![Selection::cursor(recorded_at(99))]),
    );
    assert_eq!(
        txn.apply(&text).err(),
        Some(TransactionError::Selections(
            InvariantViolation::OutOfBounds { index: 0 }
        ))
    );
}

#[test]
fn transaction_apply_canonicalizes_selections() {
    let text = BufferText::from("hello world");
    let sels = recorded(vec![
        Selection::new(recorded_at(6), recorded_at(9)),
        Selection::new(recorded_at(0), recorded_at(7)),
    ]);
    let txn = Transaction::new(identity(&text), sels);
    assert_eq!(applied(&txn, &text), "-[hello worl]>d\n");
}

#[test]
fn transaction_apply_rejects_length_mismatch() {
    let text = BufferText::from("hi");
    let mut b = ChangeSetBuilder::new(co(10));
    b.retain_rest();
    let txn = Transaction::new(
        b.finish(),
        recorded(vec![Selection::cursor(recorded_at(0))]),
    );
    let err = txn.apply(&text).unwrap_err();
    assert!(
        matches!(
            err,
            TransactionError::Apply(ApplyError::LengthMismatch {
                buf_len: 3,
                expected: 10
            })
        ),
        "unexpected error: {err}"
    );
}

#[test]
fn transaction_apply_rejects_a_selection_left_inside_a_cluster() {
    let text = BufferText::from("ab");
    let mut b = ChangeSetBuilder::new(co(3));
    b.retain(2);
    b.insert("\u{301}");
    b.retain_rest();
    let txn = Transaction::new(
        b.finish(),
        recorded(vec![Selection::cursor(recorded_at(2))]),
    );
    assert_eq!(
        txn.apply(&text).err(),
        Some(TransactionError::Selections(
            InvariantViolation::SplitsCluster { index: 0 }
        ))
    );
}
