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
    ring.push("xylophone".into());
    // First prev: stashes "x" as scratch (the prefix search is satisfied by "xylophone").
    assert_eq!(ring.prev("x"), Some("xylophone".into()));
    assert_eq!(ring.scratch, Some("x".into()));
    // Navigating forward past newest restores scratch.
    assert_eq!(ring.next(), Some("x".into()));
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
    // Next prev re-stashes current text (empty prefix matches "b").
    assert_eq!(h.prev(""), Some("b".into()));
    assert_eq!(h.scratch, Some("".into()));
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
fn prev_filters_by_typed_prefix() {
    let mut h = h(10);
    h.push("e".into());
    h.push("plum-install-grammar".into());
    h.push("pwd".into());
    // "pl" skips "pwd" (no match) and lands on "plum-install-grammar".
    assert_eq!(h.prev("pl"), Some("plum-install-grammar".into()));
    // No older entry starts with "pl" — position unchanged, returns None.
    assert_eq!(h.prev("pl"), None);
    assert_eq!(h.cursor, Some(1));
}

#[test]
fn prefix_persists_across_prev_next_steps() {
    let mut h = h(10);
    h.push("foo1".into());
    h.push("bar".into());
    h.push("foo2".into());
    assert_eq!(h.prev("fo"), Some("foo2".into()));
    assert_eq!(h.prev("fo"), Some("foo1".into()));
    assert_eq!(h.next(), Some("foo2".into()));
    // Past newest match restores the stashed prefix, not the raw entry.
    assert_eq!(h.next(), Some("fo".into()));
    assert_eq!(h.cursor, None);
}

#[test]
fn prev_first_step_miss_leaves_state_untouched() {
    let mut h = h(10);
    h.push("abc".into());
    assert_eq!(h.prev("z"), None);
    assert_eq!(h.cursor, None);
    assert_eq!(h.scratch, None);
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
fn set_capacity_defers_trim_to_next_push() {
    // Fail oracle: if set_capacity trimmed immediately, entries.len() would
    // drop to 2 right after the call instead of only on the next push.
    let mut h = h(10);
    h.push("a".into());
    h.push("b".into());
    h.push("c".into());
    assert_eq!(h.entries.len(), 3);

    // Shrink: existing entries are untouched — Vim-style deferred trim.
    h.set_capacity(2);
    assert_eq!(h.capacity, 2);
    assert_eq!(
        h.entries.len(),
        3,
        "lowering the cap must not retroactively trim"
    );

    // The next push converges to the new cap in one shot.
    h.push("d".into());
    assert_eq!(h.entries.len(), 2);
    assert_eq!(h.entries.back().map(|s| s.as_str()), Some("d"));
    assert_eq!(h.entries.front().map(|s| s.as_str()), Some("c"));
}

/// A shrink immediately followed by a raise, with no push in between, must
/// not lose entries in the transient window — `:reload-config` resets
/// `history-capacity` to its compiled-in default before `init.scm`
/// re-raises it, and an eager trim would have discarded everything past the
/// default before the raise had a chance to take effect.
#[test]
fn shrink_then_raise_with_no_push_between_resurrects_every_entry() {
    let mut h = h(20);
    for i in 0..20 {
        h.push(format!("cmd{i}"));
    }
    assert_eq!(h.entries.len(), 20);

    // The reset: shrink to a smaller default. Deferred — no trim yet.
    h.set_capacity(5);
    assert_eq!(
        h.entries.len(),
        20,
        "shrinking must not eagerly trim — nothing has pushed since"
    );

    // init.scm re-raising the setting. Still no push — nothing to converge.
    // Fail oracle: an eager trim on the shrink above would have already
    // dropped every entry past 5, and raising the cap back up here can't
    // resurrect what's already gone — entries.len() would stay at 5 instead
    // of climbing back to 20.
    h.set_capacity(20);
    assert_eq!(
        h.entries.len(),
        20,
        "raising the cap back up before any push must resurrect every entry, \
         not just whatever survived a (nonexistent) eager trim"
    );
    assert_eq!(
        h.entries.front().map(String::as_str),
        Some("cmd0"),
        "the very first entry must still be there, not just the count"
    );
}

#[test]
fn set_capacity_shrink_converges_on_a_duplicate_push() {
    // Fail oracle: the consecutive-duplicate branch used to `return` before
    // the trim loop, so a shrink only converged on a push that landed a
    // genuinely new entry — never on a resubmission of the same entry.
    let mut h = h(10);
    h.push("a".into());
    h.push("b".into());
    h.push("c".into());
    assert_eq!(h.entries.len(), 3);

    h.set_capacity(2);

    // Consecutive duplicate of the last entry — hits the dedup branch, not
    // the plain append.
    h.push("c".into());
    assert_eq!(
        h.entries.len(),
        2,
        "a deduplicated push must still converge to the shrunk cap"
    );
    assert_eq!(h.entries.back().map(|s| s.as_str()), Some("c"));
    assert_eq!(h.entries.front().map(|s| s.as_str()), Some("b"));
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
