use super::*;
use hume_editing::changeset::ChangeSetBuilder;
use hume_editing::text::BufferText;
use hume_engine::pipeline::EngineView;
use hume_engine::theme::Theme;
use hume_rope::offset::CharOffset;

fn two_bids() -> (BufferId, BufferId) {
    let mut ev = EngineView::new(Theme::default());
    (ev.buffers.insert(()), ev.buffers.insert(()))
}

fn attachment(n: u32) -> Attachment {
    Attachment::new(ServerId(n), FeatureFilter::All)
}

fn rust() -> OpenedAs {
    OpenedAs {
        language_id: "rust".to_string(),
        uri: "file:///main.rs".to_string(),
    }
}

/// "ab\n" with "X" inserted at its start.
fn insertion() -> (BufferText, BufferText, ChangeSet) {
    let before = BufferText::from("ab\n");
    let mut b = ChangeSetBuilder::new(CharOffset::new(3));
    b.insert("X");
    let cs = b.finish();
    let after = cs.apply(&before).unwrap();
    (before, after, cs)
}

#[test]
fn record_queues_a_change_only_for_an_attached_document() {
    let (attached, unattached) = two_bids();
    let mut docs = LspDocuments::default();
    docs.push(attached, attachment(0), &rust());
    let (before, after, cs) = insertion();
    let change = TextChange::new(&before, &after, &cs);

    docs.record(attached, &change);
    docs.record(unattached, &change);

    assert_eq!(docs.with_pending(), vec![attached]);
    let pending = docs.take_pending(attached);
    assert_eq!(pending.len(), 1);
    assert_eq!(pending[0].before.to_string(), "ab\n");
    assert_eq!(pending[0].version, after.generation());
    assert!(!docs.has_doc(unattached));
}

#[test]
fn record_skips_an_identity_change() {
    let (bid, _) = two_bids();
    let mut docs = LspDocuments::default();
    docs.push(bid, attachment(0), &rust());
    let text = BufferText::from("ab\n");
    let identity = ChangeSet::identity(text.len_chars());
    let change = TextChange::new(&text, &text, &identity);

    docs.record(bid, &change);

    assert!(docs.with_pending().is_empty());
}

#[test]
fn removing_the_last_attachment_drops_the_document_and_its_queue() {
    let (bid, _) = two_bids();
    let mut docs = LspDocuments::default();
    docs.push(bid, attachment(0), &rust());
    docs.push(bid, attachment(1), &rust());
    let (before, after, cs) = insertion();
    docs.record(bid, &TextChange::new(&before, &after, &cs));

    docs.remove(bid, ServerId(0));
    assert_eq!(docs.servers(bid).collect::<Vec<_>>(), vec![ServerId(1)]);
    assert_eq!(docs.with_pending(), vec![bid]);

    docs.remove(bid, ServerId(1));
    assert!(!docs.has_doc(bid));
    assert!(docs.with_pending().is_empty());
}

#[test]
fn reorder_follows_the_given_order() {
    let (bid, _) = two_bids();
    let mut docs = LspDocuments::default();
    docs.push(bid, attachment(0), &rust());
    docs.push(bid, attachment(1), &rust());

    docs.reorder(bid, &[ServerId(1), ServerId(0)]);

    assert_eq!(
        docs.servers(bid).collect::<Vec<_>>(),
        vec![ServerId(1), ServerId(0)]
    );
}
