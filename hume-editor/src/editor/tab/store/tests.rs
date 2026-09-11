use super::*;
use hume_engine::pane::Pane;
use hume_engine::pipeline::EngineView;
use hume_engine::theme::Theme;

/// Mint a real `PaneId` via a scratch `EngineView` — `PaneId` is opaque
/// (slotmap key), so a test needs a real slot to hand `TabStore` a valid
/// one, mirroring `BufferStore`'s own `make_id` test helper.
fn make_pane(ev: &mut EngineView) -> PaneId {
    let bid = ev.buffers.insert(());
    ev.panes.insert(Pane::new(bid)) // pane-lifecycle-safe: id-minter on a scratch, deliberately treeless EngineView
}

#[test]
fn new_seeds_a_single_tab_wrapping_the_initial_pane() {
    let mut ev = EngineView::new(Theme::default());
    let pid = make_pane(&mut ev);
    let (store, id) = TabStore::new(pid);

    assert_eq!(store.current(), id);
    assert_eq!(store.order(), &[id]);
    assert_eq!(store.len(), 1);
}

#[test]
fn open_after_current_inserts_right_after_current_and_switches() {
    let mut ev = EngineView::new(Theme::default());
    let p1 = make_pane(&mut ev);
    let p2 = make_pane(&mut ev);
    let (mut store, t1) = TabStore::new(p1);

    let t2 = store.open_after_current(LayoutTree::Leaf(p1), p1, LayoutTree::Leaf(p2), p2);

    assert_eq!(store.order(), &[t1, t2]);
    assert_eq!(store.current(), t2);
    assert_ne!(t1, t2);
}

#[test]
fn switch_round_trips_layout_and_focus_across_three_tabs() {
    let mut ev = EngineView::new(Theme::default());
    let p1 = make_pane(&mut ev);
    let p2 = make_pane(&mut ev);
    let p3 = make_pane(&mut ev);
    let (mut store, t1) = TabStore::new(p1);
    let t2 = store.open_after_current(LayoutTree::Leaf(p1), p1, LayoutTree::Leaf(p2), p2);
    let t3 = store.open_after_current(LayoutTree::Leaf(p2), p2, LayoutTree::Leaf(p3), p3);
    assert_eq!(store.order(), &[t1, t2, t3]);

    // Currently on t3 (layout live at LayoutTree::Leaf(p3)/focus p3, per
    // open_after_current's own return). Switch back to t1.
    let (layout, focus) = store.switch(LayoutTree::Leaf(p3), p3, t1);
    assert_eq!(layout, LayoutTree::Leaf(p1));
    assert_eq!(focus, p1);
    assert_eq!(store.current(), t1);

    // t3's own state must have been stashed correctly by that switch — jump
    // straight to it from t1 and confirm it comes back unchanged.
    let (layout, focus) = store.switch(LayoutTree::Leaf(p1), p1, t3);
    assert_eq!(layout, LayoutTree::Leaf(p3));
    assert_eq!(focus, p3);
    assert_eq!(store.current(), t3);

    // t2 (never re-visited since its own open_after_current call) must
    // still hold what it was opened with.
    let (layout, focus) = store.switch(LayoutTree::Leaf(p3), p3, t2);
    assert_eq!(layout, LayoutTree::Leaf(p2));
    assert_eq!(focus, p2);
}

#[test]
fn switch_to_current_tab_is_reachable_but_a_noop_for_the_store() {
    // TabStore::switch itself has no early-return — that's `tab::switch_to_tab`'s
    // job (the chokepoint callers actually use). Called directly here it
    // still behaves correctly: stashing then immediately re-loading the
    // same tab is idempotent.
    let mut ev = EngineView::new(Theme::default());
    let p1 = make_pane(&mut ev);
    let (mut store, t1) = TabStore::new(p1);

    let (layout, focus) = store.switch(LayoutTree::Leaf(p1), p1, t1);
    assert_eq!(layout, LayoutTree::Leaf(p1));
    assert_eq!(focus, p1);
    assert_eq!(store.current(), t1);
}

