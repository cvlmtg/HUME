use std::time::SystemTime;

use hume_core::{PaneId, fresh_ids};

use super::super::Side;
use super::super::tests::{record_edit, selected};
use super::*;

/// The text a walk of `txns` from `text` lands on, and the selections it
/// leaves as anchor/head offsets.
fn walk(text: &BufferText, txns: Vec<Transaction>) -> (String, Vec<(usize, usize)>) {
    let mut text = text.clone();
    let mut ranges = Vec::new();
    for txn in txns {
        let (state, _) = txn.apply(&text).expect("the walk maps the text");
        let (next, sels) = state.into_parts();
        ranges = offsets(&sels);
        text = next;
    }
    (text.to_string(), ranges)
}

fn offsets(sels: &SelectionSet) -> Vec<(usize, usize)> {
    sels.selections()
        .iter()
        .map(|s| (s.anchor().offset().index(), s.head().offset().index()))
        .collect()
}

/// A tree with a branch, two owners and a redo choice off the newest branch:
///
/// ```text
/// root "abc" ─ 1 "Xabc" (a) ─ 2 "XYabc" (a)
///                         └── 3 "Xbc" (b) ─ 4 "Xbcé" (b)
/// ```
///
/// Current is 1, and its redo continues along 2, the older branch.
struct Fixture {
    history: History,
    texts: Vec<String>,
    current: BufferText,
    a: PaneId,
    b: PaneId,
}

fn fixture() -> Fixture {
    let [a, b] = fresh_ids();
    let mut h = History::new();
    let t0 = BufferText::from("abc\n");
    let t1 = record_edit(&mut h, Some(a), &t0, (0, 0), "X", selected(&t0, 0, 2));
    let t2 = record_edit(&mut h, Some(a), &t1, (1, 1), "Y", selected(&t1, 1, 3));
    h.undo();
    let t3 = record_edit(&mut h, Some(b), &t1, (1, 2), "", selected(&t1, 1, 2));
    let t4 = record_edit(
        &mut h,
        Some(b),
        &t3,
        (3, 3),
        "e\u{301}",
        selected(&t3, 0, 3),
    );
    let texts = [&t0, &t1, &t2, &t3, &t4]
        .iter()
        .map(|t| t.to_string())
        .collect();
    h.goto_revision(RevisionId(2));
    h.undo();
    Fixture {
        history: h,
        texts,
        current: t1,
        a,
        b,
    }
}

/// `fixture`'s history saved and restored onto a fresh copy of its text.
fn restored(f: &Fixture) -> (History, BufferText) {
    let text = BufferText::from(f.current.to_string().as_str());
    let mut h = History::new();
    h.restore(f.history.snapshot(), &text)
        .expect("a saved tree restores");
    (h, text)
}

#[test]
fn restore_reaches_every_revision_s_text_and_selections() {
    let f = fixture();
    for id in [0, 1, 2, 3, 4] {
        let (mut h, text) = restored(&f);
        let mut original = fixture();
        let got = walk(&text, h.goto_revision(RevisionId(id)).unwrap());
        let want = walk(
            &original.current,
            original.history.goto_revision(RevisionId(id)).unwrap(),
        );
        assert_eq!(got, want, "revision {id}");
        assert_eq!(got.0, f.texts[id]);
    }
}

#[test]
fn restore_keeps_the_redo_choice_and_the_ages() {
    let f = fixture();
    let (mut h, _) = restored(&f);
    let now = SystemTime::now();

    let ages = |h: &History| -> Vec<_> {
        h.nodes(now)
            .iter()
            .map(|n| (n.id(), n.parent(), n.age()))
            .collect()
    };
    assert_eq!(ages(&h), ages(&f.history));
    h.redo();
    assert_eq!(h.current_id(), RevisionId(2));
}

#[test]
fn restore_leaves_the_last_editor_s_revisions_for_the_first_pane_to_claim() {
    let f = fixture();
    let (mut h, _) = restored(&f);
    let [_, _, c] = fresh_ids();

    assert!(h.selection_points(f.b).is_empty());
    h.claim_restored(c);
    let revisions: Vec<_> = h
        .selection_points(c)
        .iter()
        .map(|p| (p.revision, p.side))
        .collect();
    assert_eq!(
        revisions,
        [(RevisionId(4), Side::Before), (RevisionId(3), Side::Before)]
    );
    assert!(h.selection_points(f.a).is_empty());
}

#[test]
fn restore_gives_the_last_editor_s_revisions_to_a_pane_that_claimed_before() {
    let f = fixture();
    let [_, _, c] = fresh_ids();
    let text = BufferText::from(f.current.to_string().as_str());
    let mut h = History::new();
    h.claim_restored(c);

    h.restore(f.history.snapshot(), &text)
        .expect("a saved tree restores");
    let revisions: Vec<_> = h.selection_points(c).iter().map(|p| p.revision).collect();
    assert_eq!(revisions, [RevisionId(4), RevisionId(3)]);
}

