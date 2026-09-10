use hume_rope::offset::CharOffset;
use pretty_assertions::assert_eq;

use super::*;
use crate::changeset::ChangeSetBuilder;
use crate::error::{ApplyError, ValidationError};
use crate::selection::Selection;

fn co(n: usize) -> CharOffset {
    CharOffset::new(n)
}

#[test]
fn transaction_apply() {
    // "hello\n" = 6 chars; insert "!" at start → "!hello\n".
    let text = BufferText::from("hello");
    let mut b = ChangeSetBuilder::new(co(6));
    b.insert("!");
    b.retain_rest();
    let cs = b.finish();

    let sels = SelectionSet::single(Selection::collapsed(co(1)));
    let txn = Transaction::new(cs, sels.clone());

    let (new_text, new_sels) = txn.apply(&text).unwrap();
    assert_eq!(new_text.to_string(), "!hello\n");
    assert_eq!(new_sels.primary().head, co(1));
}

#[test]
fn transaction_apply_rejects_out_of_bounds_selection() {
    // "hi\n" = 3 chars; a no-op changeset; but selection points to index 99.
    let text = BufferText::from("hi");
    let mut b = ChangeSetBuilder::new(co(3));
    b.retain_rest();
    let cs = b.finish();

    // Cursor at 99 is way past buf_len (3) — this is what a buggy plugin
    // might produce.
    let sels = SelectionSet::single(Selection::collapsed(co(99)));
    let txn = Transaction::new(cs, sels);

    let err = txn.apply(&text).unwrap_err();
    assert!(
        matches!(
            err,
            TransactionError::Validation(ValidationError::SelectionOutOfBounds {
                index: 0,
                field: "head",
                value: 99,
                buf_len: 3
            })
        ),
        "unexpected error: {err}"
    );
}

#[test]
fn transaction_apply_canonicalizes_selections() {
    // A plugin-built Transaction may carry overlapping/unsorted selections
    // (constructed via from_vec_unchecked). apply must hand back a
    // canonical set, not propagate the invariant violation.
    let text = BufferText::from("hello world");
    let mut b = ChangeSetBuilder::new(co(12));
    b.retain_rest();
    let cs = b.finish();

    let sels = SelectionSet::from_vec_unchecked(
        vec![Selection::new(co(6), co(9)), Selection::new(co(0), co(7))], // unsorted + overlapping
        0,
    );
    let txn = Transaction::new(cs, sels);

    let (_, new_sels) = txn.apply(&text).unwrap();
    assert_eq!(new_sels.len(), 1);
    assert_eq!(new_sels.primary().start(), co(0));
    assert_eq!(new_sels.primary().end(), co(9));
}

#[test]
fn transaction_apply_rejects_length_mismatch() {
    // Changeset built for 10 chars, but buffer is 3 chars.
    let text = BufferText::from("hi");
    let mut b = ChangeSetBuilder::new(co(10));
    b.retain_rest();
    let cs = b.finish();

    let txn = Transaction::new(cs, SelectionSet::single(Selection::collapsed(co(0))));
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
