use hume_rope::offset::CharOffset;
use pretty_assertions::assert_eq;

use super::*;
use crate::changeset::ChangeSetBuilder;

#[test]
fn a_value_reads_only_against_its_own_text() {
    let text = BufferText::from("abc\n");
    let other = BufferText::from("abc\n");
    let tracked = Tracked::new(7, &text);
    assert_eq!(tracked.get(&text), Some(&7));
    assert_eq!(tracked.get(&other), None);
}

#[test]
fn translating_carries_the_value_to_the_new_text() {
    let text = BufferText::from("abc\n");
    let mut b = ChangeSetBuilder::new(text.end());
    b.insert("x");
    b.retain_rest();
    let cs = b.finish();
    let post = cs.apply(&text).expect("insert applies");
    let mut tracked = Tracked::new(CharOffset::new(1), &text);
    tracked.translate(&TextChange::new(&text, &post, &cs), |pos, change| {
        *pos = crate::changeset::PosMapCursor::new(change.changes().ops())
            .map(*pos, crate::changeset::Assoc::After);
    });
    assert_eq!(tracked.get(&text), None);
    assert_eq!(tracked.get(&post), Some(&CharOffset::new(2)));
}

#[test]
fn a_stale_value_is_not_translated() {
    let text = BufferText::from("abc\n");
    let stale_text = BufferText::from("zz\n");
    let mut b = ChangeSetBuilder::new(text.end());
    b.insert("x");
    b.retain_rest();
    let cs = b.finish();
    let post = cs.apply(&text).expect("insert applies");
    let mut tracked = Tracked::new(1, &stale_text);
    tracked.translate(&TextChange::new(&text, &post, &cs), |v, _| *v = 2);
    assert_eq!(tracked.into_inner(&stale_text), Some(1));
}
