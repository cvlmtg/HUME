//! [`PickerSession`](super::PickerSession) tests — matches the convention
//! every other large module in `editor/` follows for its own `mod tests`.

use super::*;

fn items(displays: &[&str]) -> Vec<PickerItem> {
    displays.iter().map(|d| item(d)).collect()
}

fn dummy_on_select() -> SteelVal {
    SteelVal::BoolV(false)
}

fn open() -> PickerSession {
    PickerSession::new(dummy_on_select(), PickerOpts::default())
}

fn open_pending() -> PickerSession {
    PickerSession::new(
        dummy_on_select(),
        PickerOpts {
            pending: true,
            ..Default::default()
        },
    )
}

fn open_live() -> PickerSession {
    PickerSession::new_live(
        dummy_on_select(),
        LivePickerOpts {
            prompt: String::new(),
            query: String::new(),
            on_query_change: dummy_on_select(),
            truncate: TruncateEnd::Head,
            actions: Vec::new(),
        },
    )
}

fn payload_str(v: &SteelVal) -> &str {
    match v {
        SteelVal::StringV(s) => s.as_str(),
        other => panic!("expected StringV payload, got {other:?}"),
    }
}

fn window_vec(s: &PickerSession, rows: usize) -> Vec<&str> {
    s.window(rows).collect()
}

#[test]
fn new_session_is_empty() {
    let s = open();
    assert_eq!(s.total_len(), 0);
    assert_eq!(s.matched_len(), 0);
    assert_eq!(s.selected(), 0);
    assert!(window_vec(&s, 10).is_empty());
}

#[test]
fn prompt_is_stored_verbatim() {
    let s = PickerSession::new(
        dummy_on_select(),
        PickerOpts {
            prompt: "files: ".to_string(),
            ..Default::default()
        },
    );
    assert_eq!(s.prompt(), "files: ");
}

#[test]
fn not_pending_by_default() {
    assert!(!open().is_pending());
}

#[test]
fn pending_flag_set_on_open_and_cleared_by_a_matching_push() {
    let mut s = open_pending();
    assert!(s.is_pending());
    s.push(items(&["a"]));
    assert!(!s.is_pending(), "a matching push must clear pending");
}

#[test]
fn pending_flag_cleared_by_a_matching_push_even_with_an_empty_batch() {
    // A clean `git status` still means the job finished — pending must
    // not stay stuck just because there was nothing to add.
    let mut s = open_pending();
    s.push(items(&[]));
    assert!(!s.is_pending());
}

#[test]
fn seed_with_items_clears_pending() {
    let mut s = open_pending();
    s.seed(items(&["a"]));
    assert!(
        !s.is_pending(),
        "seeding real items means the list is already populated — no \"still arriving\" marker needed"
    );
    assert_eq!(window_vec(&s, 10), vec!["a"]);
}

#[test]
fn empty_seed_leaves_pending_intact() {
    let mut s = open_pending();
    s.seed(items(&[]));
    assert!(
        s.is_pending(),
        "an empty seed is not a batch arrival — `#:pending`'s caller intent must survive it"
    );
}

#[test]
fn take_source_on_a_sourceless_pending_session_preserves_awaiting() {
    // `picker-source-stop!` racing a session that never had a source
    // attached (only `#:pending`) must not fabricate a "done" transition
    // — `take_source` restores `Awaiting` rather than leaving `Complete`
    // behind from its own `mem::replace`.
    let mut s = open_pending();
    assert!(s.take_source().is_none());
    assert!(
        s.is_pending(),
        "a stop racing a never-attached source must not clear pending"
    );
}

#[test]
fn two_sessions_get_distinct_tokens() {
    let s1 = open();
    let s2 = open();
    assert_ne!(s1.token(), s2.token());
}

#[test]
fn push_with_empty_query_keeps_insertion_order() {
    let mut s = open();
    s.push(items(&["b", "a", "c"]));
    assert_eq!(window_vec(&s, 10), vec!["b", "a", "c"]);
}

#[test]
fn second_push_appends_after_first() {
    let mut s = open();
    s.push(items(&["a", "b"]));
    s.push(items(&["c", "d"]));
    assert_eq!(window_vec(&s, 10), vec!["a", "b", "c", "d"]);
}

