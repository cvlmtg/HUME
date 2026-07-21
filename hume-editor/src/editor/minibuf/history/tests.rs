use super::*;

fn h(capacity: usize) -> History {
    History::new(capacity)
}

fn store(capacity: usize) -> HistoryStore {
    HistoryStore::new(capacity)
}

// ── History ───────────────────────────────────────────────────────────────

#[test]
fn push_and_prev_walks_back() {
    let mut h = h(10);
    h.push("a".into());
    h.push("b".into());
    h.push("c".into());
    assert_eq!(h.prev(""), Some("c".into()));
    assert_eq!(h.prev(""), Some("b".into()));
    assert_eq!(h.prev(""), Some("a".into()));
    assert_eq!(h.prev(""), None); // at oldest
}

#[test]
fn next_after_prev_walks_forward_then_scratch() {
    let mut h = h(10);
    h.push("a".into());
    h.push("b".into());
    h.push("c".into());
    // Walk to oldest.
    h.prev("");
    h.prev("");
    h.prev("");
    // Walk back forward.
    assert_eq!(h.next(), Some("b".into()));
    assert_eq!(h.next(), Some("c".into()));
    // Past newest restores scratch ("").
    assert_eq!(h.next(), Some("".into()));
}

#[test]
fn next_returns_none_when_not_navigating() {
    let mut h = h(10);
    h.push("a".into());
    assert_eq!(h.next(), None);
}

#[test]
fn prev_stashes_scratch_on_first_call() {
    let mut ring = History::new(10);
    ring.push("x".into());
    // First prev: stashes "typed" as scratch.
    assert_eq!(ring.prev("typed"), Some("x".into()));
    assert_eq!(ring.scratch, Some("typed".into()));
    // Navigating forward past newest restores scratch.
    assert_eq!(ring.next(), Some("typed".into()));
    assert_eq!(ring.cursor, None);
}

#[test]
fn consecutive_duplicate_push_is_skipped() {
    let mut h = h(10);
    h.push("w".into());
    h.push("w".into());
    assert_eq!(h.entries.len(), 1);
}

#[test]
fn non_consecutive_duplicate_is_kept() {
    let mut h = h(10);
    h.push("w".into());
    h.push("q".into());
    h.push("w".into());
    assert_eq!(h.entries.len(), 3);
}

#[test]
fn capacity_evicts_oldest() {
    let mut h = h(3);
    h.push("a".into());
    h.push("b".into());
    h.push("c".into());
    h.push("d".into()); // evicts "a"
    assert_eq!(h.entries.len(), 3);
    assert_eq!(h.prev(""), Some("d".into()));
    assert_eq!(h.prev(""), Some("c".into()));
    assert_eq!(h.prev(""), Some("b".into()));
    assert_eq!(h.prev(""), None);
}

#[test]
fn prev_on_empty_history_returns_none() {
    let mut h = h(10);
    assert_eq!(h.prev("x"), None);
    // Scratch should NOT have been set — there was nothing to navigate to.
    assert!(h.scratch.is_none());
}

#[test]
fn prev_at_oldest_returns_none_keeps_position() {
    let mut h = h(10);
    h.push("a".into());
    h.prev(""); // lands on "a" (oldest)
    assert_eq!(h.cursor, Some(0));
    assert_eq!(h.prev(""), None); // still at oldest
    assert_eq!(h.cursor, Some(0)); // position unchanged
}

#[test]
fn demote_to_scratch_clears_navigation() {
    let mut h = h(10);
    h.push("a".into());
    h.push("b".into());
    h.prev(""); // cursor = Some(1) = "b"
    h.demote_to_scratch();
    assert_eq!(h.cursor, None);
    assert_eq!(h.scratch, None);
    // Next prev re-stashes current text.
    assert_eq!(h.prev("edited"), Some("b".into()));
    assert_eq!(h.scratch, Some("edited".into()));
}

#[test]
fn empty_push_is_ignored() {
    let mut h = h(10);
    h.push(String::new());
    assert_eq!(h.entries.len(), 0);
}

#[test]
fn begin_session_resets_nav_but_keeps_entries() {
    let mut h = h(10);
    h.push("a".into());
    h.prev(""); // cursor = Some(0)
    h.begin_session();
    assert_eq!(h.cursor, None);
    assert_eq!(h.scratch, None);
    assert_eq!(h.entries.len(), 1); // entry still there
    // Can navigate again in the new session.
    assert_eq!(h.prev(""), Some("a".into()));
}

