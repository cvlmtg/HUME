use super::*;
use hume_engine::pane::Pane;
use hume_engine::pipeline::{EngineView, FocusedLayout};
use hume_engine::theme::Theme;

/// A one-pane layout on a real `PaneId` minted via a scratch `EngineView`.
/// `PaneId` is opaque (slotmap key), so a test needs a real slot to hand
/// `TabStore` a valid one, mirroring `BufferStore`'s own `make_id` test
/// helper.
fn make_layout(ev: &mut EngineView) -> FocusedLayout {
    let bid = ev.buffers.insert(());
    FocusedLayout::new(ev.insert_pane(Pane::new(bid)))
}

#[test]
fn new_seeds_a_single_tab_with_nothing_stashed() {
    let store = TabStore::new();

    assert_eq!(store.order(), &[store.current()]);
    assert_eq!(store.len(), 1);
    assert_eq!(store.inactive_layouts().count(), 0);
}

#[test]
fn open_after_current_inserts_right_after_current_and_switches() {
    let (mut ev, _) = EngineView::new(Theme::default());
    let l1 = make_layout(&mut ev);
    let l2 = make_layout(&mut ev);
    let mut store = TabStore::new();
    let t1 = store.current();
    let t2 = store.open_after_current(l1);
    // Back on t1, a new tab lands between t1 and t2.
    let l1 = store.take(t1);
    store.switch(l2, t1);

    let t3 = store.open_after_current(l1);

    assert_eq!(store.order(), &[t1, t3, t2]);
    assert_eq!(store.current(), t3);
}

#[test]
fn switch_round_trips_each_tab_s_layout_across_three_tabs() {
    let (mut ev, _) = EngineView::new(Theme::default());
    let l1 = make_layout(&mut ev);
    let l2 = make_layout(&mut ev);
    let l3 = make_layout(&mut ev);
    let [p1, p2, p3] = [&l1, &l2, &l3].map(FocusedLayout::focused);
    let mut store = TabStore::new();
    let t1 = store.current();
    let t2 = store.open_after_current(l1);
    let t3 = store.open_after_current(l2);
    assert_eq!(store.order(), &[t1, t2, t3]);

    // Live: l3. Each step installs the taken layout and stashes the live one.
    let live = store.take(t1);
    assert_eq!(live.focused(), p1);
    store.switch(l3, t1);
    assert_eq!(store.current(), t1);

    let next = store.take(t3);
    assert_eq!(next.focused(), p3);
    store.switch(live, t3);

    let last = store.take(t2);
    assert_eq!(last.focused(), p2);
    store.switch(next, t2);
    assert_eq!(store.current(), t2);
}

#[test]
#[should_panic(expected = "only an inactive tab is taken, and only once")]
fn take_panics_on_the_active_tab() {
    let mut store = TabStore::new();
    let t1 = store.current();

    store.take(t1);
}

#[test]
fn close_current_removes_from_order_and_promotes_survivor() {
    let (mut ev, _) = EngineView::new(Theme::default());
    let l1 = make_layout(&mut ev);
    let p1 = l1.focused();
    let mut store = TabStore::new();
    let t1 = store.current();
    store.open_after_current(l1);

    let incoming = store.take(t1);
    store.close_current(t1);

    assert_eq!(incoming.focused(), p1);
    assert_eq!(store.order(), &[t1]);
    assert_eq!(store.current(), t1);
    assert_eq!(store.inactive_layouts().count(), 0);
}

#[test]
fn next_and_prev_wrap_around() {
    let (mut ev, _) = EngineView::new(Theme::default());
    let l1 = make_layout(&mut ev);
    let l2 = make_layout(&mut ev);
    let mut store = TabStore::new();
    let t1 = store.current();
    let t2 = store.open_after_current(l1);
    store.open_after_current(l2);
    // current() == t3 here.

    assert_eq!(store.next(), t1, "wraps from the last tab to the first");
    assert_eq!(store.prev(), t2);
}

#[test]
fn next_and_prev_are_noops_with_one_tab() {
    let store = TabStore::new();
    let t1 = store.current();

    assert_eq!(store.next(), t1);
    assert_eq!(store.prev(), t1);
}

#[test]
fn adjacent_of_the_rightmost_tab_is_its_left_neighbour() {
    let (mut ev, _) = EngineView::new(Theme::default());
    let l1 = make_layout(&mut ev);
    let l2 = make_layout(&mut ev);
    let mut store = TabStore::new();
    store.open_after_current(l1);
    let t2 = store.current();
    store.open_after_current(l2);

    assert_eq!(store.adjacent(), Ok(t2));
}

#[test]
fn adjacent_of_the_leftmost_tab_is_its_right_neighbour() {
    let (mut ev, _) = EngineView::new(Theme::default());
    let l1 = make_layout(&mut ev);
    let l2 = make_layout(&mut ev);
    let mut store = TabStore::new();
    let t1 = store.current();
    let t2 = store.open_after_current(l1);
    store.take(t1);
    store.switch(l2, t1);

    assert_eq!(store.adjacent(), Ok(t2));
}

#[test]
fn adjacent_refuses_the_last_tab() {
    let store = TabStore::new();

    assert_eq!(store.adjacent(), Err(SingleTab));
}

#[test]
fn focused_panes_reads_the_active_tab_s_focus_from_the_live_layout() {
    let (mut ev, _) = EngineView::new(Theme::default());
    let l1 = make_layout(&mut ev);
    let l2 = make_layout(&mut ev);
    let [p1, p2] = [&l1, &l2].map(FocusedLayout::focused);
    let mut store = TabStore::new();
    let t1 = store.current();
    let t2 = store.open_after_current(l1);

    let panes: Vec<_> = store.focused_panes(p2).collect();

    assert_eq!(panes, vec![(t1, p1), (t2, p2)]);
}
