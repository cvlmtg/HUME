use super::*;
use hume_editing::changeset::ChangeSetBuilder;
use hume_editing::edit::TextChange;
use hume_editing::text::BufferText;
use hume_rope::offset::CharOffset;
use pretty_assertions::assert_eq;
use test_fixtures::testing::at;

fn co(n: usize) -> CharOffset {
    CharOffset::new(n)
}

/// `text` with `insert` inserted at char `pos`, and the change that did it.
fn insert(text: &BufferText, pos: usize, insert: &str) -> (BufferText, ChangeSet) {
    let mut b = ChangeSetBuilder::new(text.end());
    b.retain_to(co(pos));
    b.insert(insert);
    let cs = b.finish();
    let after = cs.apply(text).expect("built for text");
    (after, cs)
}

fn replacement(chain: DotChain) -> Option<(usize, usize, String)> {
    chain
        .finish()
        .expect("the chain is intact")
        .map(|r| (r.back, r.forward, r.text))
}

#[test]
fn finish_extracts_the_replacement_at_the_origin() {
    let text = BufferText::from("abcd\n");
    let mut chain = DotChain::new(at(&text, 2), &text);
    let (after, cs) = insert(&text, 2, "X");
    chain.push(&TextChange::new(&text, &after, &cs));
    assert_eq!(replacement(chain), Some((0, 0, "X".to_string())));
}

#[test]
fn a_second_push_composes_onto_the_first() {
    let text = BufferText::from("abcd\n");
    let mut chain = DotChain::new(at(&text, 2), &text);
    let (one, cs1) = insert(&text, 2, "X");
    chain.push(&TextChange::new(&text, &one, &cs1));
    let (two, cs2) = insert(&one, 3, "Y");
    chain.push(&TextChange::new(&one, &two, &cs2));
    assert_eq!(replacement(chain), Some((0, 0, "XY".to_string())));
}

#[test]
fn an_edit_the_chain_did_not_record_breaks_it() {
    let text = BufferText::from("abcd\n");
    let mut chain = DotChain::new(at(&text, 2), &text);
    let (foreign, _) = insert(&text, 0, "Z");
    let (after, cs) = insert(&foreign, 3, "X");
    chain.push(&TextChange::new(&foreign, &after, &cs));
    assert!(chain.finish().is_err());
}

#[test]
fn rearm_moves_an_empty_chain_to_the_current_head() {
    let text = BufferText::from("abcd\n");
    let chain = DotChain::new(at(&text, 1), &text);
    let (moved, _) = insert(&text, 0, "Z");
    let mut chain = chain
        .rearm(at(&moved, 3), &moved)
        .expect("an empty chain always rearms");
    let (after, cs) = insert(&moved, 3, "X");
    chain.push(&TextChange::new(&moved, &after, &cs));
    assert_eq!(replacement(chain), Some((0, 0, "X".to_string())));
}

#[test]
fn rearm_keeps_a_chain_whose_text_is_unchanged() {
    let text = BufferText::from("abcd\n");
    let mut chain = DotChain::new(at(&text, 2), &text);
    let (after, cs) = insert(&text, 2, "X");
    chain.push(&TextChange::new(&text, &after, &cs));
    let chain = chain
        .rearm(at(&after, 0), &after)
        .expect("nothing moved the text since the last push");
    assert_eq!(replacement(chain), Some((0, 0, "X".to_string())));
}

#[test]
fn rearm_drops_a_chain_whose_text_moved_after_a_push() {
    let text = BufferText::from("abcd\n");
    let mut chain = DotChain::new(at(&text, 2), &text);
    let (after, cs) = insert(&text, 2, "X");
    chain.push(&TextChange::new(&text, &after, &cs));
    let (foreign, _) = insert(&after, 0, "Z");
    assert!(chain.rearm(at(&foreign, 0), &foreign).is_none());
}

#[test]
fn identity_has_no_replacement() {
    let b = ChangeSetBuilder::new(co(6));
    assert!(cursor_replacement_at(&b.finish(), co(3)).is_none());
}

#[test]
fn insert_only_region_at_head() {
    // "ab|cd" → "ab|Xcd": inserting with nothing deleted, head right at
    // the insertion point.
    let mut b = ChangeSetBuilder::new(co(4));
    b.retain_to(co(2));
    b.insert("X");
    let r = cursor_replacement_at(&b.finish(), co(2)).expect("insert at head must match");
    assert_eq!((r.back, r.forward, r.text.as_str()), (0, 0, "X"));
}

#[test]
fn delete_and_insert_region_containing_head() {
    // "abcd" → "aXd": delete "bc" (old positions 1..3), insert "X". A
    // head inside the deleted span (2) reports the split around it.
    let mut b = ChangeSetBuilder::new(co(4));
    b.retain_to(co(1));
    b.delete_to(co(3));
    b.insert("X");
    let r =
        cursor_replacement_at(&b.finish(), co(2)).expect("head inside the deleted span must match");
    assert_eq!((r.back, r.forward, r.text.as_str()), (1, 1, "X"));
}

#[test]
fn insert_then_delete_region_containing_head() {
    // "abcd" → "aXd": the *other* op order a replacement can take
    // (`invert`/`compose`/`indent` all emit insert-then-delete):
    // insert "X" at old position 1, then delete "bc" (old 1..3). A head
    // inside the deleted span (2) must still report the replacement
    // text, not an empty one.
    let mut b = ChangeSetBuilder::new(co(4));
    b.retain_to(co(1));
    b.insert("X");
    b.delete_to(co(3));
    let r =
        cursor_replacement_at(&b.finish(), co(2)).expect("head inside the deleted span must match");
    assert_eq!((r.back, r.forward, r.text.as_str()), (1, 1, "X"));
}

#[test]
fn region_away_from_head_is_ignored() {
    // The edit lands at the start; head sits at the untouched end.
    let mut b = ChangeSetBuilder::new(co(4));
    b.delete_to(co(1));
    b.insert("X");
    assert!(cursor_replacement_at(&b.finish(), co(4)).is_none());
}

/// The shape a completion's `additionalTextEdits` produces alongside its
/// own cursor edit: two edited regions in one delta. Only the one at
/// `head` is extracted; the other (document-absolute, or a different
/// cursor's own edit under a multi-cursor accept) is skipped.
#[test]
fn picks_the_region_at_head_and_skips_the_other() {
    let mut b = ChangeSetBuilder::new(co(6));
    b.insert("// "); // an import, landing away from the cursor
    b.retain_to(co(2));
    b.delete_to(co(4));
    b.insert("XYZ"); // the accept's own edit, at the cursor
    let r = cursor_replacement_at(&b.finish(), co(4)).expect("the region at head must match");
    assert_eq!((r.back, r.forward, r.text.as_str()), (2, 0, "XYZ"));
}