#[test]
fn set_query_filters_non_matches() {
    let mut s = open();
    s.push(items(&["foo", "bar"]));
    s.set_query("f".to_string());
    assert_eq!(window_vec(&s, 10), vec!["foo"]);
}

#[test]
fn query_prefill_is_visible_and_applied_at_construction() {
    let mut s = PickerSession::new(
        dummy_on_select(),
        PickerOpts {
            query: "f".to_string(),
            ..Default::default()
        },
    );
    assert_eq!(s.query(), "f");
    // Applied through the same `rerank` a later push would use — a
    // batch arriving after open is filtered by the prefilled query
    // immediately, not just once a keystroke re-triggers ranking.
    s.push(items(&["foo", "bar"]));
    assert_eq!(window_vec(&s, 10), vec!["foo"]);
}

#[test]
fn replace_swaps_the_item_list_instead_of_appending() {
    let mut s = open();
    s.push(items(&["a", "b"]));
    s.replace(items(&["c"]));
    assert_eq!(window_vec(&s, 10), vec!["c"]);
    assert_eq!(s.total_len(), 1);
}

#[test]
fn replace_clears_pending_even_with_an_empty_batch() {
    let mut s = open_pending();
    s.replace(items(&[]));
    assert!(!s.is_pending());
}

#[test]
fn live_session_keeps_insertion_order_regardless_of_query() {
    // See `rebuild_filtered`'s doc for why a live session must skip
    // scoring entirely, the same branch an empty query already takes.
    let mut s = open_live();
    s.set_query("zzz-does-not-fuzzy-match-anything".to_string());
    s.push(items(&["b", "a", "c"]));
    assert_eq!(window_vec(&s, 10), vec!["b", "a", "c"]);
}

#[test]
fn live_session_insert_char_keeps_insertion_order_and_still_resets_the_cursor() {
    // A live session's `rebuild_filtered` always recomputes the same
    // identity permutation over `items` (see its doc) — this proves the
    // recompute is invisible: the ranked order survives untouched, and
    // the cursor reset rides along exactly as it would for a real
    // rebuild.
    let mut s = open_live();
    s.push(items(&["b", "a", "c"]));
    s.move_selection(2, 3);
    assert_eq!(s.selected(), 2);
    let before: Vec<String> = window_vec(&s, 10).into_iter().map(str::to_string).collect();
    let _ = s.insert_char('z');
    assert_eq!(window_vec(&s, 10), before);
    assert_eq!(s.selected(), 0);
    assert_eq!(s.scroll(), 0);
}

#[test]
fn live_session_query_change_is_pending_until_the_next_batch() {
    // A live requery's stop/debounce/respawn gap has no attached source
    // for most of its span (`picker-source-stop!` takes it immediately),
    // so `is_pending` can't ride `population` alone here the way it does
    // for a streaming/#:pending session — it must stay true from the
    // query edit itself through to the requery's own swap.
    let mut s = open_live();
    assert!(!s.is_pending());
    assert!(
        s.insert_char('a').is_some(),
        "a live session's query change must still fire its callback"
    );
    assert!(
        s.is_pending(),
        "a live query change must mark the session pending even with no source attached"
    );
    s.replace(items(&["x"]));
    assert!(
        !s.is_pending(),
        "the requery's own swap is what ends a live requery's pending window"
    );
}

#[test]
fn live_session_batch_from_a_stale_source_does_not_end_the_pending_window() {
    // A batch queued from the *outgoing* source can still land (via
    // `push`) after a keystroke has armed the next requery but before
    // `settle()` gets to the queued `picker-source-stop!` callback —
    // `drain_async_sources` runs ahead of `drain_pending_work` (see
    // `Editor::settle`'s doc). Only the requery's own swap (`replace`)
    // may end the window; an ordinary append must leave it armed.
    let mut s = open_live();
    assert!(s.insert_char('a').is_some());
    assert!(s.is_pending());

    s.push(items(&["x"]));
    assert!(
        s.is_pending(),
        "a plain append must not end a live requery's pending window — only \
             the swap `replace` performs does"
    );
}