fn owned(h: &History) -> Vec<bool> {
    h.snapshot().revisions.iter().map(|r| r.owned).collect()
}

#[test]
fn snapshot_keeps_the_newest_editor_s_revisions_when_no_pane_made_the_current_one() {
    let [a] = fresh_ids();
    let mut h = History::new();
    let t0 = BufferText::from("abc\n");
    let t1 = record_edit(&mut h, Some(a), &t0, (0, 0), "X", selected(&t0, 0, 2));
    let t2 = record_edit(&mut h, Some(a), &t1, (1, 1), "Y", selected(&t1, 1, 3));
    record_edit(&mut h, None, &t2, (0, 1), "", selected(&t2, 0, 0));
    assert_eq!(owned(&h), [false, true, true, false]);
}

#[test]
fn snapshot_keeps_the_newest_editor_s_revisions_at_the_root() {
    let [a] = fresh_ids();
    let mut h = History::new();
    let t0 = BufferText::from("abc\n");
    let t1 = record_edit(&mut h, Some(a), &t0, (0, 0), "X", selected(&t0, 0, 2));
    record_edit(&mut h, Some(a), &t1, (1, 1), "Y", selected(&t1, 1, 3));
    h.undo_n(2);
    assert_eq!(owned(&h), [false, true, true]);
}

#[test]
fn restore_after_the_root_was_promoted() {
    let mut h = History::new();
    h.set_undo_levels(1);
    let t0 = BufferText::from("abc\n");
    let t1 = record_edit(&mut h, None, &t0, (0, 0), "X", selected(&t0, 0, 0));
    let t2 = record_edit(&mut h, None, &t1, (0, 0), "Y", selected(&t1, 0, 0));
    assert_eq!(h.len(), 2, "the first edit was promoted into the root");

    let text = BufferText::from(t2.to_string().as_str());
    let mut back = History::new();
    back.restore(h.snapshot(), &text).unwrap();
    assert_eq!(walk(&text, back.undo_n(1)).0, t1.to_string());
    assert!(!back.can_undo());
}

// ── Rejections ────────────────────────────────────────────────────────────

fn rejected(f: &Fixture, text: &str, change: impl FnOnce(&mut HistorySnapshot)) -> RestoreError {
    let mut snapshot = f.history.snapshot();
    change(&mut snapshot);
    let mut h = History::new();
    let err = h
        .restore(snapshot, &BufferText::from(text))
        .expect_err("the snapshot is rejected");
    assert_eq!(
        h.len(),
        1,
        "a rejected snapshot leaves the history as it was"
    );
    err
}

fn revision<'s, 'a>(
    snapshot: &'s mut HistorySnapshot<'a>,
    id: usize,
) -> &'s mut RevisionSnapshot<'a> {
    snapshot.revisions.iter_mut().find(|r| r.id == id).unwrap()
}

fn link<'s, 'a>(snapshot: &'s mut HistorySnapshot<'a>, id: usize) -> &'s mut LinkSnapshot<'a> {
    revision(snapshot, id).link.as_mut().unwrap()
}

#[test]
fn restore_rejects_a_text_the_tree_does_not_end_on() {
    let f = fixture();
    assert!(matches!(
        rejected(&f, "something else\n", |_| {}),
        RestoreError::Change(1)
    ));
}

#[test]
fn restore_rejects_a_change_stored_for_the_wrong_direction() {
    let f = fixture();
    let text = f.current.to_string();
    let inverse = f.history.revisions[&RevisionId(2)].link().inverse.changes();
    let err = rejected(&f, &text, |s| {
        link(s, 2).change = Cow::Owned(inverse.ops().to_vec());
    });
    assert!(matches!(err, RestoreError::Change(2)));
}

#[test]
fn restore_rejects_a_selection_inside_a_cluster_or_past_the_end() {
    let f = fixture();
    let text = f.current.to_string();
    let inside = rejected(&f, &text, |s| link(s, 4).after.ranges = vec![(4, 4)]);
    assert!(matches!(inside, RestoreError::Selections(4)));
    let past = rejected(&f, &text, |s| link(s, 4).after.ranges = vec![(0, 99)]);
    assert!(matches!(past, RestoreError::Selections(4)));
}

#[test]
fn restore_rejects_a_parent_missing_or_newer_than_its_child() {
    let f = fixture();
    let text = f.current.to_string();
    let missing = rejected(&f, &text, |s| link(s, 3).parent = 9);
    assert!(matches!(missing, RestoreError::Parent(3)));
    let newer = rejected(&f, &text, |s| link(s, 3).parent = 4);
    assert!(matches!(newer, RestoreError::Parent(3)));
}

#[test]
fn restore_rejects_a_dangling_current_or_redo() {
    let f = fixture();
    let text = f.current.to_string();
    let current = rejected(&f, &text, |s| s.current = 9);
    assert!(matches!(current, RestoreError::Current));
    let redo = rejected(&f, &text, |s| revision(s, 1).redo = Some(4));
    assert!(matches!(redo, RestoreError::Redo(1)));
}