#[test]
fn restore_round_trips_entries() {
    let original: Vec<String> = vec!["a".into(), "b".into(), "c".into()];
    let h = History::restore(original.clone(), 10);
    assert_eq!(h.entries.iter().cloned().collect::<Vec<_>>(), original);
}

#[test]
fn restore_caps_to_capacity() {
    let entries: Vec<String> = (0..10).map(|i| i.to_string()).collect();
    let h = History::restore(entries, 3);
    assert_eq!(h.entries.len(), 3);
    // Most-recent entries are kept.
    assert_eq!(h.entries.back().map(|s| s.as_str()), Some("9"));
}

// ── HistoryStore ──────────────────────────────────────────────────────────

#[test]
fn kind_for_prompt_maps_colon_slash_question() {
    assert_eq!(
        HistoryStore::kind_for_prompt(":"),
        Some(HistoryKind::Command)
    );
    assert_eq!(
        HistoryStore::kind_for_prompt("/"),
        Some(HistoryKind::SearchForward)
    );
    assert_eq!(
        HistoryStore::kind_for_prompt("?"),
        Some(HistoryKind::SearchBackward)
    );
    assert_eq!(HistoryStore::kind_for_prompt("⫽"), None);
    assert_eq!(HistoryStore::kind_for_prompt("x"), None);
}

#[test]
fn snapshot_and_restore_round_trips_entries() {
    let mut s = store(10);
    s.get_mut(HistoryKind::Command).push("w".into());
    s.get_mut(HistoryKind::SearchForward).push("foo".into());
    s.get_mut(HistoryKind::SearchBackward).push("bar".into());

    let snap = s.snapshot();
    let restored = HistoryStore::restore(snap, 10);

    assert_eq!(
        restored
            .get(HistoryKind::Command)
            .entries()
            .iter()
            .cloned()
            .collect::<Vec<_>>(),
        vec!["w"],
    );
    assert_eq!(
        restored
            .get(HistoryKind::SearchForward)
            .entries()
            .iter()
            .cloned()
            .collect::<Vec<_>>(),
        vec!["foo"],
    );
    assert_eq!(
        restored
            .get(HistoryKind::SearchBackward)
            .entries()
            .iter()
            .cloned()
            .collect::<Vec<_>>(),
        vec!["bar"],
    );
}

#[test]
fn set_capacity_updates_limit_and_trims_oldest() {
    let mut h = h(10);
    h.push("a".into());
    h.push("b".into());
    h.push("c".into());
    assert_eq!(h.entries.len(), 3);

    // Shrink: oldest entries are trimmed to fit.
    h.set_capacity(2);
    assert_eq!(h.capacity, 2);
    assert_eq!(h.entries.len(), 2);
    assert_eq!(h.entries.back().map(|s| s.as_str()), Some("c"));
    assert_eq!(h.entries.front().map(|s| s.as_str()), Some("b"));

    // Future pushes respect the new limit.
    h.push("d".into());
    assert_eq!(h.entries.len(), 2);
    assert_eq!(h.entries.back().map(|s| s.as_str()), Some("d"));
}

#[test]
fn history_store_set_capacity_updates_all_rings() {
    let mut s = store(10);
    s.get_mut(HistoryKind::Command).push("w".into());
    s.get_mut(HistoryKind::SearchForward).push("foo".into());
    s.get_mut(HistoryKind::SearchBackward).push("bar".into());

    s.set_capacity(5);

    assert_eq!(s.get(HistoryKind::Command).capacity, 5);
    assert_eq!(s.get(HistoryKind::SearchForward).capacity, 5);
    assert_eq!(s.get(HistoryKind::SearchBackward).capacity, 5);
    // Existing entries fit in 5 so none were evicted.
    assert_eq!(s.get(HistoryKind::Command).entries().len(), 1);
}

#[test]
fn begin_session_all_resets_all_rings() {
    let mut s = store(10);
    s.get_mut(HistoryKind::Command).push("w".into());
    s.get_mut(HistoryKind::SearchForward).push("foo".into());
    s.get_mut(HistoryKind::Command).prev("");
    s.get_mut(HistoryKind::SearchForward).prev("");
    assert!(s.get(HistoryKind::Command).cursor.is_some());
    assert!(s.get(HistoryKind::SearchForward).cursor.is_some());

    s.begin_session_all();

    assert!(s.get(HistoryKind::Command).cursor.is_none());
    assert!(s.get(HistoryKind::SearchForward).cursor.is_none());
}