#[test]
fn non_live_session_query_change_never_sets_pending() {
    let mut s = open();
    assert!(
        s.insert_char('a').is_none(),
        "a non-live session's query change has no callback to fire"
    );
    assert!(
        !s.is_pending(),
        "a non-live session's local filter never marks the session pending"
    );
}

#[test]
fn non_live_session_with_the_same_query_still_filters() {
    // Same query as above, on a non-live session — confirms the
    // insertion-order result above comes from live mode, not from the
    // query happening to fail to fuzzy-match anyway.
    let mut s = open();
    s.set_query("zzz-does-not-fuzzy-match-anything".to_string());
    s.push(items(&["b", "a", "c"]));
    assert!(window_vec(&s, 10).is_empty());
}

#[test]
fn pop_grapheme_on_an_empty_query_is_a_no_op_even_for_a_live_session() {
    // `pop_grapheme` returns `Option<SteelVal>` — a query-content check
    // alone can't tell "returned `None`" apart from "returned the
    // callback", so this pins the return value directly on the one
    // session shape (`PickerMode::Live`) where mistaking those two
    // would matter.
    let mut s = open_live();
    assert!(s.pop_grapheme().is_none());
}

#[test]
fn better_match_ranks_first_regardless_of_insertion() {
    let mut s = open();
    // Scattered subsequence pushed before the boundary match.
    s.push(items(&["fxxbxx", "foo/bar"]));
    s.set_query("fb".to_string());
    assert_eq!(window_vec(&s, 10), vec!["foo/bar", "fxxbxx"]);
}

#[test]
fn equal_scores_tie_break_by_insertion_order() {
    let mut s = open();
    // Two score tiers (lower-scoring "fxxbxx" scattered matches, then
    // higher-scoring "foo/bar" boundary matches), pushed low-score tier
    // first — so the pre-sort array is not already in the target
    // (descending-score) order and the sort must do genuine rearranging
    // work, not just detect an already-sorted/already-reversed run and
    // leave it untouched. Within each equal-score tier, payload order
    // must still match insertion order.
    const TIER: u32 = 32;
    let mut tagged = Vec::new();
    for i in 0..TIER {
        tagged.push(PickerItem {
            display: "fxxbxx".to_string(),
            payload: SteelVal::StringV(format!("low{i}").into()),
        });
    }
    for i in 0..TIER {
        tagged.push(PickerItem {
            display: "foo/bar".to_string(),
            payload: SteelVal::StringV(format!("high{i}").into()),
        });
    }
    s.push(tagged);
    s.set_query("fb".to_string());
    assert_eq!(s.matched_len(), (2 * TIER) as usize);
    for i in 0..TIER {
        let payload = s.selected_payload().expect("has a match");
        assert_eq!(payload_str(payload), format!("high{i}"));
        s.move_selection(1, (2 * TIER) as usize);
    }
    for i in 0..TIER {
        let payload = s.selected_payload().expect("has a match");
        assert_eq!(payload_str(payload), format!("low{i}"));
        s.move_selection(1, (2 * TIER) as usize);
    }
}

#[test]
fn push_keeps_the_selection_on_the_same_item() {
    // A streaming source pushes once per frame — snapping the selection
    // back to row 0 on every batch would make an actively-scrolled
    // picker unnavigable, so a plain append must keep pointing at the
    // same item instead of resetting.
    let mut s = open();
    s.push(items(&["a", "b", "c", "d", "e"]));
    s.move_selection(3, 2);
    assert_eq!(payload_str(s.selected_payload().expect("has a match")), "d");
    s.push(items(&["f"]));
    assert_eq!(
        payload_str(s.selected_payload().expect("has a match")),
        "d",
        "a push must not move the selection off the item the user had selected"
    );
}

#[test]
fn replace_always_resets_selection_and_scroll() {
    // Unlike `push`, `replace` swaps in an unrelated item list — the old
    // selection's index cannot mean the same thing afterward, so it must
    // always land back on row 0, never a same-index coincidence.
    let mut s = open();
    s.push(items(&["a", "b", "c", "d", "e"]));
    s.move_selection(3, 2);
    assert_ne!(s.selected(), 0);
    s.replace(items(&["x", "y", "z"]));
    assert_eq!(s.selected(), 0);
    assert_eq!(s.scroll(), 0);
}