#[test]
fn close_current_removes_from_order_and_promotes_survivor() {
    let mut ev = EngineView::new(Theme::default());
    let p1 = make_pane(&mut ev);
    let p2 = make_pane(&mut ev);
    let (mut store, t1) = TabStore::new(p1);
    let t2 = store.open_after_current(LayoutTree::Leaf(p1), p1, LayoutTree::Leaf(p2), p2);
    assert_eq!(store.current(), t2);

    let (layout, focus) = store.close_current(t1);

    assert_eq!(store.order(), &[t1]);
    assert_eq!(store.len(), 1);
    assert_eq!(store.current(), t1);
    assert_eq!(layout, LayoutTree::Leaf(p1));
    assert_eq!(focus, p1);
}

#[test]
#[should_panic(expected = "close_current requires more than one tab")]
fn close_current_panics_on_the_last_tab() {
    let mut ev = EngineView::new(Theme::default());
    let p1 = make_pane(&mut ev);
    let (mut store, t1) = TabStore::new(p1);

    store.close_current(t1);
}

#[test]
fn next_and_prev_wrap_around() {
    let mut ev = EngineView::new(Theme::default());
    let p1 = make_pane(&mut ev);
    let p2 = make_pane(&mut ev);
    let p3 = make_pane(&mut ev);
    let (mut store, t1) = TabStore::new(p1);
    let t2 = store.open_after_current(LayoutTree::Leaf(p1), p1, LayoutTree::Leaf(p2), p2);
    let _t3 = store.open_after_current(LayoutTree::Leaf(p2), p2, LayoutTree::Leaf(p3), p3);
    // current() == t3 here.

    assert_eq!(store.next(), t1, "wraps from the last tab to the first");
    assert_eq!(store.prev(), t2);
}

#[test]
fn next_and_prev_are_noops_with_one_tab() {
    let mut ev = EngineView::new(Theme::default());
    let p1 = make_pane(&mut ev);
    let (store, t1) = TabStore::new(p1);

    assert_eq!(store.next(), t1);
    assert_eq!(store.prev(), t1);
}

#[test]
fn adjacent_prefers_the_left_neighbour() {
    let mut ev = EngineView::new(Theme::default());
    let p1 = make_pane(&mut ev);
    let p2 = make_pane(&mut ev);
    let p3 = make_pane(&mut ev);
    let (mut store, t1) = TabStore::new(p1);
    let t2 = store.open_after_current(LayoutTree::Leaf(p1), p1, LayoutTree::Leaf(p2), p2);
    let t3 = store.open_after_current(LayoutTree::Leaf(p2), p2, LayoutTree::Leaf(p3), p3);
    assert_eq!(store.order(), &[t1, t2, t3]);
    // current() == t3 here — its left neighbour is t2.

    assert_eq!(store.adjacent(), t2);
}

#[test]
fn adjacent_of_the_leftmost_tab_is_its_right_neighbour() {
    let mut ev = EngineView::new(Theme::default());
    let p1 = make_pane(&mut ev);
    let p2 = make_pane(&mut ev);
    let (mut store, t1) = TabStore::new(p1);
    let _t2 = store.open_after_current(LayoutTree::Leaf(p1), p1, LayoutTree::Leaf(p2), p2);
    // Switch back onto the leftmost tab, t1, which has no left neighbour.
    store.switch(LayoutTree::Leaf(p2), p2, t1);

    assert_eq!(store.adjacent(), store.order()[1]);
}

#[test]
#[should_panic(expected = "adjacent requires more than one tab")]
fn adjacent_panics_on_the_last_tab() {
    let mut ev = EngineView::new(Theme::default());
    let p1 = make_pane(&mut ev);
    let (store, _t1) = TabStore::new(p1);

    store.adjacent();
}

#[test]
fn stashed_focus_reads_an_inactive_tab_s_last_focused_pane() {
    let mut ev = EngineView::new(Theme::default());
    let p1 = make_pane(&mut ev);
    let p2 = make_pane(&mut ev);
    let (mut store, t1) = TabStore::new(p1);
    let _t2 = store.open_after_current(LayoutTree::Leaf(p1), p1, LayoutTree::Leaf(p2), p2);

    // t1 is now inactive; its stash entry was written by open_after_current.
    assert_eq!(store.stashed_focus(t1), p1);
}