#[test]
fn set_query_resets_selection_and_scroll() {
    let mut s = open();
    s.push(items(&["apple", "banana", "cherry", "date"]));
    s.move_selection(2, 2);
    assert_ne!(s.selected(), 0);
    s.set_query("a".to_string());
    assert_eq!(s.selected(), 0);
    assert_eq!(s.scroll(), 0);
}

#[test]
fn widening_query_restores_matches() {
    let mut s = open();
    s.push(items(&["foo", "bar"]));
    let _ = s.insert_char('z');
    assert_eq!(s.matched_len(), 0);
    let _ = s.pop_grapheme();
    assert_eq!(s.query(), "", "pop_grapheme must have removed the 'z'");
    assert_eq!(s.matched_len(), 2);
    let _ = s.pop_grapheme(); // query now empty; further pop is a no-op
    assert_eq!(s.query(), "");
}

#[test]
fn pop_grapheme_removes_full_cluster() {
    let mut s = open();
    // "e" + combining acute accent (U+0301) forms one grapheme cluster.
    let _ = s.insert_char('e');
    let _ = s.insert_char('\u{0301}');
    assert_eq!(s.query(), "e\u{0301}");
    let _ = s.pop_grapheme();
    assert_eq!(s.query(), "");
    assert!(s.query().is_char_boundary(0));

    // ZWJ emoji sequence: family emoji built from 4 code points joined
    // by ZWJ — one pop_grapheme must remove the whole cluster.
    for ch in "👨‍👩‍👧‍👦".chars() {
        let _ = s.insert_char(ch);
    }
    let _ = s.pop_grapheme();
    assert_eq!(s.query(), "");
}

#[test]
fn move_selection_is_bounded_no_wrap() {
    let mut s = open();
    s.push(items(&["a", "b", "c"]));
    s.move_selection(-5, 10);
    assert_eq!(s.selected(), 0);
    s.move_selection(10, 10);
    assert_eq!(s.selected(), 2);
    // Page move past the end stays clamped.
    s.move_selection(3, 10);
    assert_eq!(s.selected(), 2);
}

#[test]
fn move_selection_scrolls_to_keep_selected_visible() {
    let mut s = open();
    s.push(items(&["0", "1", "2", "3", "4", "5", "6", "7", "8", "9"]));
    s.move_selection(5, 3);
    assert_eq!(s.selected(), 5);
    assert_eq!(s.scroll(), 3); // 5 + 1 - 3
    s.move_selection(-4, 3);
    assert_eq!(s.selected(), 1);
    assert_eq!(s.scroll(), 1);
    // visible_rows == 0 is a documented no-op on the scroll clamp.
    let scroll_before = s.scroll();
    s.move_selection(1, 0);
    assert_eq!(s.scroll(), scroll_before);
}

#[test]
fn empty_filter_result_is_safe() {
    let mut s = open();
    s.push(items(&["foo", "bar"]));
    s.set_query("zzz".to_string());
    assert_eq!(s.selected(), 0);
    s.move_selection(5, 3); // must not panic
    assert_eq!(s.selected(), 0);
    assert!(window_vec(&s, 10).is_empty());
    assert!(s.selected_payload().is_none());
}

#[test]
fn window_respects_scroll_and_rows() {
    let mut s = open();
    s.push(items(&["0", "1", "2", "3", "4", "5", "6", "7", "8", "9"]));
    s.move_selection(6, 3); // scroll becomes 4
    assert_eq!(s.scroll(), 4);
    assert_eq!(window_vec(&s, 3), vec!["4", "5", "6"]);
}

#[test]
fn selected_payload_returns_top_ranked_item() {
    let mut s = open();
    s.push(items(&["fxxbxx", "foo/bar"]));
    s.set_query("fb".to_string());
    assert_eq!(
        payload_str(s.selected_payload().expect("has a match")),
        "foo/bar"
    );
    s.move_selection(1, 10);
    assert_eq!(
        payload_str(s.selected_payload().expect("has a match")),
        "fxxbxx"
    );
}
